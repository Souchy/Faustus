use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{NaiveDate, Utc};
use market_core::economy::{self as dto, Archive, History, League, Line, Overview};
use ninja_fetch::{FetchOutcome, NinjaClient};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Mutex, RwLock, watch};

use crate::{ServiceState, leagues};

pub mod imports;

type Key = (String, String, String);
type ApiError = (StatusCode, String);

#[derive(Clone)]
pub(crate) struct RawOverview {
    pub body: Value,
    pub fetched_at: chrono::DateTime<Utc>,
    checked: Instant,
    etag: Option<String>,
    max_age: Duration,
}

#[derive(Default)]
pub(crate) struct EconomyState {
    raw: RwLock<HashMap<Key, RawOverview>>,
    locks: Mutex<HashMap<Key, Arc<Mutex<()>>>>,
    pub(crate) import_lock: Mutex<()>,
    pub(crate) worker_lock: Mutex<()>,
    catalog: RwLock<Option<(Instant, Vec<Archive>)>>,
    trade_icons: Mutex<Option<(Instant, TradeStatic)>>,
}

#[derive(Deserialize)]
struct TradeStatic {
    result: Vec<TradeGroup>,
}

#[derive(Deserialize)]
struct TradeGroup {
    entries: Vec<TradeIcon>,
}

#[derive(Deserialize)]
struct TradeIcon {
    text: String,
    image: Option<String>,
}

pub fn routes() -> Router<ServiceState> {
    Router::new()
        .route("/api/v1/economy/categories", get(category_catalog))
        .route("/api/v1/economy/leagues", get(economy_leagues))
        .route("/api/v1/economy/overview", get(overview))
        .route("/api/v1/economy/refresh", post(refresh))
        .route("/api/v1/economy/dates", get(dates))
        .route("/api/v1/economy/history", get(history))
        .route("/api/v1/economy/archives", get(archives))
        .route("/api/v1/economy/import", post(start_import))
}

fn failure(error: impl std::fmt::Display) -> ApiError {
    eprintln!("Economy service: {error}");
    (StatusCode::BAD_GATEWAY, error.to_string())
}

async fn category_catalog(
    State(state): State<ServiceState>,
) -> Result<Json<Vec<dto::Category>>, ApiError> {
    let mut result = dto::categories();
    for id in state
        .economy_store
        .stored_economy_categories()
        .await
        .map_err(failure)?
    {
        if !result.iter().any(|category| category.id == id) {
            result.push(dto::Category {
                name: id.clone(),
                id,
                group: "Historical".into(),
                sources: vec!["stash".into()],
            });
        }
    }
    Ok(Json(result))
}

#[derive(Clone, Deserialize)]
pub struct OverviewQuery {
    pub league: String,
    pub category: String,
    pub source: String,
    pub date: Option<String>,
}

fn validate(query: &OverviewQuery) -> Result<(), ApiError> {
    if query.league.is_empty() || query.league.len() > 100 {
        return Err((StatusCode::BAD_REQUEST, "Invalid league.".into()));
    }
    if query.category.is_empty()
        || query.category.len() > 64
        || !query.category.bytes().all(|c| c.is_ascii_alphanumeric())
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid economy category.".into()));
    }
    // Historical stash data is allowed for categories now offered only on the exchange.
    if !matches!(query.source.as_str(), "stash" | "exchange") {
        return Err((StatusCode::BAD_REQUEST, "Unsupported price source.".into()));
    }
    if let Some(date) = &query.date {
        if date.len() != 10 || NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err((StatusCode::BAD_REQUEST, "Date must be YYYY-MM-DD.".into()));
        }
    }
    Ok(())
}

async fn economy_leagues(State(state): State<ServiceState>) -> Result<Json<Vec<League>>, ApiError> {
    let live = leagues(State(state.clone()))
        .await
        .map_err(|s| (s, "Unable to load live leagues.".into()))?
        .0;
    let mut result: Vec<League> = live
        .into_iter()
        .map(|l| League {
            id: l.id,
            name: l.name,
            live: true,
            archived: false,
        })
        .collect();
    for league in state
        .economy_store
        .imported_leagues()
        .await
        .map_err(failure)?
    {
        // A live league may also have an archive; it remains a live collector selection.
        if let Some(entry) = result.iter_mut().find(|entry| entry.id == league) {
            entry.archived = true;
        } else {
            result.push(League {
                id: league.clone(),
                name: league,
                live: false,
                archived: true,
            });
        }
    }
    merge_ggg_leagues(
        &mut result,
        state.store.ggg_leagues().await.map_err(failure)?,
    );
    Ok(Json(result))
}

fn merge_ggg_leagues(result: &mut Vec<League>, stored: Vec<String>) {
    for league in stored {
        if !result
            .iter()
            .any(|entry| entry.id == league || entry.name == league)
        {
            result.push(League {
                id: league.clone(),
                name: league,
                live: false,
                archived: false,
            });
        }
    }
}

async fn overview(
    State(state): State<ServiceState>,
    Query(query): Query<OverviewQuery>,
) -> Result<Json<Overview>, ApiError> {
    load(state, query, false).await.map(Json)
}
async fn refresh(
    State(state): State<ServiceState>,
    Query(query): Query<OverviewQuery>,
) -> Result<Json<Overview>, ApiError> {
    load(state, query, true).await.map(Json)
}

async fn load(
    state: ServiceState,
    query: OverviewQuery,
    refresh: bool,
) -> Result<Overview, ApiError> {
    validate(&query)?;
    if let Some(date) = &query.date {
        return historical_overview(&state, &query, date).await;
    }
    let live = leagues(State(state.clone()))
        .await
        .map_err(|s| (s, "Unable to load live leagues.".into()))?
        .0;
    if !live.iter().any(|league| league.id == query.league) {
        if !state
            .economy_store
            .imported_leagues()
            .await
            .map_err(failure)?
            .contains(&query.league)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Unknown league; import an archive before browsing it.".into(),
            ));
        }
        let date = state
            .economy_store
            .economy_dates(&query.league, &query.category, &query.source)
            .await
            .map_err(failure)?
            .into_iter()
            .next();
        return historical_overview(&state, &query, date.as_deref().unwrap_or("0001-01-01")).await;
    }
    let category = dto::categories()
        .into_iter()
        .find(|c| c.id == query.category)
        .ok_or((
            StatusCode::BAD_REQUEST,
            "This historical category has no supported live feed.".into(),
        ))?;
    if !category.sources.contains(&query.source) {
        return Err((StatusCode::BAD_REQUEST, "This category has no supported live stash feed. Select Exchange or a saved historical date.".into()));
    }
    let raw = raw_overview(
        &state,
        &query.league,
        &query.category,
        &query.source,
        refresh,
    )
    .await?;
    let result = normalize(&query, &raw.body, raw.fetched_at.to_rfc3339()).map_err(failure)?;
    state
        .economy_store
        .save_economy(&result)
        .await
        .map_err(failure)?;
    Ok(result)
}

async fn historical_overview(
    state: &ServiceState,
    query: &OverviewQuery,
    date: &str,
) -> Result<Overview, ApiError> {
    let mut overview = state
        .economy_store
        .historical_economy(&query.league, &query.category, &query.source, date)
        .await
        .map_err(failure)?;
    if matches!(query.category.as_str(), "SkillGem" | "ImbuedGem") {
        for line in &mut overview.lines {
            if let Some((level, quality, corrupted)) =
                line.variant.as_deref().and_then(imports::gem_variant)
            {
                line.gem_level = Some(level);
                line.gem_quality = Some(quality);
                line.corrupted = Some(corrupted);
            }
        }
    }
    let mut metadata = state
        .economy_store
        .economy_icon_metadata(&query.category)
        .await
        .map_err(failure)?;
    if metadata.is_empty() && !overview.lines.is_empty() {
        if let Some(category) = dto::categories()
            .into_iter()
            .find(|c| c.id == query.category)
        {
            let source = &category.sources[0];
            let result = async {
                let raw = raw_overview(state, "Standard", &query.category, source, false).await?;
                let live = normalize(
                    &OverviewQuery {
                        league: "Standard".into(),
                        source: source.clone(),
                        date: None,
                        ..query.clone()
                    },
                    &raw.body,
                    raw.fetched_at.to_rfc3339(),
                )
                .map_err(failure)?;
                state
                    .economy_store
                    .save_economy(&live)
                    .await
                    .map_err(failure)?;
                Ok::<_, ApiError>(live.lines)
            }
            .await;
            match result {
                Ok(lines) => metadata = lines,
                Err(error) => icon_error(&mut overview.lines, &error.1),
            }
        }
    }
    enrich_icons(&mut overview.lines, &metadata);
    if matches!(query.category.as_str(), "Currency" | "Fragment") {
        let categories = if query.category == "Currency" {
            ["Currency", "Fragment"]
        } else {
            ["Fragment", "Currency"]
        };
        for category in categories {
            if overview.lines.iter().all(|line| line.icon.is_some()) {
                break;
            }
            let result = async {
                let raw = raw_overview(state, "Standard", category, "stash", false).await?;
                currency_icon_metadata(&raw.body).map_err(failure)
            }
            .await;
            match result {
                Ok(metadata) => enrich_icons(&mut overview.lines, &metadata),
                Err(error) => icon_error(&mut overview.lines, &error.1),
            }
        }
    }
    if matches!(query.category.as_str(), "Currency" | "Fragment")
        && overview.lines.iter().any(|line| line.icon.is_none())
    {
        if let Err(error) = trade_icons(state, &mut overview.lines).await {
            icon_error(&mut overview.lines, &error.1);
        }
    }
    Ok(overview)
}

fn currency_icon_metadata(body: &Value) -> Result<Vec<Line>, String> {
    let details = body
        .get("currencyDetails")
        .ok_or("Currency overview is missing currencyDetails icon metadata.")?;
    let details: Vec<ninja_fetch::CurrencyDetails> =
        serde_json::from_value(details.clone()).map_err(|error| error.to_string())?;
    Ok(details
        .into_iter()
        .map(|item| Line {
            id: item.id.to_string(),
            name: item.name,
            icon: item.icon,
            ..Line::default()
        })
        .collect())
}

fn icon_error(lines: &mut [Line], error: &str) {
    eprintln!("Historical icon metadata: {error}");
    for line in lines.iter_mut().filter(|line| line.icon.is_none()) {
        line.metadata.insert(
            "icon_error".into(),
            format!("Icon metadata unavailable: {error}").into(),
        );
    }
}

async fn trade_icons(state: &ServiceState, lines: &mut [Line]) -> Result<(), ApiError> {
    let mut cached = state.economy.trade_icons.lock().await;
    if cached
        .as_ref()
        .is_none_or(|(checked, _)| checked.elapsed() >= Duration::from_secs(24 * 60 * 60))
    {
        let client = reqwest::Client::builder()
            .user_agent(state.user_agent.as_ref())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(failure)?;
        let metadata: TradeStatic = client
            .get("https://www.pathofexile.com/api/trade/data/static")
            .send()
            .await
            .map_err(failure)?
            .error_for_status()
            .map_err(failure)?
            .json()
            .await
            .map_err(failure)?;
        *cached = Some((Instant::now(), metadata));
    }
    if let Some((_, metadata)) = cached.as_ref() {
        enrich_trade_icons(lines, metadata);
    }
    Ok(())
}

fn enrich_trade_icons(lines: &mut [Line], metadata: &TradeStatic) {
    let candidates: Vec<Line> = metadata
        .result
        .iter()
        .flat_map(|group| &group.entries)
        .map(|entry| Line {
            name: entry.text.clone(),
            icon: entry.image.clone(),
            ..Line::default()
        })
        .collect();
    for line in lines
        .iter_mut()
        .filter(|line| line.icon.is_none() && icon_variant(line).is_empty())
    {
        enrich_icons(std::slice::from_mut(line), &candidates);
        if line.icon.is_some() {
            line.metadata.remove("icon_error");
            line.metadata.insert("icon_source".into(),
                "Grinding Gear Games official trade static metadata (https://www.pathofexile.com/api/trade/data/static); image: web.poecdn.com CDN; exact unambiguous name (display only; prices remain separate)".into());
        }
    }
}

fn enrich_icons(lines: &mut [Line], metadata: &[Line]) {
    let mut icons = BTreeMap::<&str, Vec<(&Line, String)>>::new();
    let base = url::Url::parse("https://web.poecdn.com").expect("static CDN URL");
    for item in metadata {
        if let Some(icon) = item.icon.as_deref() {
            if let Ok(url) = base.join(icon) {
                if url.scheme() == "https"
                    && url.host_str() == Some("web.poecdn.com")
                    && url.username().is_empty()
                    && url.password().is_none()
                {
                    icons
                        .entry(&item.name)
                        .or_default()
                        .push((item, url.to_string()));
                }
            }
        }
    }
    for line in lines {
        if line.icon.is_some() {
            continue;
        }
        let Some(candidates) = icons.get(line.name.as_str()) else {
            continue;
        };
        let provider_id = line.metadata.get("archive_item_id").and_then(Value::as_str);
        let exact: Vec<_> = candidates
            .iter()
            .filter(|(candidate, _)| {
                provider_id.is_some_and(|id| candidate.id.split(':').next() == Some(id))
                    && compatible_icon_variant(line, candidate)
            })
            .collect();
        let variant: Vec<_> = candidates
            .iter()
            .filter(|(candidate, _)| {
                compatible_icon_variant(line, candidate)
                    && icon_variant(line) == icon_variant(candidate)
            })
            .collect();
        let selected = if !exact.is_empty() {
            exact
        } else if !variant.is_empty() {
            variant
        } else {
            candidates
                .iter()
                .filter(|(candidate, _)| compatible_icon_variant(line, candidate))
                .collect()
        };
        let unique: std::collections::BTreeSet<&str> =
            selected.iter().map(|(_, icon)| icon.as_str()).collect();
        if unique.len() == 1 {
            line.icon = unique.first().map(|icon| (*icon).to_owned());
            line.metadata.remove("icon_error");
            line.metadata.insert("icon_source".into(),
                "poe.ninja supported overview metadata; image: Grinding Gear Games web.poecdn.com CDN; exact name with provider ID/variant matching or unambiguous artwork fallback (display only; prices remain separate)".into());
        }
    }
}

fn icon_variant(line: &Line) -> String {
    line.variant
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty() && !part.ends_with(" links"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn compatible_icon_variant(archive: &Line, candidate: &Line) -> bool {
    archive
        .base_type
        .as_ref()
        .zip(candidate.base_type.as_ref())
        .is_none_or(|(a, b)| a == b)
        && archive
            .gem_level
            .zip(candidate.gem_level)
            .is_none_or(|(a, b)| a == b)
        && archive
            .gem_quality
            .zip(candidate.gem_quality)
            .is_none_or(|(a, b)| a == b)
        && archive
            .corrupted
            .zip(candidate.corrupted)
            .is_none_or(|(a, b)| a == b)
}

pub(crate) async fn raw_overview(
    state: &ServiceState,
    league: &str,
    category: &str,
    source: &str,
    refresh: bool,
) -> Result<RawOverview, ApiError> {
    let key = (league.to_owned(), category.to_owned(), source.to_owned());
    let lock = state
        .economy
        .locks
        .lock()
        .await
        .entry(key.clone())
        .or_default()
        .clone();
    let _guard = lock.lock().await;
    let cached = state.economy.raw.read().await.get(&key).cloned();
    if let Some(cached) = &cached {
        let ttl = if refresh {
            cached.max_age
        } else {
            cached.max_age.max(Duration::from_secs(15 * 60))
        };
        if cached.checked.elapsed() < ttl {
            return Ok(cached.clone());
        }
    }
    let client = NinjaClient::new(&state.user_agent).map_err(failure)?;
    let outcome = client
        .fetch_economy_overview(
            league,
            category,
            source,
            cached.as_ref().and_then(|c| c.etag.as_deref()),
        )
        .await
        .map_err(failure)?;
    let entry = match outcome {
        FetchOutcome::Modified { body, metadata, .. } => RawOverview {
            body,
            fetched_at: Utc::now(),
            checked: Instant::now(),
            etag: metadata.etag,
            max_age: cache_age(metadata.cache_control.as_deref()),
        },
        FetchOutcome::NotModified { metadata } => {
            let mut cached = cached
                .ok_or_else(|| failure("poe.ninja returned 304 without a cached overview."))?;
            cached.checked = Instant::now();
            if metadata.etag.is_some() {
                cached.etag = metadata.etag;
            }
            if metadata.cache_control.is_some() {
                cached.max_age = cache_age(metadata.cache_control.as_deref());
            }
            cached
        }
    };
    state.economy.raw.write().await.insert(key, entry.clone());
    Ok(entry)
}

fn cache_age(header: Option<&str>) -> Duration {
    let seconds = header
        .and_then(|h| {
            h.split(',').find_map(|s| {
                s.trim()
                    .strip_prefix("max-age=")?
                    .trim_matches('"')
                    .parse()
                    .ok()
            })
        })
        .unwrap_or(300_u64);
    Duration::from_secs(seconds.max(60))
}

async fn dates(
    State(state): State<ServiceState>,
    Query(query): Query<OverviewQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    validate(&query)?;
    state
        .economy_store
        .economy_dates(&query.league, &query.category, &query.source)
        .await
        .map(Json)
        .map_err(failure)
}

#[derive(Deserialize)]
struct HistoryQuery {
    #[serde(flatten)]
    overview: OverviewQuery,
    item: String,
    days: Option<i64>,
}
async fn history(
    State(state): State<ServiceState>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<History>, ApiError> {
    validate(&query.overview)?;
    if query.item.len() > 500 || query.days.is_some_and(|days| !(1..=3650).contains(&days)) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid item or history range.".into(),
        ));
    }
    state
        .economy_store
        .economy_history(
            &query.overview.league,
            &query.overview.category,
            &query.overview.source,
            &query.item,
            query.days,
        )
        .await
        .map(Json)
        .map_err(failure)
}

#[derive(Deserialize)]
struct ArchiveQuery {
    #[serde(default)]
    refresh: bool,
}
async fn archives(
    State(state): State<ServiceState>,
    Query(query): Query<ArchiveQuery>,
) -> Result<Json<Vec<Archive>>, ApiError> {
    archive_catalog(&state, query.refresh).await.map(Json)
}

pub async fn archive_catalog(
    state: &ServiceState,
    refresh: bool,
) -> Result<Vec<Archive>, ApiError> {
    let cached = state.economy.catalog.read().await.clone();
    let result = if let Some((checked, entries)) =
        cached.filter(|(checked, _)| !refresh && checked.elapsed() < Duration::from_secs(900))
    {
        let _ = checked;
        entries
    } else {
        let client = reqwest::Client::builder()
            .user_agent(state.user_agent.as_ref())
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(failure)?;
        let response = client
            .get("https://poe.ninja/poe1/api/data/dumps")
            .send()
            .await
            .map_err(failure)?
            .error_for_status()
            .map_err(failure)?
            .json::<Vec<PublishedArchive>>()
            .await
            .map_err(failure)?;
        let mut entries: Vec<Archive> = response
            .into_iter()
            .map(|entry| {
                let url = imports::download_url(&entry.league_name);
                Archive {
                    name: entry.league_name,
                    status: "available".into(),
                    rows: 0,
                    skipped: 0,
                    error: None,
                    start: Some(entry.min_date[..10.min(entry.min_date.len())].into()),
                    end: Some(entry.max_date[..10.min(entry.max_date.len())].into()),
                    league: None,
                    progress: "Daily stash prices; softcore challenge league only.".into(),
                    license: "CC BY-SA (poe.ninja)".into(),
                    url,
                }
            })
            .collect();
        entries.reverse();
        state
            .economy
            .catalog
            .write()
            .await
            .replace((Instant::now(), entries.clone()));
        entries
    };
    let saved = state
        .economy_store
        .economy_imports()
        .await
        .map_err(failure)?;
    Ok(merge_archive_catalog(result, saved))
}

fn merge_archive_catalog(mut result: Vec<Archive>, saved: Vec<Archive>) -> Vec<Archive> {
    let mut names = HashSet::new();
    result.retain(|entry| names.insert(entry.name.clone()));
    for saved in saved {
        if let Some(entry) = result.iter_mut().find(|entry| entry.name == saved.name) {
            *entry = saved;
        } else {
            result.push(saved);
        }
    }
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedArchive {
    league_name: String,
    min_date: String,
    max_date: String,
}

#[derive(Deserialize)]
struct ImportRequest {
    name: String,
}
async fn start_import(
    State(state): State<ServiceState>,
    Json(query): Json<ImportRequest>,
) -> Result<Json<Archive>, ApiError> {
    let entry = enqueue_import(state, &query.name).await?;
    Ok(Json(entry))
}

pub async fn enqueue_import(state: ServiceState, name: &str) -> Result<Archive, ApiError> {
    let _guard = state.economy.import_lock.lock().await;
    let mut entry = archive_catalog(&state, false)
        .await?
        .into_iter()
        .find(|entry| entry.name == name)
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Archive is not in the published catalog.".into(),
        ))?;
    let lease = state
        .economy_store
        .try_economy_import_lock(name)
        .map_err(failure)?;
    if let Some(saved) = state
        .economy_store
        .economy_imports()
        .await
        .map_err(failure)?
        .into_iter()
        .find(|saved| saved.name == name)
    {
        entry = saved;
    }
    if matches!(
        entry.status.as_str(),
        "queued" | "downloading" | "importing" | "completed"
    ) {
        return Ok(entry);
    }
    let lease = lease.ok_or((
        StatusCode::CONFLICT,
        "Archive is already being imported by another process.".into(),
    ))?;
    entry.status = "queued".into();
    entry.error = None;
    state
        .economy_store
        .save_economy_import(&entry, None)
        .await
        .map_err(failure)?;
    let queued = entry.clone();
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = imports::import_locked(&state, entry, None, false, lease).await {
            eprintln!("Archive import failed: {error}");
        }
    });
    Ok(queued)
}

pub async fn recover_imports(state: &ServiceState) -> Result<(), std::io::Error> {
    for mut entry in state
        .economy_store
        .economy_imports()
        .await
        .map_err(std::io::Error::other)?
    {
        if matches!(
            entry.status.as_str(),
            "queued" | "downloading" | "importing"
        ) {
            let Some(_lease) = state.economy_store.try_economy_import_lock(&entry.name)? else {
                continue;
            };
            entry.status = "failed".into();
            entry.error =
                Some("Service stopped before the import completed. Retry the import.".into());
            state
                .economy_store
                .save_economy_import(&entry, None)
                .await
                .map_err(std::io::Error::other)?;
        }
    }
    Ok(())
}

pub async fn collect(state: ServiceState, mut selection: watch::Receiver<Option<String>>) {
    'selection: loop {
        let Some(league) = crate::wait_for_selection(&mut selection).await else {
            return;
        };
        loop {
            let next_collection = tokio::time::Instant::now() + Duration::from_secs(15 * 60);
            for category in dto::categories() {
                for source in category.sources {
                    let query = OverviewQuery {
                        league: league.clone(),
                        category: category.id.clone(),
                        source,
                        date: None,
                    };
                    tokio::select! {
                        result = load(state.clone(), query, true) => {
                            if let Err(error) = result { eprintln!("Economy collection for {league}/{}: {}", category.id, error.1); }
                        }

                        changed = selection.changed() => { if changed.is_err() { return; } continue 'selection; }
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        changed = selection.changed() => { if changed.is_err() { return; } continue 'selection; }
                    }
                }
            }
            tokio::select! {
                _ = tokio::time::sleep_until(next_collection) => {}
                changed = selection.changed() => { if changed.is_err() { return; } continue 'selection; }
            }
        }
    }
}

pub async fn ingest_economy_once(state: ServiceState, league: &str) -> Result<(), String> {
    let mut failures = Vec::new();
    for category in dto::categories() {
        for source in category.sources {
            let query = OverviewQuery {
                league: league.into(),
                category: category.id.clone(),
                source,
                date: None,
            };
            if let Err(error) = load(state.clone(), query, true).await {
                failures.push(format!("{}: {}", category.id, error.1));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}
fn decimal(value: &Value, key: &str) -> Option<String> {
    let value = value.get(key)?;
    if value.is_number() {
        Some(value.to_string())
    } else {
        None
    }
}
fn spark(line: &mut Line, value: &Value) {
    line.change = decimal(value, "totalChange");
    line.sparkline = value
        .get("data")
        .and_then(Value::as_array)
        .map(|points| {
            points
                .iter()
                .map(|p| {
                    if p.is_number() {
                        Some(p.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
}

fn normalize(query: &OverviewQuery, body: &Value, fetched_at: String) -> Result<Overview, String> {
    let rows = body
        .get("lines")
        .and_then(Value::as_array)
        .ok_or("Overview response is missing lines.")?;
    for row in rows {
        for field in [
            "primaryValue",
            "chaosValue",
            "divineValue",
            "chaosEquivalent",
            "volumePrimaryValue",
            "maxVolumeRate",
        ] {
            if let Some(value) = row.get(field).filter(|value| !value.is_null()) {
                let price = value
                    .to_string()
                    .parse::<rust_decimal::Decimal>()
                    .map_err(|_| {
                        format!("Provider {field} is not a representable decimal price.")
                    })?;
                if !value.is_number() || price < rust_decimal::Decimal::ZERO {
                    return Err(format!(
                        "Provider {field} must be a non-negative numeric price."
                    ));
                }
            }
        }
        for field in [
            "listingCount",
            "count",
            "gemLevel",
            "gemQuality",
            "links",
            "mapTier",
        ] {
            if row
                .get(field)
                .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
            {
                return Err(format!("Provider {field} must be a non-negative integer."));
            }
        }
    }
    let mut rates = BTreeMap::new();
    let mut quote_currency = "Chaos Orb".to_owned();
    let mut lines = Vec::new();
    if query.source == "exchange" {
        let core = body
            .get("core")
            .ok_or("Exchange overview is missing core.")?;
        let primary = text(core, "primary").ok_or("Exchange overview is missing primary quote.")?;
        let items: Vec<&Value> = body
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                core.get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            )
            .collect();
        let find = |id: &str| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
                .copied()
        };
        quote_currency = find(&primary)
            .and_then(|item| text(item, "name"))
            .unwrap_or(primary.clone());
        if let Some(values) = core.get("rates").and_then(Value::as_object) {
            for (id, value) in values {
                let name = find(id)
                    .and_then(|item| text(item, "name"))
                    .unwrap_or(id.clone());
                let rate = value
                    .to_string()
                    .parse::<rust_decimal::Decimal>()
                    .map_err(|_| {
                        format!("Provider rate for {id} is not a representable decimal.")
                    })?;
                if rate < rust_decimal::Decimal::ZERO {
                    return Err(format!("Provider rate for {id} is negative."));
                }
                if rate > rust_decimal::Decimal::ZERO {
                    let inverse = rust_decimal::Decimal::ONE
                        .checked_div(rate)
                        .ok_or_else(|| format!("Provider rate for {id} cannot be inverted."))?;
                    rates.insert(name, inverse.normalize().to_string());
                }
            }
        }
        for value in rows {
            let id = text(value, "id").ok_or("Exchange line is missing id.")?;
            let item = find(&id);
            let mut line = Line {
                id: id.clone(),
                name: item.and_then(|i| text(i, "name")).unwrap_or(id),
                icon: item.and_then(|i| text(i, "image")),
                details_id: item.and_then(|i| text(i, "detailsId")),
                price: decimal(value, "primaryValue"),
                volume: decimal(value, "volumePrimaryValue"),
                pair_rate: decimal(value, "maxVolumeRate"),
                pair: text(value, "maxVolumeCurrency")
                    .map(|id| find(&id).and_then(|i| text(i, "name")).unwrap_or(id)),
                ..Line::default()
            };
            if let Some(s) = value.get("sparkline") {
                spark(&mut line, s);
            }
            lines.push(line);
        }
    } else if matches!(query.category.as_str(), "Currency" | "Fragment") {
        let details = body
            .get("currencyDetails")
            .and_then(Value::as_array)
            .ok_or("Stash currency metadata is missing.")?;
        for value in rows {
            let name =
                text(value, "currencyTypeName").ok_or("Currency line is missing its name.")?;
            let detail = details
                .iter()
                .find(|d| d.get("name").and_then(Value::as_str) == Some(&name));
            let mut line = Line {
                id: text(value, "detailsId").unwrap_or_else(|| name.clone()),
                name: name.clone(),
                icon: detail.and_then(|d| text(d, "icon")),
                details_id: text(value, "detailsId"),
                price: decimal(value, "chaosEquivalent"),
                listings: value
                    .pointer("/receive/listing_count")
                    .and_then(Value::as_u64),
                samples: value.pointer("/receive/count").and_then(Value::as_u64),
                ..Line::default()
            };
            if let Some(s) = value.get("receiveSparkLine") {
                spark(&mut line, s);
            }
            if matches!(name.as_str(), "Chaos Orb" | "Divine Orb") {
                if let Some(price) = &line.price {
                    rates.insert(name, price.clone());
                }
            }
            lines.push(line);
        }
    } else {
        for value in rows {
            let id = value
                .get("id")
                .and_then(Value::as_u64)
                .ok_or("Item line is missing numeric id.")?;
            let mut line = Line {
                id: id.to_string(),
                name: text(value, "name").ok_or("Item line is missing name.")?,
                icon: text(value, "icon"),
                details_id: text(value, "detailsId"),
                price: decimal(value, "chaosValue"),
                divine_price: decimal(value, "divineValue"),
                listings: value.get("listingCount").and_then(Value::as_u64),
                samples: value.get("count").and_then(Value::as_u64),
                variant: text(value, "variant"),
                base_type: text(value, "baseType"),
                item_type: text(value, "itemType"),
                links: value.get("links").and_then(Value::as_u64),
                gem_level: value.get("gemLevel").and_then(Value::as_u64),
                gem_quality: value.get("gemQuality").and_then(Value::as_u64),
                corrupted: value.get("corrupted").and_then(Value::as_bool),
                map_tier: value.get("mapTier").and_then(Value::as_u64),
                ..Line::default()
            };
            for key in [
                "implicitModifiers",
                "explicitModifiers",
                "mutatedModifiers",
                "flavourText",
                "levelRequired",
                "itemClass",
                "stackSize",
                "mapRegion",
                "prophecyText",
                "artFilename",
                "tradeInfo",
                "tradeFilter",
                "metadata",
            ] {
                if let Some(value) = value.get(key) {
                    line.metadata.insert(key.into(), value.clone());
                }
            }
            if let Some(s) = value.get("sparkLine") {
                spark(&mut line, s);
            }
            // Provider line IDs must remain variant-specific, even across upstream ID reuse.
            line.id = format!(
                "{id}:{}",
                imports::identity(&[
                    &line.variant.clone().unwrap_or_default(),
                    &line.links.map(|n| n.to_string()).unwrap_or_default(),
                    &line.gem_level.map(|n| n.to_string()).unwrap_or_default(),
                    &line.gem_quality.map(|n| n.to_string()).unwrap_or_default(),
                    &line.corrupted.map(|b| b.to_string()).unwrap_or_default(),
                    &line.map_tier.map(|n| n.to_string()).unwrap_or_default(),
                ])
            );
            lines.push(line);
        }
    }
    rates.insert(quote_currency.clone(), "1".into());
    Ok(Overview {
        league: query.league.clone(),
        category: query.category.clone(),
        source: query.source.clone(),
        fetched_at,
        quote_currency,
        rates,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ggg_catalog_preserves_live_and_poe_archive_labels() {
        let mut leagues = vec![
            League {
                id: "live-slug".into(),
                name: "Live".into(),
                live: true,
                archived: false,
            },
            League {
                id: "Archive".into(),
                name: "Archive".into(),
                live: false,
                archived: true,
            },
        ];
        merge_ggg_leagues(
            &mut leagues,
            vec!["Live".into(), "Archive".into(), "GGG only".into()],
        );
        assert_eq!(leagues.len(), 3);
        assert!(leagues[0].live);
        assert!(!leagues[0].archived);
        assert!(leagues[1].archived);
        assert!(!leagues[2].live);
        assert!(!leagues[2].archived);
        assert_eq!(leagues[2].name, "GGG only");
    }

    #[test]
    fn archive_catalog_lists_each_import_identity_once_with_saved_status() {
        let archive = |name: &str, status: &str, rows: u64| Archive {
            name: name.into(),
            status: status.into(),
            rows,
            skipped: 0,
            error: None,
            start: None,
            end: None,
            league: None,
            progress: String::new(),
            license: "CC BY-SA (poe.ninja)".into(),
            url: imports::download_url(name),
        };
        let result = merge_archive_catalog(
            vec![
                archive("Allflame", "available", 0),
                archive("Allflame", "available", 0),
                archive("Ancestors", "available", 0),
                archive("Ancestors", "available", 0),
                archive("Ancestors", "available", 0),
                archive("Mirage", "available", 0),
            ],
            vec![
                archive("Ancestors", "completed", 42),
                archive("Ancestors", "completed", 42),
                archive("Keepers", "failed", 0),
            ],
        );
        assert_eq!(
            result
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Allflame", "Ancestors", "Mirage", "Keepers"]
        );
        assert_eq!(result[0].status, "available");
        assert_eq!(result[1].status, "completed");
        assert_eq!(result[1].rows, 42);
        assert_eq!(result[3].status, "failed");
    }

    #[tokio::test]
    async fn existing_archives_recover_level_only_variants_without_reimporting() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("gems.sqlite3"))
            .await
            .unwrap();
        store
            .save_economy(&Overview {
                league: "Old".into(),
                category: "SkillGem".into(),
                source: "stash".into(),
                fetched_at: "2026-01-01".into(),
                quote_currency: "Chaos Orb".into(),
                rates: BTreeMap::new(),
                lines: vec![Line {
                    id: "archive:original".into(),
                    name: "Fireball".into(),
                    variant: Some("21c".into()),
                    price: Some("30".into()),
                    icon: Some("https://web.poecdn.com/fireball.png".into()),
                    ..Line::default()
                }],
            })
            .await
            .unwrap();
        let (selection, _) = watch::channel(None);
        let state = ServiceState::new(store, "test/0.1", selection);
        let overview = historical_overview(
            &state,
            &OverviewQuery {
                league: "Old".into(),
                category: "SkillGem".into(),
                source: "stash".into(),
                date: None,
            },
            "2026-01-01",
        )
        .await
        .unwrap();
        assert_eq!(overview.lines[0].id, "archive:original");
        assert_eq!(overview.lines[0].price.as_deref(), Some("30"));
        assert_eq!(overview.lines[0].gem_level, Some(21));
        assert_eq!(overview.lines[0].gem_quality, Some(0));
        assert_eq!(overview.lines[0].corrupted, Some(true));
    }

    #[test]
    fn historical_emblem_icons_use_catalog_entries_without_live_prices() {
        let names = [
            "Unrelenting Timeless Karui Emblem",
            "Unrelenting Timeless Maraketh Emblem",
            "Unrelenting Timeless Eternal Emblem",
            "Unrelenting Timeless Templar Emblem",
            "Unrelenting Timeless Vaal Emblem",
        ];
        let body = json!({
            "lines": [],
            "currencyDetails": names.iter().enumerate().map(|(index, name)| json!({
                "id": index,
                "name": name,
                "icon": format!("https://web.poecdn.com/gen/image/emblem{index}.png")
            })).collect::<Vec<_>>()
        });
        let live = normalize(
            &OverviewQuery {
                league: "Standard".into(),
                category: "Fragment".into(),
                source: "stash".into(),
                date: None,
            },
            &body,
            "2026-07-20T00:00:00Z".into(),
        )
        .unwrap();
        assert!(
            live.lines.is_empty(),
            "Icon-only catalog entries must not become price rows"
        );
        let metadata = currency_icon_metadata(&body).unwrap();
        assert_eq!(metadata.len(), 5);
        assert!(metadata.iter().all(|line| line.price.is_none()));
        let mut lines: Vec<Line> = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| Line {
                id: format!("archive:{index}"),
                name: name.into(),
                price: Some("45".into()),
                metadata: BTreeMap::from([("icon_error".into(), "prior error".into())]),
                ..Line::default()
            })
            .collect();
        enrich_icons(&mut lines, &metadata);
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(line.id, format!("archive:{index}"));
            assert_eq!(line.name, names[index]);
            assert_eq!(line.price.as_deref(), Some("45"));
            assert_eq!(
                line.icon.as_deref(),
                Some(format!("https://web.poecdn.com/gen/image/emblem{index}.png").as_str())
            );
            assert!(!line.metadata.contains_key("icon_error"));
        }
        assert!(currency_icon_metadata(&json!({"lines":[]})).is_err());
        assert!(currency_icon_metadata(&json!({"currencyDetails":null})).is_err());
        assert!(
            currency_icon_metadata(&json!({"currencyDetails":[{"id":"invalid","name":"Emblem"}]}))
                .is_err()
        );
    }

    #[tokio::test]
    async fn official_trade_icons_fill_exact_unambiguous_names_from_cached_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("icons.sqlite3"))
            .await
            .unwrap();
        let (selection, _) = watch::channel(None);
        let state = ServiceState::new(store, "test/0.1", selection);
        let metadata: TradeStatic = serde_json::from_value(json!({
            "result": [{"id":"currency","entries":[
                {"text":"Lonely Reliquary Key","image":"/gen/image/lonely.png"},
                {"text":"Lonely Reliquary Key","image":"https://web.poecdn.com/gen/image/lonely.png"},
                {"text":"Cosmic Reliquary Key","image":"/gen/image/cosmic.png"},
                {"text":"Ambiguous Key","image":"/gen/image/a.png"},
                {"text":"Ambiguous Key","image":"/gen/image/b.png"},
                {"text":"Unsafe Key","image":"https://untrusted.example/key.png"},
                {"text":"Unsafe Key","image":"https://user@web.poecdn.com/key.png"},
                {"text":"Unsafe Key","image":"http://web.poecdn.com/key.png"},
                {"text":"Missing Key"},
                {"text":"Variant Key","image":"/gen/image/variant.png"}
            ]}]
        })).unwrap();
        *state.economy.trade_icons.lock().await = Some((Instant::now(), metadata));
        let mut lines: Vec<Line> = [
            "Lonely Reliquary Key",
            "Cosmic Reliquary Key",
            "Ambiguous Key",
            "Unsafe Key",
            "Missing Key",
            "Absent Key",
            "Variant Key",
            "lonely reliquary key",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| Line {
            id: format!("archive:{index}"),
            name: name.into(),
            price: Some("25".into()),
            ..Line::default()
        })
        .collect();
        lines[0]
            .metadata
            .insert("icon_error".into(), "prior error".into());
        lines[1].icon = Some("https://web.poecdn.com/existing.png".into());
        lines[6].variant = Some("Special".into());
        let original = lines.clone();
        trade_icons(&state, &mut lines).await.unwrap();
        assert_eq!(
            lines[0].icon.as_deref(),
            Some("https://web.poecdn.com/gen/image/lonely.png")
        );
        assert!(
            lines[0].metadata["icon_source"]
                .as_str()
                .unwrap()
                .contains("official trade")
        );
        assert!(!lines[0].metadata.contains_key("icon_error"));
        assert_eq!(
            serde_json::to_value(&lines[1]).unwrap(),
            serde_json::to_value(&original[1]).unwrap()
        );
        assert!(lines[2..].iter().all(|line| line.icon.is_none()));
        for (line, mut expected) in lines.into_iter().zip(original) {
            expected.icon = line.icon.clone();
            expected.metadata = line.metadata.clone();
            assert_eq!(
                serde_json::to_value(line).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }

    #[test]
    fn exchange_rates_are_inverted_to_quote_units_and_metadata_resolves() {
        let query = OverviewQuery {
            league: "Standard".into(),
            category: "Currency".into(),
            source: "exchange".into(),
            date: None,
        };
        let body = json!({
            "core":{"primary":"chaos","rates":{"divine":0.005},"items":[{"id":"chaos","name":"Chaos Orb"},{"id":"divine","name":"Divine Orb"}]},
            "items":[{"id":"gcp","name":"Gemcutter's Prism","image":"/image.png"}],
            "lines":[{"id":"gcp","primaryValue":1.25,"volumePrimaryValue":100,"sparkline":{"totalChange":4,"data":[0,null,4]}}]
        });
        let overview = normalize(&query, &body, "2026-01-01T00:00:00Z".into()).unwrap();
        assert_eq!(overview.quote_currency, "Chaos Orb");
        assert_eq!(overview.rates["Divine Orb"], "200");
        assert_eq!(overview.lines[0].price.as_deref(), Some("1.25"));
        assert_eq!(overview.lines[0].sparkline[1], None);
        assert!(normalize(&query, &json!({"lines":[]}), String::new()).is_err());
    }

    #[test]
    fn historical_icons_require_unambiguous_metadata_and_do_not_change_price_identity() {
        let mut lines = vec![Line {
            id: "archive:a".into(),
            name: "Fireball".into(),
            price: Some("12".into()),
            ..Line::default()
        }];
        let mut metadata = vec![Line {
            name: "Fireball".into(),
            icon: Some("/gen/image/fire.png".into()),
            ..Line::default()
        }];
        enrich_icons(&mut lines, &metadata);
        assert_eq!(
            lines[0].icon.as_deref(),
            Some("https://web.poecdn.com/gen/image/fire.png")
        );
        assert_eq!(lines[0].id, "archive:a");
        assert_eq!(lines[0].price.as_deref(), Some("12"));
        assert!(lines[0].metadata.contains_key("icon_source"));
        lines[0].icon = None;
        metadata.push(Line {
            name: "Fireball".into(),
            icon: Some("/gen/image/other.png".into()),
            ..Line::default()
        });
        enrich_icons(&mut lines, &metadata);
        assert!(lines[0].icon.is_none());
    }

    #[test]
    fn historical_icons_match_saved_variants_instead_of_rejecting_all_artwork() {
        let mut lines = vec![
            Line {
                name: "Map (Tier 16)".into(),
                variant: Some(", Gen-24, 1-4 links".into()),
                ..Line::default()
            },
            Line {
                name: "Phantom Boots".into(),
                variant: Some("Elder/Hunter".into()),
                ..Line::default()
            },
            Line {
                name: "Storm Call".into(),
                variant: Some("20/20".into()),
                gem_level: Some(20),
                metadata: BTreeMap::from([("archive_item_id".into(), "4476".into())]),
                ..Line::default()
            },
        ];
        let metadata = vec![
            Line {
                name: "Map (Tier 16)".into(),
                variant: Some("Gen-24".into()),
                icon: Some("/map24.png".into()),
                ..Line::default()
            },
            Line {
                name: "Map (Tier 16)".into(),
                variant: Some("Gen-23".into()),
                icon: Some("/map23.png".into()),
                ..Line::default()
            },
            Line {
                name: "Phantom Boots".into(),
                variant: Some("Elder/Hunter".into()),
                icon: Some("/boots.png".into()),
                ..Line::default()
            },
            Line {
                name: "Phantom Boots".into(),
                icon: Some("/eldritch-boots.png".into()),
                ..Line::default()
            },
            Line {
                id: "4476:variant".into(),
                name: "Storm Call".into(),
                variant: Some("20/20".into()),
                gem_level: Some(20),
                icon: Some("/storm.png".into()),
                ..Line::default()
            },
            Line {
                id: "99:variant".into(),
                name: "Storm Call".into(),
                variant: Some("20/20".into()),
                gem_level: Some(20),
                icon: Some("/imbued-storm.png".into()),
                ..Line::default()
            },
        ];
        enrich_icons(&mut lines, &metadata);
        assert!(lines[0].icon.as_deref().unwrap().ends_with("/map24.png"));
        assert!(lines[1].icon.as_deref().unwrap().ends_with("/boots.png"));
        assert!(lines[2].icon.as_deref().unwrap().ends_with("/storm.png"));
    }

    #[test]
    fn malformed_provider_prices_and_unrepresentable_rates_are_rejected() {
        let query = OverviewQuery {
            league: "Standard".into(),
            category: "SkillGem".into(),
            source: "stash".into(),
            date: None,
        };
        for value in [json!(-1), json!("NaN"), json!(1e100)] {
            let body = json!({"lines":[{"id":1,"name":"Fireball","chaosValue":value}]});
            assert!(normalize(&query, &body, String::new()).is_err());
        }
        let body =
            json!({"lines":[{"id":1,"name":"Fireball","chaosValue":0.00001,"listingCount":-1}]});
        assert!(normalize(&query, &body, String::new()).is_err());
        let query = OverviewQuery {
            source: "exchange".into(),
            category: "Currency".into(),
            ..query
        };
        let body = json!({"core":{"primary":"chaos","rates":{"divine":1e100}},"lines":[]});
        assert!(normalize(&query, &body, String::new()).is_err());
    }

    #[test]
    fn item_variants_stay_distinct_even_if_upstream_ids_are_reused() {
        let query = OverviewQuery {
            league: "Standard".into(),
            category: "SkillGem".into(),
            source: "stash".into(),
            date: None,
        };
        let body = json!({"lines":[
            {"id":1,"name":"Fireball","chaosValue":40,"gemLevel":20,"gemQuality":20,"corrupted":false},
            {"id":1,"name":"Fireball","chaosValue":140,"gemLevel":21,"gemQuality":20,"corrupted":true}
        ]});
        let overview = normalize(&query, &body, String::new()).unwrap();
        assert_ne!(overview.lines[0].id, overview.lines[1].id);
        assert_eq!(overview.lines[1].corrupted, Some(true));
        assert!(
            validate(&OverviewQuery {
                date: Some("not a date".into()),
                ..query
            })
            .is_err()
        );
        assert_eq!(
            cache_age(Some("public,max-age=600")),
            Duration::from_secs(600)
        );
    }

    #[tokio::test]
    async fn shared_raw_cache_preserves_source_time_and_deduplicates_consumers() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("economy.sqlite3"))
            .await
            .unwrap();
        let (selected, _) = watch::channel(None);
        let state = ServiceState::new(store, "test", selected);
        let time = Utc::now();
        state.economy.raw.write().await.insert(
            ("Standard".into(), "SkillGem".into(), "stash".into()),
            RawOverview {
                body: json!({"lines":[]}),
                fetched_at: time,
                checked: Instant::now(),
                etag: Some("test".into()),
                max_age: Duration::from_secs(300),
            },
        );
        let (a, b) = tokio::join!(
            raw_overview(&state, "Standard", "SkillGem", "stash", true),
            raw_overview(&state, "Standard", "SkillGem", "stash", false)
        );
        assert_eq!(a.unwrap().fetched_at, time);
        assert_eq!(b.unwrap().fetched_at, time);
    }
}
