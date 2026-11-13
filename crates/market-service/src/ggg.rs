use std::{collections::HashMap, time::Duration};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::Utc;
use faust_fetch::ExchangePage;
use market_core::ggg::{self as dto, HOUR, ImportJob, Item};
use serde::Deserialize;
use tokio::sync::{Mutex, watch};
use tokio::time::Instant;

use crate::ServiceState;

const METADATA_URL: &str = "https://repoe-fork.github.io/base_items.json";
const EXCHANGE_URL: &str = "https://web.poecdn.com/api/currency-exchange";
static IMPORT_LOCK: Mutex<()> = Mutex::const_new(());
static NEXT_REQUEST: Mutex<Option<Instant>> = Mutex::const_new(None);
type ApiError = (StatusCode, String);

#[derive(Default)]
pub struct GggState {
    jobs: Mutex<HashMap<String, Job>>,
    metadata_checked: Mutex<Option<Instant>>,
}

struct Job {
    dto: ImportJob,
    cancel: watch::Sender<bool>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct Range {
    pub league: String,
    pub start: i64,
    pub end: i64,
}

#[derive(Deserialize)]
struct League {
    league: String,
}

#[derive(Deserialize)]
struct Hour {
    league: String,
    hour: i64,
}

#[derive(Deserialize)]
struct HistoryQuery {
    league: String,
    market: String,
    start: i64,
    end: i64,
}

pub fn routes() -> Router<ServiceState> {
    Router::new()
        .route("/api/v1/ggg/markets", get(markets))
        .route("/api/v1/ggg/coverage", get(coverage))
        .route("/api/v1/ggg/history", get(history))
        .route("/api/v1/ggg/import", get(import_status).post(start_import))
        .route("/api/v1/ggg/cancel", post(cancel))
}

fn bad(error: impl ToString) -> ApiError {
    (StatusCode::BAD_REQUEST, error.to_string())
}

fn failure(error: impl ToString) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn validate(league: &str, start: i64, end: i64) -> Result<(), ApiError> {
    dto::validate_range(league, start, end, Utc::now().timestamp()).map_err(bad)
}

async fn markets(
    State(state): State<ServiceState>,
    Query(query): Query<Hour>,
) -> Result<Json<dto::Overview>, ApiError> {
    validate(&query.league, query.hour, query.hour)?;
    let complete = state
        .store
        .ggg_hour_complete(&query.league, query.hour)
        .await
        .map_err(failure)?;
    let markets = state
        .store
        .ggg_markets(&query.league, query.hour)
        .await
        .map_err(failure)?;
    Ok(Json(dto::Overview {
        league: query.league,
        hour: query.hour,
        complete,
        markets,
        platform: "PC".into(),
    }))
}

async fn coverage(
    State(state): State<ServiceState>,
    Query(query): Query<Range>,
) -> Result<Json<dto::Coverage>, ApiError> {
    validate(&query.league, query.start, query.end)?;
    state
        .store
        .ggg_coverage(&query.league, query.start, query.end)
        .await
        .map(Json)
        .map_err(failure)
}

async fn history(
    State(state): State<ServiceState>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<dto::History>, ApiError> {
    validate(&query.league, query.start, query.end)?;
    if query.market.is_empty()
        || query.market.len() > 4096
        || query.market.chars().any(char::is_control)
    {
        return Err(bad("Invalid market ID."));
    }
    state
        .store
        .ggg_history(&query.league, &query.market, query.start, query.end)
        .await
        .map(Json)
        .map_err(failure)
}

async fn import_status(
    State(state): State<ServiceState>,
    Query(query): Query<League>,
) -> Result<Json<Option<ImportJob>>, ApiError> {
    dto::validate_league(&query.league).map_err(bad)?;
    Ok(Json(
        state
            .ggg
            .jobs
            .lock()
            .await
            .get(&query.league)
            .map(|job| job.dto.clone()),
    ))
}

async fn cancel(
    State(state): State<ServiceState>,
    Json(query): Json<League>,
) -> Result<Json<Option<ImportJob>>, ApiError> {
    dto::validate_league(&query.league).map_err(bad)?;
    let mut jobs = state.ggg.jobs.lock().await;
    let job = jobs.get_mut(&query.league);
    if let Some(job) = job {
        if job.dto.status == "running" {
            job.cancel.send_replace(true);
            job.dto.status = "cancelled".into();
        }
        return Ok(Json(Some(job.dto.clone())));
    }
    Ok(Json(None))
}

async fn start_import(
    State(state): State<ServiceState>,
    Json(query): Json<Range>,
) -> Result<Json<ImportJob>, ApiError> {
    validate(&query.league, query.start, query.end)?;
    start_job(state, query).await.map(Json)
}

fn range_chunks(range: &Range) -> impl Iterator<Item = (i64, i64)> + '_ {
    (range.start..=range.end)
        .step_by((dto::MAX_HOURS * HOUR) as usize)
        .map(|start| (start, (start + (dto::MAX_HOURS - 1) * HOUR).min(range.end)))
}

async fn import_coverage(state: &ServiceState, range: &Range) -> Result<dto::Coverage, ApiError> {
    let mut result = dto::Coverage {
        league: range.league.clone(),
        start: range.start,
        end: range.end,
        expected_hours: 0,
        completed_hours: vec![],
        failed_hours: vec![],
    };
    for (start, end) in range_chunks(range) {
        let coverage = state
            .store
            .ggg_coverage(&range.league, start, end)
            .await
            .map_err(failure)?;
        result.expected_hours += coverage.expected_hours;
        result.completed_hours.extend(coverage.completed_hours);
        result.failed_hours.extend(coverage.failed_hours);
    }
    Ok(result)
}

pub(crate) async fn archive_status(
    state: &ServiceState,
    range: &Range,
) -> Result<ImportJob, ApiError> {
    let jobs = state.ggg.jobs.lock().await;
    let current = jobs.get(&range.league).map(|job| job.dto.clone());
    drop(jobs);
    if let Some(job) = &current
        && job.start == range.start
        && job.end == range.end
    {
        return Ok(job.clone());
    }
    let coverage = import_coverage(state, range).await?;
    let completed = coverage.completed_hours.len() as u64;
    Ok(ImportJob {
        league: range.league.clone(),
        start: range.start,
        end: range.end,
        status: if current.as_ref().is_some_and(|job| job.status == "running") {
            "running"
        } else if completed == coverage.expected_hours {
            "completed"
        } else if !coverage.failed_hours.is_empty() {
            "failed"
        } else if completed > 0 {
            "partial"
        } else {
            "available"
        }.into(),
        completed_hours: completed,
        total_hours: coverage.expected_hours,
        error: coverage.failed_hours.first().map(|failure| format!("Hour {}: {}", failure.hour, failure.error)),
        warning: current.filter(|job| job.status == "running")
            .map(|_| "Another GGG range is running for this league; progress here measures full dataset coverage. Wait for it to finish, then retry any remaining hours, or cancel it on Currency Exchange.".into()),
        platform: "PC".into(),
    })
}

pub(crate) async fn start_archive_import(
    state: ServiceState,
    query: Range,
) -> Result<ImportJob, ApiError> {
    dto::validate_league(&query.league).map_err(bad)?;
    if query.end < query.start {
        return Err(bad("Dataset range has no completed hours."));
    }
    for (start, end) in range_chunks(&query) {
        validate(&query.league, start, end)?;
    }
    start_job(state, query).await
}

async fn start_job(state: ServiceState, query: Range) -> Result<ImportJob, ApiError> {
    if let Some(job) = state.ggg.jobs.lock().await.get(&query.league)
        && job.dto.status == "running"
        && job.dto.start == query.start
        && job.dto.end == query.end
    {
        return Ok(job.dto.clone());
    }
    let permit = IMPORT_LOCK.try_lock().map_err(|_| {
        (
            StatusCode::CONFLICT,
            "Another PC GGG import is active; cancel it or wait for completion.".into(),
        )
    })?;
    let coverage = import_coverage(&state, &query).await?;
    let (sender, mut receiver) = watch::channel(false);
    let job = ImportJob {
        league: query.league.clone(),
        start: query.start,
        end: query.end,
        status: "running".into(),
        completed_hours: coverage.completed_hours.len() as u64,
        total_hours: coverage.expected_hours,
        error: None,
        warning: None,
        platform: "PC".into(),
    };
    state.ggg.jobs.lock().await.insert(
        query.league.clone(),
        Job {
            dto: job.clone(),
            cancel: sender,
        },
    );
    tokio::spawn(async move {
        let _permit = permit;
        let result = tokio::select! {
            biased;
            _=receiver.changed()=>None,
            result=run_import(&state,&query,EXCHANGE_URL)=>Some(result),
        };
        let coverage = import_coverage(&state, &query).await;
        let mut metadata_warning = None;
        for (start, end) in range_chunks(&query) {
            match state
                .store
                .ggg_metadata_summary(&query.league, start, end)
                .await
            {
                Ok((total, unknown, uncategorized)) if unknown > 0 || uncategorized > 0 => {
                    metadata_warning = Some(format!(
                        "Partial item metadata in hours {start}..{end}: metadata_unknown={unknown}/{total}; category_unknown={uncategorized}/{total}. Raw identities are retained; no name-based price matching."
                    ));
                    break;
                }
                Err(error) => {
                    metadata_warning = Some(format!("Unable to summarize item metadata: {error}"));
                    break;
                }
                _ => {}
            }
        }
        let mut jobs = state.ggg.jobs.lock().await;
        if let Some(job) = jobs.get_mut(&query.league) {
            match coverage {
                Ok(coverage) => job.dto.completed_hours = coverage.completed_hours.len() as u64,
                Err(error) => {
                    job.dto.status = "failed".into();
                    job.dto.error = Some(format!("Unable to read final GGG coverage: {}", error.1));
                    eprintln!("GGG import: {}", error.1);
                    return;
                }
            }
            if let Some(summary) = metadata_warning {
                job.dto.warning = Some(match job.dto.warning.take() {
                    Some(warning) => format!("{warning} {summary}"),
                    None => summary,
                });
            }
            if job.dto.status != "cancelled" {
                match result {
                    None => job.dto.status = "cancelled".into(),
                    Some(Ok(())) => job.dto.status = "completed".into(),
                    Some(Err(error)) => {
                        job.dto.status = "failed".into();
                        job.dto.error = Some(error);
                    }
                }
            }
        }
    });
    Ok(job)
}

async fn run_import(state: &ServiceState, range: &Range, exchange_url: &str) -> Result<(), String> {
    let coverage = import_coverage(state, range)
        .await
        .map_err(|error| error.1)?;
    let client = reqwest::Client::builder()
        .user_agent(format!(
            "{} GGG-PC-hourly-import/RePoE-metadata",
            state.user_agent
        ))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|e| e.to_string())?;
    if let Err(error) = load_metadata(state, &client).await
        && let Some(job) = state.ggg.jobs.lock().await.get_mut(&range.league)
    {
        job.dto.warning = Some(format!(
            "Item metadata unavailable: {error}. Cached names or raw metadata IDs retained."
        ));
    }
    if coverage.completed_hours.len() as u64 == coverage.expected_hours {
        return Ok(());
    }
    for hour in (range.start..=range.end).step_by(HOUR as usize) {
        if state
            .store
            .ggg_hour_complete(&range.league, hour)
            .await
            .map_err(|e| e.to_string())?
        {
            continue;
        }
        if let Some(deadline) = *NEXT_REQUEST.lock().await {
            tokio::time::sleep_until(deadline).await;
        }
        let page = fetch_hour(&client, hour, exchange_url).await;
        let result = match page {
            Ok(page) => state
                .store
                .record_ggg_hour(&range.league, hour, &page)
                .await
                .map_err(|e| e.to_string()),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            state
                .store
                .record_ggg_failure(&range.league, hour, &error)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(job) = state.ggg.jobs.lock().await.get_mut(&range.league) {
                job.dto.error = Some(format!("Hour {hour}: {error}"));
            }
        } else if let Some(job) = state.ggg.jobs.lock().await.get_mut(&range.league) {
            job.dto.completed_hours += 1;
        }
    }
    let coverage = import_coverage(state, range)
        .await
        .map_err(|error| error.1)?;
    if !coverage.failed_hours.is_empty() {
        return Err(format!(
            "{} hourly requests failed; start the range again to resume. {}",
            coverage.failed_hours.len(),
            coverage.failed_hours[0].error
        ));
    }
    if let Some(job) = state.ggg.jobs.lock().await.get_mut(&range.league) {
        job.dto.error = None;
    }
    Ok(())
}

fn retry_delay(headers: &HeaderMap) -> Duration {
    let delay = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.parse::<u64>().ok().or_else(|| {
                chrono::DateTime::parse_from_rfc2822(v)
                    .ok()
                    .map(|date| (date.timestamp() - Utc::now().timestamp()).max(0) as u64)
            })
        })
        .unwrap_or(1);
    Duration::from_secs(delay.max(1))
}

async fn fetch_hour(
    client: &reqwest::Client,
    hour: i64,
    exchange_url: &str,
) -> Result<ExchangePage, String> {
    *NEXT_REQUEST.lock().await = Some(Instant::now() + Duration::from_secs(1));
    let response = client
        .get(format!("{exchange_url}/{hour}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let delay = retry_delay(response.headers());
    *NEXT_REQUEST.lock().await = Some(
        Instant::now()
            .checked_add(delay)
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(100 * 365 * 86400)),
    );
    if !response.status().is_success() {
        return Err(format!(
            "GGG PC exchange returned HTTP {}",
            response.status()
        ));
    }
    response
        .json::<ExchangePage>()
        .await
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct BaseItem {
    name: String,
    item_class: String,
    #[serde(default)]
    tags: Vec<String>,
}

fn category(id: &str, class: &str, tags: &[String]) -> &'static str {
    if id.starts_with("Metadata/Items/Scarabs/") && class == "MapFragment" {
        return "Scarab";
    }
    if class == "StackableCurrency" && tags.iter().any(|tag| tag == "mushrune") {
        return "Oil";
    }
    if id.starts_with("Metadata/Items/Currency/CurrencyDelveCrafting")
        && class == "StackableCurrency"
    {
        return "Fossil";
    }
    if id.starts_with("Metadata/Items/Currency/CurrencyEssence") && class == "StackableCurrency" {
        return "Essence";
    }
    if class == "MapFragment"
        && matches!(
            id,
            "Metadata/Items/MapFragments/RatsAllflamePack"
                | "Metadata/Items/MapFragments/BetrayalAllflamePack"
                | "Metadata/Items/MapFragments/ResplendenceAllflamePack"
                | "Metadata/Items/MapFragments/PropagationAllflamePack"
                | "Metadata/Items/MapFragments/MeatSackAllflamePack"
                | "Metadata/Items/MapFragments/WildwoodAllflamePack"
                | "Metadata/Items/MapFragments/CleansedAllflamePack"
                | "Metadata/Items/MapFragments/GoldAllflamePack"
                | "Metadata/Items/MapFragments/ToadAllflamePack"
        )
    {
        return "AllflameEmber";
    }
    if class == "MapFragment" && id.starts_with("Metadata/Items/MapFragments/AtlasMemory/") {
        return "Memory";
    }
    if class == "StackableCurrency" {
        if tags.iter().any(|tag| tag == "essence") {
            return "Essence";
        }
        if tags
            .iter()
            .any(|tag| matches!(tag.as_str(), "int_tattoo" | "str_tattoo" | "dex_tattoo"))
        {
            return "Tattoo";
        }
        if tags.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "int_runegraft" | "str_runegraft" | "dex_runegraft"
            )
        }) {
            return "Runegraft";
        }
        if id.starts_with("Metadata/Items/Currency/AncestralOmen") {
            return "Omen";
        }
        if id.starts_with("Metadata/Items/Currency/Astrolabe") {
            return "Astrolabe";
        }
        if id.starts_with("Metadata/Items/Currency/LegionCocoon") {
            return "EnshroudingCrystal";
        }
        if id.starts_with("Metadata/Items/Deepwater/Deepwater") {
            return "Ducat";
        }
        if tags.iter().any(|tag| tag == "affliction_orb") {
            return "DeliriumOrb";
        }
        if tags.iter().any(|tag| {
            matches!(
                tag.as_str(),
                "expedition_currency_faction1"
                    | "expedition_currency_faction2"
                    | "expedition_currency_faction3"
                    | "expedition_currency_faction4"
            )
        }) {
            return "Artifact";
        }
        return "Currency";
    }
    match class {
        "DivinationCard" => "DivinationCard",
        "MapFragment" => "Fragment",
        "IncubatorStackable" => "Incubator",
        "NecropolisPack" => "AllflameEmber",
        "DelveSocketableCurrency" | "DelveStackableSocketableCurrency" => "Resonator",
        _ => "Unknown",
    }
}

fn fallback_items() -> Vec<Item> {
    [
        ("CurrencyRerollRare", "Chaos Orb"),
        ("CurrencyModValues", "Divine Orb"),
        ("CurrencyAddModToRare", "Exalted Orb"),
        ("CurrencyMirror", "Mirror of Kalandra"),
        ("CurrencyPortal", "Portal Scroll"),
        ("CurrencyIdentification", "Scroll of Wisdom"),
        ("CurrencyAddModToMagic", "Orb of Augmentation"),
        ("CurrencyConvertToNormal", "Orb of Scouring"),
        ("CurrencyCorrupt", "Vaal Orb"),
        ("CurrencyGemQuality", "Gemcutter's Prism"),
        ("CurrencyMapQuality", "Cartographer's Chisel"),
        ("CurrencyPassiveRefund", "Orb of Regret"),
        ("CurrencyRemoveMod", "Orb of Annulment"),
        ("CurrencyRerollImplicit", "Blessed Orb"),
        ("CurrencyRerollMagic", "Orb of Alteration"),
        ("CurrencyRerollMapType", "Orb of Horizons"),
        ("CurrencyRerollSocketColours", "Chromatic Orb"),
        ("CurrencyRerollSocketLinks", "Orb of Fusing"),
        ("CurrencyRerollSocketNumbers", "Jeweller's Orb"),
        ("CurrencyRerollUnique", "Ancient Orb"),
        ("CurrencySilverCoin", "Silver Coin"),
        ("CurrencyUpgradeMagicToRare", "Regal Orb"),
        ("CurrencyUpgradeMapTier", "Harbinger's Orb"),
        ("CurrencyUpgradeRandomly", "Orb of Chance"),
        ("CurrencyUpgradeToMagic", "Orb of Transmutation"),
        ("CurrencyUpgradeToRare", "Orb of Alchemy"),
    ]
    .into_iter()
    .map(|(id, name)| Item {
        id: format!("Metadata/Items/Currency/{id}"),
        name: name.into(),
        category: "Currency".into(),
        icon: None,
    })
    .collect()
}

async fn load_metadata(state: &ServiceState, client: &reqwest::Client) -> Result<(), String> {
    let store = &state.store;
    let now = Utc::now().timestamp();
    if state
        .ggg
        .metadata_checked
        .lock()
        .await
        .is_some_and(|checked| checked.elapsed() < Duration::from_secs(7 * 86400))
    {
        return Ok(());
    }
    // Exact, verified metadata identities are usable even if the public catalog is offline.
    // Saving fallback entries does not mark the remote catalog as refreshed.
    store
        .save_ggg_fallback_items(&fallback_items())
        .await
        .map_err(|e| e.to_string())?;
    let bases: HashMap<String, BaseItem> = client
        .get(METADATA_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if bases.is_empty() {
        return Err("RePoE returned an empty catalog".into());
    }
    let mut items: Vec<_> = bases
        .into_iter()
        .map(|(id, base)| {
            let category = category(&id, &base.item_class, &base.tags).into();
            Item {
                id,
                name: base.name,
                category,
                icon: None,
            }
        })
        .collect();
    items.extend(fallback_items());
    store
        .save_ggg_catalog(&items, now)
        .await
        .map_err(|e| e.to_string())?;
    *state.ggg.metadata_checked.lock().await = Some(Instant::now());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[tokio::test]
    async fn archive_coverage_spans_long_ranges_and_survives_service_restart() {
        let directory = tempfile::tempdir().unwrap();
        let store =
            market_store::SqliteStore::connect(directory.path().join("archive-coverage.sqlite3"))
                .await
                .unwrap();
        let boundary = dto::MAX_HOURS * HOUR;
        let range = Range {
            league: "Test".into(),
            start: 0,
            end: boundary + HOUR,
        };
        assert_eq!(
            range_chunks(&range).collect::<Vec<_>>(),
            vec![(0, boundary - HOUR), (boundary, boundary + HOUR)]
        );
        for hour in [0, boundary - HOUR, boundary] {
            store
                .record_ggg_hour(
                    "Test",
                    hour,
                    &ExchangePage {
                        next_change_id: faust_fetch::ExchangeCursor::new((hour + HOUR) as u64),
                        markets: vec![],
                    },
                )
                .await
                .unwrap();
        }
        let (selection, _) = watch::channel(None);
        let state = ServiceState::new(store.clone(), "test", selection);
        let job = archive_status(&state, &range).await.unwrap();
        assert_eq!(job.status, "partial");
        assert_eq!(job.completed_hours, 3);
        assert_eq!(job.total_hours, dto::MAX_HOURS as u64 + 2);
        store
            .record_ggg_failure("Test", boundary + HOUR, "HTTP 503")
            .await
            .unwrap();
        let (selection, _) = watch::channel(None);
        let restarted = ServiceState::new(store, "test", selection);
        let job = archive_status(&restarted, &range).await.unwrap();
        assert_eq!(job.status, "failed");
        assert_eq!(job.completed_hours, 3);
        assert!(job.error.unwrap().contains("HTTP 503"));
        let empty = Range {
            league: "Not imported".into(),
            start: 0,
            end: HOUR,
        };
        assert_eq!(
            archive_status(&restarted, &empty).await.unwrap().status,
            "available"
        );
        let saved = Range {
            league: "Test".into(),
            start: boundary - HOUR,
            end: boundary,
        };
        let saved = archive_status(&restarted, &saved).await.unwrap();
        assert_eq!(saved.status, "completed");
        assert_eq!(saved.completed_hours, 2);
    }

    #[test]
    fn metadata_mapping_is_exact_and_conservative() {
        assert_eq!(
            category(
                "Metadata/Items/Currency/Mushrune1",
                "StackableCurrency",
                &["mushrune".into()]
            ),
            "Oil"
        );
        assert_eq!(
            category(
                "Metadata/Items/Currency/MushruneCorrupt",
                "StackableCurrency",
                &["mushrune".into()]
            ),
            "Oil"
        );
        assert_eq!(
            category("Metadata/Items/Scarabs/ScarabBreach1", "MapFragment", &[]),
            "Scarab"
        );
        assert_eq!(
            category("Metadata/Items/Currency/Unknown", "UnverifiedClass", &[]),
            "Unknown"
        );
        assert_eq!(
            category(
                "Metadata/Items/Currency/RunegraftAttributeGlobalDefences",
                "StackableCurrency",
                &["str_runegraft".into()]
            ),
            "Runegraft"
        );
        assert_eq!(
            category(
                "Metadata/Items/MapFragments/BetrayalAllflamePack",
                "MapFragment",
                &["default".into()]
            ),
            "AllflameEmber"
        );
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "5".parse().unwrap());
        assert_eq!(retry_delay(&headers), Duration::from_secs(5));
        headers.insert(
            "retry-after",
            "Thu, 01 Jan 2099 00:00:00 GMT".parse().unwrap(),
        );
        assert!(retry_delay(&headers) > Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancellation_status_and_read_only_missing_hour() {
        let store =
            market_store::SqliteStore::connect("file:ggg-api-test?mode=memory&cache=shared")
                .await
                .unwrap();
        let (selection, _) = watch::channel(None);
        let state = ServiceState::new(store, "test", selection);
        let (sender, _) = watch::channel(false);
        state.ggg.jobs.lock().await.insert(
            "Test".into(),
            Job {
                cancel: sender,
                dto: ImportJob {
                    league: "Test".into(),
                    start: 0,
                    end: 0,
                    status: "running".into(),
                    completed_hours: 0,
                    total_hours: 1,
                    error: None,
                    warning: None,
                    platform: "PC".into(),
                },
            },
        );
        let job = cancel(
            State(state.clone()),
            Json(League {
                league: "Test".into(),
            }),
        )
        .await
        .unwrap()
        .0
        .unwrap();
        assert_eq!(job.status, "cancelled");
        assert!(*state.ggg.jobs.lock().await["Test"].cancel.borrow());
        let range = Range {
            league: "Test".into(),
            start: 0,
            end: 0,
        };
        assert_eq!(
            archive_status(&state, &range).await.unwrap().status,
            "cancelled"
        );
        state
            .ggg
            .jobs
            .lock()
            .await
            .get_mut("Test")
            .unwrap()
            .dto
            .status = "running".into();
        let existing = start_archive_import(state.clone(), range).await.unwrap();
        assert_eq!(existing.status, "running");
        assert_eq!(existing.total_hours, 1);
        let wider_range = Range {
            league: "Test".into(),
            start: 0,
            end: HOUR,
        };
        let ongoing = archive_status(&state, &wider_range).await.unwrap();
        assert_eq!(ongoing.status, "running");
        assert_eq!(ongoing.total_hours, 2);
        assert!(ongoing.warning.unwrap().contains("full dataset coverage"));
        let router = crate::router(state);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ggg/markets?league=Missing&hour=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let overview: dto::Overview = serde_json::from_slice(&body).unwrap();
        assert!(!overview.complete);
        assert!(overview.markets.is_empty());
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ggg/coverage?league=Missing&start=1&end=3600")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn worker_resumes_failed_hours_skips_completed_hours_and_handles_zero_markets() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let app = Router::new().route(
            "/{hour}",
            get(move |axum::extract::Path(hour): axum::extract::Path<i64>| {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Json(ExchangePage {
                        next_change_id: faust_fetch::ExchangeCursor::new((hour + HOUR) as u64),
                        markets: vec![],
                    })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let store =
            market_store::SqliteStore::connect("file:ggg-resume-test?mode=memory&cache=shared")
                .await
                .unwrap();
        store
            .save_ggg_catalog(&[], Utc::now().timestamp())
            .await
            .unwrap();
        store
            .record_ggg_failure("Test", 0, "old failure")
            .await
            .unwrap();
        store
            .record_ggg_hour(
                "Test",
                HOUR,
                &ExchangePage {
                    next_change_id: faust_fetch::ExchangeCursor::new((HOUR * 2) as u64),
                    markets: vec![],
                },
            )
            .await
            .unwrap();
        let (selection, _) = watch::channel(None);
        let state = ServiceState::new(store, "test", selection);
        let range = Range {
            league: "Test".into(),
            start: 0,
            end: HOUR * 2,
        };
        *state.ggg.metadata_checked.lock().await = Some(Instant::now());
        let url = format!("http://{address}");
        run_import(&state, &range, &url).await.unwrap();
        let coverage = state.store.ggg_coverage("Test", 0, HOUR * 2).await.unwrap();
        assert_eq!(coverage.completed_hours, vec![0, HOUR, HOUR * 2]);
        assert!(coverage.failed_hours.is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        run_import(&state, &range, &url).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn imports_are_globally_serialized_and_restart_resume_does_not_claim_a_worker() {
        let store =
            market_store::SqliteStore::connect("file:ggg-job-test?mode=memory&cache=shared")
                .await
                .unwrap();
        store
            .record_ggg_hour(
                "Test",
                0,
                &ExchangePage {
                    next_change_id: faust_fetch::ExchangeCursor::new(HOUR as u64),
                    markets: vec![],
                },
            )
            .await
            .unwrap();
        let (selection, _) = watch::channel(None);
        let state = ServiceState::new(store.clone(), "test", selection);
        *state.ggg.metadata_checked.lock().await = Some(Instant::now());
        let query = Range {
            league: "Test".into(),
            start: 0,
            end: 0,
        };
        let permit = IMPORT_LOCK.lock().await;
        let error = start_import(State(state.clone()), Json(query.clone()))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::CONFLICT);
        drop(permit);
        let job = start_import(State(state.clone()), Json(query.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(job.completed_hours, 1);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.ggg.jobs.lock().await["Test"].dto.status == "completed" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let (selection, _) = watch::channel(None);
        let restarted = ServiceState::new(store, "test", selection);
        *restarted.ggg.metadata_checked.lock().await = Some(Instant::now());
        assert!(
            import_status(
                State(restarted.clone()),
                Query(League {
                    league: "Test".into()
                })
            )
            .await
            .unwrap()
            .0
            .is_none()
        );
        let job = start_import(State(restarted.clone()), Json(query))
            .await
            .unwrap()
            .0;
        assert_eq!(job.completed_hours, 1);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if restarted.ggg.jobs.lock().await["Test"].dto.status == "completed" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
