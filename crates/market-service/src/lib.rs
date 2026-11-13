use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, Method, StatusCode, header::CONTENT_TYPE},
    routing::get,
};
use chrono::Utc;
use faustus_fetch::{FaustusClient, FetchError as FaustusFetchError};
use market_core::{
    CurrentExchangePriceLine, CurrentExchangePriceOverview, ExchangeMarketSnapshot,
    ExchangePriceCurrency, GemAnalysisData, GemAnalysisOverview, GemMarketLine, GemMarketOverview,
    GemSnapshotOption, LeagueOption, ServiceStatus,
};
use market_store::{NinjaSnapshot, SqliteStore, StoreError};
use ninja_fetch::{CurrencyCategory, FetchError as NinjaFetchError, FetchOutcome, NinjaClient};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock, watch};
use tower_http::cors::CorsLayer;

const LEAGUE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const EXCHANGE_PRICE_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const GEM_OVERVIEW_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const EXCHANGE_PAGES_PER_BATCH: usize = 30;
const EXCHANGE_PAGE_DELAY: Duration = Duration::from_secs(1);
const EXCHANGE_BATCH_RETRY_DELAY: Duration = Duration::from_secs(60);
const SOURCE_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);
const GEM_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(15 * 60);
const SNAPSHOT_REFRESH_COOLDOWN: Duration = Duration::from_secs(60);

pub mod analysis;
pub mod economy;
pub mod ggg;

#[derive(Clone)]
pub struct ServiceState {
    store: SqliteStore,
    economy_store: SqliteStore,
    user_agent: Arc<str>,
    selected_league: watch::Sender<Option<String>>,
    leagues: Arc<RwLock<Option<CachedLeagues>>>,
    exchange_prices: Arc<RwLock<HashMap<(String, String), CachedExchangePrices>>>,
    gem_overviews: Arc<RwLock<HashMap<String, CachedGemOverview>>>,
    snapshot_captures: Arc<Mutex<HashMap<String, CachedCapture>>>,
    economy: Arc<economy::EconomyState>,
    ggg: Arc<ggg::GggState>,
}

#[derive(Clone)]
struct CachedCapture {
    checked_at: Instant,
    snapshot: Option<GemSnapshotOption>,
    data: GemAnalysisData,
}

#[derive(Clone)]
struct CachedLeagues {
    fetched_at: Instant,
    options: Vec<LeagueOption>,
}

#[derive(Clone)]
struct CachedExchangePrices {
    checked_at: Instant,
    overview: CurrentExchangePriceOverview,
}

#[derive(Clone)]
struct CachedGemOverview {
    checked_at: Instant,
    overview: GemMarketOverview,
}

impl ServiceState {
    pub fn new(
        store: SqliteStore,
        user_agent: impl Into<Arc<str>>,
        selected_league: watch::Sender<Option<String>>,
    ) -> Self {
        Self {
            economy_store: store.clone(),
            store,
            user_agent: user_agent.into(),
            selected_league,
            leagues: Arc::default(),
            exchange_prices: Arc::default(),
            gem_overviews: Arc::default(),
            snapshot_captures: Arc::default(),
            economy: Arc::default(),
            ggg: Arc::default(),
        }
    }

    pub fn with_economy_store(mut self, store: SqliteStore) -> Self {
        self.economy_store = store;
        self
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IngestSummary {
    pub exchange_cursor: Option<u64>,
    pub exchange_markets: usize,
    pub ninja_currency_lines: usize,
    pub ninja_fragment_lines: usize,
    pub ninja_not_modified: usize,
}

#[derive(Debug, Error)]
pub enum IngestError {
    #[error(transparent)]
    Faustus(#[from] FaustusFetchError),
    #[error(transparent)]
    Ninja(#[from] NinjaFetchError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub fn router(state: ServiceState) -> Router {
    Router::new()
        .merge(economy::routes())
        .merge(analysis::routes())
        .merge(ggg::routes())
        .route("/health", get(health))
        .route("/api/v1/leagues", get(leagues))
        .route("/api/v1/selection", axum::routing::post(select_league))
        .route("/api/v1/exchange/prices", get(exchange_prices))
        .route("/api/v1/gems", get(gem_overview))
        .route("/api/v1/gem-analysis", get(gem_analysis))
        .route(
            "/api/v1/gem-analysis/refresh",
            axum::routing::post(refresh_gem_analysis),
        )
        .route("/api/v1/status", get(status))
        .route("/api/v1/exchange/markets", get(exchange_markets))
        .with_state(state)
}

pub fn cors_layer(origins: Vec<HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([CONTENT_TYPE])
}

pub async fn ingest_once(
    store: &SqliteStore,
    league_id: &str,
    user_agent: &str,
) -> Result<IngestSummary, IngestError> {
    let faustus = FaustusClient::new(user_agent)?;
    let ninja = NinjaClient::new(user_agent)?;
    let exchange = record_next_exchange_page(store, &faustus).await?;
    let (ninja_currency_lines, ninja_fragment_lines, ninja_not_modified) =
        refresh_ninja_overviews(store, &ninja, league_id).await?;

    Ok(IngestSummary {
        exchange_cursor: Some(exchange.next_cursor),
        exchange_markets: exchange.market_count,
        ninja_currency_lines,
        ninja_fragment_lines,
        ninja_not_modified,
    })
}

struct RecordedExchangePage {
    next_cursor: u64,
    market_count: usize,
    caught_up: bool,
}

async fn record_next_exchange_page(
    store: &SqliteStore,
    client: &FaustusClient,
) -> Result<RecordedExchangePage, IngestError> {
    let requested_cursor = store.exchange_cursor().await?;
    let fetched_page = client.fetch_page(requested_cursor).await?;
    let result = RecordedExchangePage {
        next_cursor: fetched_page.page.next_change_id.value(),
        market_count: fetched_page.page.markets.len(),
        caught_up: fetched_page.page.is_caught_up(requested_cursor),
    };

    store
        .record_exchange_page(&fetched_page, Utc::now())
        .await?;
    Ok(result)
}

async fn refresh_ninja_overviews(
    store: &SqliteStore,
    client: &NinjaClient,
    league_id: &str,
) -> Result<(usize, usize, usize), IngestError> {
    let mut currency_lines = 0;
    let mut fragment_lines = 0;
    let mut not_modified = 0;

    for (category, line_count) in [
        (CurrencyCategory::Currency, &mut currency_lines),
        (CurrencyCategory::Fragment, &mut fragment_lines),
    ] {
        let category_name = match category {
            CurrencyCategory::Currency => "Currency",
            CurrencyCategory::Fragment => "Fragment",
        };
        let validators = store
            .ninja_cache_validators(league_id, category_name)
            .await?;
        let outcome = client
            .fetch_currency_overview(league_id, category, validators.etag.as_deref())
            .await?;

        match outcome {
            FetchOutcome::NotModified { metadata } => {
                not_modified += 1;
                store
                    .record_ninja_not_modified(
                        league_id,
                        category_name,
                        Utc::now(),
                        metadata.etag.as_deref(),
                        metadata.cache_control.as_deref(),
                    )
                    .await?;
            }
            FetchOutcome::Modified {
                body,
                raw_json,
                metadata,
            } => {
                *line_count = body.lines.len();
                store
                    .record_ninja_snapshot(NinjaSnapshot {
                        league_id,
                        category: category_name,
                        fetched_at: Utc::now(),
                        raw_json: &raw_json,
                        etag: metadata.etag.as_deref(),
                        cache_control: metadata.cache_control.as_deref(),
                    })
                    .await?;
            }
        }
    }

    Ok((currency_lines, fragment_lines, not_modified))
}

fn retry_delay(error: &IngestError) -> Duration {
    match error {
        IngestError::Faustus(FaustusFetchError::HttpStatus {
            retry_after: Some(seconds),
            ..
        }) => seconds
            .parse::<u64>()
            .map(Duration::from_secs)
            .unwrap_or(EXCHANGE_BATCH_RETRY_DELAY),
        _ => EXCHANGE_BATCH_RETRY_DELAY,
    }
}

pub async fn run_periodic_collector(
    store: SqliteStore,
    mut selected_league: watch::Receiver<Option<String>>,
    user_agent: String,
) {
    let faustus = match FaustusClient::new(&user_agent) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Unable to initialize GGG collector: {error}");
            return;
        }
    };
    let ninja = match NinjaClient::new(&user_agent) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Unable to initialize poe.ninja collector: {error}");
            return;
        }
    };

    'selection: loop {
        let Some(league_id) = wait_for_selection(&mut selected_league).await else {
            return;
        };
        let mut ninja_refresh_due = true;
        let mut last_ninja_refresh = Instant::now();

        loop {
            let mut caught_up = false;
            let mut backoff = EXCHANGE_BATCH_RETRY_DELAY;

            for page_index in 0..EXCHANGE_PAGES_PER_BATCH {
                match record_next_exchange_page(&store, &faustus).await {
                    Ok(page) => {
                        caught_up = page.caught_up;
                        eprintln!(
                            "Recorded GGG cursor {} with {} market rows{}",
                            page.next_cursor,
                            page.market_count,
                            if caught_up { " (caught up)" } else { "" }
                        );
                    }
                    Err(error) => {
                        backoff = retry_delay(&error);
                        eprintln!("GGG page collection failed: {error}");
                        break;
                    }
                }

                if caught_up {
                    break;
                }
                if page_index + 1 < EXCHANGE_PAGES_PER_BATCH {
                    tokio::select! {
                        _ = tokio::time::sleep(EXCHANGE_PAGE_DELAY) => {}
                        changed = selected_league.changed() => {
                            if changed.is_err() {
                                return;
                            }
                            continue 'selection;
                        }
                    }
                }
            }

            if ninja_refresh_due || last_ninja_refresh.elapsed() >= SOURCE_REFRESH_INTERVAL {
                match refresh_ninja_overviews(&store, &ninja, &league_id).await {
                    Ok((currency_lines, fragment_lines, not_modified)) => eprintln!(
                        "Refreshed poe.ninja for {league_id}: currency={currency_lines}, fragments={fragment_lines}, unchanged={not_modified}"
                    ),
                    Err(error) => eprintln!("poe.ninja refresh failed: {error}"),
                }
                ninja_refresh_due = false;
                last_ninja_refresh = Instant::now();
            }

            let delay = if caught_up {
                SOURCE_REFRESH_INTERVAL.saturating_sub(last_ninja_refresh.elapsed())
            } else {
                backoff
            };
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                changed = selected_league.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue 'selection;
                }
            }
        }
    }
}

async fn wait_for_selection(
    selected_league: &mut watch::Receiver<Option<String>>,
) -> Option<String> {
    loop {
        if let Some(league_id) = selected_league.borrow_and_update().clone() {
            return Some(league_id);
        }
        selected_league.changed().await.ok()?;
    }
}

pub async fn run_periodic_gem_collector(
    state: ServiceState,
    selected_league: watch::Receiver<Option<String>>,
) {
    collect_gem_snapshots(state, selected_league, GEM_SNAPSHOT_INTERVAL).await;
}

async fn collect_gem_snapshots(
    state: ServiceState,
    mut selected_league: watch::Receiver<Option<String>>,
    interval: Duration,
) {
    'selection: loop {
        let Some(league) = wait_for_selection(&mut selected_league).await else {
            return;
        };
        loop {
            tokio::select! {
                result = gem_analysis_inner(state.clone(), GemAnalysisQuery {
                    league: league.clone(), snapshot: None,
                }, true) => {
                    match result {
                        Ok(Json(overview)) => {
                            if let Some(error) = &overview.data.gem_error {
                                eprintln!("Gem snapshot collection for {league}: {error}");
                            }
                            if let Some(error) = &overview.data.price_error {
                                eprintln!("Gem snapshot material collection for {league}: {error}");
                            }
                        }
                        Err(error) => eprintln!("Gem snapshot collection for {league} failed: {error}"),
                    }
                }
                changed = selected_league.changed() => {
                    if changed.is_err() { return; }
                    continue 'selection;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                changed = selected_league.changed() => {
                    if changed.is_err() { return; }
                    continue 'selection;
                }
            }
        }
    }
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn leagues(State(state): State<ServiceState>) -> Result<Json<Vec<LeagueOption>>, StatusCode> {
    let cached = state.leagues.read().await.clone();
    if let Some(cached) = cached
        && cached.fetched_at.elapsed() < LEAGUE_CACHE_TTL
    {
        return Ok(Json(cached.options));
    }

    let client = NinjaClient::new(&state.user_agent).map_err(|_| StatusCode::BAD_GATEWAY)?;
    let outcome = client
        .fetch_leagues(None)
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let options = match outcome {
        FetchOutcome::Modified { body, .. } => body
            .into_iter()
            .map(|league| LeagueOption {
                id: league.id,
                name: league.name,
            })
            .collect::<Vec<_>>(),
        FetchOutcome::NotModified { .. } => {
            return Err(StatusCode::BAD_GATEWAY);
        }
    };

    *state.leagues.write().await = Some(CachedLeagues {
        fetched_at: Instant::now(),
        options: options.clone(),
    });
    Ok(Json(options))
}

#[derive(Debug, Deserialize)]
struct SelectLeagueRequest {
    id: String,
}

async fn select_league(
    State(state): State<ServiceState>,
    Json(selection): Json<SelectLeagueRequest>,
) -> Result<Json<LeagueOption>, StatusCode> {
    let options = leagues(State(state.clone())).await?.0;
    let selected = options
        .into_iter()
        .find(|league| league.id == selection.id)
        .ok_or(StatusCode::BAD_REQUEST)?;
    state
        .selected_league
        .send_replace(Some(selected.id.clone()));
    Ok(Json(selected))
}

async fn status(State(state): State<ServiceState>) -> Result<Json<ServiceStatus>, StatusCode> {
    let exchange_cursor = state
        .store
        .exchange_cursor()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let exchange_pages = state
        .store
        .exchange_page_count()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let exchange_markets = state
        .store
        .exchange_market_count()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let ninja_snapshots = state
        .store
        .ninja_snapshot_count()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let exchange_cursor = exchange_cursor.map(|cursor| cursor.value());
    let exchange_cursor_at = exchange_cursor
        .and_then(|cursor| i64::try_from(cursor).ok())
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0));

    Ok(Json(ServiceStatus {
        exchange_cursor,
        exchange_cursor_at,
        exchange_pages,
        exchange_markets,
        ninja_snapshots,
        selected_league: state.selected_league.borrow().clone(),
    }))
}

#[derive(Debug, Deserialize)]
struct ExchangeMarketsQuery {
    league: Option<String>,
    limit: Option<i64>,
}

async fn exchange_markets(
    State(state): State<ServiceState>,
    Query(query): Query<ExchangeMarketsQuery>,
) -> Result<Json<Vec<ExchangeMarketSnapshot>>, StatusCode> {
    let markets = state
        .store
        .latest_exchange_markets(query.league.as_deref(), query.limit.unwrap_or(100))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(markets))
}

#[derive(Debug, Deserialize)]
struct ExchangePricesQuery {
    league: String,
    #[serde(rename = "type")]
    category: String,
}

async fn exchange_prices(
    State(state): State<ServiceState>,
    Query(query): Query<ExchangePricesQuery>,
) -> Result<Json<CurrentExchangePriceOverview>, StatusCode> {
    load_exchange_prices(state, query, false).await
}

async fn load_exchange_prices(
    state: ServiceState,
    query: ExchangePricesQuery,
    refresh: bool,
) -> Result<Json<CurrentExchangePriceOverview>, StatusCode> {
    let category = match query.category.as_str() {
        "Currency" => CurrencyCategory::Currency,
        "Fragment" => CurrencyCategory::Fragment,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let options = leagues(State(state.clone())).await?.0;
    if !options.iter().any(|league| league.id == query.league) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let key = (query.league.clone(), query.category.clone());
    let cached = state.exchange_prices.read().await.get(&key).cloned();
    if let Some(cached) = &cached
        && !refresh
        && cached.checked_at.elapsed() < EXCHANGE_PRICE_CACHE_TTL
    {
        return Ok(Json(cached.overview.clone()));
    }

    let _ = category;
    let raw = economy::raw_overview(&state, &query.league, &query.category, "exchange", refresh)
        .await
        .map_err(|error| error.0)?;
    let body = serde_json::from_value(raw.body).map_err(|error| {
        eprintln!("Invalid exchange overview: {error}");
        StatusCode::BAD_GATEWAY
    })?;
    let mut overview = current_price_overview(&query.league, &query.category, body);
    overview.fetched_at = raw.fetched_at;
    let entry = CachedExchangePrices {
        checked_at: Instant::now(),
        overview,
    };
    let overview = entry.overview.clone();
    state.exchange_prices.write().await.insert(key, entry);
    Ok(Json(overview))
}

#[derive(Debug, Deserialize)]
struct GemOverviewQuery {
    league: String,
}

async fn gem_overview(
    State(state): State<ServiceState>,
    Query(query): Query<GemOverviewQuery>,
) -> Result<Json<GemMarketOverview>, StatusCode> {
    load_gem_overview(state, query, false).await
}

async fn load_gem_overview(
    state: ServiceState,
    query: GemOverviewQuery,
    refresh: bool,
) -> Result<Json<GemMarketOverview>, StatusCode> {
    let options = leagues(State(state.clone())).await?.0;
    if !options.iter().any(|league| league.id == query.league) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let cached = state.gem_overviews.read().await.get(&query.league).cloned();
    if let Some(cached) = &cached
        && !refresh
        && cached.checked_at.elapsed() < GEM_OVERVIEW_CACHE_TTL
    {
        return Ok(Json(cached.overview.clone()));
    }

    let raw = economy::raw_overview(&state, &query.league, "SkillGem", "stash", refresh)
        .await
        .map_err(|error| error.0)?;
    let body = serde_json::from_value(raw.body).map_err(|error| {
        eprintln!("Invalid gem overview: {error}");
        StatusCode::BAD_GATEWAY
    })?;
    let mut overview = gem_market_overview(&query.league, body);
    overview.fetched_at = raw.fetched_at;
    let entry = CachedGemOverview {
        checked_at: Instant::now(),
        overview,
    };
    let overview = entry.overview.clone();
    state
        .gem_overviews
        .write()
        .await
        .insert(query.league, entry);
    Ok(Json(overview))
}

#[derive(Debug, Deserialize)]
struct GemAnalysisQuery {
    league: String,
    snapshot: Option<i64>,
}

async fn gem_analysis(
    State(state): State<ServiceState>,
    Query(query): Query<GemAnalysisQuery>,
) -> Result<Json<GemAnalysisOverview>, StatusCode> {
    gem_analysis_inner(state, query, false).await
}

async fn refresh_gem_analysis(
    State(state): State<ServiceState>,
    Query(query): Query<GemOverviewQuery>,
) -> Result<Json<GemAnalysisOverview>, StatusCode> {
    gem_analysis_inner(
        state,
        GemAnalysisQuery {
            league: query.league,
            snapshot: None,
        },
        true,
    )
    .await
}

async fn gem_analysis_inner(
    state: ServiceState,
    query: GemAnalysisQuery,
    refresh: bool,
) -> Result<Json<GemAnalysisOverview>, StatusCode> {
    let (snapshot, data) = if let Some(id) = query.snapshot {
        if id <= 0 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let (snapshot, data) = state
            .store
            .gem_analysis_snapshot(&query.league, id)
            .await
            .map_err(|error| {
                eprintln!("Failed to read gem snapshot: {error}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .ok_or(StatusCode::NOT_FOUND)?;
        (Some(snapshot), data)
    } else {
        let mut captures = state.snapshot_captures.lock().await;
        let recent = captures
            .get(&query.league)
            .filter(|capture| capture.checked_at.elapsed() < SNAPSHOT_REFRESH_COOLDOWN)
            .cloned();
        if refresh && let Some(recent) = recent {
            (recent.snapshot, recent.data)
        } else {
            let (gems, prices) = tokio::join!(
                load_gem_overview(
                    state.clone(),
                    GemOverviewQuery {
                        league: query.league.clone()
                    },
                    refresh,
                ),
                load_exchange_prices(
                    state.clone(),
                    ExchangePricesQuery {
                        league: query.league.clone(),
                        category: "Currency".to_owned(),
                    },
                    refresh,
                ),
            );
            let (gems, gem_error) = match gems {
                Ok(Json(gems)) => (Some(gems), None),
                Err(error) => (
                    None,
                    Some(format!("Gem prices returned HTTP {}", error.as_u16())),
                ),
            };
            let (prices, price_error) = match prices {
                Ok(Json(prices)) => (Some(prices), None),
                Err(error) => (
                    None,
                    Some(format!("Material prices returned HTTP {}", error.as_u16())),
                ),
            };
            let data = GemAnalysisData {
                gems,
                gem_error,
                prices,
                price_error,
            };
            let snapshot = if data.gems.is_some() {
                Some(
                    state
                        .store
                        .record_gem_analysis_snapshot(&query.league, &data, Utc::now())
                        .await
                        .map_err(|error| {
                            eprintln!("Failed to record gem snapshot: {error}");
                            StatusCode::INTERNAL_SERVER_ERROR
                        })?,
                )
            } else {
                None
            };
            if refresh {
                captures.insert(
                    query.league.clone(),
                    CachedCapture {
                        checked_at: Instant::now(),
                        snapshot: snapshot.clone(),
                        data: data.clone(),
                    },
                );
            }
            (snapshot, data)
        }
    };
    let mut snapshots = state
        .store
        .gem_analysis_snapshots(&query.league)
        .await
        .map_err(|error| {
            eprintln!("Failed to list gem snapshots: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if let Some(selected) = &snapshot
        && !snapshots.iter().any(|entry| entry.id == selected.id)
    {
        snapshots.push(selected.clone());
    }
    Ok(Json(GemAnalysisOverview {
        snapshot,
        data,
        snapshots,
    }))
}

fn gem_market_overview(league: &str, overview: ninja_fetch::ItemOverview) -> GemMarketOverview {
    GemMarketOverview {
        league: league.to_owned(),
        fetched_at: Utc::now(),
        gems: overview
            .lines
            .into_iter()
            .map(|line| GemMarketLine {
                name: line.name,
                gem_kind: line.metadata.and_then(|metadata| metadata.gem_kind),
                variant: line.variant,
                gem_level: line.gem_level,
                gem_quality: line.gem_quality,
                corrupted: line.corrupted.unwrap_or(false),
                chaos_value: line.chaos_value.map(|value| value.to_string()),
                sample_count: line.count,
                listing_count: line.listing_count,
                sparkline_change: line
                    .spark_line
                    .as_ref()
                    .and_then(|sparkline| sparkline.total_change.as_ref())
                    .map(ToString::to_string),
                sparkline: line
                    .spark_line
                    .map(|sparkline| {
                        sparkline
                            .data
                            .into_iter()
                            .map(|value| value.map(|value| value.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect(),
    }
}

fn current_price_overview(
    league: &str,
    category: &str,
    overview: ninja_fetch::ExchangeOverview,
) -> CurrentExchangePriceOverview {
    let item_by_id = overview
        .items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let core_item_by_id = overview
        .core
        .items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let currency = |id: &str| {
        let item = core_item_by_id.get(id).copied();
        ExchangePriceCurrency {
            id: id.to_owned(),
            name: item.map_or_else(|| id.to_owned(), |item| item.name.clone()),
            image: item.and_then(|item| item.image.clone()),
        }
    };

    CurrentExchangePriceOverview {
        league: league.to_owned(),
        category: category.to_owned(),
        fetched_at: Utc::now(),
        primary: currency(&overview.core.primary),
        secondary: overview.core.secondary.as_deref().map(currency),
        lines: overview
            .lines
            .into_iter()
            .map(|line| {
                let item = item_by_id.get(line.id.as_str()).copied();
                let max_volume_currency = line.max_volume_currency.map(|id| {
                    item_by_id
                        .get(id.as_str())
                        .or_else(|| core_item_by_id.get(id.as_str()))
                        .copied()
                        .map_or(id.clone(), |item| item.name.clone())
                });
                CurrentExchangePriceLine {
                    id: line.id.clone(),
                    name: item.map_or(line.id, |item| item.name.clone()),
                    item_category: item.and_then(|item| item.category.clone()),
                    image: item.and_then(|item| item.image.clone()),
                    primary_value: line.primary_value.map(|value| value.to_string()),
                    volume_primary_value: line.volume_primary_value.map(|value| value.to_string()),
                    max_volume_currency,
                    max_volume_rate: line.max_volume_rate.map(|value| value.to_string()),
                    sparkline_change: line
                        .sparkline
                        .as_ref()
                        .and_then(|sparkline| sparkline.total_change.as_ref())
                        .map(ToString::to_string),
                    sparkline: line
                        .sparkline
                        .map(|sparkline| {
                            sparkline
                                .data
                                .into_iter()
                                .map(|value| value.map(|value| value.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    async fn cached_analysis_state() -> (tempfile::TempDir, ServiceState) {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("market.sqlite3"))
            .await
            .unwrap();
        let (selected_league, _) = watch::channel(None);
        let state = ServiceState::new(store, "test/0.1", selected_league);
        *state.leagues.write().await = Some(CachedLeagues {
            fetched_at: Instant::now(),
            options: vec![LeagueOption {
                id: "Standard".to_owned(),
                name: "Standard".to_owned(),
            }],
        });
        state.gem_overviews.write().await.insert("Standard".to_owned(), CachedGemOverview {
            checked_at: Instant::now(),
            overview: gem_market_overview("Standard", serde_json::from_str(
                r#"{"lines":[{"id":1,"name":"Fireball","chaosValue":2,"gemLevel":1,"gemQuality":0,"count":12,"listingCount":12}]}"#,
            ).unwrap()),
        });
        state.exchange_prices.write().await.insert(
            ("Standard".to_owned(), "Currency".to_owned()),
            CachedExchangePrices {
                checked_at: Instant::now(),
                overview: current_price_overview("Standard", "Currency", serde_json::from_str(
                    r#"{"core":{"primary":"chaos","rates":{},"items":[{"id":"chaos","name":"Chaos Orb"}]},
                        "lines":[{"id":"gcp","primaryValue":1}],"items":[{"id":"gcp","name":"Gemcutter's Prism"}]}"#,
                ).unwrap()),
            },
        );
        (directory, state)
    }

    async fn analysis_response(app: Router, uri: &str) -> GemAnalysisOverview {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
    }

    async fn refresh_response(state: &ServiceState) -> GemAnalysisOverview {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/gem-analysis/refresh?league=Standard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn explicit_refresh_bypasses_feed_ttls_and_coalesces_attempts_including_failures() {
        let (_directory, mut state) = cached_analysis_state().await;
        // An invalid user-agent makes any attempted provider access fail without network requests.
        state.user_agent = "\n".into();
        let cached = analysis_response(
            router(state.clone()),
            "/api/v1/gem-analysis?league=Standard",
        )
        .await;
        assert!(cached.data.gems.is_some());
        assert!(cached.data.prices.is_some());
        let (first, concurrent) = tokio::join!(refresh_response(&state), refresh_response(&state));
        assert!(first.data.gems.is_none());
        assert!(first.data.prices.is_none());
        assert!(first.data.gem_error.is_some());
        assert!(first.data.price_error.is_some());
        assert!(first.snapshot.is_none());
        assert_eq!(concurrent.data, first.data);
        let checked_at = state.snapshot_captures.lock().await["Standard"].checked_at;
        let repeated = refresh_response(&state).await;
        assert_eq!(repeated.data, first.data);
        assert_eq!(
            state.snapshot_captures.lock().await["Standard"].checked_at,
            checked_at
        );
        state
            .snapshot_captures
            .lock()
            .await
            .get_mut("Standard")
            .unwrap()
            .checked_at = Instant::now() - SNAPSHOT_REFRESH_COOLDOWN;
        refresh_response(&state).await;
        assert!(state.snapshot_captures.lock().await["Standard"].checked_at > checked_at);
        assert_eq!(
            state
                .store
                .gem_analysis_snapshots("Standard")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn collector_waits_for_selection_retries_and_switches_leagues_without_a_client() {
        assert_eq!(GEM_SNAPSHOT_INTERVAL, Duration::from_secs(900));
        let (_directory, mut state) = cached_analysis_state().await;
        state.user_agent = "\n".into();
        let (selection, receiver) = watch::channel(None);
        let collector = tokio::spawn(collect_gem_snapshots(
            state.clone(),
            receiver,
            Duration::from_millis(10),
        ));
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(state.snapshot_captures.lock().await.is_empty());
        selection.send_replace(Some("Standard".to_owned()));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state
                    .snapshot_captures
                    .lock()
                    .await
                    .contains_key("Standard")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let first = state.snapshot_captures.lock().await["Standard"].checked_at;
        state
            .snapshot_captures
            .lock()
            .await
            .get_mut("Standard")
            .unwrap()
            .checked_at = Instant::now() - SNAPSHOT_REFRESH_COOLDOWN;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.snapshot_captures.lock().await["Standard"].checked_at > first {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        selection.send_replace(Some("Other".to_owned()));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.snapshot_captures.lock().await.contains_key("Other") {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(selection);
        tokio::time::timeout(Duration::from_secs(2), collector)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn analysis_route_records_deduplicates_and_reads_immutable_snapshots_without_provider_access()
     {
        let (_directory, mut state) = cached_analysis_state().await;
        let initial = analysis_response(
            router(state.clone()),
            "/api/v1/gem-analysis?league=Standard",
        )
        .await;
        let first = initial.snapshot.clone().unwrap();
        assert_eq!(initial.snapshots.len(), 1);
        let repeated = analysis_response(
            router(state.clone()),
            "/api/v1/gem-analysis?league=Standard",
        )
        .await;
        assert_eq!(repeated.snapshot, initial.snapshot);
        assert_eq!(repeated.snapshots.len(), 1);
        state
            .gem_overviews
            .write()
            .await
            .get_mut("Standard")
            .unwrap()
            .overview
            .gems[0]
            .chaos_value = Some("20".to_owned());
        state
            .exchange_prices
            .write()
            .await
            .get_mut(&("Standard".to_owned(), "Currency".to_owned()))
            .unwrap()
            .overview
            .lines[0]
            .primary_value = Some("9".to_owned());
        let latest = analysis_response(
            router(state.clone()),
            "/api/v1/gem-analysis?league=Standard",
        )
        .await;
        assert_ne!(latest.snapshot, initial.snapshot);
        assert_eq!(latest.snapshots[0], latest.snapshot.unwrap());
        assert_eq!(latest.snapshots[1], first);

        state.user_agent = "\n".into();
        *state.leagues.write().await = None;
        state.gem_overviews.write().await.clear();
        state.exchange_prices.write().await.clear();
        let historical = analysis_response(
            router(state),
            &format!("/api/v1/gem-analysis?league=Standard&snapshot={}", first.id),
        )
        .await;
        assert_eq!(historical.data, initial.data);
        assert_eq!(historical.snapshot, Some(first));
        assert_eq!(
            historical.data.prices.unwrap().lines[0]
                .primary_value
                .as_deref(),
            Some("1")
        );
    }

    #[tokio::test]
    async fn analysis_route_preserves_independent_provider_errors_without_substituting_current_or_saved_prices()
     {
        for fail_gems in [true, false] {
            let (_directory, mut state) = cached_analysis_state().await;
            state.user_agent = "\n".into();
            if fail_gems {
                state
                    .gem_overviews
                    .write()
                    .await
                    .get_mut("Standard")
                    .unwrap()
                    .checked_at = Instant::now() - GEM_OVERVIEW_CACHE_TTL - Duration::from_secs(1);
            } else {
                state
                    .exchange_prices
                    .write()
                    .await
                    .get_mut(&("Standard".to_owned(), "Currency".to_owned()))
                    .unwrap()
                    .checked_at =
                    Instant::now() - EXCHANGE_PRICE_CACHE_TTL - Duration::from_secs(1);
            }
            let overview = analysis_response(
                router(state.clone()),
                "/api/v1/gem-analysis?league=Standard",
            )
            .await;
            if fail_gems {
                assert!(overview.data.gems.is_none());
                assert!(
                    overview
                        .data
                        .gem_error
                        .as_deref()
                        .unwrap()
                        .contains("HTTP 502")
                );
                assert!(overview.data.prices.is_some());
                assert!(overview.snapshot.is_none());
                assert!(overview.snapshots.is_empty());
            } else {
                assert!(overview.data.gems.is_some());
                assert!(overview.data.prices.is_none());
                assert!(
                    overview
                        .data
                        .price_error
                        .as_deref()
                        .unwrap()
                        .contains("HTTP 502")
                );
                let saved = analysis_response(
                    router(state),
                    &format!(
                        "/api/v1/gem-analysis?league=Standard&snapshot={}",
                        overview.snapshot.unwrap().id,
                    ),
                )
                .await;
                assert_eq!(saved.data, overview.data);
            }
        }
    }

    #[tokio::test]
    async fn analysis_route_rejects_missing_cross_league_and_invalid_snapshot_ids() {
        let (_directory, state) = cached_analysis_state().await;
        let initial = analysis_response(
            router(state.clone()),
            "/api/v1/gem-analysis?league=Standard",
        )
        .await;
        let id = initial.snapshot.unwrap().id;
        for (uri, status) in [
            (
                format!("/api/v1/gem-analysis?league=Other&snapshot={id}"),
                StatusCode::NOT_FOUND,
            ),
            (
                "/api/v1/gem-analysis?league=Standard&snapshot=999999".to_owned(),
                StatusCode::NOT_FOUND,
            ),
            (
                "/api/v1/gem-analysis?league=Standard&snapshot=0".to_owned(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/v1/gem-analysis?league=Standard&snapshot=-1".to_owned(),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/api/v1/gem-analysis?league=Standard&snapshot=invalid".to_owned(),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let response = router(state.clone())
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status);
        }
    }

    #[tokio::test]
    async fn analysis_route_keeps_an_older_pinned_instant_beyond_the_recent_menu_limit() {
        let (_directory, state) = cached_analysis_state().await;
        let initial = analysis_response(
            router(state.clone()),
            "/api/v1/gem-analysis?league=Standard",
        )
        .await;
        let first = initial.snapshot.unwrap();
        let mut data = initial.data;
        for index in 0..200 {
            data.gems.as_mut().unwrap().gems[0].chaos_value = Some((index + 3).to_string());
            state
                .store
                .record_gem_analysis_snapshot(
                    "Standard",
                    &data,
                    first.captured_at + chrono::Duration::seconds(index + 1),
                )
                .await
                .unwrap();
        }
        assert_eq!(
            state
                .store
                .gem_analysis_snapshots("Standard")
                .await
                .unwrap()
                .len(),
            200
        );
        let saved = analysis_response(
            router(state),
            &format!("/api/v1/gem-analysis?league=Standard&snapshot={}", first.id,),
        )
        .await;
        assert_eq!(saved.snapshots.len(), 201);
        assert_eq!(saved.snapshots.last(), Some(&first));
        assert_eq!(saved.snapshot, Some(first));
    }

    #[tokio::test]
    async fn cors_allows_only_configured_origins_and_json_get_post() {
        let app = Router::new()
            .route("/health", get(health))
            .layer(cors_layer(vec![
                HeaderValue::from_static("http://tauri.localhost"),
                HeaderValue::from_static("https://market.example"),
            ]));

        for origin in [
            "http://tauri.localhost",
            "https://market.example",
            "https://untrusted.example",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("OPTIONS")
                        .uri("/health")
                        .header("origin", origin)
                        .header("access-control-request-method", "POST")
                        .header("access-control-request-headers", "content-type")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            if origin == "https://untrusted.example" {
                assert!(
                    response
                        .headers()
                        .get("access-control-allow-origin")
                        .is_none()
                );
            } else {
                assert_eq!(response.headers()["access-control-allow-origin"], origin);
                assert_eq!(
                    response.headers()["access-control-allow-methods"],
                    "GET,POST"
                );
                assert_eq!(
                    response.headers()["access-control-allow-headers"],
                    "content-type"
                );
            }
            assert!(
                response
                    .headers()
                    .get("access-control-allow-credentials")
                    .is_none()
            );
        }

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "http://tauri.localhost")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "http://tauri.localhost"
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn current_price_mapping_resolves_item_and_currency_names() {
        let response: ninja_fetch::ExchangeOverview = serde_json::from_str(
            r#"{
                "core": {
                    "primary": "chaos",
                    "secondary": "divine",
                    "rates": {},
                    "items": [
                        {"id":"chaos","name":"Chaos Orb"},
                        {"id":"divine","name":"Divine Orb"}
                    ]
                },
                "lines": [{
                    "id":"accelerating-catalyst",
                    "primaryValue":10.47,
                    "volumePrimaryValue":104.7,
                    "maxVolumeCurrency":"divine",
                    "maxVolumeRate":60,
                    "sparkline":{"totalChange":12.5,"data":[1,2]}
                }],
                "items": [{
                    "id":"accelerating-catalyst",
                    "name":"Accelerating Catalyst",
                    "category":"Catalysts"
                }]
            }"#,
        )
        .unwrap();

        let mapped = current_price_overview("Standard", "Currency", response);

        assert_eq!(mapped.primary.name, "Chaos Orb");
        assert_eq!(mapped.secondary.as_ref().unwrap().name, "Divine Orb");
        assert_eq!(mapped.lines[0].name, "Accelerating Catalyst");
        assert_eq!(
            mapped.lines[0].max_volume_currency.as_deref(),
            Some("Divine Orb")
        );
        assert_eq!(mapped.lines[0].primary_value.as_deref(), Some("10.47"));
        assert_eq!(mapped.lines[0].sparkline_change.as_deref(), Some("12.5"));
    }

    #[tokio::test]
    async fn gem_route_returns_cached_standard_variants() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("market.sqlite3"))
            .await
            .unwrap();
        let (selected_league, _) = watch::channel(None);
        let state = ServiceState::new(store, "test/0.1", selected_league);
        let now = Instant::now();
        *state.leagues.write().await = Some(CachedLeagues {
            fetched_at: now,
            options: vec![LeagueOption {
                id: "Standard".to_owned(),
                name: "Standard".to_owned(),
            }],
        });
        state.gem_overviews.write().await.insert(
            "Standard".to_owned(),
            CachedGemOverview {
                checked_at: now,
                overview: GemMarketOverview {
                    league: "Standard".to_owned(),
                    fetched_at: Utc::now(),
                    gems: vec![GemMarketLine {
                        name: "Cleave of Rage".to_owned(),
                        gem_kind: Some("transfigured".to_owned()),
                        variant: Some("20/20".to_owned()),
                        gem_level: Some(20),
                        gem_quality: Some(20),
                        corrupted: false,
                        chaos_value: Some("10".to_owned()),
                        sample_count: Some(12),
                        listing_count: Some(12),
                        sparkline_change: Some("15".to_owned()),
                        sparkline: vec![Some("0".to_owned()), Some("15".to_owned())],
                    }],
                },
            },
        );

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/gems?league=Standard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let overview: GemMarketOverview = serde_json::from_slice(&body).unwrap();
        assert_eq!(overview.league, "Standard");
        assert_eq!(overview.gems[0].variant.as_deref(), Some("20/20"));
        assert_eq!(overview.gems[0].chaos_value.as_deref(), Some("10"));
        assert_eq!(overview.gems[0].gem_kind.as_deref(), Some("transfigured"));
    }

    #[test]
    fn gem_mapping_preserves_provider_categories_without_guessing_from_names() {
        let source: ninja_fetch::ItemOverview = serde_json::from_str(
            r#"{"lines": [
                {"id": 1, "name": "Empower Support", "metadata": {"gemKind": "exceptional"}},
                {"id": 2, "name": "Cleave of Rage", "metadata": {"gemKind": "transfigured"}},
                {"id": 3, "name": "Sunder of Trarthus", "metadata": {"gemKind": "trarthus"}},
                {"id": 4, "name": "Purity of Fire"},
                {"id": 5, "name": "Future Gem", "metadata": {"gemKind": "future"}}
            ]}"#,
        )
        .unwrap();
        let mapped = gem_market_overview("Standard", source);
        assert_eq!(mapped.gems[0].gem_kind.as_deref(), Some("exceptional"));
        assert_eq!(mapped.gems[1].gem_kind.as_deref(), Some("transfigured"));
        assert_eq!(mapped.gems[2].gem_kind.as_deref(), Some("trarthus"));
        assert_eq!(mapped.gems[3].gem_kind, None);
        assert_eq!(mapped.gems[4].gem_kind.as_deref(), Some("future"));
    }

    #[tokio::test]
    async fn read_routes_return_empty_database_state() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("market.sqlite3"))
            .await
            .unwrap();
        let (selected_league, _) = watch::channel(None);
        let state = ServiceState::new(store, "test/0.1", selected_league);
        let app = router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let status_json = std::str::from_utf8(&body).unwrap();
        assert!(status_json.contains("\"exchange_pages\":0"));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/exchange/markets?league=Standard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(body.as_ref(), b"[]");
    }

    #[tokio::test]
    async fn collector_waits_for_client_selection_and_receives_current_league() {
        let (sender, mut receiver) = watch::channel(None);
        let waiting = tokio::spawn(async move { wait_for_selection(&mut receiver).await });

        sender.send(Some("CurrentLeague".to_owned())).unwrap();

        assert_eq!(waiting.await.unwrap().as_deref(), Some("CurrentLeague"));
    }

    #[tokio::test]
    async fn client_can_list_and_select_only_a_cached_economy_league() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("market.sqlite3"))
            .await
            .unwrap();
        let (selected_league, selected_league_rx) = watch::channel(None);
        let state = ServiceState::new(store, "test/0.1", selected_league);
        *state.leagues.write().await = Some(CachedLeagues {
            fetched_at: Instant::now(),
            options: vec![
                LeagueOption {
                    id: "CurrentSoftcore".to_owned(),
                    name: "Current Challenge League".to_owned(),
                },
                LeagueOption {
                    id: "Standard".to_owned(),
                    name: "Standard".to_owned(),
                },
            ],
        });
        let app = router(state);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/leagues")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let leagues: Vec<LeagueOption> = serde_json::from_slice(&body).unwrap();
        assert_eq!(leagues[0].id, "CurrentSoftcore");

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/selection")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":"CurrentSoftcore"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            selected_league_rx.borrow().as_deref(),
            Some("CurrentSoftcore")
        );
    }
}
