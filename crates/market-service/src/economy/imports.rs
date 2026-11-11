use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

use chrono::NaiveDate;
use market_core::economy::{Archive, Line, Observation, Point};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::ServiceState;

type ImportError = Box<dyn std::error::Error + Send + Sync>;
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;
const MAX_EXPANDED: u64 = 2 * 1024 * 1024 * 1024;

pub fn download_url(name: &str) -> String {
    let mut url =
        url::Url::parse("https://poe.ninja/poe1/api/data/dumps/dump").expect("static URL");
    url.query_pairs_mut().append_pair("name", name);
    url.to_string()
}

pub fn identity(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.len().to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

pub async fn import_archive(
    state: &ServiceState,
    entry: Archive,
    local: Option<PathBuf>,
    replace: bool,
) -> Result<Archive, ImportError> {
    let lease = state
        .economy_store
        .try_economy_import_lock(&entry.name)?
        .ok_or("Archive is already being imported by another process.")?;
    import_locked(state, entry, local, replace, lease).await
}

pub(super) async fn import_locked(
    state: &ServiceState,
    mut entry: Archive,
    local: Option<PathBuf>,
    replace: bool,
    _lease: File,
) -> Result<Archive, ImportError> {
    let _worker = state.economy.worker_lock.lock().await;
    if replace {
        entry.status = "queued".into();
        state
            .economy_store
            .save_economy_import(&entry, None)
            .await?;
    }
    let result = run(state, &mut entry, local).await;
    match result {
        Ok(()) => Ok(entry),
        Err(error) => {
            entry.status = "failed".into();
            entry.error = Some(error.to_string());
            entry.progress =
                "Incomplete rows are hidden. Retry to restart this archive safely.".into();
            state
                .economy_store
                .save_economy_import(&entry, None)
                .await?;
            Err(error)
        }
    }
}

async fn run(
    state: &ServiceState,
    entry: &mut Archive,
    local: Option<PathBuf>,
) -> Result<(), ImportError> {
    if let Some(saved) = state
        .economy_store
        .economy_imports()
        .await?
        .into_iter()
        .find(|saved| saved.name == entry.name && saved.status == "completed")
    {
        *entry = saved;
        return Ok(());
    }
    entry.status = "downloading".into();
    entry.rows = 0;
    entry.skipped = 0;
    entry.error = None;
    entry.start = None;
    entry.end = None;
    state.economy_store.save_economy_import(entry, None).await?;
    let mut temporary = tempfile::NamedTempFile::new()?;
    if let Some(path) = local {
        let file = File::open(path)?;
        let count = std::io::copy(&mut file.take(MAX_DOWNLOAD + 1), &mut temporary)?;
        if count > MAX_DOWNLOAD {
            return Err("Archive exceeds the 256 MiB compressed limit.".into());
        }
    } else {
        let client = reqwest::Client::builder()
            .user_agent(state.user_agent.as_ref())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(15 * 60))
            .build()?;
        let mut response = client
            .get(download_url(&entry.name))
            .send()
            .await?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_DOWNLOAD)
        {
            return Err("Archive exceeds the 256 MiB compressed limit.".into());
        }
        let mut size = 0_u64;
        while let Some(chunk) = response.chunk().await? {
            size += chunk.len() as u64;
            if size > MAX_DOWNLOAD {
                return Err("Archive exceeds the 256 MiB compressed limit.".into());
            }
            temporary.write_all(&chunk)?;
        }
    }
    temporary.flush()?;
    let mut file = temporary.reopen()?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    let checksum = format!("{:x}", hash.finalize());
    entry.status = "importing".into();
    entry.progress = "Reading softcore currency and item CSVs.".into();
    let previous_checksum = state
        .economy_store
        .economy_import_checksum(&entry.name)
        .await?;
    if previous_checksum.as_deref() != Some(&checksum) {
        state
            .economy_store
            .clear_incomplete_import(&entry.name)
            .await?;
    }
    state
        .economy_store
        .save_economy_import(entry, Some(&checksum))
        .await?;
    let file = temporary.reopen()?;
    let name = entry.name.clone();
    let (sender, mut receiver) = mpsc::channel::<Batch>(2);
    let parser = tokio::task::spawn_blocking(move || parse_zip(file, &name, sender));
    let mut reasons = BTreeMap::<String, u64>::new();
    let mut imported_league = None;
    let mut save_error = None;
    while let Some(batch) = receiver.recv().await {
        if let Some(existing) = &imported_league {
            if existing != &batch.league {
                save_error = Some(
                    "Archive contains more than one selected softcore challenge league.".to_owned(),
                );
                break;
            }
        }
        imported_league = Some(batch.league.clone());
        for observation in &batch.observations {
            let date = &observation.point.date;
            if entry.start.as_ref().is_none_or(|start| date < start) {
                entry.start = Some(date.clone());
            }
            if entry.end.as_ref().is_none_or(|end| date > end) {
                entry.end = Some(date.clone());
            }
        }
        if let Err(error) = state
            .economy_store
            .save_observations(
                &batch.league,
                "stash",
                &batch.observations,
                Some(&entry.name),
            )
            .await
        {
            save_error = Some(error.to_string());
            break;
        }
        entry.rows += batch.observations.len() as u64;
        for (reason, count) in batch.reasons {
            entry.skipped += count;
            *reasons.entry(reason).or_default() += count;
        }
        entry.progress = format!(
            "{} observations imported; {} rows excluded.",
            entry.rows, entry.skipped
        );
        state.economy_store.save_economy_import(entry, None).await?;
    }
    drop(receiver);
    let parsed = parser.await?;
    if let Some(error) = save_error {
        return Err(error.into());
    }
    parsed?;
    if entry.rows == 0 {
        return Err("No valid softcore challenge observations were found in this archive.".into());
    }
    entry.league = imported_league;
    entry.status = "completed".into();
    entry.progress = if reasons.is_empty() {
        "Import complete. Daily stash prices; no intraday volume or listing counts.".into()
    } else {
        format!(
            "Import complete. Exclusions: {}",
            reasons
                .iter()
                .map(|(r, n)| format!("{r}: {n}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    entry.license =
        "poe.ninja economy data dumps, CC BY-SA; source: https://poe.ninja/poe1/data".into();
    state
        .economy_store
        .save_economy_import(entry, Some(&checksum))
        .await?;
    Ok(())
}

struct Batch {
    league: String,
    observations: Vec<Observation>,
    reasons: BTreeMap<String, u64>,
}

fn parse_zip(file: File, league: &str, sender: mpsc::Sender<Batch>) -> Result<(), ImportError> {
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() > 256 {
        return Err("Archive has too many entries.".into());
    }
    let mut expanded = 0_u64;
    let mut selected = 0;
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        if file.enclosed_name().is_none() {
            return Err("Unsafe archive entry path.".into());
        }
        expanded = expanded
            .checked_add(file.size())
            .ok_or("Archive size overflow.")?;
        if expanded > MAX_EXPANDED {
            return Err("Archive exceeds the 2 GiB expanded limit.".into());
        }
        let name = file.name().to_owned();
        let kind = if name == format!("{league}.currency.csv") {
            "currency"
        } else if name == format!("{league}.items.csv") {
            "items"
        } else {
            continue;
        };
        selected += 1;
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(b';')
            .trim(csv::Trim::All)
            .from_reader(file);
        let headers = reader.headers()?.clone();
        for required in if kind == "currency" {
            &["League", "Date", "Get", "Pay", "Value"][..]
        } else {
            &[
                "League", "Date", "Id", "Type", "Name", "BaseType", "Variant", "Links", "Value",
            ][..]
        } {
            if !headers
                .iter()
                .any(|h| h.trim_start_matches('\u{feff}') == *required)
            {
                return Err(format!("Missing CSV column {required} in {name}.").into());
            }
        }
        let mut batch = Batch {
            league: league.into(),
            observations: Vec::with_capacity(10000),
            reasons: BTreeMap::new(),
        };
        for record in reader.records() {
            let record = record?;
            let get = |key: &str| {
                headers
                    .iter()
                    .position(|h| h.trim_start_matches('\u{feff}') == key)
                    .and_then(|i| record.get(i))
                    .unwrap_or("")
            };
            match observation(&get, kind, league) {
                Ok(observation) => batch.observations.push(observation),
                Err(reason) => *batch.reasons.entry(reason).or_default() += 1,
            }
            if batch.observations.len() >= 10000 || batch.reasons.values().sum::<u64>() >= 10000 {
                sender
                    .blocking_send(batch)
                    .map_err(|_| "Import receiver stopped.")?;
                batch = Batch {
                    league: league.into(),
                    observations: Vec::with_capacity(10000),
                    reasons: BTreeMap::new(),
                };
            }
        }
        if !batch.observations.is_empty() || !batch.reasons.is_empty() {
            sender
                .blocking_send(batch)
                .map_err(|_| "Import receiver stopped.")?;
        }
    }
    if selected != 2 {
        return Err(format!(
            "Expected {league}.currency.csv and {league}.items.csv in the archive."
        )
        .into());
    }
    Ok(())
}

fn observation<'a>(
    get: &impl Fn(&str) -> &'a str,
    kind: &str,
    league: &str,
) -> Result<Observation, String> {
    if get("League") != league {
        return Err("non-selected league".into());
    }
    let date = get("Date");
    if date.len() != 10 || NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
        return Err("invalid date".into());
    }
    let price = Decimal::from_str(get("Value")).map_err(|_| "invalid decimal price")?;
    if price < Decimal::ZERO {
        return Err("negative price".into());
    }
    let mut line = Line::default();
    let category;
    if kind == "currency" {
        if get("Pay") != "Chaos Orb" {
            return Err("non-Chaos quote (not converted)".into());
        }
        line.name = get("Get").into();
        line.id = format!("archive:{}", identity(&[get("Get"), get("Pay")]));
        category = "Currency".to_owned();
    } else {
        category = get("Type").to_owned();
        if category.is_empty()
            || category.len() > 64
            || !category.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err("invalid category".into());
        }
        line.name = get("Name").into();
        line.id = format!(
            "archive:{}",
            identity(&[
                get("Id"),
                get("Name"),
                get("BaseType"),
                get("Variant"),
                get("Links")
            ])
        );
        line.base_type = nonempty(get("BaseType"));
        line.variant = nonempty(get("Variant"));
        let links = get("Links").trim_end_matches(" links");
        line.links = if links.is_empty() || links == "1-4" {
            None
        } else {
            let links = links.parse::<u64>().map_err(|_| "invalid links")?;
            if !(1..=6).contains(&links) {
                return Err("invalid links".into());
            }
            Some(links)
        };
        if !get("Links").is_empty() {
            line.metadata
                .insert("archive_links".into(), get("Links").into());
            if links == "1-4" {
                line.variant = Some(
                    [get("Variant"), get("Links")]
                        .into_iter()
                        .filter(|v| !v.is_empty())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
        }
        if matches!(category.as_str(), "SkillGem" | "ImbuedGem") {
            let variant = get("Variant");
            let corrupted = variant.ends_with('c');
            if let Some((level, quality)) = variant.trim_end_matches('c').split_once('/') {
                line.gem_level = level.parse().ok();
                line.gem_quality = quality.parse().ok();
                line.corrupted = Some(corrupted);
            }
        }
        line.metadata
            .insert("archive_item_id".into(), get("Id").into());
    }
    if line.name.is_empty() {
        return Err("missing item name".into());
    }
    line.metadata.insert(
        "data_source".into(),
        "poe.ninja daily dump (CC BY-SA)".into(),
    );
    Ok(Observation {
        category,
        line,
        point: Point {
            date: date.into(),
            price: price.normalize().to_string(),
            currency: "Chaos Orb".into(),
            granularity: "daily".into(),
        },
    })
}

fn nonempty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_entry() -> Archive {
        Archive {
            name: "TestLeague".into(),
            status: "available".into(),
            rows: 0,
            skipped: 0,
            error: None,
            start: None,
            end: None,
            league: None,
            progress: String::new(),
            license: "CC BY-SA".into(),
            url: download_url("TestLeague"),
        }
    }

    fn fixture(path: &std::path::Path, unsafe_entry: bool) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in [
            (
                "TestLeague.currency.csv",
                "League;Date;Get;Pay;Value;Confidence\nTestLeague;2026-01-01;Divine Orb;Chaos Orb;200;High\n",
            ),
            (
                "TestLeague.items.csv",
                "League;Date;Id;Type;Name;BaseType;Variant;Links;Value;Confidence\nTestLeague;2026-01-01;42;UniqueWeapon;\"A; B\";Axe;;6 links;40;High\nTestLeague;2026-01-02;42;UniqueWeapon;\"A; B\";Axe;;6 links;50;Low\nHardcore TestLeague;2026-01-02;42;UniqueWeapon;Other;Axe;;6 links;60;High\nTestLeague;2026-01-03;43;UniqueWeapon;Bad;Axe;;;NaN;High\n",
            ),
            ("Hardcore TestLeague.items.csv", "not a selected CSV"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        if unsafe_entry {
            zip.start_file("../unsafe.csv", options).unwrap();
            zip.write_all(b"unsafe").unwrap();
        }
        zip.finish().unwrap();
    }

    #[tokio::test]
    async fn local_zip_import_is_scoped_idempotent_queryable_and_persistent() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("economy.sqlite3");
        let file = directory.path().join("archive.zip");
        fixture(&file, false);
        let store = market_store::SqliteStore::connect(&database).await.unwrap();
        let (selection, _) = tokio::sync::watch::channel(None);
        let state = ServiceState::new(store.clone(), "test", selection);
        let imported = import_archive(&state, archive_entry(), Some(file.clone()), false)
            .await
            .unwrap();
        assert_eq!(imported.status, "completed");
        assert_eq!(imported.rows, 3);
        assert_eq!(imported.skipped, 2);
        assert_eq!(imported.league.as_deref(), Some("TestLeague"));
        let overview = store
            .historical_economy("TestLeague", "UniqueWeapon", "stash", "2026-01-01")
            .await
            .unwrap();
        assert_eq!(overview.lines.len(), 1);
        assert_eq!(overview.lines[0].name, "A; B");
        assert_eq!(overview.lines[0].links, Some(6));
        assert_eq!(
            overview.rates.get("Divine Orb").map(String::as_str),
            Some("200")
        );
        let id = &overview.lines[0].id;
        let history = store
            .economy_history("TestLeague", "UniqueWeapon", "stash", id, Some(7))
            .await
            .unwrap();
        assert_eq!(history.points.len(), 2);
        assert_eq!(history.points[0].price, "40");
        assert!(
            store
                .economy_dates("Hardcore TestLeague", "UniqueWeapon", "stash")
                .await
                .unwrap()
                .is_empty()
        );
        let repeated = import_archive(&state, archive_entry(), Some(file), false)
            .await
            .unwrap();
        assert_eq!(repeated.rows, imported.rows);
        let reopened = market_store::SqliteStore::connect(&database).await.unwrap();
        assert_eq!(
            reopened
                .economy_history("TestLeague", "UniqueWeapon", "stash", id, None)
                .await
                .unwrap()
                .points
                .len(),
            2
        );
        assert_eq!(
            reopened.economy_imports().await.unwrap()[0].status,
            "completed"
        );
        use tower::ServiceExt;
        let uri = format!(
            "/api/v1/economy/history?league=TestLeague&category=UniqueWeapon&source=stash&item={id}&days=7"
        );
        let response = crate::router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn unsafe_or_incomplete_imports_are_failed_and_hidden_then_retryable() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("economy.sqlite3"))
            .await
            .unwrap();
        let (selection, _) = tokio::sync::watch::channel(None);
        let state = ServiceState::new(store.clone(), "test", selection);
        let file = directory.path().join("archive.zip");
        fixture(&file, true);
        assert!(
            import_archive(&state, archive_entry(), Some(file.clone()), false)
                .await
                .is_err()
        );
        assert_eq!(store.economy_imports().await.unwrap()[0].status, "failed");
        assert!(store.stored_economy_categories().await.unwrap().is_empty());
        assert!(
            store
                .economy_dates("TestLeague", "UniqueWeapon", "stash")
                .await
                .unwrap()
                .is_empty()
        );
        fixture(&file, false);
        assert_eq!(
            import_archive(&state, archive_entry(), Some(file), false)
                .await
                .unwrap()
                .status,
            "completed"
        );
        assert!(
            store
                .stored_economy_categories()
                .await
                .unwrap()
                .contains(&"UniqueWeapon".to_owned())
        );
        let mut interrupted = archive_entry();
        interrupted.status = "importing".into();
        store.save_economy_import(&interrupted, None).await.unwrap();
        let lease = store
            .try_economy_import_lock("TestLeague")
            .unwrap()
            .unwrap();
        let other_store =
            market_store::SqliteStore::connect(directory.path().join("economy.sqlite3"))
                .await
                .unwrap();
        assert!(
            other_store
                .try_economy_import_lock("TestLeague")
                .unwrap()
                .is_none()
        );
        assert!(
            other_store
                .try_economy_import_lock("OtherLeague")
                .unwrap()
                .is_some()
        );
        super::super::recover_imports(&state).await.unwrap();
        assert_eq!(
            store.economy_imports().await.unwrap()[0].status,
            "importing"
        );
        assert!(
            import_archive(&state, interrupted, None, true)
                .await
                .is_err()
        );
        assert_eq!(
            store.economy_imports().await.unwrap()[0].status,
            "importing"
        );
        drop(lease);
        super::super::recover_imports(&state).await.unwrap();
        assert_eq!(store.economy_imports().await.unwrap()[0].status, "failed");
    }

    #[test]
    fn archive_rows_preserve_variants_and_reject_wrong_leagues_and_currency_direction() {
        let mut fields = BTreeMap::from([
            ("League", "Ancestors"),
            ("Date", "2026-06-25"),
            ("Id", "42"),
            ("Type", "SkillGem"),
            ("Name", "Fireball"),
            ("BaseType", "Fireball"),
            ("Variant", "21/20c"),
            ("Links", ""),
            ("Value", "12.123456789"),
            ("Get", "Divine Orb"),
            ("Pay", "Chaos Orb"),
        ]);
        let row = observation(
            &|key| fields.get(key).copied().unwrap_or(""),
            "items",
            "Ancestors",
        )
        .unwrap();
        assert_eq!(row.line.gem_level, Some(21));
        assert_eq!(row.line.corrupted, Some(true));
        assert_eq!(row.point.price, "12.123456789");
        fields.insert("Variant", "20/20");
        let other = observation(
            &|key| fields.get(key).copied().unwrap_or(""),
            "items",
            "Ancestors",
        )
        .unwrap();
        assert_ne!(row.line.id, other.line.id);
        assert!(
            observation(
                &|key| fields.get(key).copied().unwrap_or(""),
                "items",
                "Hardcore Ancestors"
            )
            .is_err()
        );
        fields.insert("Value", "NaN");
        assert!(
            observation(
                &|key| fields.get(key).copied().unwrap_or(""),
                "items",
                "Ancestors"
            )
            .is_err()
        );
        fields.insert("Value", "1");
        fields.insert("Pay", "Divine Orb");
        assert!(
            observation(
                &|key| fields.get(key).copied().unwrap_or(""),
                "currency",
                "Ancestors"
            )
            .is_err()
        );
    }
}
