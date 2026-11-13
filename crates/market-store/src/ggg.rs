use faustus_fetch::ExchangePage;
use market_core::ggg::{Coverage, HOUR, History, HistoryPoint, HourFailure, Item, Market};
use sqlx::{Row, Sqlite, Transaction, sqlite::SqliteRow};

use crate::{SqliteStore, StoreError};

const SELECT_MARKETS: &str = r#"
SELECT o.*, p.market,
 a.metadata a_id, a.name a_name, a.category a_category, a.icon a_icon,
 b.metadata b_id, b.name b_name, b.category b_category, b.icon b_icon
FROM ggg_observations o JOIN ggg_leagues l ON l.id=o.league
JOIN ggg_pairs p ON p.id=o.pair
JOIN ggg_items a ON a.id=p.a JOIN ggg_items b ON b.id=p.b
WHERE l.name=? AND o.hour BETWEEN ? AND ? AND (? IS NULL OR p.market=?)
ORDER BY o.hour, p.market"#;

fn validate(league: &str, start: i64, end: i64) -> Result<(), StoreError> {
    market_core::ggg::validate_range(league, start, end, chrono::Utc::now().timestamp())
        .map_err(StoreError::Ggg)
}

async fn league_id(tx: &mut Transaction<'_, Sqlite>, league: &str) -> Result<i64, StoreError> {
    sqlx::query("INSERT INTO ggg_leagues(name) VALUES (?) ON CONFLICT DO NOTHING")
        .bind(league)
        .execute(&mut **tx)
        .await?;
    Ok(
        sqlx::query_scalar("SELECT id FROM ggg_leagues WHERE name=?")
            .bind(league)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn intern(tx: &mut Transaction<'_, Sqlite>, item: &Item) -> Result<i64, StoreError> {
    sqlx::query(
        "INSERT INTO ggg_items(metadata,name,category,icon) VALUES (?,?,?,?)
         ON CONFLICT(metadata) DO UPDATE SET
         name=CASE WHEN excluded.name<>excluded.metadata THEN excluded.name ELSE ggg_items.name END,
         category=CASE WHEN excluded.category<>'Unknown' THEN excluded.category ELSE ggg_items.category END,
         icon=COALESCE(excluded.icon,ggg_items.icon)",
    )
    .bind(&item.id)
    .bind(&item.name)
    .bind(&item.category)
    .bind(&item.icon)
    .execute(&mut **tx)
    .await?;
    Ok(
        sqlx::query_scalar("SELECT id FROM ggg_items WHERE metadata=?")
            .bind(&item.id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

fn unknown(id: &str) -> Item {
    Item {
        id: id.into(),
        name: id.into(),
        category: "Unknown".into(),
        icon: None,
    }
}

fn market(row: &SqliteRow) -> Result<Market, StoreError> {
    let item = |prefix: &str| -> Result<Item, sqlx::Error> {
        Ok(Item {
            id: row.try_get(format!("{prefix}_id").as_str())?,
            name: row.try_get(format!("{prefix}_name").as_str())?,
            category: row.try_get(format!("{prefix}_category").as_str())?,
            icon: row.try_get(format!("{prefix}_icon").as_str())?,
        })
    };
    let stat = |key| -> Result<String, sqlx::Error> { Ok(row.try_get::<i64, _>(key)?.to_string()) };
    Ok(Market {
        id: row.try_get("market")?,
        a: item("a")?,
        b: item("b")?,
        volume_a: stat("volume_a")?,
        volume_b: stat("volume_b")?,
        low_stock_a: stat("low_stock_a")?,
        low_stock_b: stat("low_stock_b")?,
        high_stock_a: stat("high_stock_a")?,
        high_stock_b: stat("high_stock_b")?,
        low_ratio_a: stat("low_ratio_a")?,
        low_ratio_b: stat("low_ratio_b")?,
        high_ratio_a: stat("high_ratio_a")?,
        high_ratio_b: stat("high_ratio_b")?,
    })
}

impl SqliteStore {
    pub async fn ggg_leagues(&self) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT name FROM ggg_leagues l WHERE EXISTS
             (SELECT 1 FROM ggg_hours h WHERE h.league=l.id AND h.complete=1)
             ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn ggg_metadata_summary(
        &self,
        league: &str,
        start: i64,
        end: i64,
    ) -> Result<(u64, u64, u64), StoreError> {
        validate(league, start, end)?;
        let (total, unknown, uncategorized): (i64, i64, i64) = sqlx::query_as(
            "WITH pairs AS (
               SELECT p.a,p.b FROM ggg_observations o JOIN ggg_pairs p ON p.id=o.pair
               JOIN ggg_leagues l ON l.id=o.league WHERE l.name=? AND o.hour BETWEEN ? AND ?
             ), used AS (SELECT a id FROM pairs UNION SELECT b id FROM pairs)
             SELECT count(*),
               COALESCE(sum(CASE WHEN i.name=i.metadata OR trim(i.name)='' THEN 1 ELSE 0 END),0),
               COALESCE(sum(CASE WHEN i.category='Unknown' THEN 1 ELSE 0 END),0)
             FROM used JOIN ggg_items i ON i.id=used.id",
        )
        .bind(league)
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await?;
        Ok((total as u64, unknown as u64, uncategorized as u64))
    }

    pub async fn save_ggg_fallback_items(&self, items: &[Item]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for item in items {
            intern(&mut tx, item).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn ggg_catalog_updated_at(&self) -> Result<Option<i64>, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT fetched_at FROM ggg_catalog_state WHERE id=1")
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn save_ggg_catalog(
        &self,
        items: &[Item],
        fetched_at: i64,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for item in items {
            intern(&mut tx, item).await?;
        }
        sqlx::query(
            "INSERT INTO ggg_catalog_state(id,fetched_at) VALUES(1,?)
            ON CONFLICT(id) DO UPDATE SET fetched_at=excluded.fetched_at",
        )
        .bind(fetched_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn ggg_markets(&self, league: &str, hour: i64) -> Result<Vec<Market>, StoreError> {
        validate(league, hour, hour)?;
        sqlx::query(SELECT_MARKETS)
            .bind(league)
            .bind(hour)
            .bind(hour)
            .bind(Option::<&str>::None)
            .bind(Option::<&str>::None)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(market)
            .collect()
    }

    pub async fn ggg_first_trade_hours(
        &self,
        league: &str,
        start: i64,
        end: i64,
        markets: &[Market],
    ) -> Result<Vec<(String, i64)>, StoreError> {
        validate(league, start, end)?;
        let ids = markets.iter().map(|market| &market.id).collect::<Vec<_>>();
        Ok(sqlx::query_as(
            "SELECT p.market,(SELECT o.hour FROM ggg_observations o INDEXED BY ggg_history
               JOIN ggg_hours h ON h.league=o.league AND h.hour=o.hour AND h.complete=1
               WHERE o.league=l.id AND o.pair=p.id AND o.hour>=?2 AND o.hour<?3
                 AND o.volume_a>0 AND o.volume_b>0 ORDER BY o.hour LIMIT 1) AS hour
             FROM ggg_pairs p CROSS JOIN ggg_leagues l
             WHERE l.name=?1 AND p.market IN (SELECT value FROM json_each(?4))
               AND hour IS NOT NULL",
        )
        .bind(league)
        .bind(start)
        .bind(end)
        .bind(serde_json::to_string(&ids)?)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn ggg_hour_complete(&self, league: &str, hour: i64) -> Result<bool, StoreError> {
        validate(league, hour, hour)?;
        Ok(sqlx::query_scalar::<_, bool>(
            "SELECT h.complete FROM ggg_hours h JOIN ggg_leagues l ON l.id=h.league
             WHERE l.name=? AND h.hour=?",
        )
        .bind(league)
        .bind(hour)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false))
    }

    pub async fn ggg_coverage(
        &self,
        league: &str,
        start: i64,
        end: i64,
    ) -> Result<Coverage, StoreError> {
        validate(league, start, end)?;
        let rows = sqlx::query(
            "SELECT h.hour,h.complete,h.error FROM ggg_hours h JOIN ggg_leagues l ON l.id=h.league
             WHERE l.name=? AND h.hour BETWEEN ? AND ? ORDER BY h.hour",
        )
        .bind(league)
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;
        let mut result = Coverage {
            league: league.into(),
            start,
            end,
            expected_hours: ((end - start) / HOUR + 1) as u64,
            completed_hours: vec![],
            failed_hours: vec![],
        };
        for row in rows {
            let hour = row.try_get("hour")?;
            if row.try_get::<bool, _>("complete")? {
                result.completed_hours.push(hour);
            } else if let Some(error) = row.try_get::<Option<String>, _>("error")? {
                result.failed_hours.push(HourFailure { hour, error });
            }
        }
        Ok(result)
    }

    pub async fn ggg_history(
        &self,
        league: &str,
        id: &str,
        start: i64,
        end: i64,
    ) -> Result<History, StoreError> {
        let coverage = self.ggg_coverage(league, start, end).await?;
        let rows = sqlx::query(SELECT_MARKETS)
            .bind(league)
            .bind(start)
            .bind(end)
            .bind(id)
            .bind(id)
            .fetch_all(&self.pool)
            .await?;
        let points = rows
            .iter()
            .map(|row| {
                Ok(HistoryPoint {
                    hour: row.try_get("hour")?,
                    market: market(row)?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        Ok(History {
            league: league.into(),
            market: id.into(),
            points,
            coverage,
        })
    }

    pub async fn record_ggg_failure(
        &self,
        league: &str,
        hour: i64,
        error: &str,
    ) -> Result<(), StoreError> {
        validate(league, hour, hour)?;
        let mut tx = self.pool.begin().await?;
        let league = league_id(&mut tx, league).await?;
        sqlx::query(
            "INSERT INTO ggg_hours(league,hour,complete,error) VALUES(?,?,0,?)
            ON CONFLICT(league,hour) DO UPDATE SET error=excluded.error WHERE ggg_hours.complete=0",
        )
        .bind(league)
        .bind(hour)
        .bind(error)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn record_ggg_hour(
        &self,
        league: &str,
        hour: i64,
        page: &ExchangePage,
    ) -> Result<(), StoreError> {
        validate(league, hour, hour)?;
        if page.next_change_id.value() != (hour + HOUR) as u64 {
            return Err(StoreError::Ggg(
                "Hourly page next_change_id must equal requested hour + 3600.".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let league_key = league_id(&mut tx, league).await?;
        let complete: Option<bool> =
            sqlx::query_scalar("SELECT complete FROM ggg_hours WHERE league=? AND hour=?")
                .bind(league_key)
                .bind(hour)
                .fetch_optional(&mut *tx)
                .await?;
        if complete == Some(true) {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO ggg_hours(league,hour,complete,error) VALUES(?,?,1,NULL)
            ON CONFLICT(league,hour) DO UPDATE SET complete=1,error=NULL",
        )
        .bind(league_key)
        .bind(hour)
        .execute(&mut *tx)
        .await?;
        for source in page.markets.iter().filter(|m| m.league == league) {
            if source.market_pair.len() != 2
                || source.market_pair[0] == source.market_pair[1]
                || source.market_pair.iter().any(String::is_empty)
                || source.market_id.is_empty()
            {
                return Err(StoreError::Ggg(
                    "Market must identify two distinct items.".into(),
                ));
            }
            let a = intern(&mut tx, &unknown(&source.market_pair[0])).await?;
            let b = intern(&mut tx, &unknown(&source.market_pair[1])).await?;
            sqlx::query("INSERT INTO ggg_pairs(market,a,b) VALUES(?,?,?) ON CONFLICT DO NOTHING")
                .bind(&source.market_id)
                .bind(a)
                .bind(b)
                .execute(&mut *tx)
                .await?;
            let (pair, stored_a, stored_b): (i64, i64, i64) =
                sqlx::query_as("SELECT id,a,b FROM ggg_pairs WHERE market=?")
                    .bind(&source.market_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if (a, b) != (stored_a, stored_b) {
                return Err(StoreError::Ggg("Market identity changed pair.".into()));
            }
            let mut stats = Vec::with_capacity(10);
            for values in [
                &source.volume_traded,
                &source.lowest_stock,
                &source.highest_stock,
                &source.lowest_ratio,
                &source.highest_ratio,
            ] {
                for id in &source.market_pair {
                    let value = values
                        .get(id)
                        .ok_or_else(|| StoreError::Ggg(format!("Missing statistic for {id}")))?;
                    stats.push(i64::try_from(*value).map_err(|_| {
                        StoreError::Ggg(format!("Statistic exceeds SQLite i64 maximum for {id}"))
                    })?);
                }
            }
            let mut query = sqlx::query("INSERT INTO ggg_observations
                (league,hour,pair,volume_a,volume_b,low_stock_a,low_stock_b,high_stock_a,high_stock_b,
                 low_ratio_a,low_ratio_b,high_ratio_a,high_ratio_b) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)")
                .bind(league_key).bind(hour).bind(pair);
            for stat in stats {
                query = query.bind(stat);
            }
            query.execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use faustus_fetch::{ExchangeCursor, ExchangeMarket};
    use std::collections::BTreeMap;

    async fn store() -> SqliteStore {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::migrate(&pool).await.unwrap();
        SqliteStore { pool }
    }

    fn page(hour: i64) -> ExchangePage {
        let stat = |a, b| BTreeMap::from([("A".into(), a), ("B".into(), b)]);
        ExchangePage {
            next_change_id: ExchangeCursor::new((hour + HOUR) as u64),
            markets: vec![ExchangeMarket {
                league: "Test".into(),
                market_id: "A|B".into(),
                market_pair: vec!["A".into(), "B".into()],
                volume_traded: stat(0, 0),
                lowest_stock: stat(1, 2),
                highest_stock: stat(3, 4),
                lowest_ratio: stat(5, 6),
                highest_ratio: stat(7, 8),
            }],
        }
    }

    #[tokio::test]
    async fn hourly_roundtrip_scoping_idempotency_history_and_coverage() {
        let store = store().await;
        store
            .record_ggg_failure("Test", 0, "rate limited")
            .await
            .unwrap();
        store
            .record_ggg_hour("Test", HOUR, &page(HOUR))
            .await
            .unwrap();
        store
            .record_ggg_hour("Test", HOUR, &page(HOUR))
            .await
            .unwrap();
        store
            .record_ggg_hour("Other", HOUR, &page(HOUR))
            .await
            .unwrap();
        assert!(store.ggg_markets("Other", HOUR).await.unwrap().is_empty());
        assert!(store.ggg_hour_complete("Other", HOUR).await.unwrap());
        let history = store.ggg_history("Test", "A|B", 0, HOUR * 2).await.unwrap();
        assert_eq!(history.points.len(), 1);
        assert_eq!(history.coverage.expected_hours, 3);
        assert_eq!(history.coverage.completed_hours, vec![HOUR]);
        assert_eq!(history.coverage.failed_hours.len(), 1);
        let m = &history.points[0].market;
        assert_eq!(
            [
                &m.volume_a,
                &m.volume_b,
                &m.low_stock_a,
                &m.low_stock_b,
                &m.high_stock_a,
                &m.high_stock_b,
                &m.low_ratio_a,
                &m.low_ratio_b,
                &m.high_ratio_a,
                &m.high_ratio_b
            ],
            ["0", "0", "1", "2", "3", "4", "5", "6", "7", "8"]
        );
        store.record_ggg_hour("Test", 0, &page(0)).await.unwrap();
        store
            .record_ggg_failure("Test", 0, "late failure")
            .await
            .unwrap();
        assert!(
            store
                .ggg_coverage("Test", 0, HOUR)
                .await
                .unwrap()
                .failed_hours
                .is_empty()
        );
        let items: i64 = sqlx::query_scalar("SELECT count(*) FROM ggg_items")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(items, 2);
    }

    #[tokio::test]
    async fn corrupt_cursor_overflow_and_partial_hour_roll_back() {
        let store = store().await;
        let mut bad = page(0);
        bad.next_change_id = ExchangeCursor::new(0);
        assert!(store.record_ggg_hour("Test", 0, &bad).await.is_err());
        let mut bad = page(0);
        let mut invalid = bad.markets[0].clone();
        invalid.volume_traded.insert("A".into(), u64::MAX);
        bad.markets.push(invalid);
        assert!(
            store
                .record_ggg_hour("Test", 0, &bad)
                .await
                .unwrap_err()
                .to_string()
                .contains("i64")
        );
        assert!(!store.ggg_hour_complete("Test", 0).await.unwrap());
        assert!(store.ggg_markets("Test", 0).await.unwrap().is_empty());
        let items: i64 = sqlx::query_scalar("SELECT count(*) FROM ggg_items")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(items, 0);
        let mut bad = page(0);
        bad.markets[0].highest_ratio.clear();
        assert!(store.record_ggg_hour("Test", 0, &bad).await.is_err());
        assert!(!store.ggg_hour_complete("Test", 0).await.unwrap());
    }

    #[tokio::test]
    async fn catalog_is_shared_and_unknown_page_items_do_not_replace_verified_identity() {
        let store = store().await;
        store
            .save_ggg_catalog(
                &[Item {
                    id: "A".into(),
                    name: "Verified name".into(),
                    category: "Oil".into(),
                    icon: Some("verified.png".into()),
                }],
                123,
            )
            .await
            .unwrap();
        store.record_ggg_hour("Test", 0, &page(0)).await.unwrap();
        let market = &store.ggg_markets("Test", 0).await.unwrap()[0];
        assert_eq!(market.a.name, "Verified name");
        assert_eq!(market.a.category, "Oil");
        assert_eq!(market.a.icon.as_deref(), Some("verified.png"));
        assert_eq!(market.b.name, "B");
        assert_eq!(market.b.category, "Unknown");
        assert_eq!(store.ggg_catalog_updated_at().await.unwrap(), Some(123));
        store
            .save_ggg_fallback_items(&[unknown("A")])
            .await
            .unwrap();
        assert_eq!(store.ggg_catalog_updated_at().await.unwrap(), Some(123));
        assert_eq!(
            store.ggg_markets("Test", 0).await.unwrap()[0].a.name,
            "Verified name"
        );
        assert_eq!(
            store.ggg_metadata_summary("Test", 0, 0).await.unwrap(),
            (2, 1, 1)
        );
        store
            .record_ggg_failure("FailedOnly", 0, "offline")
            .await
            .unwrap();
        assert_eq!(store.ggg_leagues().await.unwrap(), vec!["Test"]);
    }
}
