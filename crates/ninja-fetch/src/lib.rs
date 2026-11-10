use std::{collections::BTreeMap, str::FromStr, time::Duration};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode, Url, header};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Number;
use thiserror::Error;

const API_BASE: &str = "https://poe.ninja/poe1/api/economy";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurrencyCategory {
    Currency,
    Fragment,
}

impl CurrencyCategory {
    fn as_query_value(self) -> &'static str {
        match self {
            Self::Currency => "Currency",
            Self::Fragment => "Fragment",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyLeague {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyOverview {
    pub lines: Vec<CurrencyLine>,
    pub currency_details: Vec<CurrencyDetails>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeOverview {
    pub core: ExchangeCore,
    pub lines: Vec<ExchangePriceLine>,
    pub items: Vec<EconomyItem>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ItemOverview {
    pub lines: Vec<ItemOverviewLine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemOverviewLine {
    pub id: u64,
    pub name: String,
    pub icon: Option<String>,
    pub variant: Option<String>,
    pub corrupted: Option<bool>,
    pub gem_level: Option<u8>,
    pub gem_quality: Option<u8>,
    pub count: Option<u64>,
    pub chaos_value: Option<Number>,
    pub listing_count: Option<u64>,
    pub spark_line: Option<SparkLine>,
    pub metadata: Option<ItemMetadata>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemMetadata {
    pub gem_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeCore {
    pub primary: String,
    pub secondary: Option<String>,
    pub rates: BTreeMap<String, Number>,
    pub items: Vec<EconomyItem>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangePriceLine {
    pub id: String,
    pub primary_value: Option<Number>,
    pub volume_primary_value: Option<Number>,
    pub max_volume_currency: Option<String>,
    pub max_volume_rate: Option<Number>,
    pub sparkline: Option<SparkLine>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EconomyItem {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub category: Option<String>,
    pub details_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyLine {
    pub currency_type_name: String,
    pub pay: Option<CurrencySide>,
    pub receive: Option<CurrencySide>,
    pub pay_spark_line: Option<SparkLine>,
    pub receive_spark_line: Option<SparkLine>,
    pub low_confidence_pay_spark_line: Option<SparkLine>,
    pub low_confidence_receive_spark_line: Option<SparkLine>,
    pub chaos_equivalent: Option<Number>,
    pub details_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CurrencySide {
    pub id: Option<u64>,
    pub league_id: Option<u64>,
    pub pay_currency_id: Option<u64>,
    pub get_currency_id: Option<u64>,
    pub sample_time_utc: Option<DateTime<Utc>>,
    pub count: Option<u64>,
    pub value: Option<Number>,
    pub data_point_count: Option<u64>,
    pub includes_secondary: Option<bool>,
    pub listing_count: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SparkLine {
    pub total_change: Option<Number>,
    pub data: Vec<Option<Number>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyDetails {
    pub id: u64,
    pub name: String,
    pub icon: Option<String>,
    pub trade_id: Option<String>,
}

impl CurrencyLine {
    pub fn chaos_equivalent_decimal(&self) -> Result<Option<Decimal>, rust_decimal::Error> {
        self.chaos_equivalent
            .as_ref()
            .map(decimal_from_json_number)
            .transpose()
    }
}

impl CurrencySide {
    pub fn value_decimal(&self) -> Result<Option<Decimal>, rust_decimal::Error> {
        self.value
            .as_ref()
            .map(decimal_from_json_number)
            .transpose()
    }
}

pub fn decimal_from_json_number(value: &Number) -> Result<Decimal, rust_decimal::Error> {
    Decimal::from_str(&value.to_string())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResponseMetadata {
    pub etag: Option<String>,
    pub cache_control: Option<String>,
    pub retry_after: Option<String>,
}

#[derive(Clone, Debug)]
pub enum FetchOutcome<T> {
    NotModified {
        metadata: ResponseMetadata,
    },
    Modified {
        body: T,
        raw_json: Bytes,
        metadata: ResponseMetadata,
    },
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("failed to create HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("request to poe.ninja failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid poe.ninja API URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("poe.ninja API returned HTTP {status}")]
    HttpStatus {
        status: StatusCode,
        retry_after: Option<String>,
    },
    #[error("poe.ninja API returned invalid JSON: {0}")]
    Decode(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct NinjaClient {
    http: Client,
}

impl NinjaClient {
    pub fn new(user_agent: &str) -> Result<Self, FetchError> {
        let http = Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(FetchError::Client)?;

        Ok(Self { http })
    }

    pub async fn fetch_leagues(
        &self,
        etag: Option<&str>,
    ) -> Result<FetchOutcome<Vec<EconomyLeague>>, FetchError> {
        self.fetch_json(Url::parse(&format!("{API_BASE}/leagues"))?, etag)
            .await
    }

    pub async fn fetch_currency_overview(
        &self,
        league: &str,
        category: CurrencyCategory,
        etag: Option<&str>,
    ) -> Result<FetchOutcome<CurrencyOverview>, FetchError> {
        let mut url = Url::parse(&format!("{API_BASE}/stash/current/currency/overview"))?;
        url.query_pairs_mut()
            .append_pair("league", league)
            .append_pair("type", category.as_query_value());

        self.fetch_json(url, etag).await
    }

    pub async fn fetch_exchange_overview(
        &self,
        league: &str,
        category: CurrencyCategory,
        etag: Option<&str>,
    ) -> Result<FetchOutcome<ExchangeOverview>, FetchError> {
        let mut url = Url::parse(&format!("{API_BASE}/exchange/current/overview"))?;
        url.query_pairs_mut()
            .append_pair("league", league)
            .append_pair("type", category.as_query_value());

        self.fetch_json(url, etag).await
    }

    pub async fn fetch_skill_gem_overview(
        &self,
        league: &str,
        etag: Option<&str>,
    ) -> Result<FetchOutcome<ItemOverview>, FetchError> {
        let mut url = Url::parse(&format!("{API_BASE}/stash/current/item/overview"))?;
        url.query_pairs_mut()
            .append_pair("league", league)
            .append_pair("type", "SkillGem");

        self.fetch_json(url, etag).await
    }

    async fn fetch_json<T: DeserializeOwned>(
        &self,
        url: Url,
        etag: Option<&str>,
    ) -> Result<FetchOutcome<T>, FetchError> {
        let mut request = self.http.get(url);
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }

        let response = request.send().await?;
        let status = response.status();
        let metadata = response_metadata(response.headers());

        if status == StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::NotModified { metadata });
        }
        if !status.is_success() {
            return Err(FetchError::HttpStatus {
                status,
                retry_after: metadata.retry_after,
            });
        }

        let raw_json = response.bytes().await?;
        let body = serde_json::from_slice(&raw_json)?;

        Ok(FetchOutcome::Modified {
            body,
            raw_json,
            metadata,
        })
    }
}

fn response_metadata(headers: &header::HeaderMap) -> ResponseMetadata {
    let get = |name| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };

    ResponseMetadata {
        etag: get("etag"),
        cache_control: get("cache-control"),
        retry_after: get("retry-after"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERVIEW_FIXTURE: &str = r#"{
        "lines": [{
            "currencyTypeName": "Divine Orb",
            "pay": {
                "id": 0,
                "league_id": 415,
                "pay_currency_id": 3,
                "get_currency_id": 1,
                "sample_time_utc": "2026-11-09T13:46:48.4880118Z",
                "count": 23,
                "value": 0.002500,
                "data_point_count": 1,
                "includes_secondary": false,
                "listing_count": 205
            },
            "receive": {
                "id": 0,
                "league_id": 415,
                "pay_currency_id": 1,
                "get_currency_id": 3,
                "sample_time_utc": "2026-11-09T13:46:48.4880118Z",
                "count": 9,
                "value": 16.0,
                "data_point_count": 1,
                "includes_secondary": true,
                "listing_count": 67
            },
            "paySparkLine": {"totalChange": 12.8, "data": [0, -4.08, null]},
            "receiveSparkLine": {"totalChange": 70.07, "data": [0, 17.17, 55.36]},
            "chaosEquivalent": 400,
            "lowConfidencePaySparkLine": {"totalChange": 12.8, "data": [0, -4.08, null]},
            "lowConfidenceReceiveSparkLine": {"totalChange": 70.07, "data": [0, 17.17, 55.36]},
            "detailsId": "divine-orb"
        }, {
            "currencyTypeName": "Rare item with one side only",
            "receive": {
                "value": 1.25,
                "sample_time_utc": "2026-11-09T13:46:48.4880118Z",
                "listing_count": 8
            },
            "chaosEquivalent": 1.25
        }],
        "currencyDetails": [{
            "id": 1,
            "name": "Chaos Orb",
            "icon": "https://web.poecdn.com/chaos.png",
            "tradeId": "chaos"
        }]
    }"#;

    #[test]
    fn parses_optional_sides_timestamps_and_listing_counts() {
        let overview: CurrencyOverview = serde_json::from_str(OVERVIEW_FIXTURE).unwrap();
        let divine = &overview.lines[0];
        let pay = divine.pay.as_ref().unwrap();
        let receive = divine.receive.as_ref().unwrap();

        assert_eq!(
            divine.chaos_equivalent_decimal().unwrap(),
            Some(Decimal::from(400))
        );
        assert_eq!(pay.value_decimal().unwrap(), Some(Decimal::new(25, 4)));
        assert_eq!(pay.listing_count, Some(205));
        assert_eq!(receive.listing_count, Some(67));
        assert!(pay.sample_time_utc.is_some());
        assert!(overview.lines[1].pay.is_none());
        assert_eq!(
            overview.currency_details[0].trade_id.as_deref(),
            Some("chaos")
        );
    }

    #[test]
    fn overview_url_encodes_league_and_category_parameters() {
        let mut url = Url::parse(&format!("{API_BASE}/stash/current/currency/overview")).unwrap();
        url.query_pairs_mut()
            .append_pair("league", "Hardcore League")
            .append_pair("type", CurrencyCategory::Fragment.as_query_value());

        assert!(url.as_str().contains("league=Hardcore+League"));
        assert!(url.as_str().ends_with("type=Fragment"));
    }

    #[test]
    fn parses_current_exchange_prices_and_reference_currency() {
        let fixture = r#"{
            "core": {
                "primary": "chaos",
                "secondary": "divine",
                "rates": {"divine": 0.001593},
                "items": [{"id":"chaos","name":"Chaos Orb","category":"Currency"}]
            },
            "lines": [{
                "id": "accelerating-catalyst",
                "primaryValue": 10.47,
                "volumePrimaryValue": 104.7,
                "maxVolumeCurrency": "divine",
                "maxVolumeRate": 60,
                "sparkline": {"totalChange": 233.3, "data": [1, null, 2]}
            }],
            "items": [{
                "id": "accelerating-catalyst",
                "name": "Accelerating Catalyst",
                "image": "/items/catalyst.png",
                "category": "Catalysts",
                "detailsId": "accelerating-catalyst"
            }]
        }"#;
        let overview: ExchangeOverview = serde_json::from_str(fixture).unwrap();

        assert_eq!(overview.core.primary, "chaos");
        assert_eq!(
            overview.lines[0]
                .primary_value
                .as_ref()
                .unwrap()
                .to_string(),
            "10.47"
        );
        assert_eq!(
            overview.lines[0]
                .volume_primary_value
                .as_ref()
                .unwrap()
                .to_string(),
            "104.7"
        );
        assert_eq!(overview.items[0].name, "Accelerating Catalyst");
        assert_eq!(overview.lines[0].sparkline.as_ref().unwrap().data.len(), 3);
    }

    #[test]
    fn parses_skill_gem_level_quality_and_corruption_variants() {
        let fixture = r#"{
            "lines": [{
                "id": 7299,
                "name": "Fireball",
                "icon": "https://example.test/fireball.png",
                "variant": "21/23c",
                "corrupted": true,
                "gemLevel": 21,
                "gemQuality": 23,
                "count": 1,
                "chaosValue": 4404,
                "listingCount": 4,
                "sparkLine": {"totalChange": 12.5, "data": [0, 5, null]}
            }, {
                "id": 3947,
                "name": "Fireball",
                "variant": "20",
                "gemLevel": 20,
                "chaosValue": 31.5,
                "listingCount": 14
            }]
        }"#;
        let overview: ItemOverview = serde_json::from_str(fixture).unwrap();

        assert_eq!(overview.lines[0].gem_level, Some(21));
        assert_eq!(overview.lines[0].gem_quality, Some(23));
        assert_eq!(overview.lines[0].corrupted, Some(true));
        assert_eq!(overview.lines[0].count, Some(1));
        assert_eq!(overview.lines[1].corrupted, None);
        assert_eq!(overview.lines[1].listing_count, Some(14));
        assert_eq!(overview.lines[0].spark_line.as_ref().unwrap().data.len(), 3);
    }

    #[test]
    fn parses_optional_gem_kind_metadata_and_preserves_unknown_categories() {
        let overview: ItemOverview = serde_json::from_str(
            r#"{"lines": [
                {"id": 1, "name": "Empower Support", "metadata": {"gemKind": "exceptional"}},
                {"id": 2, "name": "Cleave of Rage", "metadata": {"gemKind": "transfigured"}},
                {"id": 3, "name": "Sunder of Trarthus", "metadata": {"gemKind": "trarthus"}},
                {"id": 4, "name": "Fireball"},
                {"id": 5, "name": "Future Gem", "metadata": {"gemKind": "future"}},
                {"id": 6, "name": "Multistrike Support", "metadata": {}}
            ]}"#,
        )
        .unwrap();
        let kinds = overview
            .lines
            .iter()
            .map(|line| {
                line.metadata
                    .as_ref()
                    .and_then(|metadata| metadata.gem_kind.as_deref())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                Some("exceptional"),
                Some("transfigured"),
                Some("trarthus"),
                None,
                Some("future"),
                None
            ]
        );
    }
}
