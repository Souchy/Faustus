use serde::{Deserialize, Serialize};

use crate::economy::Line;

#[derive(Clone, Debug)]
pub struct Observation {
    pub category: String,
    pub source: String,
    pub line: Line,
    pub observed_at: String,
    pub granularity: String,
    pub price: String,
    pub currency: String,
    pub archive: Option<String>,
    pub volume: Option<String>,
    pub confidence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PairAlternative {
    pub market: String,
    pub quote: String,
    pub native_start: String,
    pub native_end: String,
    pub start_price: String,
    pub end_price: String,
    pub growth: String,
    pub growth_percent: String,
    pub start_volume: String,
    pub end_volume: String,
    pub selected: bool,
    #[serde(default)]
    pub start_observed_at: String,
    #[serde(default)]
    pub end_observed_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FlipRow {
    pub id: String,
    pub name: String,
    pub category: String,
    pub venue: String,
    pub source: String,
    pub start_price: String,
    pub end_price: String,
    pub growth: String,
    pub growth_percent: String,
    pub start_observed_at: String,
    pub end_observed_at: String,
    pub pair: String,
    pub alternatives: Vec<PairAlternative>,
    pub native_start: String,
    pub native_end: String,
    pub history_category: Option<String>,
    pub history_source: Option<String>,
    pub history_item: Option<String>,
    #[serde(default)]
    pub history_archive: Option<String>,
    #[serde(default)]
    pub history_granularity: Option<String>,
    #[serde(default)]
    pub start_confidence: Option<String>,
    #[serde(default)]
    pub end_confidence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SparkPoint {
    pub time: i64,
    pub price: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sparkline {
    pub id: String,
    pub points: Vec<SparkPoint>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sparklines {
    pub series: Vec<Sparkline>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Excluded {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Flips {
    pub league: String,
    pub start: i64,
    pub end: i64,
    pub rows: Vec<FlipRow>,
    pub excluded: Vec<Excluded>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_size")]
    pub limit: usize,
    #[serde(default)]
    pub excluded_total: usize,
    #[serde(default)]
    pub categories: Vec<String>,
}

fn page_size() -> usize {
    100
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Period {
    pub league: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub bounds_source: String,
    pub hours: Vec<i64>,
    pub days: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ArbitrageLoop {
    pub id: String,
    pub path: Vec<String>,
    pub rates: Vec<String>,
    pub market_ids: Vec<String>,
    pub return_percent: String,
    pub start_amount: String,
    pub end_amount: String,
    pub minimum_volume: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Arbitrage {
    pub league: String,
    pub hour: i64,
    pub complete: bool,
    pub loops: Vec<ArbitrageLoop>,
    pub warnings: Vec<String>,
}
