use std::{path::Path, time::Duration};

use chrono::{DateTime, Utc};
use faustus_fetch::{ExchangeCursor, FetchedPage};
use market_core::ExchangeMarketSnapshot;
use sha2::{Digest, Sha256};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use thiserror::Error;

const EXCHANGE_PROVIDER_KEY: &str = "ggg_exchange";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite operation failed: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("failed to encode market data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stored exchange cursor is invalid: {0}")]
    Cursor(#[from] std::num::ParseIntError),
}

#[derive(Clone, Debug)]
pub struct SqliteStore {
    pool: SqlitePool,
}

#[derive(Clone, Copy, Debug)]
pub struct NinjaSnapshot<'a> {
    pub league_id: &'a str,
    pub category: &'a str,
    pub fetched_at: DateTime<Utc>,
    pub raw_json: &'a [u8],
    pub etag: Option<&'a str>,
    pub cache_control: Option<&'a str>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheValidators {
    pub etag: Option<String>,
    pub cache_control: Option<String>,
}

impl SqliteStore {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        migrate(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn record_exchange_page(
        &self,
        fetched_page: &FetchedPage,
        fetched_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let requested_cursor_key = fetched_page
            .requested_cursor
            .map(|cursor| cursor.value().to_string())
            .unwrap_or_default();
        let next_change_id = fetched_page.page.next_change_id.value().to_string();
        let fetched_at = fetched_at.to_rfc3339();
        let content_hash = content_hash(&fetched_page.raw_json);
        let mut transaction = self.pool.begin().await?;

        sqlx::query(
            r#"INSERT OR IGNORE INTO exchange_pages
                    (requested_cursor_key, next_change_id, fetched_at, content_hash, raw_json)
                    VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(&requested_cursor_key)
        .bind(&next_change_id)
        .bind(&fetched_at)
        .bind(&content_hash)
        .bind(fetched_page.raw_json.as_ref())
        .execute(&mut *transaction)
        .await?;

        let page_id: i64 = sqlx::query_scalar(
            "SELECT id FROM exchange_pages WHERE requested_cursor_key = ? AND content_hash = ?",
        )
        .bind(&requested_cursor_key)
        .bind(&content_hash)
        .fetch_one(&mut *transaction)
        .await?;

        for market in &fetched_page.page.markets {
            sqlx::query(
                     r#"INSERT OR IGNORE INTO exchange_markets
                         (page_id, league, market_id, market_pair_json, volume_traded_json,
                          lowest_stock_json, highest_stock_json, lowest_ratio_json, highest_ratio_json)
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            )
            .bind(page_id)
            .bind(&market.league)
            .bind(&market.market_id)
            .bind(serde_json::to_string(&market.market_pair)?)
            .bind(serde_json::to_string(&market.volume_traded)?)
            .bind(serde_json::to_string(&market.lowest_stock)?)
            .bind(serde_json::to_string(&market.highest_stock)?)
            .bind(serde_json::to_string(&market.lowest_ratio)?)
            .bind(serde_json::to_string(&market.highest_ratio)?)
            .execute(&mut *transaction)
            .await?;
        }

        sqlx::query(
            r#"INSERT INTO collector_state
               (provider_key, cursor, last_checked_at, last_success_at, last_error)
               VALUES (?, ?, ?, ?, NULL)
               ON CONFLICT(provider_key) DO UPDATE SET
                   cursor = excluded.cursor,
                   last_checked_at = excluded.last_checked_at,
                   last_success_at = excluded.last_success_at,
                   last_error = NULL"#,
        )
        .bind(EXCHANGE_PROVIDER_KEY)
        .bind(&next_change_id)
        .bind(&fetched_at)
        .bind(&fetched_at)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(())
    }

    pub async fn record_ninja_snapshot(
        &self,
        snapshot: NinjaSnapshot<'_>,
    ) -> Result<(), StoreError> {
        let provider_key = ninja_provider_key(snapshot.league_id, snapshot.category);
        let fetched_at = snapshot.fetched_at.to_rfc3339();
        let content_hash = content_hash(snapshot.raw_json);
        let mut transaction = self.pool.begin().await?;

        sqlx::query(
                r#"INSERT OR IGNORE INTO ninja_snapshots
                       (league_id, category, fetched_at, content_hash, etag, cache_control, raw_json)
                    VALUES (?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind(snapshot.league_id)
        .bind(snapshot.category)
        .bind(&fetched_at)
        .bind(&content_hash)
        .bind(snapshot.etag)
        .bind(snapshot.cache_control)
        .bind(snapshot.raw_json)
        .execute(&mut *transaction)
        .await?;

        upsert_provider_state(
            &mut transaction,
            &provider_key,
            &fetched_at,
            snapshot.etag,
            snapshot.cache_control,
        )
        .await?;

        transaction.commit().await?;
        Ok(())
    }

    pub async fn record_ninja_not_modified(
        &self,
        league_id: &str,
        category: &str,
        checked_at: DateTime<Utc>,
        etag: Option<&str>,
        cache_control: Option<&str>,
    ) -> Result<(), StoreError> {
        let provider_key = ninja_provider_key(league_id, category);
        let checked_at = checked_at.to_rfc3339();
        let mut transaction = self.pool.begin().await?;

        upsert_provider_state(
            &mut transaction,
            &provider_key,
            &checked_at,
            etag,
            cache_control,
        )
        .await?;

        transaction.commit().await?;
        Ok(())
    }

    pub async fn exchange_page_count(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM exchange_pages")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn exchange_market_count(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM exchange_markets")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn ninja_snapshot_count(&self) -> Result<i64, StoreError> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM ninja_snapshots")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn ninja_cache_validators(
        &self,
        league_id: &str,
        category: &str,
    ) -> Result<CacheValidators, StoreError> {
        let provider_key = ninja_provider_key(league_id, category);
        let validators: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT etag, cache_control FROM collector_state WHERE provider_key = ?",
        )
        .bind(provider_key)
        .fetch_optional(&self.pool)
        .await?;

        Ok(match validators {
            Some((etag, cache_control)) => CacheValidators {
                etag,
                cache_control,
            },
            None => CacheValidators::default(),
        })
    }

    pub async fn exchange_cursor(&self) -> Result<Option<ExchangeCursor>, StoreError> {
        let cursor: Option<String> =
            sqlx::query_scalar("SELECT cursor FROM collector_state WHERE provider_key = ?")
                .bind(EXCHANGE_PROVIDER_KEY)
                .fetch_optional(&self.pool)
                .await?
                .flatten();

        Ok(cursor
            .and_then(|value| value.parse::<u64>().ok())
            .map(ExchangeCursor::new))
    }

    pub async fn latest_exchange_markets(
        &self,
        league: Option<&str>,
        limit: i64,
    ) -> Result<Vec<ExchangeMarketSnapshot>, StoreError> {
        let rows = sqlx::query(
            "WITH ranked_markets AS (
                 SELECT markets.league, markets.market_id, markets.market_pair_json,
                        markets.volume_traded_json, markets.lowest_stock_json,
                        markets.highest_stock_json, markets.lowest_ratio_json,
                        markets.highest_ratio_json, pages.next_change_id,
                        ROW_NUMBER() OVER (
                            PARTITION BY markets.league, markets.market_id
                            ORDER BY CAST(pages.next_change_id AS INTEGER) DESC, pages.id DESC
                        ) AS row_number
                 FROM exchange_markets AS markets
                 JOIN exchange_pages AS pages ON pages.id = markets.page_id
                 WHERE (? IS NULL OR markets.league = ?)
             )
             SELECT league, market_id, market_pair_json, volume_traded_json,
                    lowest_stock_json, highest_stock_json, lowest_ratio_json,
                    highest_ratio_json, next_change_id
             FROM ranked_markets
             WHERE row_number = 1
             ORDER BY league, market_id
             LIMIT ?",
        )
        .bind(league)
        .bind(league)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                use sqlx::Row;

                Ok(ExchangeMarketSnapshot {
                    league: row.try_get("league")?,
                    market_id: row.try_get("market_id")?,
                    market_pair: serde_json::from_str(
                        &row.try_get::<String, _>("market_pair_json")?,
                    )?,
                    volume_traded: serde_json::from_str(
                        &row.try_get::<String, _>("volume_traded_json")?,
                    )?,
                    lowest_stock: serde_json::from_str(
                        &row.try_get::<String, _>("lowest_stock_json")?,
                    )?,
                    highest_stock: serde_json::from_str(
                        &row.try_get::<String, _>("highest_stock_json")?,
                    )?,
                    lowest_ratio: serde_json::from_str(
                        &row.try_get::<String, _>("lowest_ratio_json")?,
                    )?,
                    highest_ratio: serde_json::from_str(
                        &row.try_get::<String, _>("highest_ratio_json")?,
                    )?,
                    next_change_id: row.try_get::<String, _>("next_change_id")?.parse()?,
                })
            })
            .collect()
    }
}

async fn migrate(pool: &SqlitePool) -> Result<(), StoreError> {
    sqlx::query("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY)")
        .execute(pool)
        .await?;

    let version: Option<i64> =
        sqlx::query_scalar("SELECT version FROM schema_migrations WHERE version = 1")
            .fetch_optional(pool)
            .await?;
    if version.is_none() {
        let mut transaction = pool.begin().await?;
        sqlx::raw_sql(include_str!("../migrations/0001_initial.sql"))
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO schema_migrations (version) VALUES (1)")
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
    }

    Ok(())
}

async fn upsert_provider_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_key: &str,
    checked_at: &str,
    etag: Option<&str>,
    cache_control: Option<&str>,
) -> Result<(), StoreError> {
    sqlx::query(
        r#"INSERT INTO collector_state
           (provider_key, last_checked_at, last_success_at, etag, cache_control)
           VALUES (?, ?, ?, ?, ?)
           ON CONFLICT(provider_key) DO UPDATE SET
               last_checked_at = excluded.last_checked_at,
               last_success_at = excluded.last_success_at,
               etag = COALESCE(excluded.etag, collector_state.etag),
               cache_control = COALESCE(excluded.cache_control, collector_state.cache_control),
               last_error = NULL"#,
    )
    .bind(provider_key)
    .bind(checked_at)
    .bind(checked_at)
    .bind(etag)
    .bind(cache_control)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn ninja_provider_key(league_id: &str, category: &str) -> String {
    format!("poe_ninja:{league_id}:{category}")
}

fn content_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bytes::Bytes;
    use chrono::TimeZone;
    use faustus_fetch::{ExchangeMarket, ExchangePage, ResponseMetadata};

    use super::*;

    async fn store() -> (tempfile::TempDir, SqliteStore) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("market.sqlite3");
        let store = SqliteStore::connect(path).await.unwrap();
        (directory, store)
    }

    #[tokio::test]
    async fn records_exchange_page_and_cursor_atomically_and_idempotently() {
        let (_directory, store) = store().await;
        let raw_json = Bytes::from_static(br#"{"next_change_id":1722027600,"markets":[]}"#);
        let fetched_page = FetchedPage {
            requested_cursor: None,
            page: ExchangePage {
                next_change_id: ExchangeCursor::new(1_722_027_600),
                markets: vec![ExchangeMarket {
                    league: "Settlers".to_owned(),
                    market_id: "A|B".to_owned(),
                    market_pair: vec!["A".to_owned(), "B".to_owned()],
                    volume_traded: BTreeMap::from([("A".to_owned(), 2), ("B".to_owned(), 3)]),
                    lowest_stock: BTreeMap::new(),
                    highest_stock: BTreeMap::new(),
                    lowest_ratio: BTreeMap::new(),
                    highest_ratio: BTreeMap::new(),
                }],
            },
            raw_json,
            metadata: ResponseMetadata::default(),
        };
        let fetched_at = Utc.with_ymd_and_hms(2026, 11, 9, 14, 0, 0).unwrap();

        store
            .record_exchange_page(&fetched_page, fetched_at)
            .await
            .unwrap();
        store
            .record_exchange_page(&fetched_page, fetched_at)
            .await
            .unwrap();

        assert_eq!(store.exchange_page_count().await.unwrap(), 1);
        assert_eq!(store.exchange_market_count().await.unwrap(), 1);
        assert_eq!(
            store.exchange_cursor().await.unwrap(),
            Some(ExchangeCursor::new(1_722_027_600))
        );
        let markets = store
            .latest_exchange_markets(Some("Settlers"), 10)
            .await
            .unwrap();
        assert_eq!(markets.len(), 1);
        assert_eq!(markets[0].volume_traded.get("A"), Some(&2));
    }

    #[tokio::test]
    async fn records_changed_ninja_payloads_but_not_304_checks_as_snapshots() {
        let (_directory, store) = store().await;
        let fetched_at = Utc.with_ymd_and_hms(2026, 11, 9, 14, 0, 0).unwrap();
        let payload = br#"{"lines":[],"currencyDetails":[]}"#;
        let snapshot = NinjaSnapshot {
            league_id: "Standard",
            category: "Currency",
            fetched_at,
            raw_json: payload,
            etag: Some("W/\"sample\""),
            cache_control: Some("public, max-age=300"),
        };

        store.record_ninja_snapshot(snapshot).await.unwrap();
        store
            .record_ninja_not_modified(
                "Standard",
                "Currency",
                fetched_at + chrono::Duration::minutes(5),
                Some("W/\"sample\""),
                Some("public, max-age=300"),
            )
            .await
            .unwrap();

        assert_eq!(store.ninja_snapshot_count().await.unwrap(), 1);
    }
}
