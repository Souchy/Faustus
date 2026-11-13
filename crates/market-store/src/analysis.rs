use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use market_core::{
    analysis::{FlipRow, Observation, Period, SparkPoint},
    economy::Line,
};
use sqlx::Row;

use crate::{SqliteStore, StoreError};

impl SqliteStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn analysis_price_samples(
        &self,
        league: &str,
        category: &str,
        source: &str,
        item: &str,
        archive: Option<&str>,
        granularity: &str,
        currency: &str,
        hours: &[i64],
    ) -> Result<Vec<SparkPoint>, StoreError> {
        let daily = granularity == "daily";
        let times = hours
            .iter()
            .map(|hour| {
                let at = DateTime::from_timestamp(*hour, 0).expect("validated analysis timestamp");
                let start = if daily {
                    at.format("%Y-%m-%d").to_string()
                } else {
                    at.format("%Y-%m-%dT%H:%M:%S").to_string()
                };
                let end = if daily {
                    format!("{start}~")
                } else {
                    (at + chrono::Duration::hours(1))
                        .format("%Y-%m-%dT%H:%M:%S")
                        .to_string()
                };
                serde_json::json!({"time":hour,"start":start,"end":end})
            })
            .collect::<Vec<_>>();
        // Daily samples can seek the covering date index without random price-table reads.
        let (index, observation_time) = if daily {
            ("INDEXED BY economy_dates", "o.observed_at=s.start")
        } else {
            ("", "o.observed_at>=s.start AND o.observed_at<s.end")
        };
        let query = format!(
            "WITH samples AS (
               SELECT json_extract(value,'$.time') AS time,
                 json_extract(value,'$.start') AS start,json_extract(value,'$.end') AS end
               FROM json_each(?1)
             )
             SELECT s.time,(SELECT o.price FROM economy_observations o {index}
                 WHERE o.league=?2 AND o.category=?3 AND o.source=?4 AND o.item=?5
                   AND {observation_time} AND o.currency=?6
                   AND o.granularity=?7 AND coalesce(o.archive,'')=coalesce(?8,'')
                   AND (o.archive IS NULL OR EXISTS (SELECT 1 FROM economy_imports a
                     WHERE a.name=o.archive AND json_extract(a.status_json,'$.status')='completed'))
                 ORDER BY o.observed_at DESC LIMIT 1) AS price
             FROM samples s ORDER BY s.time"
        );
        let rows = sqlx::query(&query)
            .bind(serde_json::to_string(&times)?)
            .bind(league)
            .bind(category)
            .bind(source)
            .bind(item)
            .bind(currency)
            .bind(granularity)
            .bind(archive)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| SparkPoint {
                time: row.get("time"),
                price: row.get("price"),
            })
            .collect())
    }

    pub async fn analysis_currency_item(
        &self,
        league: &str,
        source: &str,
        name: &str,
    ) -> Result<Option<String>, StoreError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT item FROM economy_items INDEXED BY economy_analysis_metadata
             WHERE league=? AND category='Currency' AND source=? AND json_extract(line_json,'$.name')=? LIMIT 2",
        ).bind(league).bind(source).bind(name).fetch_all(&self.pool).await?;
        Ok(if ids.len() == 1 {
            ids.into_iter().next()
        } else {
            None
        })
    }

    pub async fn analysis_period(&self, league: &str) -> Result<Period, StoreError> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT observed_at,granularity FROM economy_observation_times
             WHERE league=? AND (archive='' OR archive IN
               (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed'))
             ORDER BY observed_at",
        )
        .bind(league)
        .fetch_all(&self.pool)
        .await?;
        let mut days = BTreeSet::new();
        let current_hour = Utc::now().timestamp().div_euclid(3600) * 3600;
        let completed_hours = sqlx::query_scalar::<_, i64>(
            "SELECT h.hour FROM ggg_hours h JOIN ggg_leagues l ON l.id=h.league
             WHERE l.name=? AND h.complete=1 ORDER BY h.hour",
        )
        .bind(league)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .collect::<BTreeSet<_>>();
        let exchange_hours = sqlx::query_scalar::<_, i64>(
            "SELECT h.hour FROM ggg_hours h JOIN ggg_leagues l ON l.id=h.league
             WHERE l.name=? AND h.complete=1 AND EXISTS(
               SELECT 1 FROM ggg_observations o WHERE o.league=h.league AND o.hour=h.hour
                 AND o.volume_a>0 AND o.volume_b>0)
             ORDER BY h.hour",
        )
        .bind(league)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .filter(|hour| *hour < current_hour)
        .collect::<BTreeSet<_>>();
        let mut hours = exchange_hours.clone();
        for (observed_at, granularity) in rows {
            if granularity == "daily" {
                days.insert(observed_at);
            } else {
                let timestamp = DateTime::parse_from_rfc3339(&observed_at).map_err(|error| {
                    sqlx::Error::Protocol(format!("Invalid saved observation timestamp: {error}"))
                })?;
                hours.insert(timestamp.timestamp().div_euclid(3600) * 3600);
            }
        }
        hours.retain(|hour| *hour < current_hour);
        let day_timestamp = |value: &str| -> Result<i64, StoreError> {
            let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|error| {
                sqlx::Error::Protocol(format!("Invalid saved archive date: {error}"))
            })?;
            Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
        };
        let mut start = hours.first().copied();
        let mut end = hours.last().copied();
        for day in &days {
            let value = day_timestamp(day)?;
            start = Some(start.map_or(value, |start| start.min(value)));
            let last_hour = (value + 23 * 3600).min(current_hour - 3600);
            end = Some(end.map_or(last_hour, |end| end.max(last_hour)));
        }
        let archives = self.economy_imports().await?;
        let mut archive_bounds = None::<(i64, i64)>;
        for archive in archives.iter().filter(|archive| {
            archive.status == "completed" && archive.league.as_deref() == Some(league)
        }) {
            if let (Some(first), Some(last)) = (&archive.start, &archive.end) {
                let first = day_timestamp(first)?;
                let last = (day_timestamp(last)? + 23 * 3600).min(current_hour - 3600);
                archive_bounds = Some(archive_bounds.map_or((first, last), |(start, end)| {
                    (start.min(first), end.max(last))
                }));
            }
        }
        let (mut bounds_source, mut warnings) = if let Some((first, last)) = archive_bounds {
            start = Some(start.map_or(first, |value| value.min(first)));
            end = Some(end.map_or(last, |value| value.max(last)));
            ("archive", vec!["Exact official league dates are unavailable; bounds use published archive dates and saved coverage. Ticks represent actual saved observations, not interpolated prices.".into()])
        } else if start.is_some() {
            ("observed", vec!["Official league dates are unavailable; bounds use saved observations. Import more history on Currency Exchange to extend coverage.".into()])
        } else {
            ("unavailable", vec!["No saved history or published archive bounds for this league. Import history before choosing a period.".into()])
        };
        // A partial import must not shrink a league to the small range it happened to fetch.
        let mut tightened = false;
        if let (Some(bound), Some(first)) = (start, exchange_hours.first().copied())
            && bound < first
            && (bound..first)
                .step_by(3600)
                .all(|hour| completed_hours.contains(&hour))
        {
            start = Some(first);
            tightened = true;
        }
        if let (Some(bound), Some(last)) = (end, exchange_hours.last().copied())
            && bound > last
            && (last + 3600..=bound)
                .step_by(3600)
                .all(|hour| completed_hours.contains(&hour))
        {
            end = Some(last);
            tightened = true;
        }
        if tightened {
            bounds_source = "observed";
            warnings = vec!["Official league datetimes are unavailable. Bounds use the observed active exchange window where fully fetched outer hours confirm no traded markets; empty imported hours do not extend it. Individual items can begin trading later or stop earlier.".into()];
        }
        hours.retain(|hour| {
            start.is_none_or(|bound| *hour >= bound) && end.is_none_or(|bound| *hour <= bound)
        });
        Ok(Period {
            league: league.into(),
            start,
            end,
            bounds_source: bounds_source.into(),
            hours: hours.into_iter().collect(),
            days: days.into_iter().collect(),
            warnings,
        })
    }

    pub async fn analysis_observations(
        &self,
        league: &str,
        hour: i64,
    ) -> Result<Vec<Observation>, StoreError> {
        self.analysis_endpoint(league, hour, true).await
    }

    pub async fn analysis_prices(
        &self,
        league: &str,
        hour: i64,
    ) -> Result<Vec<Observation>, StoreError> {
        self.analysis_endpoint(league, hour, false).await
    }

    pub async fn analysis_first_prices(
        &self,
        league: &str,
        start: i64,
        end: i64,
        templates: &[Observation],
    ) -> Result<Vec<Observation>, StoreError> {
        let start = DateTime::from_timestamp(start, 0)
            .ok_or_else(|| sqlx::Error::Protocol("Invalid analysis start".into()))?;
        let end = DateTime::from_timestamp(end, 0)
            .ok_or_else(|| sqlx::Error::Protocol("Invalid analysis end".into()))?;
        let mut groups = BTreeMap::<_, Vec<(usize, serde_json::Value)>>::new();
        for (index, row) in templates.iter().enumerate() {
            groups.entry(&row.granularity).or_default().push((
                index,
                serde_json::json!({
                    "index":index,"category":row.category,"source":row.source,
                    "item":row.line.id,"currency":row.currency,
                    "archive":row.archive,"granularity":row.granularity
                }),
            ));
        }
        let mut result = Vec::new();
        for (granularity, mut requests) in groups {
            let format = if granularity == "daily" {
                "%Y-%m-%d"
            } else {
                "%Y-%m-%dT%H:%M:%S"
            };
            let times = sqlx::query_scalar::<_, String>(
                "SELECT DISTINCT observed_at FROM economy_observation_times
                 WHERE league=? AND granularity=? AND observed_at>=? AND observed_at<?
                   AND (archive='' OR archive IN (SELECT name FROM economy_imports
                     WHERE json_extract(status_json,'$.status')='completed'))
                 ORDER BY observed_at",
            )
            .bind(league)
            .bind(granularity)
            .bind(start.format(format).to_string())
            .bind(end.format(format).to_string())
            .fetch_all(&self.pool)
            .await?;
            // Resolve identities in saved-time order using covering seeks, dropping each after its first price.
            for time in times {
                let values = requests.iter().map(|(_, value)| value).collect::<Vec<_>>();
                let rows = sqlx::query(
                    "WITH requests AS MATERIALIZED (
                   SELECT CAST(json_extract(value,'$.index') AS INTEGER) AS idx,
                     CAST(json_extract(value,'$.category') AS TEXT) AS category,
                     CAST(json_extract(value,'$.source') AS TEXT) AS source,
                     CAST(json_extract(value,'$.item') AS TEXT) AS item,
                     CAST(json_extract(value,'$.currency') AS TEXT) AS currency,
                     CAST(coalesce(json_extract(value,'$.archive'),'') AS TEXT) AS archive,
                     CAST(json_extract(value,'$.granularity') AS TEXT) AS granularity
                   FROM json_each(?1)
                 )
                 SELECT r.idx,o.observed_at,o.price FROM requests r
                   CROSS JOIN economy_observations o INDEXED BY economy_dates
                 WHERE o.league=?2 AND o.category=r.category AND o.source=r.source
                   AND o.observed_at=?3 AND o.item=r.item AND o.currency=r.currency
                   AND o.granularity=r.granularity AND coalesce(o.archive,'')=r.archive
                   AND CAST(o.price AS REAL)>0
                   AND (o.archive IS NULL OR EXISTS (SELECT 1 FROM economy_imports a
                     WHERE a.name=o.archive AND json_extract(a.status_json,'$.status')='completed'))",
                )
                .bind(serde_json::to_string(&values)?)
                .bind(league)
                .bind(&time)
                .fetch_all(&self.pool)
                .await?;
                let mut found = HashSet::new();
                for row in rows {
                    let index = row.get::<i64, _>("idx") as usize;
                    let mut observation = templates[index].clone();
                    observation.observed_at = row.get("observed_at");
                    observation.price = row.get("price");
                    observation.confidence = None;
                    result.push(observation);
                    found.insert(index);
                }
                requests.retain(|(index, _)| !found.contains(index));
                if requests.is_empty() {
                    break;
                }
            }
        }
        Ok(result)
    }

    pub async fn analysis_confidence(
        &self,
        league: &str,
        rows: &mut [FlipRow],
    ) -> Result<(), StoreError> {
        for rows in rows.chunks_mut(500) {
            let identities=rows.iter().filter(|row| row.source!="GGG Exchange").map(|row|serde_json::json!({
            "id":row.id,"category":row.history_category,"source":row.history_source,"item":row.history_item,
            "archive":row.history_archive,"currency":row.pair,"start":row.start_observed_at,"end":row.end_observed_at,
            "start_price":row.native_start,"end_price":row.native_end,
        })).collect::<Vec<_>>();
            let confidence=sqlx::query(
            "SELECT json_extract(n.value,'$.id') AS id,
               (SELECT c.confidence FROM economy_observation_confidence c INDEXED BY economy_confidence_dates WHERE c.item=i.rowid
                 AND c.observed_at=json_extract(n.value,'$.start') AND c.currency=json_extract(n.value,'$.currency')
                 AND c.archive=coalesce(json_extract(n.value,'$.archive'),'')
                 AND (c.price IS NULL OR c.price=json_extract(n.value,'$.start_price'))) AS start_confidence,
               (SELECT c.confidence FROM economy_observation_confidence c INDEXED BY economy_confidence_dates WHERE c.item=i.rowid
                 AND c.observed_at=json_extract(n.value,'$.end') AND c.currency=json_extract(n.value,'$.currency')
                 AND c.archive=coalesce(json_extract(n.value,'$.archive'),'')
                 AND (c.price IS NULL OR c.price=json_extract(n.value,'$.end_price'))) AS end_confidence
             FROM json_each(?) n LEFT JOIN economy_items i ON i.league=?
               AND i.category=json_extract(n.value,'$.category') AND i.source=json_extract(n.value,'$.source')
               AND i.item=json_extract(n.value,'$.item')",
        ).bind(serde_json::to_string(&identities)?).bind(league).fetch_all(&self.pool).await?;
            let positions = rows
                .iter()
                .enumerate()
                .map(|(index, row)| (row.id.clone(), index))
                .collect::<std::collections::HashMap<_, _>>();
            for quality in confidence {
                if let Some(&index) = positions.get(&quality.get::<String, _>("id")) {
                    let row = &mut rows[index];
                    row.start_confidence = quality.get("start_confidence");
                    row.end_confidence = quality.get("end_confidence");
                }
            }
        }
        Ok(())
    }

    pub async fn analysis_cluster_items(&self, league: &str) -> Result<Vec<Line>, StoreError> {
        let items = sqlx::query_scalar::<_, String>(
            "SELECT line_json FROM economy_items
             WHERE league=? AND category='ClusterJewel' AND source='stash'",
        )
        .bind(league)
        .fetch_all(&self.pool)
        .await?;
        items
            .into_iter()
            .map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .collect()
    }

    async fn analysis_endpoint(
        &self,
        league: &str,
        hour: i64,
        with_confidence: bool,
    ) -> Result<Vec<Observation>, StoreError> {
        let timestamp = DateTime::<Utc>::from_timestamp(hour, 0)
            .ok_or_else(|| sqlx::Error::Protocol("Invalid analysis hour".into()))?;
        let end = timestamp + chrono::Duration::hours(1);
        let day = timestamp.format("%Y-%m-%d").to_string();
        let start = timestamp.format("%Y-%m-%dT%H:%M:%S").to_string();
        let end = end.format("%Y-%m-%dT%H:%M:%S").to_string();
        let categories = sqlx::query_as::<_, (String, String)>(
            "SELECT DISTINCT category,source FROM economy_categories
             WHERE league=? AND (archive='' OR archive IN
               (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed'))",
        )
        .bind(league)
        .fetch_all(&self.pool)
        .await?;
        let mut result = Vec::new();
        for (category, source) in categories {
            // Stay on the covering date index; provider quote-volume is not comparable item liquidity.
            let rows = sqlx::query(
                "WITH candidates AS (
                   SELECT league,category,source,item,observed_at,granularity,price,currency,archive
                     FROM economy_observations
                     WHERE league=?1 AND category=?2 AND source=?3 AND observed_at=?4 AND granularity='daily'
                   UNION ALL
                   SELECT league,category,source,item,observed_at,granularity,price,currency,archive
                     FROM economy_observations
                     WHERE league=?1 AND category=?2 AND source=?3 AND observed_at>=?5 AND observed_at<?6
                       AND granularity<>'daily'
                 ), ranked AS (
                   SELECT *,row_number() OVER (
                     PARTITION BY item,currency,granularity,COALESCE(archive,'')
                     ORDER BY observed_at DESC) AS rank
                   FROM candidates WHERE archive IS NULL OR archive IN
                     (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed')
                 )
                 SELECT r.*,CASE WHEN ?7 THEN (SELECT c.confidence FROM economy_observation_confidence c
                   WHERE c.item=i.rowid AND c.observed_at=r.observed_at AND c.currency=r.currency
                   AND (c.price IS NULL OR c.price=r.price)
                   AND c.archive=coalesce(r.archive,'')) END AS confidence,json_object(
                   'id',i.item,
                   'name',json_extract(i.line_json, '$.name'),
                   'variant',json_extract(i.line_json, '$.variant'),
                   'base_type',json_extract(i.line_json, '$.base_type'),
                   'links',json_extract(i.line_json, '$.links'),
                   'gem_level',json_extract(i.line_json, '$.gem_level'),
                   'gem_quality',json_extract(i.line_json, '$.gem_quality'),
                   'corrupted',json_extract(i.line_json, '$.corrupted'),
                   'map_tier',json_extract(i.line_json, '$.map_tier')
                 ) AS line_json FROM ranked r
                 JOIN economy_items i INDEXED BY economy_analysis_metadata USING(league,category,source,item)
                 WHERE rank=1",
            )
            .bind(league)
            .bind(&category)
            .bind(&source)
            .bind(&day)
            .bind(&start)
            .bind(&end)
            .bind(with_confidence)
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                let mut metadata: serde_json::Value = serde_json::from_str(row.get("line_json"))?;
                if let Some(value) = metadata.get_mut("corrupted")
                    && let Some(number) = value.as_i64()
                {
                    *value = serde_json::Value::Bool(number != 0);
                }
                let mut line: Line = serde_json::from_value(metadata)?;
                line.price = None;
                line.divine_price = None;
                line.listings = None;
                line.samples = None;
                line.volume = None;
                result.push(Observation {
                    category: category.clone(),
                    source: source.clone(),
                    line,
                    observed_at: row.get("observed_at"),
                    granularity: row.get("granularity"),
                    price: row.get("price"),
                    currency: row.get("currency"),
                    archive: row.get("archive"),
                    volume: None,
                    confidence: row.get("confidence"),
                });
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use market_core::economy::{Archive, Observation, Overview, Point};

    use super::*;

    fn exchange_page(hour: i64, traded: bool) -> faust_fetch::ExchangePage {
        let market = serde_json::json!({
            "league":"Test","market_id":"a|b","market_pair":["a","b"],
            "volume_traded":{"a":1,"b":2},"lowest_stock":{"a":1,"b":1},
            "highest_stock":{"a":1,"b":1},"lowest_ratio":{"a":1,"b":1},
            "highest_ratio":{"a":1,"b":1}
        });
        serde_json::from_value(serde_json::json!({
            "next_change_id":hour+3600,"markets":if traded {vec![market]} else {vec![]}
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn exact_endpoint_query_preserves_variants_and_never_uses_another_day_or_hour() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("analysis.sqlite3"))
            .await
            .unwrap();
        let empty = store.analysis_period("Test").await.unwrap();
        assert_eq!(empty.bounds_source, "unavailable");
        assert!(empty.start.is_none() && empty.end.is_none());
        assert!(!empty.warnings.is_empty());
        for (at, price) in [
            ("2026-01-01T10:15:00Z", "2"),
            ("2026-01-01T10:45:00Z", "3"),
            ("2026-01-01T11:00:00Z", "100"),
        ] {
            store
                .save_economy(&Overview {
                    league: "Test".into(),
                    category: "Currency".into(),
                    source: "exchange".into(),
                    fetched_at: at.into(),
                    quote_currency: "Chaos Orb".into(),
                    rates: BTreeMap::new(),
                    lines: vec![Line {
                        id: "oil".into(),
                        name: "Oil".into(),
                        price: Some(price.into()),
                        ..Line::default()
                    }],
                })
                .await
                .unwrap();
        }
        let hour = DateTime::parse_from_rfc3339("2026-01-01T10:00:00Z")
            .unwrap()
            .timestamp();
        let period = store.analysis_period("Test").await.unwrap();
        assert_eq!(period.bounds_source, "observed");
        assert_eq!(period.hours, vec![hour, hour + 3600]);
        assert!(period.days.is_empty());
        let rows = store.analysis_observations("Test", hour).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].price, "3");
        assert_eq!(rows[0].observed_at, "2026-01-01T10:45:00Z");
        assert!(
            store
                .analysis_observations("Test", hour - 3600)
                .await
                .unwrap()
                .is_empty()
        );
        let archive = Archive {
            name: "Dump".into(),
            league: Some("Test".into()),
            status: "running".into(),
            rows: 1,
            skipped: 0,
            error: None,
            start: None,
            end: None,
            progress: String::new(),
            license: String::new(),
            url: String::new(),
        };
        store.save_economy_import(&archive, None).await.unwrap();
        store
            .save_observations(
                "Test",
                "stash",
                &[Observation {
                    category: "SkillGem".into(),
                    line: Line {
                        id: "gem21c".into(),
                        name: "Fireball".into(),
                        variant: Some("21c".into()),
                        ..Line::default()
                    },
                    point: Point {
                        date: "2026-01-01".into(),
                        price: "50".into(),
                        currency: "Chaos Orb".into(),
                        granularity: "daily".into(),
                        volume: None,
                        confidence: Some("High".into()),
                    },
                }],
                Some("Dump"),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .analysis_observations("Test", hour)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(store.analysis_period("Test").await.unwrap().days.is_empty());
        store
            .save_economy_import(
                &Archive {
                    status: "completed".into(),
                    start: Some("2026-01-01".into()),
                    end: Some("2026-01-05".into()),
                    ..archive
                },
                None,
            )
            .await
            .unwrap();
        let rows = store.analysis_observations("Test", hour).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .find(|row| row.line.id == "gem21c")
                .unwrap()
                .confidence
                .as_deref(),
            Some("High")
        );
        assert!(
            rows.iter()
                .any(|row| row.line.variant.as_deref() == Some("21c") && row.price == "50")
        );
        assert!(
            store
                .analysis_observations("Test", hour + 86400)
                .await
                .unwrap()
                .is_empty()
        );
        let period = store.analysis_period("Test").await.unwrap();
        assert_eq!(period.bounds_source, "archive");
        assert_eq!(period.days, vec!["2026-01-01"]);
        assert_eq!(period.start, Some(hour - 10 * 3600));
        assert_eq!(period.end, Some(hour - 10 * 3600 + 5 * 86400 - 3600));

        // Simulate an existing pre-manifest database to verify migration backfill.
        sqlx::raw_sql(
            "DROP INDEX economy_analysis_metadata;
             DROP TABLE economy_observation_times;
             DELETE FROM schema_migrations WHERE version=8;",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;
        let reopened = SqliteStore::connect(directory.path().join("analysis.sqlite3"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(reopened.analysis_period("Test").await.unwrap()).unwrap(),
            serde_json::to_value(period).unwrap()
        );
        assert_eq!(
            reopened
                .analysis_observations("Test", hour)
                .await
                .unwrap()
                .len(),
            2
        );
        reopened.clear_incomplete_import("Dump").await.unwrap();
        let period = reopened.analysis_period("Test").await.unwrap();
        assert!(period.days.is_empty());
        assert_eq!(period.hours, vec![hour, hour + 3600]);
    }

    #[tokio::test]
    async fn period_ticks_include_only_completed_past_exchange_hours() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("exchange-period.sqlite3"))
            .await
            .unwrap();
        store
            .record_ggg_failure("Test", 0, "offline")
            .await
            .unwrap();
        let current_hour = Utc::now().timestamp().div_euclid(3600) * 3600;
        for hour in [3600, 7200] {
            store
                .record_ggg_hour("Test", hour, &exchange_page(hour, true))
                .await
                .unwrap();
        }
        store
            .record_ggg_hour("Test", 10800, &exchange_page(10800, false))
            .await
            .unwrap();
        for hour in [current_hour] {
            sqlx::query("INSERT INTO ggg_hours(league,hour,complete) SELECT id,?,1 FROM ggg_leagues WHERE name='Test'")
                .bind(hour).execute(&store.pool).await.unwrap();
        }
        let period = store.analysis_period("Test").await.unwrap();
        assert_eq!(period.hours, vec![3600, 7200]);
        assert_eq!(period.start, Some(3600));
        assert_eq!(period.end, Some(7200));
        assert_eq!(period.bounds_source, "observed");
    }

    #[tokio::test]
    async fn confirmed_empty_exchange_hours_do_not_extend_archive_datetime_bounds() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("active-window.sqlite3"))
            .await
            .unwrap();
        let start = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        let end = start + 3 * 86400 - 3600;
        let first = start + 22 * 3600;
        let last = start + 2 * 86400 + 2 * 3600;
        store
            .save_economy_import(
                &Archive {
                    name: "Dump".into(),
                    league: Some("Test".into()),
                    status: "completed".into(),
                    start: Some("2026-01-01".into()),
                    end: Some("2026-01-03".into()),
                    rows: 0,
                    skipped: 0,
                    error: None,
                    progress: String::new(),
                    license: String::new(),
                    url: String::new(),
                },
                None,
            )
            .await
            .unwrap();
        for hour in [first, last] {
            store
                .record_ggg_hour("Test", hour, &exchange_page(hour, true))
                .await
                .unwrap();
        }
        let partial = store.analysis_period("Test").await.unwrap();
        assert_eq!((partial.start, partial.end), (Some(start), Some(end)));
        for hour in (start..=end).step_by(3600) {
            store
                .record_ggg_hour("Test", hour, &exchange_page(hour, false))
                .await
                .unwrap();
        }
        let complete = store.analysis_period("Test").await.unwrap();
        assert_eq!((complete.start, complete.end), (Some(first), Some(last)));
        assert_eq!(complete.hours, vec![first, last]);
        assert_eq!(complete.bounds_source, "observed");
        assert!(complete.warnings[0].contains("Official league datetimes are unavailable"));
        sqlx::query("DELETE FROM ggg_hours WHERE hour=?")
            .bind(start)
            .execute(&store.pool)
            .await
            .unwrap();
        let gap = store.analysis_period("Test").await.unwrap();
        assert_eq!((gap.start, gap.end), (Some(start), Some(last)));
    }

    #[tokio::test]
    async fn confidence_backfill_matches_prices_and_dates_without_replacing_observations() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("quality.sqlite3"))
            .await
            .unwrap();
        store
            .save_economy_import(
                &Archive {
                    name: "Dump".into(),
                    league: Some("Test".into()),
                    status: "completed".into(),
                    rows: 1,
                    skipped: 0,
                    error: None,
                    start: Some("2026-01-01".into()),
                    end: Some("2026-01-01".into()),
                    progress: String::new(),
                    license: String::new(),
                    url: String::new(),
                },
                None,
            )
            .await
            .unwrap();
        let original = Observation {
            category: "SkillGem".into(),
            line: Line {
                id: "gem".into(),
                name: "Gem".into(),
                ..Line::default()
            },
            point: Point {
                date: "2026-01-01".into(),
                price: "2".into(),
                currency: "Chaos Orb".into(),
                granularity: "daily".into(),
                volume: None,
                confidence: None,
            },
        };
        store
            .save_observations("Test", "stash", &[original.clone()], Some("Dump"))
            .await
            .unwrap();
        let mut input = original.clone();
        input.point.confidence = Some("High".into());
        assert_eq!(
            store
                .backfill_observation_confidence("Test", "stash", &[input.clone()], "Other")
                .await
                .unwrap(),
            1
        );
        input.point.price = "100".into();
        assert_eq!(
            store
                .backfill_observation_confidence("Test", "stash", &[input.clone()], "Dump")
                .await
                .unwrap(),
            1
        );
        assert_eq!(sqlx::query_scalar::<_,i64>(
            "SELECT count(*) FROM economy_observations o JOIN economy_items i USING(league,category,source,item)
             JOIN economy_observation_confidence c ON c.item=i.rowid AND c.observed_at=o.observed_at
               AND c.archive=o.archive AND c.price=o.price"
        ).fetch_one(&store.pool).await.unwrap(),0);
        let hour = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            store.analysis_observations("Test", hour).await.unwrap()[0].confidence,
            None
        );
        assert_eq!(
            store
                .economy_history("Test", "SkillGem", "stash", "gem", None)
                .await
                .unwrap()
                .points[0]
                .confidence,
            None
        );
        input.point.price = "2".into();
        assert_eq!(
            store
                .backfill_observation_confidence("Test", "stash", &[input], "Dump")
                .await
                .unwrap(),
            1
        );
        let stored:(String,String)=sqlx::query_as("SELECT o.price,c.confidence FROM economy_observations o JOIN economy_items i USING(league,category,source,item) JOIN economy_observation_confidence c ON c.item=i.rowid AND c.observed_at=o.observed_at AND c.archive=o.archive AND c.price=o.price").fetch_one(&store.pool).await.unwrap();
        assert_eq!(stored, ("2".into(), "High".into()));
        assert_eq!(
            store.analysis_observations("Test", hour).await.unwrap()[0]
                .confidence
                .as_deref(),
            Some("High")
        );
        assert_eq!(
            store.analysis_prices("Test", hour).await.unwrap()[0].confidence,
            None
        );
        sqlx::query("UPDATE economy_observation_confidence SET price=NULL WHERE archive='Dump'")
            .execute(&store.pool)
            .await
            .unwrap();
        assert_eq!(
            store
                .economy_history("Test", "SkillGem", "stash", "gem", None)
                .await
                .unwrap()
                .points[0]
                .confidence
                .as_deref(),
            Some("High")
        );
        store.clear_incomplete_import("Dump").await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM economy_observation_confidence WHERE archive='Dump'"
            )
            .fetch_one(&store.pool)
            .await
            .unwrap(),
            0
        );
    }
}
