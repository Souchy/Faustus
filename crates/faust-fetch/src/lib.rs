use std::collections::BTreeMap;

use bytes::Bytes;
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const EXCHANGE_URL: &str = "https://web.poecdn.com/api/currency-exchange";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ExchangeCursor(u64);

impl ExchangeCursor {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExchangePage {
    pub next_change_id: ExchangeCursor,
    pub markets: Vec<ExchangeMarket>,
}

impl ExchangePage {
    pub fn is_caught_up(&self, requested_cursor: Option<ExchangeCursor>) -> bool {
        requested_cursor.is_some_and(|cursor| self.next_change_id == cursor)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExchangeMarket {
    pub league: String,
    pub market_id: String,
    pub market_pair: Vec<String>,
    pub volume_traded: BTreeMap<String, u64>,
    pub lowest_stock: BTreeMap<String, u64>,
    pub highest_stock: BTreeMap<String, u64>,
    pub lowest_ratio: BTreeMap<String, u64>,
    pub highest_ratio: BTreeMap<String, u64>,
}

#[derive(Clone, Debug)]
pub struct FetchedPage {
    pub requested_cursor: Option<ExchangeCursor>,
    pub page: ExchangePage,
    pub raw_json: Bytes,
    pub metadata: ResponseMetadata,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResponseMetadata {
    pub etag: Option<String>,
    pub cache_control: Option<String>,
    pub retry_after: Option<String>,
    pub rate_limit_headers: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("failed to create HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    #[error("request to the exchange API failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("exchange API returned HTTP {status}")]
    HttpStatus {
        status: StatusCode,
        retry_after: Option<String>,
    },
    #[error("exchange API returned invalid JSON: {0}")]
    Decode(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct FaustClient {
    http: Client,
}

impl FaustClient {
    pub fn new(user_agent: &str) -> Result<Self, FetchError> {
        let http = Client::builder()
            .user_agent(user_agent)
            .build()
            .map_err(FetchError::Client)?;

        Ok(Self { http })
    }

    pub async fn fetch_page(
        &self,
        requested_cursor: Option<ExchangeCursor>,
    ) -> Result<FetchedPage, FetchError> {
        let url = match requested_cursor {
            Some(cursor) => format!("{EXCHANGE_URL}/{}", cursor.value()),
            None => EXCHANGE_URL.to_owned(),
        };
        let response = self.http.get(url).send().await?;
        let status = response.status();
        let metadata = response_metadata(response.headers());

        if !status.is_success() {
            return Err(FetchError::HttpStatus {
                status,
                retry_after: metadata.retry_after,
            });
        }

        let raw_json = response.bytes().await?;
        let page = serde_json::from_slice(&raw_json)?;

        Ok(FetchedPage {
            requested_cursor,
            page,
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
    let rate_limit_headers = headers
        .iter()
        .filter(|(name, _)| name.as_str().starts_with("x-rate-limit-"))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect();

    ResponseMetadata {
        etag: get("etag"),
        cache_control: get("cache-control"),
        retry_after: get("retry-after"),
        rate_limit_headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
        "next_change_id": 1722027600,
        "markets": [{
            "league": "Settlers",
            "market_id": "Metadata/Items/Currency/CurrencyPortal|Metadata/Items/Currency/CurrencyIdentification",
            "market_pair": [
                "Metadata/Items/Currency/CurrencyPortal",
                "Metadata/Items/Currency/CurrencyIdentification"
            ],
            "volume_traded": {
                "Metadata/Items/Currency/CurrencyPortal": 3,
                "Metadata/Items/Currency/CurrencyIdentification": 4
            },
            "lowest_stock": {
                "Metadata/Items/Currency/CurrencyPortal": 0,
                "Metadata/Items/Currency/CurrencyIdentification": 0
            },
            "highest_stock": {
                "Metadata/Items/Currency/CurrencyPortal": 2,
                "Metadata/Items/Currency/CurrencyIdentification": 2
            },
            "lowest_ratio": {
                "Metadata/Items/Currency/CurrencyPortal": 1,
                "Metadata/Items/Currency/CurrencyIdentification": 2
            },
            "highest_ratio": {
                "Metadata/Items/Currency/CurrencyPortal": 1,
                "Metadata/Items/Currency/CurrencyIdentification": 1
            }
        }]
    }"#;

    #[test]
    fn parses_pair_keyed_market_statistics() {
        let page: ExchangePage = serde_json::from_str(FIXTURE).unwrap();
        let market = &page.markets[0];

        assert_eq!(page.next_change_id.value(), 1_722_027_600);
        assert_eq!(market.market_pair.len(), 2);
        assert_eq!(market.volume_traded.values().sum::<u64>(), 7);
        assert_eq!(
            market
                .lowest_ratio
                .get("Metadata/Items/Currency/CurrencyPortal"),
            Some(&1)
        );
    }

    #[test]
    fn only_a_repeated_cursor_means_caught_up() {
        let page: ExchangePage = serde_json::from_str(FIXTURE).unwrap();

        assert!(!page.is_caught_up(None));
        assert!(!page.is_caught_up(Some(ExchangeCursor::new(1_722_027_599))));
        assert!(page.is_caught_up(Some(ExchangeCursor::new(1_722_027_600))));
    }
}
