use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod analysis;
pub mod economy;
pub mod ggg;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServiceStatus {
    pub exchange_cursor: Option<u64>,
    pub exchange_cursor_at: Option<DateTime<Utc>>,
    pub exchange_pages: i64,
    pub exchange_markets: i64,
    pub ninja_snapshots: i64,
    pub selected_league: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LeagueOption {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExchangeMarketSnapshot {
    pub league: String,
    pub market_id: String,
    pub market_pair: Vec<String>,
    pub volume_traded: std::collections::BTreeMap<String, u64>,
    pub lowest_stock: std::collections::BTreeMap<String, u64>,
    pub highest_stock: std::collections::BTreeMap<String, u64>,
    pub lowest_ratio: std::collections::BTreeMap<String, u64>,
    pub highest_ratio: std::collections::BTreeMap<String, u64>,
    pub next_change_id: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExchangePriceCurrency {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurrentExchangePriceOverview {
    pub league: String,
    pub category: String,
    pub fetched_at: DateTime<Utc>,
    pub primary: ExchangePriceCurrency,
    pub secondary: Option<ExchangePriceCurrency>,
    pub lines: Vec<CurrentExchangePriceLine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurrentExchangePriceLine {
    pub id: String,
    pub name: String,
    pub item_category: Option<String>,
    pub image: Option<String>,
    pub primary_value: Option<String>,
    pub volume_primary_value: Option<String>,
    pub max_volume_currency: Option<String>,
    pub max_volume_rate: Option<String>,
    pub sparkline_change: Option<String>,
    pub sparkline: Vec<Option<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GemMarketOverview {
    pub league: String,
    pub fetched_at: DateTime<Utc>,
    pub gems: Vec<GemMarketLine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GemAnalysisData {
    pub gems: Option<GemMarketOverview>,
    pub gem_error: Option<String>,
    pub prices: Option<CurrentExchangePriceOverview>,
    pub price_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GemSnapshotOption {
    pub id: i64,
    pub captured_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GemAnalysisOverview {
    pub snapshot: Option<GemSnapshotOption>,
    pub data: GemAnalysisData,
    pub snapshots: Vec<GemSnapshotOption>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GemMarketLine {
    pub name: String,
    #[serde(default)]
    pub gem_kind: Option<String>,
    pub variant: Option<String>,
    pub gem_level: Option<u8>,
    pub gem_quality: Option<u8>,
    pub corrupted: bool,
    pub chaos_value: Option<String>,
    pub sample_count: Option<u64>,
    pub listing_count: Option<u64>,
    pub sparkline_change: Option<String>,
    #[serde(default)]
    pub sparkline: Vec<Option<String>>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstrumentId(String);

impl InstrumentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LeagueId(String);

impl LeagueId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceSource {
    FaustusExchange,
    PoeNinjaStash,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketQuote {
    pub source: PriceSource,
    pub league: LeagueId,
    pub instrument: InstrumentId,
    pub quote_currency: InstrumentId,
    pub amount: Decimal,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PriceComparison {
    pub league: LeagueId,
    pub instrument: InstrumentId,
    pub quote_currency: InstrumentId,
    pub faustus_amount: Decimal,
    pub stash_amount: Decimal,
    pub percent_difference: Decimal,
    pub faustus_observed_at: DateTime<Utc>,
    pub stash_observed_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ComparisonError {
    #[error("quotes must refer to the same instrument")]
    DifferentInstrument,
    #[error("quotes must refer to the same league")]
    DifferentLeague,
    #[error("quotes must use the same quote currency")]
    DifferentQuoteCurrency,
    #[error("expected one Faustus quote and one poe.ninja stash quote")]
    UnexpectedSources,
    #[error("quote amounts must be positive")]
    NonPositiveAmount,
}

impl PriceComparison {
    pub fn from_quotes(first: &MarketQuote, second: &MarketQuote) -> Result<Self, ComparisonError> {
        if first.instrument != second.instrument {
            return Err(ComparisonError::DifferentInstrument);
        }
        if first.league != second.league {
            return Err(ComparisonError::DifferentLeague);
        }
        if first.quote_currency != second.quote_currency {
            return Err(ComparisonError::DifferentQuoteCurrency);
        }

        let (faustus, stash) = match (first.source, second.source) {
            (PriceSource::FaustusExchange, PriceSource::PoeNinjaStash) => (first, second),
            (PriceSource::PoeNinjaStash, PriceSource::FaustusExchange) => (second, first),
            _ => return Err(ComparisonError::UnexpectedSources),
        };

        if faustus.amount <= Decimal::ZERO || stash.amount <= Decimal::ZERO {
            return Err(ComparisonError::NonPositiveAmount);
        }

        Ok(Self {
            league: faustus.league.clone(),
            instrument: faustus.instrument.clone(),
            quote_currency: faustus.quote_currency.clone(),
            faustus_amount: faustus.amount,
            stash_amount: stash.amount,
            percent_difference: (faustus.amount / stash.amount - Decimal::ONE)
                * Decimal::ONE_HUNDRED,
            faustus_observed_at: faustus.observed_at,
            stash_observed_at: stash.observed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn gem_lines_without_category_metadata_remain_readable() {
        let line: GemMarketLine =
            serde_json::from_str(r#"{"name":"Fireball","corrupted":false}"#).unwrap();
        assert_eq!(line.gem_kind, None);
        assert!(line.sparkline.is_empty());
    }

    fn quote(source: PriceSource, amount: Decimal) -> MarketQuote {
        MarketQuote {
            source,
            league: LeagueId::new("Standard"),
            instrument: InstrumentId::new("Mirror of Kalandra"),
            quote_currency: InstrumentId::new("Chaos Orb"),
            amount,
            observed_at: Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn comparison_is_directional_and_order_independent() {
        let faustus = quote(PriceSource::FaustusExchange, Decimal::from(120));
        let stash = quote(PriceSource::PoeNinjaStash, Decimal::from(100));

        let forward = PriceComparison::from_quotes(&faustus, &stash).unwrap();
        let reverse = PriceComparison::from_quotes(&stash, &faustus).unwrap();

        assert_eq!(forward.percent_difference, Decimal::from(20));
        assert_eq!(reverse, forward);
    }

    #[test]
    fn comparison_rejects_different_instruments() {
        let faustus = quote(PriceSource::FaustusExchange, Decimal::from(120));
        let mut stash = quote(PriceSource::PoeNinjaStash, Decimal::from(100));
        stash.instrument = InstrumentId::new("Divine Orb");

        assert_eq!(
            PriceComparison::from_quotes(&faustus, &stash),
            Err(ComparisonError::DifferentInstrument)
        );
    }

    #[test]
    fn decimal_amounts_serialize_as_strings() {
        let quote = quote(PriceSource::FaustusExchange, Decimal::new(125, 2));
        let json = serde_json::to_value(quote).unwrap();

        assert_eq!(json["amount"], "1.25");
    }
}
