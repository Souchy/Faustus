use std::collections::{BTreeMap, HashMap, HashSet};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use market_core::{
    analysis::{
        Arbitrage, ArbitrageLoop, Excluded, FlipRow, Flips, Observation, PairAlternative, Period,
        SparkPoint, Sparkline, Sparklines,
    },
    economy,
    ggg::{Item, Market},
};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::ServiceState;

const CHAOS: &str = "Metadata/Items/Currency/CurrencyRerollRare";
type ApiError = (StatusCode, String);

pub fn routes() -> Router<ServiceState> {
    Router::new()
        .route("/api/v1/analysis/flips", get(flips))
        .route("/api/v1/analysis/period", get(period))
        .route("/api/v1/analysis/sparklines", post(sparklines))
        .route("/api/v1/analysis/arbitrage", get(arbitrage))
}

fn failure(error: impl std::fmt::Display) -> ApiError {
    eprintln!("Market analysis: {error}");
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn validate_hour(league: &str, hour: i64) -> Result<(), ApiError> {
    if league.trim().is_empty() || league.len() > 100 || league.chars().any(char::is_control) {
        return Err((StatusCode::BAD_REQUEST, "Invalid league.".into()));
    }
    let current_hour = Utc::now().timestamp().div_euclid(3600) * 3600;
    if hour < 0 || hour % 3600 != 0 || hour >= current_hour {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose a completed UTC hour (Unix seconds, aligned to 3600).".into(),
        ));
    }
    Ok(())
}

fn filter_decimal(value: Option<&str>, label: &str) -> Result<Option<Decimal>, ApiError> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| {
            let parsed = value.parse::<Decimal>().map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("{label} must be a nonnegative decimal."),
                )
            })?;
            if parsed < Decimal::ZERO {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("{label} cannot be negative."),
                ));
            }
            Ok(parsed)
        })
        .transpose()
}

fn number(value: Decimal) -> String {
    value.normalize().to_string()
}

fn positive(value: &str) -> Option<Decimal> {
    value
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value > Decimal::ZERO)
}

fn confidence_rank(value: &str) -> Option<u8> {
    match value {
        "Low" => Some(1),
        "Medium" => Some(2),
        "High" => Some(3),
        _ => None,
    }
}

fn growth(start: Decimal, end: Decimal) -> Option<(Decimal, Decimal)> {
    let absolute = end.checked_sub(start)?;
    let percent = absolute
        .checked_div(start)?
        .checked_mul(Decimal::ONE_HUNDRED)?;
    Some((absolute, percent))
}

fn utc_hour(hour: i64) -> String {
    DateTime::<Utc>::from_timestamp(hour, 0)
        .expect("validated timestamp")
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[derive(Deserialize)]
struct FlipQuery {
    league: String,
    start: i64,
    end: i64,
    min_price: Option<String>,
    max_price: Option<String>,
    min_volume: Option<String>,
    min_confidence: Option<String>,
    #[serde(default)]
    strict_start: bool,
    #[serde(default)]
    offset: usize,
    #[serde(default = "page_size")]
    limit: usize,
    sort: Option<String>,
    search: Option<String>,
    category: Option<String>,
    venue: Option<String>,
}

fn page_size() -> usize {
    100
}

#[derive(Deserialize)]
struct SparkQuery {
    league: String,
    start: i64,
    end: i64,
    rows: Vec<FlipRow>,
}

fn sample_hours(start: i64, end: i64, daily: bool) -> Vec<i64> {
    let step = if daily { 86400 } else { 3600 };
    let start = start.div_euclid(step) * step;
    let end = end.div_euclid(step) * step;
    let intervals = (end - start) / step;
    let count = (intervals + 1).min(60);
    (0..count)
        .map(|index| start + index * intervals / (count - 1).max(1) * step)
        .collect()
}

async fn sparklines(
    State(state): State<ServiceState>,
    Json(query): Json<SparkQuery>,
) -> Result<Json<Sparklines>, ApiError> {
    validate_hour(&query.league, query.start)?;
    validate_hour(&query.league, query.end)?;
    if query.end <= query.start || query.rows.len() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose an increasing period and at most 100 displayed rows.".into(),
        ));
    }
    let mut ids = HashSet::new();
    for row in &query.rows {
        if row.id.len() > 2048
            || !ids.insert(&row.id)
            || row.pair.len() > 512
            || row
                .history_item
                .as_ref()
                .is_some_and(|item| item.len() > 2048)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid or duplicate sparkline row.".into(),
            ));
        }
        let valid = if row.source == "GGG Exchange" {
            row.id.starts_with("ggg:")
                && row
                    .alternatives
                    .iter()
                    .filter(|pair| pair.selected && pair.market.len() <= 2048)
                    .count()
                    == 1
        } else {
            matches!(row.source.as_str(), "poe.ninja dump" | "poe.ninja snapshot")
                && matches!(row.history_source.as_deref(), Some("stash" | "exchange"))
                && matches!(
                    row.history_granularity.as_deref(),
                    Some("daily" | "snapshot")
                )
                && row.history_item.is_some()
                && row.history_category.is_some()
                && row
                    .history_category
                    .as_ref()
                    .is_some_and(|category| category.len() <= 64)
        };
        if !valid {
            return Err((
                StatusCode::BAD_REQUEST,
                "Sparkline row has no supported history identity.".into(),
            ));
        }
    }
    let mut ggg = HashMap::new();
    if query.rows.iter().any(|row| row.source == "GGG Exchange") {
        for hour in sample_hours(query.start, query.end, false) {
            let markets = state
                .economy_store
                .ggg_markets(&query.league, hour)
                .await
                .map_err(failure)?;
            let quotes = quotes(&markets);
            let rates = chaos_rates(&quotes);
            ggg.insert(hour, (quotes, rates));
        }
    }
    let mut conversions =
        HashMap::<(String, String, Option<String>, String), Vec<SparkPoint>>::new();
    let mut result = Vec::new();
    for row in query.rows {
        let mut warnings = vec!["At most 60 evenly spaced saved hours/days; gaps at sampled observations are not interpolated. Unsampled fluctuations and gaps are not represented.".into()];
        let points = if row.source == "GGG Exchange" {
            let selected = row
                .alternatives
                .iter()
                .find(|pair| pair.selected)
                .expect("validated pair");
            let id = row.id.strip_prefix("ggg:").unwrap();
            sample_hours(query.start, query.end, false)
                .into_iter()
                .map(|hour| {
                    let price = ggg.get(&hour).and_then(|(quotes, rates)| {
                        let pair = quotes
                            .get(id)?
                            .iter()
                            .find(|pair| pair.market == selected.market)?;
                        pair.rate
                            .checked_mul(*rates.get(&pair.quote.id)?)
                            .map(number)
                    });
                    SparkPoint { time: hour, price }
                })
                .collect::<Vec<_>>()
        } else {
            let category = row.history_category.as_deref().unwrap();
            let source = row.history_source.as_deref().unwrap();
            let item = row.history_item.as_deref().unwrap();
            let granularity = row.history_granularity.as_deref().unwrap();
            let hours = sample_hours(query.start, query.end, granularity == "daily");
            let mut prices = state
                .economy_store
                .analysis_price_samples(
                    &query.league,
                    category,
                    source,
                    item,
                    row.history_archive.as_deref(),
                    granularity,
                    &row.pair,
                    &hours,
                )
                .await
                .map_err(failure)?;
            if row.pair != "Chaos Orb" {
                let key = (
                    source.to_owned(),
                    row.pair.clone(),
                    row.history_archive.clone(),
                    granularity.to_owned(),
                );
                if !conversions.contains_key(&key) {
                    let conversion = if let Some(item) = state
                        .economy_store
                        .analysis_currency_item(&query.league, source, &row.pair)
                        .await
                        .map_err(failure)?
                    {
                        state
                            .economy_store
                            .analysis_price_samples(
                                &query.league,
                                "Currency",
                                source,
                                &item,
                                row.history_archive.as_deref(),
                                granularity,
                                "Chaos Orb",
                                &hours,
                            )
                            .await
                            .map_err(failure)?
                    } else {
                        hours
                            .iter()
                            .map(|time| SparkPoint {
                                time: *time,
                                price: None,
                            })
                            .collect()
                    };
                    conversions.insert(key.clone(), conversion);
                }
                for (price, rate) in prices.iter_mut().zip(conversions.get(&key).unwrap()) {
                    price.price = price
                        .price
                        .as_deref()
                        .and_then(positive)
                        .zip(rate.price.as_deref().and_then(positive))
                        .and_then(|(price, rate)| price.checked_mul(rate))
                        .map(number);
                }
            } else {
                for price in &mut prices {
                    price.price = price.price.as_deref().and_then(positive).map(number);
                }
            }
            prices
        };
        if points.iter().any(|point| point.price.is_none()) {
            warnings.push(
                "Some sampled observations or contemporaneous Chaos conversions are unavailable."
                    .into(),
            );
        }
        result.push(Sparkline {
            id: row.id,
            points,
            warnings,
        });
    }
    Ok(Json(Sparklines { series: result }))
}

#[derive(Deserialize)]
struct PeriodQuery {
    league: String,
}

async fn period(
    State(state): State<ServiceState>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<Period>, ApiError> {
    validate_hour(&query.league, 0)?;
    state
        .economy_store
        .analysis_period(&query.league)
        .await
        .map(Json)
        .map_err(failure)
}

async fn flips(
    State(state): State<ServiceState>,
    Query(query): Query<FlipQuery>,
) -> Result<Json<Flips>, ApiError> {
    validate_hour(&query.league, query.start)?;
    validate_hour(&query.league, query.end)?;
    if query.end <= query.start {
        return Err((
            StatusCode::BAD_REQUEST,
            "Choose an increasing period.".into(),
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Page size must be between 1 and 100.".into(),
        ));
    }
    let sort = query.sort.as_deref().unwrap_or("percent-desc");
    if !matches!(
        sort,
        "percent-desc" | "percent-asc" | "value-desc" | "value-asc"
    ) {
        return Err((StatusCode::BAD_REQUEST, "Unsupported growth sort.".into()));
    }
    if query
        .venue
        .as_deref()
        .is_some_and(|venue| !matches!(venue, "" | "exchange" | "trade"))
    {
        return Err((StatusCode::BAD_REQUEST, "Unsupported trading venue.".into()));
    }
    let minimum = filter_decimal(query.min_price.as_deref(), "Minimum starting price")?;
    let maximum = filter_decimal(query.max_price.as_deref(), "Maximum starting price")?;
    let min_volume = filter_decimal(query.min_volume.as_deref(), "Minimum exchange item volume")?
        .unwrap_or(Decimal::ZERO);
    let min_confidence = query
        .min_confidence
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|value| {
            confidence_rank(value).ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "Minimum confidence must be Low, Medium or High.".into(),
                )
            })
        })
        .transpose()?;
    if minimum.zip(maximum).is_some_and(|(min, max)| min > max) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Minimum starting price exceeds maximum.".into(),
        ));
    }
    let period = state
        .economy_store
        .analysis_period(&query.league)
        .await
        .map_err(failure)?;
    if period.start.is_some_and(|start| query.start < start)
        || period.end.is_some_and(|end| query.end > end)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Selected period is outside this league's known date bounds.".into(),
        ));
    }
    let (
        mut start_markets,
        end_markets,
        mut start_observations,
        mut end_observations,
        start_complete,
        end_complete,
    ) = tokio::try_join!(
        state.economy_store.ggg_markets(&query.league, query.start),
        state.economy_store.ggg_markets(&query.league, query.end),
        state
            .economy_store
            .analysis_prices(&query.league, query.start),
        state
            .economy_store
            .analysis_prices(&query.league, query.end),
        state
            .economy_store
            .ggg_hour_complete(&query.league, query.start),
        state
            .economy_store
            .ggg_hour_complete(&query.league, query.end),
    )
    .map_err(failure)?;
    let start_context = if query.strict_start {
        StartContext::default()
    } else {
        first_in_period_starts(
            &state,
            &query,
            &mut start_markets,
            &end_markets,
            &mut start_observations,
            &end_observations,
        )
        .await?
    };
    enrich_cluster_observations(
        &state,
        &query.league,
        &mut start_observations,
        &mut end_observations,
    )
    .await?;
    let mut result = build_flips_with_starts(
        &query.league,
        query.start,
        query.end,
        &start_markets,
        &end_markets,
        &start_observations,
        &end_observations,
        min_volume,
        &start_context,
    );
    if !start_complete || !end_complete {
        result.warnings.push(if query.strict_start {
            "GGG selected-hour coverage is incomplete. Strict start date requires prices in the selected start and end hours; missing hours are not substituted.".into()
        } else {
            "GGG selected-hour coverage is incomplete. Starts use the first saved traded price inside the period; missing imported hours can hide earlier trading. Ending prices still require the selected end hour.".into()
        });
    }
    result.rows.retain(|row| {
        let price = row
            .start_price
            .parse::<Decimal>()
            .expect("calculated decimal");
        minimum.is_none_or(|min| price >= min) && maximum.is_none_or(|max| price <= max)
    });
    if let Some(minimum) = min_confidence {
        state
            .economy_store
            .analysis_confidence(&query.league, &mut result.rows)
            .await
            .map_err(failure)?;
        result.rows.retain(|row| {
            row.source == "GGG Exchange"
                || [
                    row.start_confidence.as_deref(),
                    row.end_confidence.as_deref(),
                ]
                .into_iter()
                .any(|confidence| {
                    confidence
                        .and_then(confidence_rank)
                        .is_some_and(|rank| rank >= minimum)
                })
        });
        result.warnings.push("The minimum provider-confidence filter applies only to poe.ninja/archive rows. Either actual endpoint can meet the threshold; Unknown cannot qualify by itself. GGG rows remain included without a provider-confidence requirement. Filters do not use period averages.".into());
    }
    result.categories = result
        .rows
        .iter()
        .map(|row| row.category.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let search = query.search.as_deref().unwrap_or("").trim().to_lowercase();
    result.rows.retain(|row| {
        (search.is_empty() || row.name.to_lowercase().contains(&search))
            && query
                .category
                .as_deref()
                .is_none_or(|category| category.is_empty() || category == row.category)
            && query
                .venue
                .as_deref()
                .is_none_or(|venue| venue.is_empty() || venue == row.venue)
    });
    result.rows.sort_by(|a, b| {
        let value = |row: &FlipRow| {
            if sort.starts_with("percent") {
                row.growth_percent.parse::<Decimal>().unwrap()
            } else {
                row.growth.parse::<Decimal>().unwrap()
            }
        };
        let order = value(a).cmp(&value(b));
        (if sort.ends_with("desc") {
            order.reverse()
        } else {
            order
        })
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.id.cmp(&b.id))
    });
    result.total = result.rows.len();
    result.offset = query.offset;
    result.limit = query.limit;
    result.excluded_total = result.excluded.len();
    result.excluded.truncate(100);
    result.rows = result
        .rows
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();
    if min_confidence.is_none() {
        state
            .economy_store
            .analysis_confidence(&query.league, &mut result.rows)
            .await
            .map_err(failure)?;
    }
    Ok(Json(result))
}

#[derive(Default)]
struct StartContext {
    ggg_hours: HashMap<String, i64>,
    ggg_rates: HashMap<(i64, String), Decimal>,
    ninja_rates: HashMap<(FeedKey, String, String), Option<Decimal>>,
}

async fn first_in_period_starts(
    state: &ServiceState,
    query: &FlipQuery,
    starts: &mut Vec<Market>,
    ends: &[Market],
    ninja_starts: &mut Vec<Observation>,
    ninja_ends: &[Observation],
) -> Result<StartContext, ApiError> {
    let mut context = StartContext::default();
    let first_hours = state
        .economy_store
        .ggg_first_trade_hours(&query.league, query.start, query.end, ends)
        .await
        .map_err(failure)?;
    let mut by_hour = BTreeMap::<i64, HashSet<String>>::new();
    for (pair, hour) in first_hours {
        if hour > query.start {
            by_hour.entry(hour).or_default().insert(pair);
        }
    }
    for (hour, pairs) in by_hour {
        let markets = state
            .economy_store
            .ggg_markets(&query.league, hour)
            .await
            .map_err(failure)?;
        for (currency, rate) in chaos_rates(&quotes(&markets)) {
            context.ggg_rates.insert((hour, currency), rate);
        }
        starts.retain(|market| !pairs.contains(&market.id));
        for market in markets
            .into_iter()
            .filter(|market| pairs.contains(&market.id))
        {
            context.ggg_hours.insert(market.id.clone(), hour);
            starts.push(market);
        }
    }
    let existing = ninja_starts
        .iter()
        .filter(|row| positive(&row.price).is_some())
        .map(observation_key)
        .collect::<HashSet<_>>();
    let missing = ninja_ends
        .iter()
        .filter(|row| !existing.contains(&observation_key(row)))
        .cloned()
        .collect::<Vec<_>>();
    let firsts = state
        .economy_store
        .analysis_first_prices(&query.league, query.start, query.end, &missing)
        .await
        .map_err(failure)?;
    let mut conversions =
        BTreeMap::<(String, Option<String>, String, String), Vec<&Observation>>::new();
    for first in &firsts {
        conversions
            .entry((
                first.source.clone(),
                first.archive.clone(),
                first.granularity.clone(),
                first.currency.clone(),
            ))
            .or_default()
            .push(first);
    }
    for ((source, archive, granularity, currency), observations) in conversions {
        let hours = observations
            .iter()
            .map(|row| observation_hour(row))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?
            .into_iter()
            .collect::<Vec<_>>();
        let rates = if currency == "Chaos Orb" {
            hours
                .iter()
                .map(|hour| (*hour, Some(Decimal::ONE)))
                .collect::<HashMap<_, _>>()
        } else if let Some(item) = state
            .economy_store
            .analysis_currency_item(&query.league, &source, &currency)
            .await
            .map_err(failure)?
        {
            state
                .economy_store
                .analysis_price_samples(
                    &query.league,
                    "Currency",
                    &source,
                    &item,
                    archive.as_deref(),
                    &granularity,
                    "Chaos Orb",
                    &hours,
                )
                .await
                .map_err(failure)?
                .into_iter()
                .map(|point| (point.time, point.price.as_deref().and_then(positive)))
                .collect()
        } else {
            HashMap::new()
        };
        for first in observations {
            context.ninja_rates.insert(
                (feed_key(first), first.observed_at.clone(), currency.clone()),
                rates.get(&observation_hour(first)?).copied().flatten(),
            );
        }
    }
    let replaced = firsts.iter().map(observation_key).collect::<HashSet<_>>();
    ninja_starts.retain(|row| !replaced.contains(&observation_key(row)));
    ninja_starts.extend(firsts);
    Ok(context)
}

fn observation_hour(row: &Observation) -> Result<i64, ApiError> {
    let timestamp = if row.granularity == "daily" {
        format!("{}T00:00:00Z", row.observed_at)
    } else {
        row.observed_at.clone()
    };
    DateTime::parse_from_rfc3339(&timestamp)
        .map(|at| at.timestamp().div_euclid(3600) * 3600)
        .map_err(failure)
}

async fn enrich_cluster_observations(
    state: &ServiceState,
    league: &str,
    starts: &mut [Observation],
    ends: &mut [Observation],
) -> Result<(), ApiError> {
    if !starts
        .iter()
        .chain(ends.iter())
        .any(|row| row.category == "ClusterJewel")
    {
        return Ok(());
    }
    let items = state
        .economy_store
        .analysis_cluster_items(league)
        .await
        .map_err(failure)?;
    let metadata = state
        .economy_store
        .current_economy("Standard", "ClusterJewel", "stash")
        .await
        .map_err(failure)?;
    let mut resolved = HashMap::new();
    for mut line in items {
        if line
            .metadata
            .get("levelRequired")
            .and_then(serde_json::Value::as_u64)
            .is_none()
        {
            let provider_id = line
                .metadata
                .get("archive_item_id")
                .and_then(serde_json::Value::as_str);
            let levels = metadata
                .iter()
                .flat_map(|overview| &overview.lines)
                .filter(|candidate| {
                    provider_id.is_some_and(|id| candidate.id.split(':').next() == Some(id))
                        && candidate.name == line.name
                        && candidate.variant == line.variant
                        && candidate.base_type == line.base_type
                })
                .filter_map(|candidate| {
                    candidate
                        .metadata
                        .get("levelRequired")
                        .and_then(serde_json::Value::as_u64)
                })
                .collect::<std::collections::BTreeSet<_>>();
            if levels.len() == 1 {
                line.metadata
                    .insert("levelRequired".into(), (*levels.first().unwrap()).into());
            }
        }
        resolved.insert(line.id.clone(), line);
    }
    for observation in starts
        .iter_mut()
        .chain(ends.iter_mut())
        .filter(|row| row.category == "ClusterJewel")
    {
        if let Some(line) = resolved.get(&observation.line.id) {
            observation.line.metadata = line.metadata.clone();
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Quote {
    item: Item,
    quote: Item,
    market: String,
    rate: Decimal,
    volume: Decimal,
}

fn quotes(markets: &[Market]) -> BTreeMap<String, Vec<Quote>> {
    let mut result = BTreeMap::<String, Vec<Quote>>::new();
    for market in markets {
        let Some(a) = positive(&market.volume_a) else {
            continue;
        };
        let Some(b) = positive(&market.volume_b) else {
            continue;
        };
        for (item, quote, amount, paid) in
            [(&market.a, &market.b, a, b), (&market.b, &market.a, b, a)]
        {
            if let Some(rate) = paid.checked_div(amount) {
                result.entry(item.id.clone()).or_default().push(Quote {
                    item: item.clone(),
                    quote: quote.clone(),
                    market: market.id.clone(),
                    rate,
                    volume: amount,
                });
            }
        }
    }
    result
}

fn chaos_rates(quotes: &BTreeMap<String, Vec<Quote>>) -> HashMap<String, Decimal> {
    let mut rates = HashMap::from([(CHAOS.to_owned(), Decimal::ONE)]);
    for (id, entries) in quotes {
        if let Some(pair) = entries.iter().find(|pair| pair.quote.id == CHAOS) {
            rates.insert(id.clone(), pair.rate);
        }
    }
    rates
}

fn ggg_name_key(item: &Item) -> String {
    item.name.trim().to_lowercase()
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn build_flips(
    league: &str,
    start: i64,
    end: i64,
    start_markets: &[Market],
    end_markets: &[Market],
    start_ninja: &[Observation],
    end_ninja: &[Observation],
    min_volume: Decimal,
) -> Flips {
    build_flips_with_starts(
        league,
        start,
        end,
        start_markets,
        end_markets,
        start_ninja,
        end_ninja,
        min_volume,
        &StartContext::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn build_flips_with_starts(
    league: &str,
    start: i64,
    end: i64,
    start_markets: &[Market],
    end_markets: &[Market],
    start_ninja: &[Observation],
    end_ninja: &[Observation],
    min_volume: Decimal,
    context: &StartContext,
) -> Flips {
    let mut result = Flips {
        league: league.into(), start, end, rows: Vec::new(), excluded: Vec::new(),
        total: 0, offset: 0, limit: 100, excluded_total: 0, categories: Vec::new(),
        warnings: vec!["Historical gross price growth, not executable profit. GGG uses traded hourly averages; archive prices are daily estimates with unknown liquidity. Converted quotes use each endpoint's direct Chaos cross-rate.".into()],
    };
    let starts = quotes(start_markets);
    let ends = quotes(end_markets);
    let original_markets = start_markets
        .iter()
        .filter(|market| !context.ggg_hours.contains_key(&market.id))
        .cloned()
        .collect::<Vec<_>>();
    let start_rates = chaos_rates(&quotes(&original_markets));
    let end_rates = chaos_rates(&ends);
    let start_label = DateTime::from_timestamp(start, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M UTC")
        .to_string();
    let end_label = DateTime::from_timestamp(end, 0)
        .unwrap()
        .format("%Y-%m-%d %H:%M UTC")
        .to_string();
    let start_items = start_markets
        .iter()
        .flat_map(|market| [&market.a.id, &market.b.id])
        .collect::<HashSet<_>>();
    let end_items = end_markets
        .iter()
        .flat_map(|market| [&market.a.id, &market.b.id])
        .collect::<HashSet<_>>();
    let mut ggg_identities = HashSet::new();
    let items = start_markets
        .iter()
        .chain(end_markets)
        .flat_map(|market| [&market.a, &market.b])
        .map(|item| (item.id.clone(), item.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut name_counts = HashMap::<String, usize>::new();
    for item in items.values() {
        *name_counts.entry(ggg_name_key(item)).or_default() += 1;
    }
    for (id, item) in items {
        let mut alternatives = Vec::<(PairAlternative, Decimal, bool)>::new();
        let mut shared_pairs = 0;
        let mut volume_eligible_pairs = 0;
        let mut converted_pairs = 0;
        let mut missing_conversions = std::collections::BTreeSet::new();
        for first in starts.get(&id).into_iter().flatten() {
            let Some(last) = ends
                .get(&id)
                .and_then(|entries| entries.iter().find(|pair| pair.market == first.market))
            else {
                continue;
            };
            shared_pairs += 1;
            if first.volume < min_volume && last.volume < min_volume {
                continue;
            }
            volume_eligible_pairs += 1;
            let first_hour = context
                .ggg_hours
                .get(&first.market)
                .copied()
                .unwrap_or(start);
            let start_conversion = if first_hour == start {
                start_rates.get(&first.quote.id)
            } else {
                context.ggg_rates.get(&(first_hour, first.quote.id.clone()))
            };
            let end_conversion = end_rates.get(&first.quote.id);
            if start_conversion.is_none() {
                missing_conversions.insert(format!(
                    "{} at start ({})",
                    first.quote.name,
                    utc_hour(first_hour)
                ));
            }
            if end_conversion.is_none() {
                missing_conversions.insert(format!("{} at end ({end_label})", first.quote.name));
            }
            let (Some(start_conversion), Some(end_conversion)) = (start_conversion, end_conversion)
            else {
                continue;
            };
            converted_pairs += 1;
            let Some(start_price) = first.rate.checked_mul(*start_conversion) else {
                continue;
            };
            let Some(end_price) = last.rate.checked_mul(*end_conversion) else {
                continue;
            };
            let Some((absolute, percent)) = growth(start_price, end_price) else {
                continue;
            };
            alternatives.push((
                PairAlternative {
                    market: first.market.clone(),
                    quote: first.quote.name.clone(),
                    native_start: number(first.rate),
                    native_end: number(last.rate),
                    start_price: number(start_price),
                    end_price: number(end_price),
                    growth: number(absolute),
                    growth_percent: number(percent),
                    start_volume: number(first.volume),
                    end_volume: number(last.volume),
                    selected: false,
                    start_observed_at: utc_hour(first_hour),
                    end_observed_at: utc_hour(end),
                },
                first.volume.min(last.volume),
                first.quote.id == CHAOS,
            ));
        }
        alternatives.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.0.market.cmp(&b.0.market))
        });
        let Some((reference, _, _)) = alternatives.first_mut() else {
            let reason = if !start_items.contains(&id) {
                format!("No market observation in the selected start hour ({start_label}).")
            } else if !end_items.contains(&id) {
                format!("No market observation in the selected end hour ({end_label}).")
            } else if !starts.contains_key(&id) {
                format!(
                    "No usable traded price in the start hour ({start_label}): item and quote traded volumes must both be positive. This is not a minimum-volume filter."
                )
            } else if !ends.contains_key(&id) {
                format!(
                    "No usable traded price in the end hour ({end_label}): item and quote traded volumes must both be positive. This is not a minimum-volume filter."
                )
            } else if shared_pairs == 0 {
                format!(
                    "No identical traded pair in the start hour ({start_label}) and end hour ({end_label}). Different quote pairs are not spliced together."
                )
            } else if volume_eligible_pairs == 0 {
                format!(
                    "All {shared_pairs} shared pairs fail your configured minimum of {} item units: neither starting nor ending observation reaches it.",
                    number(min_volume)
                )
            } else if converted_pairs == 0 {
                format!(
                    "Missing direct quote-to-Chaos conversion: {}.",
                    missing_conversions
                        .into_iter()
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            } else {
                "Price or growth arithmetic exceeds supported precision.".into()
            };
            result.excluded.push(Excluded {
                name: item.name.clone(),
                reason: format!("GGG: {reason}"),
            });
            continue;
        };
        reference.selected = true;
        let reference = reference.clone();
        if name_counts.get(&ggg_name_key(&item)) == Some(&1) {
            ggg_identities.insert(ggg_name_key(&item));
        }
        result.rows.push(FlipRow {
            id: format!("ggg:{id}"),
            name: item.name,
            category: item.category,
            venue: "exchange".into(),
            source: "GGG Exchange".into(),
            start_price: reference.start_price,
            end_price: reference.end_price,
            growth: reference.growth,
            growth_percent: reference.growth_percent,
            start_observed_at: utc_hour(
                context
                    .ggg_hours
                    .get(&reference.market)
                    .copied()
                    .unwrap_or(start),
            ),
            end_observed_at: utc_hour(end),
            pair: reference.quote,
            native_start: reference.native_start,
            native_end: reference.native_end,
            history_category: None,
            history_source: None,
            history_item: None,
            history_archive: None,
            history_granularity: None,
            start_confidence: None,
            end_confidence: None,
            alternatives: alternatives
                .into_iter()
                .map(|(alternative, _, _)| alternative)
                .collect(),
        });
    }
    add_ninja_rows(
        &mut result,
        start_ninja,
        end_ninja,
        &ggg_identities,
        min_volume,
        context,
    );
    if result.rows.is_empty() {
        result.warnings.push("No comparable prices with a start inside the selected period and a price at the selected end. Import GGG hours or a matching poe.ninja archive.".into());
    }
    result
}

fn observation_key(observation: &Observation) -> (String, String, String, String, String, String) {
    (
        observation.category.clone(),
        observation.source.clone(),
        observation.line.id.clone(),
        observation.archive.clone().unwrap_or_default(),
        observation.granularity.clone(),
        observation.currency.clone(),
    )
}

fn simple_name(observation: &Observation) -> Option<String> {
    let line = &observation.line;
    if line.variant.is_some()
        || line.gem_level.is_some()
        || line.gem_quality.is_some()
        || line.corrupted.is_some()
        || line.links.is_some()
        || line.base_type.is_some()
    {
        None
    } else {
        Some(line.name.trim().to_lowercase())
    }
}

fn cluster_level_range(line: &economy::Line) -> Option<(u64, u64)> {
    match line
        .metadata
        .get("levelRequired")
        .and_then(serde_json::Value::as_u64)?
    {
        1 => Some((1, 49)),
        50 => Some((50, 67)),
        68 => Some((68, 74)),
        75 => Some((75, 83)),
        84 => Some((84, 100)),
        _ => None,
    }
}

fn item_label(category: &str, line: &economy::Line) -> String {
    let mut details = Vec::new();
    if let Some(variant) = &line.variant {
        details.push(variant.clone());
    }
    if let Some(level) = line.gem_level {
        details.push(format!("level {level}"));
    }
    if let Some(quality) = line.gem_quality {
        details.push(format!("quality {quality}"));
    }
    if line.corrupted == Some(true) {
        details.push("corrupted".into());
    }
    if let Some(links) = line.links {
        details.push(format!("{links} links"));
    }
    if let Some(tier) = line.map_tier {
        details.push(format!("tier {tier}"));
    }
    if let Some(base_type) = &line.base_type {
        details.push(base_type.clone());
    }
    if category == "ClusterJewel" {
        details.push(match cluster_level_range(line) {
            Some((minimum, maximum)) => format!("item level {minimum}-{maximum}"),
            None => "item level unknown".into(),
        });
    }
    if details.is_empty() {
        line.name.clone()
    } else {
        format!("{} ({})", line.name, details.join(", "))
    }
}

type FeedKey = (String, Option<String>, String);

fn feed_key(observation: &Observation) -> FeedKey {
    (
        observation.source.clone(),
        observation.archive.clone(),
        observation.granularity.clone(),
    )
}

fn rate_key(observation: &Observation) -> (FeedKey, String) {
    let length = if observation.granularity == "daily" {
        10
    } else {
        13
    };
    (
        feed_key(observation),
        observation.observed_at.chars().take(length).collect(),
    )
}

fn ninja_rates(
    observations: &[Observation],
) -> HashMap<(FeedKey, String), HashMap<String, Decimal>> {
    let mut result = HashMap::<(FeedKey, String), HashMap<String, Decimal>>::new();
    for observation in observations {
        let rates = result
            .entry(rate_key(observation))
            .or_insert_with(|| HashMap::from([("Chaos Orb".into(), Decimal::ONE)]));
        if observation.currency == "Chaos Orb"
            && observation.category == "Currency"
            && let Some(price) = positive(&observation.price)
        {
            rates.insert(observation.line.name.clone(), price);
        }
    }
    result
}

fn add_ninja_rows(
    result: &mut Flips,
    starts: &[Observation],
    ends: &[Observation],
    ggg_identities: &HashSet<String>,
    min_volume: Decimal,
    context: &StartContext,
) {
    if min_volume > Decimal::ZERO && (!starts.is_empty() || !ends.is_empty()) {
        result.warnings.push("The minimum item-volume filter applies only to GGG observations. poe.ninja/archive liquidity is unknown; those items remain included without a volume guarantee.".into());
    }
    let start_by_key = starts
        .iter()
        .map(|observation| (observation_key(observation), observation))
        .collect::<BTreeMap<_, _>>();
    let end_by_key = ends
        .iter()
        .map(|observation| (observation_key(observation), observation))
        .collect::<BTreeMap<_, _>>();
    let keys = start_by_key
        .keys()
        .chain(end_by_key.keys())
        .cloned()
        .collect::<HashSet<_>>();
    let start_rates = ninja_rates(starts);
    let end_rates = ninja_rates(ends);
    let exchange_categories = economy::categories()
        .into_iter()
        .filter(|category| category.sources.iter().any(|source| source == "exchange"))
        .map(|category| category.id)
        .collect::<HashSet<_>>();
    let mut candidates = Vec::new();
    let mut unidentified_cluster_ids = HashSet::new();
    for key in keys {
        let first = start_by_key.get(&key);
        let last = end_by_key.get(&key);
        let observation = first.or(last).expect("key exists");
        if simple_name(observation).is_some_and(|name| ggg_identities.contains(&name)) {
            continue;
        }
        let name = item_label(&observation.category, &observation.line);
        let (Some(first), Some(last)) = (first, last) else {
            result.excluded.push(Excluded { name, reason: "poe.ninja: no usable start before the ending observation inside the selected period, or no selected-end price in the same feed/archive.".into() });
            continue;
        };
        let Some(start_native) = positive(&first.price) else {
            result.excluded.push(Excluded {
                name,
                reason: "poe.ninja: invalid or zero starting price.".into(),
            });
            continue;
        };
        let Some(end_native) = positive(&last.price) else {
            result.excluded.push(Excluded {
                name,
                reason: "poe.ninja: invalid or zero ending price.".into(),
            });
            continue;
        };
        let start_rate = context
            .ninja_rates
            .get(&(
                feed_key(first),
                first.observed_at.clone(),
                first.currency.clone(),
            ))
            .copied()
            .unwrap_or_else(|| {
                start_rates
                    .get(&rate_key(first))
                    .and_then(|rates| rates.get(&first.currency))
                    .copied()
            });
        let converted = start_rate
            .as_ref()
            .zip(
                end_rates
                    .get(&rate_key(last))
                    .and_then(|rates| rates.get(&last.currency)),
            )
            .and_then(|(a, b)| Some((start_native.checked_mul(*a)?, end_native.checked_mul(*b)?)));
        let Some((start_price, end_price)) = converted else {
            result.excluded.push(Excluded {
                name,
                reason: "poe.ninja: no contemporaneous Chaos conversion in the same feed/archive."
                    .into(),
            });
            continue;
        };
        let Some((absolute, percent)) = growth(start_price, end_price) else {
            result.excluded.push(Excluded {
                name,
                reason: "Price arithmetic exceeds supported precision.".into(),
            });
            continue;
        };
        let venue = if exchange_categories.contains(&first.category) {
            "exchange"
        } else {
            "trade"
        };
        let id = format!(
            "ninja:{}:{}:{}:{}",
            first.category,
            first.source,
            first.archive.as_deref().unwrap_or("snapshot"),
            first.line.id
        );
        if first.category == "ClusterJewel" && cluster_level_range(&first.line).is_none() {
            unidentified_cluster_ids.insert(id.clone());
        }
        candidates.push(FlipRow {
            id,
            name,
            category: first.category.clone(),
            venue: venue.into(),
            source: if first.archive.is_some() {
                "poe.ninja dump"
            } else {
                "poe.ninja snapshot"
            }
            .into(),
            start_price: number(start_price),
            end_price: number(end_price),
            growth: number(absolute),
            growth_percent: number(percent),
            start_observed_at: first.observed_at.clone(),
            end_observed_at: last.observed_at.clone(),
            pair: first.currency.clone(),
            native_start: number(start_native),
            native_end: number(end_native),
            history_category: Some(first.category.clone()),
            history_source: Some(first.source.clone()),
            history_item: Some(first.line.id.clone()),
            history_archive: first.archive.clone(),
            history_granularity: Some(first.granularity.clone()),
            start_confidence: first.confidence.clone(),
            end_confidence: last.confidence.clone(),
            alternatives: Vec::new(),
        });
    }
    candidates.sort_by(|a, b| {
        let priority = |row: &FlipRow| {
            (
                row.source != "poe.ninja dump",
                !row.id.contains(":exchange:"),
            )
        };
        priority(a).cmp(&priority(b)).then_with(|| a.id.cmp(&b.id))
    });
    let mut identities = HashSet::new();
    for candidate in candidates {
        if identities.insert((
            candidate.category.clone(),
            candidate.name.clone(),
            unidentified_cluster_ids
                .contains(&candidate.id)
                .then(|| candidate.id.clone()),
        )) {
            result.rows.push(candidate);
        }
    }
}

#[derive(Deserialize)]
struct ArbitrageQuery {
    league: String,
    hour: i64,
    min_volume: Option<String>,
}

async fn arbitrage(
    State(state): State<ServiceState>,
    Query(query): Query<ArbitrageQuery>,
) -> Result<Json<Arbitrage>, ApiError> {
    validate_hour(&query.league, query.hour)?;
    let min_volume = filter_decimal(
        query.min_volume.as_deref(),
        "Minimum Chaos-equivalent volume",
    )?
    .unwrap_or(Decimal::ZERO);
    let (markets, complete) = tokio::try_join!(
        state.economy_store.ggg_markets(&query.league, query.hour),
        state
            .economy_store
            .ggg_hour_complete(&query.league, query.hour),
    )
    .map_err(failure)?;
    Ok(Json(build_arbitrage(
        &query.league,
        query.hour,
        &markets,
        complete,
        min_volume,
    )))
}

fn build_arbitrage(
    league: &str,
    hour: i64,
    markets: &[Market],
    complete: bool,
    min_volume: Decimal,
) -> Arbitrage {
    let graph = quotes(markets);
    let mut loops = Vec::new();
    for first in graph.get(CHAOS).into_iter().flatten() {
        for second in graph.get(&first.quote.id).into_iter().flatten() {
            if second.quote.id == CHAOS || second.quote.id == first.quote.id {
                continue;
            }
            let Some(third) = graph
                .get(&second.quote.id)
                .and_then(|edges| edges.iter().find(|edge| edge.quote.id == CHAOS))
            else {
                continue;
            };
            let calculated = (|| {
                let after_two = first.rate.checked_mul(second.rate)?;
                let multiplier = after_two.checked_mul(third.rate)?;
                let percent = multiplier
                    .checked_sub(Decimal::ONE)?
                    .checked_mul(Decimal::ONE_HUNDRED)?;
                let capacity = first
                    .volume
                    .min(second.volume.checked_div(first.rate)?)
                    .min(third.volume.checked_div(after_two)?);
                Some((multiplier, percent, capacity))
            })();
            let Some((multiplier, percent, capacity)) = calculated else {
                continue;
            };
            if percent <= Decimal::new(1, 4) || capacity < min_volume {
                continue;
            }
            loops.push(ArbitrageLoop {
                id: format!("{}>{}>{}", first.quote.id, second.quote.id, CHAOS),
                path: vec![
                    first.item.name.clone(),
                    first.quote.name.clone(),
                    second.quote.name.clone(),
                    third.quote.name.clone(),
                ],
                rates: vec![number(first.rate), number(second.rate), number(third.rate)],
                market_ids: vec![
                    first.market.clone(),
                    second.market.clone(),
                    third.market.clone(),
                ],
                return_percent: number(percent),
                start_amount: "1".into(),
                end_amount: number(multiplier),
                minimum_volume: number(capacity),
            });
        }
    }
    loops.sort_by(|a, b| {
        b.return_percent
            .parse::<Decimal>()
            .unwrap()
            .cmp(&a.return_percent.parse().unwrap())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut warnings = vec![
        "Implied historical loops, not live/executable arbitrage. Hourly trades may not be concurrent; reversed averages are not opposite-side quotes. Bid/ask spread, available orders, rounding, slippage and gold costs are unknown.".into(),
        "Volume is a Chaos-equivalent bottleneck of historical traded amounts, not executable capacity. Each loop starts with 1 Chaos Orb.".into(),
    ];
    if !complete {
        warnings.push("The selected GGG hour has not been successfully fetched. Import it on Currency Exchange.".into());
    }
    if loops.len() > 500 {
        warnings.push(format!(
            "Showing the top 500 of {} implied loops.",
            loops.len()
        ));
        loops.truncate(500);
    }
    Arbitrage {
        league: league.into(),
        hour,
        complete,
        loops,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use market_core::economy::Line;
    use tower::ServiceExt;

    use super::*;

    const DIVINE: &str = "Metadata/Items/Currency/CurrencyModValues";
    const OIL: &str = "Metadata/Items/Currency/MushruneCorrupt";
    const CHROMATIC: &str = "Metadata/Items/Currency/CurrencyRerollSocketColours";
    const START: i64 = 1767261600;
    const END: i64 = START + 86400;

    fn item(id: &str) -> Item {
        let (name, category) = match id {
            CHAOS => ("Chaos Orb", "Currency"),
            DIVINE => ("Divine Orb", "Currency"),
            OIL => ("Tainted Oil", "Oil"),
            CHROMATIC => ("Chromatic Orb", "Currency"),
            _ => (id, "Unknown"),
        };
        Item {
            id: id.into(),
            name: name.into(),
            category: category.into(),
            icon: None,
        }
    }

    fn market(a: &str, b: &str, volume_a: &str, volume_b: &str) -> Market {
        Market {
            id: format!("{a}|{b}"),
            a: item(a),
            b: item(b),
            volume_a: volume_a.into(),
            volume_b: volume_b.into(),
            low_stock_a: "0".into(),
            low_stock_b: "0".into(),
            high_stock_a: "0".into(),
            high_stock_b: "0".into(),
            low_ratio_a: "1".into(),
            low_ratio_b: "1".into(),
            high_ratio_a: "1".into(),
            high_ratio_b: "1".into(),
        }
    }

    fn ninja(id: &str, name: &str, category: &str, day: &str, price: &str) -> Observation {
        Observation {
            category: category.into(),
            source: "stash".into(),
            line: Line {
                id: id.into(),
                name: name.into(),
                ..Line::default()
            },
            observed_at: day.into(),
            granularity: "daily".into(),
            price: price.into(),
            currency: "Chaos Orb".into(),
            archive: Some("Dump".into()),
            volume: None,
            confidence: None,
        }
    }

    async fn save_markets(store: &market_store::SqliteStore, hour: i64, markets: &[Market]) {
        let items = markets
            .iter()
            .flat_map(|market| [market.a.clone(), market.b.clone()])
            .collect::<Vec<_>>();
        let markets = markets
            .iter()
            .map(|market| {
                let volumes = serde_json::Map::from_iter([
                    (
                        market.a.id.clone(),
                        serde_json::json!(market.volume_a.parse::<u64>().unwrap()),
                    ),
                    (
                        market.b.id.clone(),
                        serde_json::json!(market.volume_b.parse::<u64>().unwrap()),
                    ),
                ]);
                serde_json::json!({
                    "league":"Test","market_id":market.id,"market_pair":[market.a.id,market.b.id],
                    "volume_traded":volumes,"lowest_stock":volumes,"highest_stock":volumes,
                    "lowest_ratio":volumes,"highest_ratio":volumes
                })
            })
            .collect::<Vec<_>>();
        let page = serde_json::from_value(serde_json::json!({
            "next_change_id":hour + 3600,"markets":markets
        }))
        .unwrap();
        store.record_ggg_hour("Test", hour, &page).await.unwrap();
        store.save_ggg_catalog(&items, hour).await.unwrap();
    }

    #[tokio::test]
    async fn late_exchange_starts_use_first_trades_and_their_own_conversions_without_moving_the_end()
     {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("late-ggg.sqlite3"))
            .await
            .unwrap();
        for (hour, markets) in [
            (START - 3600, vec![market(OIL, DIVINE, "1", "1")]),
            (
                START,
                vec![
                    market(OIL, DIVINE, "0", "0"),
                    market(DIVINE, CHAOS, "10", "1000"),
                ],
            ),
            (
                START + 3600,
                vec![
                    market(OIL, DIVINE, "1", "2"),
                    market(DIVINE, CHAOS, "10", "2000"),
                    market(CHROMATIC, CHAOS, "1", "10"),
                ],
            ),
            (
                START + 7200,
                vec![
                    market(OIL, CHAOS, "1", "50"),
                    market(OIL, DIVINE, "20", "60"),
                    market(DIVINE, CHAOS, "10", "3000"),
                ],
            ),
            (
                END,
                vec![
                    market(OIL, CHAOS, "10", "1000"),
                    market(OIL, DIVINE, "10", "40"),
                    market(DIVINE, CHAOS, "10", "4000"),
                    market("EndOnly", CHAOS, "1", "2"),
                ],
            ),
        ] {
            save_markets(&store, hour, &markets).await;
        }
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        for minimum in [0, 2, 11] {
            let response = app.clone().oneshot(Request::builder()
                .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&min_volume={minimum}"))
                .body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let result: Flips = serde_json::from_slice(&body).unwrap();
            assert!(
                !result
                    .rows
                    .iter()
                    .any(|row| row.name == "Chromatic Orb" || row.name == "EndOnly")
            );
            let oil = result.rows.iter().find(|row| row.name == "Tainted Oil");
            if minimum > 10 {
                assert!(
                    oil.is_none(),
                    "The minimum must not select a later high-volume start"
                );
                continue;
            }
            let oil = oil.unwrap();
            assert_eq!(oil.start_price, "50");
            assert_eq!(oil.end_price, "100");
            assert_eq!(oil.start_observed_at, utc_hour(START + 7200));
            assert_eq!(oil.end_observed_at, utc_hour(END));
            let divine = oil
                .alternatives
                .iter()
                .find(|pair| pair.quote == "Divine Orb")
                .unwrap();
            assert_eq!(divine.start_price, "400");
            assert_eq!(divine.end_price, "1600");
            assert_eq!(divine.start_observed_at, utc_hour(START + 3600));
            assert_eq!(divine.start_volume, "1");
        }
        let response = app.oneshot(Request::builder()
            .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&strict_start=true"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let strict: Flips = serde_json::from_slice(&body).unwrap();
        assert!(!strict.rows.iter().any(|row| row.name == "Tainted Oil"));
        let divine = strict
            .rows
            .iter()
            .find(|row| row.name == "Divine Orb")
            .unwrap();
        assert_eq!(divine.start_observed_at, utc_hour(START));
        assert_eq!(divine.start_price, "100");
    }

    #[tokio::test]
    async fn late_archive_starts_keep_actual_dates_confidence_and_same_day_conversion() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("late-ninja.sqlite3"))
            .await
            .unwrap();
        store
            .save_economy_import(
                &economy::Archive {
                    name: "Dump".into(),
                    league: Some("Test".into()),
                    status: "completed".into(),
                    rows: 0,
                    skipped: 0,
                    downloaded_bytes: 0,
                    download_total_bytes: None,
                    total_rows: None,
                    error: None,
                    start: None,
                    end: None,
                    progress: String::new(),
                    license: String::new(),
                    url: String::new(),
                },
                None,
            )
            .await
            .unwrap();
        for (day, rows) in [
            (
                "2025-12-31",
                vec![("before", "Before period", "SkillGem", "1", "Chaos Orb")],
            ),
            (
                "2026-01-01",
                vec![
                    ("divine", "Divine Orb", "Currency", "100", "Chaos Orb"),
                    ("existing", "Already present", "SkillGem", "10", "Chaos Orb"),
                    ("zero", "Zero at start", "SkillGem", "0", "Chaos Orb"),
                ],
            ),
            (
                "2026-01-02",
                vec![
                    ("divine", "Divine Orb", "Currency", "200", "Chaos Orb"),
                    ("late", "Late gem", "SkillGem", "2", "Divine Orb"),
                    ("zero", "Zero at start", "SkillGem", "5", "Chaos Orb"),
                    ("no-end", "No end price", "SkillGem", "3", "Chaos Orb"),
                ],
            ),
            (
                "2026-01-03",
                vec![
                    ("divine", "Divine Orb", "Currency", "300", "Chaos Orb"),
                    ("late", "Late gem", "SkillGem", "3", "Divine Orb"),
                    ("zero", "Zero at start", "SkillGem", "10", "Chaos Orb"),
                    ("existing", "Already present", "SkillGem", "20", "Chaos Orb"),
                    ("before", "Before period", "SkillGem", "9", "Chaos Orb"),
                    ("end-only", "End only", "SkillGem", "9", "Chaos Orb"),
                ],
            ),
        ] {
            let observations = rows
                .into_iter()
                .map(
                    |(id, name, category, price, currency)| economy::Observation {
                        category: category.into(),
                        line: Line {
                            id: id.into(),
                            name: name.into(),
                            ..Line::default()
                        },
                        point: economy::Point {
                            date: day.into(),
                            price: price.into(),
                            currency: currency.into(),
                            granularity: "daily".into(),
                            volume: None,
                            confidence: Some(
                                if day == "2026-01-02" { "Low" } else { "High" }.into(),
                            ),
                        },
                    },
                )
                .collect::<Vec<_>>();
            store
                .save_observations("Test", "stash", &observations, Some("Dump"))
                .await
                .unwrap();
        }
        let end = END + 86400;
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/analysis/flips?league=Test&start={START}&end={end}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: Flips = serde_json::from_slice(&body).unwrap();
        let late = result
            .rows
            .iter()
            .find(|row| row.name == "Late gem")
            .unwrap();
        assert_eq!(late.start_price, "400");
        assert_eq!(late.end_price, "900");
        assert_eq!(late.start_observed_at, "2026-01-02");
        assert_eq!(late.start_confidence.as_deref(), Some("Low"));
        assert_eq!(late.end_confidence.as_deref(), Some("High"));
        let zero = result
            .rows
            .iter()
            .find(|row| row.name == "Zero at start")
            .unwrap();
        assert_eq!(zero.start_price, "5");
        assert_eq!(zero.start_observed_at, "2026-01-02");
        let existing = result
            .rows
            .iter()
            .find(|row| row.name == "Already present")
            .unwrap();
        assert_eq!(existing.start_price, "10");
        assert_eq!(existing.start_observed_at, "2026-01-01");
        for excluded in ["Before period", "End only", "No end price"] {
            assert!(
                !result.rows.iter().any(|row| row.name == excluded),
                "{excluded}"
            );
        }
        let response = app.oneshot(Request::builder()
            .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={end}&strict_start=true"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let strict: Flips = serde_json::from_slice(&body).unwrap();
        assert!(
            !strict
                .rows
                .iter()
                .any(|row| row.name == "Late gem" || row.name == "Zero at start")
        );
        let existing = strict
            .rows
            .iter()
            .find(|row| row.name == "Already present")
            .unwrap();
        assert_eq!(existing.start_price, "10");
        assert_eq!(existing.start_observed_at, "2026-01-01");
    }

    #[tokio::test]
    async fn late_snapshot_start_uses_first_positive_tick_and_its_hourly_conversion() {
        let directory = tempfile::tempdir().unwrap();
        let store =
            market_store::SqliteStore::connect(directory.path().join("late-snapshot.sqlite3"))
                .await
                .unwrap();
        for (at, id, name, category, price, currency) in [
            (
                START - 3600,
                "gem",
                "Late snapshot",
                "SkillGem",
                "1",
                "Divine Orb",
            ),
            (
                START,
                "divine",
                "Divine Orb",
                "Currency",
                "100",
                "Chaos Orb",
            ),
            (
                START + 4500,
                "gem",
                "Late snapshot",
                "SkillGem",
                "2",
                "Divine Orb",
            ),
            (
                START + 5400,
                "divine",
                "Divine Orb",
                "Currency",
                "200",
                "Chaos Orb",
            ),
            (
                START + 6300,
                "gem",
                "Late snapshot",
                "SkillGem",
                "4",
                "Divine Orb",
            ),
            (
                END + 900,
                "gem",
                "Late snapshot",
                "SkillGem",
                "3",
                "Divine Orb",
            ),
            (
                END + 1800,
                "divine",
                "Divine Orb",
                "Currency",
                "300",
                "Chaos Orb",
            ),
        ] {
            store
                .save_observations(
                    "Test",
                    "stash",
                    &[economy::Observation {
                        category: category.into(),
                        line: Line {
                            id: id.into(),
                            name: name.into(),
                            ..Line::default()
                        },
                        point: economy::Point {
                            date: utc_hour(at),
                            price: price.into(),
                            currency: currency.into(),
                            granularity: "snapshot".into(),
                            volume: None,
                            confidence: None,
                        },
                    }],
                    None,
                )
                .await
                .unwrap();
        }
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/analysis/flips?league=Test&start={START}&end={END}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: Flips = serde_json::from_slice(&body).unwrap();
        let row = result
            .rows
            .iter()
            .find(|row| row.name == "Late snapshot")
            .unwrap();
        assert_eq!(row.start_observed_at, utc_hour(START + 4500));
        assert_eq!(row.end_observed_at, utc_hour(END + 900));
        assert_eq!(row.start_price, "400");
        assert_eq!(row.end_price, "900");
    }

    #[test]
    fn exclusions_identify_missing_hours_trades_pairs_conversions_and_explicit_volume_filters() {
        let traded = market(OIL, CHAOS, "1", "10");
        for (starts, ends, minimum, expected) in [
            (
                vec![],
                vec![traded.clone()],
                Decimal::ZERO,
                "No market observation in the selected start hour",
            ),
            (
                vec![traded.clone()],
                vec![],
                Decimal::ZERO,
                "No market observation in the selected end hour",
            ),
            (
                vec![market(OIL, CHAOS, "0", "0")],
                vec![traded.clone()],
                Decimal::ZERO,
                "No usable traded price in the start hour",
            ),
            (
                vec![traded.clone()],
                vec![market(OIL, CHAOS, "0", "0")],
                Decimal::ZERO,
                "No usable traded price in the end hour",
            ),
            (
                vec![traded.clone()],
                vec![traded.clone()],
                Decimal::from(2),
                "configured minimum of 2 item units",
            ),
            (
                vec![market(OIL, DIVINE, "1", "1")],
                vec![traded.clone()],
                Decimal::ZERO,
                "No identical traded pair",
            ),
            (
                vec![market(OIL, DIVINE, "1", "1")],
                vec![market(OIL, DIVINE, "1", "2")],
                Decimal::ZERO,
                "Missing direct quote-to-Chaos conversion",
            ),
        ] {
            let result = build_flips("Test", START, END, &starts, &ends, &[], &[], minimum);
            let reason = &result
                .excluded
                .iter()
                .find(|row| row.name == "Tainted Oil")
                .unwrap()
                .reason;
            assert!(reason.contains(expected), "{reason}");
            assert!(!reason.contains("sufficient volume"));
            if minimum == Decimal::ZERO {
                assert!(!reason.contains("configured minimum"));
            }
        }
        let result = build_flips(
            "Test",
            START,
            END,
            &[traded.clone()],
            &[traded],
            &[],
            &[],
            Decimal::ZERO,
        );
        assert!(result.rows.iter().any(|row| row.name == "Tainted Oil"));
    }

    #[tokio::test]
    async fn cluster_rows_recover_exact_provider_level_brackets_without_changing_prices() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("clusters.sqlite3"))
            .await
            .unwrap();
        let mut metadata = Vec::new();
        for (provider_id, level) in [
            ("63200", 75),
            ("63479", 50),
            ("63537", 68),
            ("63729", 1),
            ("63957", 84),
        ] {
            let line = Line {
                id: format!("{provider_id}:variant"),
                name: "12% increased Chaos Damage".into(),
                variant: Some("8 passives".into()),
                base_type: Some("Large Cluster Jewel".into()),
                metadata: BTreeMap::from([("levelRequired".into(), level.into())]),
                ..Line::default()
            };
            metadata.push(line);
        }
        store
            .save_economy(&economy::Overview {
                league: "Standard".into(),
                category: "ClusterJewel".into(),
                source: "stash".into(),
                fetched_at: utc_hour(END),
                quote_currency: "Chaos Orb".into(),
                rates: BTreeMap::new(),
                lines: metadata,
            })
            .await
            .unwrap();
        for day in ["2026-01-01", "2026-01-02"] {
            let observations = [
                "63200",
                "63479",
                "63537",
                "63729",
                "63957",
                "unknown",
                "unknown-other",
            ]
            .map(|provider_id| economy::Observation {
                category: "ClusterJewel".into(),
                line: Line {
                    id: format!("archive:{provider_id}"),
                    name: "12% increased Chaos Damage".into(),
                    variant: Some("8 passives".into()),
                    base_type: Some("Large Cluster Jewel".into()),
                    metadata: BTreeMap::from([("archive_item_id".into(), provider_id.into())]),
                    ..Line::default()
                },
                point: economy::Point {
                    date: day.into(),
                    price: if day.ends_with("01") { "10" } else { "20" }.into(),
                    currency: "Chaos Orb".into(),
                    granularity: "daily".into(),
                    volume: None,
                    confidence: None,
                },
            });
            store
                .save_observations("Test", "stash", &observations, None)
                .await
                .unwrap();
        }
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let response = app.oneshot(Request::builder()
            .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&category=ClusterJewel"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: Flips = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.rows.len(), 7);
        for (id, label) in [
            ("63200", "75-83"),
            ("63479", "50-67"),
            ("63537", "68-74"),
            ("63729", "1-49"),
            ("63957", "84-100"),
            ("unknown", "unknown"),
            ("unknown-other", "unknown"),
        ] {
            let row = result
                .rows
                .iter()
                .find(|row| row.history_item.as_deref() == Some(&format!("archive:{id}")))
                .unwrap();
            assert!(
                row.name.contains(&format!("item level {label}")),
                "{}",
                row.name
            );
            assert_eq!(row.start_price, "10");
            assert_eq!(row.end_price, "20");
        }
    }

    #[test]
    fn stable_reference_prefers_chaos_without_cherry_picking_largest_growth() {
        let starts = vec![
            market(OIL, CHAOS, "10", "100"),
            market(OIL, DIVINE, "100", "10"),
            market(DIVINE, CHAOS, "100", "20000"),
        ];
        let ends = vec![
            market(OIL, CHAOS, "10", "150"),
            market(OIL, DIVINE, "100", "100"),
            market(DIVINE, CHAOS, "100", "30000"),
        ];
        let result = build_flips("Test", START, END, &starts, &ends, &[], &[], Decimal::ZERO);
        let row = result
            .rows
            .iter()
            .find(|row| row.name == "Tainted Oil")
            .unwrap();
        assert_eq!(row.start_price, "10");
        assert_eq!(row.end_price, "15");
        assert_eq!(row.growth, "5");
        assert_eq!(row.growth_percent, "50");
        assert_eq!(row.pair, "Chaos Orb");
        assert_eq!(row.alternatives.len(), 2);
        let alternative = row
            .alternatives
            .iter()
            .find(|pair| pair.quote == "Divine Orb")
            .unwrap();
        assert_eq!(alternative.start_price, "20");
        assert_eq!(alternative.end_price, "300");
        assert_eq!(alternative.native_start, "0.1");
        assert_eq!(alternative.native_end, "1");
        assert!(!alternative.selected);
    }

    #[test]
    fn conversion_uses_endpoint_rates_and_requires_same_pair_and_trades() {
        let starts = vec![
            market(OIL, DIVINE, "10", "20"),
            market(DIVINE, CHAOS, "10", "2000"),
        ];
        let ends = vec![
            market(OIL, DIVINE, "10", "20"),
            market(DIVINE, CHAOS, "10", "3000"),
        ];
        let result = build_flips("Test", START, END, &starts, &ends, &[], &[], Decimal::ZERO);
        let row = result
            .rows
            .iter()
            .find(|row| row.name == "Tainted Oil")
            .unwrap();
        assert_eq!(row.start_price, "400");
        assert_eq!(row.end_price, "600");
        assert_eq!(row.growth_percent, "50");
        let missing = build_flips(
            "Test",
            START,
            END,
            &starts,
            &[market(OIL, CHAOS, "10", "6000")],
            &[],
            &[],
            Decimal::ZERO,
        );
        assert!(!missing.rows.iter().any(|row| row.name == "Tainted Oil"));
        assert!(missing.excluded.iter().any(|row| row.name == "Tainted Oil"));
        let zero = build_flips(
            "Test",
            START,
            END,
            &[market(OIL, CHAOS, "0", "0")],
            &[market(OIL, CHAOS, "1", "10")],
            &[],
            &[],
            Decimal::ZERO,
        );
        assert!(!zero.rows.iter().any(|row| row.name == "Tainted Oil"));
    }

    #[test]
    fn minimum_volume_accepts_either_actual_endpoint_inclusively() {
        for (start, end, eligible) in [(1, 10, true), (10, 1, true), (9, 9, false), (10, 10, true)]
        {
            let result = build_flips(
                "Test",
                START,
                END,
                &[market(OIL, CHAOS, &start.to_string(), "1000")],
                &[market(OIL, CHAOS, &end.to_string(), "1000")],
                &[],
                &[],
                Decimal::from(10),
            );
            assert_eq!(
                result.rows.iter().any(|row| row.name == "Tainted Oil"),
                eligible,
                "start={start}, end={end}"
            );
        }
    }

    #[test]
    fn minimum_volume_filters_reference_pairs_without_using_quote_volume() {
        let starts = vec![
            market(OIL, CHAOS, "1", "10000"),
            market(OIL, DIVINE, "20", "20"),
            market(DIVINE, CHAOS, "20", "4000"),
        ];
        let ends = vec![
            market(OIL, CHAOS, "1", "20000"),
            market(OIL, DIVINE, "20", "40"),
            market(DIVINE, CHAOS, "20", "4000"),
        ];
        let result = build_flips(
            "Test",
            START,
            END,
            &starts,
            &ends,
            &[],
            &[],
            Decimal::from(10),
        );
        let row = result
            .rows
            .iter()
            .find(|row| row.name == "Tainted Oil")
            .unwrap();
        assert_eq!(row.pair, "Divine Orb");
        assert_eq!(row.start_price, "200");
        assert_eq!(row.end_price, "400");
    }

    #[test]
    fn ggg_is_preferred_without_cross_provider_splicing_and_variants_stay_distinct() {
        let starts = vec![
            ninja("oil", "Tainted Oil", "Oil", "2026-01-01", "1"),
            ninja("fire20", "Fireball", "SkillGem", "2026-01-01", "10"),
            ninja("fire21", "Fireball", "SkillGem", "2026-01-01", "40"),
        ];
        let mut starts = starts;
        starts[1].line.gem_level = Some(20);
        starts[2].line.gem_level = Some(21);
        let mut ends = starts.clone();
        for observation in &mut ends {
            observation.price = "80".into();
            observation.observed_at = "2026-01-02".into();
        }
        let result = build_flips(
            "Test",
            START,
            END,
            &[market(OIL, CHAOS, "10", "100")],
            &[market(OIL, CHAOS, "10", "200")],
            &starts,
            &ends,
            Decimal::ZERO,
        );
        assert_eq!(
            result
                .rows
                .iter()
                .filter(|row| row.name == "Tainted Oil")
                .count(),
            1
        );
        assert_eq!(
            result
                .rows
                .iter()
                .find(|row| row.name == "Tainted Oil")
                .unwrap()
                .source,
            "GGG Exchange"
        );
        let gems = result
            .rows
            .iter()
            .filter(|row| row.category == "SkillGem")
            .collect::<Vec<_>>();
        assert_eq!(gems.len(), 2);
        assert!(gems.iter().all(|row| row.venue == "trade"));
        assert!(gems.iter().any(|row| row.growth_percent == "700"));
        assert!(gems.iter().any(|row| row.growth_percent == "100"));
        ends[0].archive = Some("Different dump".into());
        let result = build_flips("Test", START, END, &[], &[], &starts, &ends, Decimal::ZERO);
        assert!(!result.rows.iter().any(|row| row.name == "Tainted Oil"));
    }

    #[test]
    fn ggg_volume_filter_preserves_archives_with_explicit_unknown_liquidity() {
        let starts = [
            ninja("a", "Item A", "SkillGem", "2026-01-01", "0"),
            ninja("b", "Item B", "SkillGem", "2026-01-01", "10"),
        ];
        let mut ends = starts.clone();
        for observation in &mut ends {
            observation.price = "20".into();
        }
        let result = build_flips("Test", START, END, &[], &[], &starts, &ends, Decimal::ZERO);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].name, "Item B");
        let filtered = build_flips("Test", START, END, &[], &[], &starts, &ends, Decimal::ONE);
        assert_eq!(filtered.rows.len(), 1);
        assert_eq!(filtered.rows[0].name, "Item B");
        assert!(
            filtered
                .warnings
                .iter()
                .any(|warning| warning.contains("liquidity is unknown"))
        );
    }

    #[test]
    fn archive_conversion_uses_same_archive_day_rates_and_keeps_native_quote() {
        let mut start_gem = ninja("fire", "Fireball", "SkillGem", "2026-01-01", "2");
        start_gem.currency = "Divine Orb".into();
        let start_rate = ninja("divine", "Divine Orb", "Currency", "2026-01-01", "200");
        let mut end_gem = start_gem.clone();
        end_gem.observed_at = "2026-01-02".into();
        let mut end_rate = start_rate.clone();
        end_rate.price = "300".into();
        end_rate.observed_at = "2026-01-02".into();
        let result = build_flips(
            "Test",
            START,
            END,
            &[],
            &[],
            &[start_gem.clone(), start_rate.clone()],
            &[end_gem.clone(), end_rate.clone()],
            Decimal::ZERO,
        );
        let gem = result
            .rows
            .iter()
            .find(|row| row.name == "Fireball")
            .unwrap();
        assert_eq!(gem.start_price, "400");
        assert_eq!(gem.end_price, "600");
        assert_eq!(gem.pair, "Divine Orb");
        end_rate.archive = Some("Other".into());
        let missing = build_flips(
            "Test",
            START,
            END,
            &[],
            &[],
            &[start_gem, start_rate],
            &[end_gem, end_rate],
            Decimal::ZERO,
        );
        assert!(!missing.rows.iter().any(|row| row.name == "Fireball"));
        assert!(
            missing
                .excluded
                .iter()
                .any(|row| row.reason.contains("conversion"))
        );
    }

    #[test]
    fn historical_loops_use_three_native_pairs_and_explicit_nonexecution_warnings() {
        let markets = vec![
            market(CHAOS, OIL, "1000", "10"),
            market(OIL, DIVINE, "10", "2"),
            market(DIVINE, CHAOS, "2", "1100"),
        ];
        let result = build_arbitrage("Test", START, &markets, true, Decimal::ZERO);
        assert!(result.complete);
        assert_eq!(result.loops.len(), 1);
        assert_eq!(
            result.loops[0].path,
            ["Chaos Orb", "Tainted Oil", "Divine Orb", "Chaos Orb"]
        );
        assert_eq!(result.loops[0].return_percent, "10");
        assert_eq!(result.loops[0].end_amount, "1.1");
        assert_eq!(result.loops[0].minimum_volume, "1000");
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("not live/executable"))
        );
        assert!(
            build_arbitrage("Test", START, &markets, true, Decimal::from(1001))
                .loops
                .is_empty()
        );
        assert!(
            build_arbitrage("Test", START, &markets[..2], true, Decimal::ZERO)
                .loops
                .is_empty()
        );
    }

    #[test]
    fn validates_hours_and_nonnegative_finite_filters() {
        assert!(validate_hour("Test", START).is_ok());
        assert!(validate_hour("Test", START + 1).is_err());
        assert!(validate_hour("Test", Utc::now().timestamp().div_euclid(3600) * 3600).is_err());
        assert!(validate_hour("\n", START).is_err());
        assert!(filter_decimal(Some("NaN"), "Price").is_err());
        assert!(filter_decimal(Some("-1"), "Price").is_err());
        assert_eq!(
            filter_decimal(Some("0"), "Price").unwrap(),
            Some(Decimal::ZERO)
        );
        let hours = sample_hours(START, END + 200 * 86400, false);
        assert_eq!(hours.len(), 60);
        assert_eq!(hours[0], START);
        assert_eq!(*hours.last().unwrap(), END + 200 * 86400);
        assert!(hours.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(sample_hours(START, START + 3600, true).len(), 1);
    }

    #[test]
    fn archive_confidence_remains_specific_to_each_endpoint() {
        let mut first = ninja("item", "Item", "SkillGem", "2026-01-01", "1");
        first.confidence = Some("High".into());
        let mut last = ninja("item", "Item", "SkillGem", "2026-01-02", "2");
        last.confidence = Some("Low".into());
        let result = build_flips(
            "Test",
            START,
            END,
            &[],
            &[],
            &[first],
            &[last],
            Decimal::ZERO,
        );
        assert_eq!(result.rows[0].start_confidence.as_deref(), Some("High"));
        assert_eq!(result.rows[0].end_confidence.as_deref(), Some("Low"));
        assert_eq!(result.rows[0].history_archive.as_deref(), Some("Dump"));
        assert_eq!(result.rows[0].history_granularity.as_deref(), Some("daily"));
    }

    #[tokio::test]
    async fn batched_sparklines_use_saved_hours_and_historical_chaos_conversion_with_gaps() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("sparklines.sqlite3"))
            .await
            .unwrap();
        for (at, item_price, divine_price) in [
            (START, "2", "100"),
            (START + 3600, "3", "200"),
            (START + 3 * 3600, "4", "300"),
        ] {
            for (category, name, price, currency) in [
                ("SkillGem", "Gem", item_price, "Divine Orb"),
                ("Currency", "Divine Orb", divine_price, "Chaos Orb"),
            ] {
                store
                    .save_economy(&economy::Overview {
                        league: "Test".into(),
                        category: category.into(),
                        source: "stash".into(),
                        fetched_at: utc_hour(at),
                        quote_currency: currency.into(),
                        rates: BTreeMap::new(),
                        lines: vec![Line {
                            id: name.into(),
                            name: name.into(),
                            price: Some(price.into()),
                            ..Line::default()
                        }],
                    })
                    .await
                    .unwrap();
            }
        }
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/analysis/flips?league=Test&start={START}&end={}",
                        START + 3 * 3600
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let flips: Flips =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let row = flips.rows.iter().find(|row| row.name == "Gem").unwrap();
        let body =
            serde_json::json!({"league":"Test","start":START,"end":START+3*3600,"rows":[row]});
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/analysis/sparklines")
                    .header("Content-Type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Sparklines =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(
            result.series[0]
                .points
                .iter()
                .map(|point| point.price.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("200"), Some("600"), None, Some("1200")]
        );
        assert!(
            result.series[0]
                .warnings
                .iter()
                .any(|warning| warning.contains("unavailable"))
        );
        let mut bad = body.clone();
        bad["rows"] = serde_json::json!(vec![row; 101]);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/analysis/sparklines")
                    .header("Content-Type", "application/json")
                    .body(Body::from(bad.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn routes_return_explicit_missing_coverage_and_reject_invalid_filters() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("routes.sqlite3"))
            .await
            .unwrap();
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/analysis/flips?league=Test&start={START}&end={END}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let flips: Flips = serde_json::from_slice(&body).unwrap();
        assert!(flips.rows.is_empty());
        assert!(
            flips
                .warnings
                .iter()
                .any(|warning| warning.contains("incomplete"))
        );
        let invalid = app.oneshot(Request::builder()
            .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&min_price=10&max_price=1"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn confidence_filter_accepts_either_observation_before_paging_and_preserves_ggg() {
        let directory = tempfile::tempdir().unwrap();
        let store =
            market_store::SqliteStore::connect(directory.path().join("confidence-filter.sqlite3"))
                .await
                .unwrap();
        for (day, first) in [("2026-01-01", true), ("2026-01-02", false)] {
            let mut labels = (0..505)
                .map(|index| (format!("High gem {index:03}"), Some("High")))
                .collect::<Vec<_>>();
            for (name, start, end) in [
                ("High-low", Some("High"), Some("Low")),
                ("Low-high", Some("Low"), Some("High")),
                ("High-medium", Some("High"), Some("Medium")),
                ("Medium-high", Some("Medium"), Some("High")),
                ("Unknown-start", None, Some("High")),
                ("Unknown-end", Some("High"), None),
                ("Low-low", Some("Low"), Some("Low")),
                ("Medium-low", Some("Medium"), Some("Low")),
                ("Low-medium", Some("Low"), Some("Medium")),
                ("Medium-medium", Some("Medium"), Some("Medium")),
                ("Unknown-low", None, Some("Low")),
                ("Low-unknown", Some("Low"), None),
                ("Unknown-unknown", None, None),
            ] {
                labels.push((name.into(), if first { start } else { end }));
            }
            let observations = labels
                .into_iter()
                .map(|(name, confidence)| economy::Observation {
                    category: "SkillGem".into(),
                    line: Line {
                        id: name.clone(),
                        name,
                        ..Line::default()
                    },
                    point: economy::Point {
                        date: day.into(),
                        price: if first { "1" } else { "2" }.into(),
                        currency: "Chaos Orb".into(),
                        granularity: "daily".into(),
                        volume: None,
                        confidence: confidence.map(str::to_owned),
                    },
                })
                .collect::<Vec<_>>();
            store
                .save_observations("Test", "stash", &observations, None)
                .await
                .unwrap();
        }
        save_markets(&store, START, &[market(OIL, CHAOS, "10", "100")]).await;
        save_markets(&store, END, &[market(OIL, CHAOS, "10", "200")]).await;
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        for (filter, expected_total, expected_rows) in [
            ("", 520, 100),
            ("&min_confidence=Low", 519, 100),
            ("&min_confidence=Medium", 516, 100),
            ("&min_confidence=High", 513, 100),
            ("&min_confidence=High&offset=500", 513, 13),
            ("&min_confidence=High&category=SkillGem&offset=500", 511, 11),
            ("&min_confidence=High&min_volume=1000", 511, 100),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/api/v1/analysis/flips?league=Test&start={START}&end={END}{filter}"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let result: Flips = serde_json::from_slice(&body).unwrap();
            assert_eq!(result.total, expected_total, "{filter}");
            assert_eq!(result.rows.len(), expected_rows, "{filter}");
            if filter.contains("min_confidence=High") {
                for row in result
                    .rows
                    .iter()
                    .filter(|row| row.source != "GGG Exchange")
                {
                    assert!(
                        row.start_confidence.as_deref() == Some("High")
                            || row.end_confidence.as_deref() == Some("High")
                    );
                }
            }
            if filter == "&min_confidence=High&offset=500" {
                assert!(result.rows.iter().any(|row| row.source == "GGG Exchange"));
            }
        }
        let invalid = app.oneshot(Request::builder()
            .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&min_confidence=Unknown"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn starting_price_bounds_are_inclusive_and_rank_across_categories() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("filters.sqlite3"))
            .await
            .unwrap();
        for (category, name, start, end) in [
            ("Currency", "Currency A", "10", "20"),
            ("SkillGem", "Gem B", "5", "2"),
            ("UniqueWeapon", "Unique C", "30", "60"),
        ] {
            for (at, price) in [(START, start), (END, end)] {
                store
                    .save_economy(&economy::Overview {
                        league: "Test".into(),
                        category: category.into(),
                        source: "stash".into(),
                        fetched_at: utc_hour(at),
                        quote_currency: "Chaos Orb".into(),
                        rates: BTreeMap::new(),
                        lines: vec![Line {
                            id: name.into(),
                            name: name.into(),
                            price: Some(price.into()),
                            ..Line::default()
                        }],
                    })
                    .await
                    .unwrap();
            }
        }
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/analysis/flips?league=Test&start={START}&end={END}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: Flips = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.rows.last().unwrap().growth_percent, "-60");
        let response = app.oneshot(Request::builder()
            .uri(format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&min_price=10&max_price=10"))
            .body(Body::empty()).unwrap()).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: Flips = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].name, "Currency A");
        assert_eq!(result.rows[0].start_price, "10");
    }

    #[tokio::test]
    async fn pagination_filters_and_stable_ranking_apply_before_returning_a_page() {
        let directory = tempfile::tempdir().unwrap();
        let store = market_store::SqliteStore::connect(directory.path().join("pages.sqlite3"))
            .await
            .unwrap();
        for at in [START, END] {
            for category in ["Currency", "SkillGem"] {
                let count = if category == "Currency" {
                    if at == START { 305 } else { 105 }
                } else {
                    1
                };
                let price = match (category, at == START) {
                    ("Currency", true) => "1",
                    ("Currency", false) => "2",
                    (_, true) => "10",
                    (_, false) => "5",
                };
                store
                    .save_economy(&economy::Overview {
                        league: "Test".into(),
                        category: category.into(),
                        source: "stash".into(),
                        fetched_at: utc_hour(at),
                        quote_currency: "Chaos Orb".into(),
                        rates: BTreeMap::new(),
                        lines: (0..count)
                            .map(|index| Line {
                                id: format!("{category}-{index:03}"),
                                name: format!("{category} {index:03}"),
                                price: Some(price.into()),
                                ..Line::default()
                            })
                            .collect(),
                    })
                    .await
                    .unwrap();
            }
        }
        let (selection, _) = tokio::sync::watch::channel(None);
        let app = crate::router(ServiceState::new(store, "Test", selection));
        let base =
            format!("/api/v1/analysis/flips?league=Test&start={START}&end={END}&min_volume=1");
        let mut pages = Vec::new();
        for (parameters, total, names) in [
            ("", 106, vec!["Currency 000", "Currency 099"]),
            ("&offset=100", 106, vec!["Currency 100", "SkillGem 000"]),
            (
                "&search=Currency%2010",
                5,
                vec!["Currency 100", "Currency 104"],
            ),
            (
                "&category=SkillGem",
                1,
                vec!["SkillGem 000", "SkillGem 000"],
            ),
            ("&venue=trade", 1, vec!["SkillGem 000", "SkillGem 000"]),
            (
                "&sort=value-asc&limit=1",
                106,
                vec!["SkillGem 000", "SkillGem 000"],
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("{base}{parameters}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let result: Flips = serde_json::from_slice(&body).unwrap();
            assert_eq!(result.total, total);
            assert!(result.rows.len() <= result.limit);
            assert_eq!(result.excluded_total, 200);
            assert_eq!(result.excluded.len(), 100);
            assert_eq!(result.categories, vec!["Currency", "SkillGem"]);
            if !names.is_empty() {
                assert_eq!(result.rows.first().unwrap().name, names[0]);
                assert_eq!(result.rows.last().unwrap().name, names[1]);
            }
            pages.push(result);
        }
        assert_eq!(pages[0].rows.len(), 100);
        assert_eq!(pages[1].offset, 100);
        assert_eq!(pages[1].rows.len(), 6);
        assert!(
            pages[0]
                .rows
                .iter()
                .all(|first| pages[1].rows.iter().all(|second| first.id != second.id))
        );
        for parameters in ["&limit=0", "&limit=101", "&sort=invalid", "&venue=invalid"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("{base}{parameters}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/v1/analysis/flips?league=Test&start={}&end={END}",
                        START - 3600
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
