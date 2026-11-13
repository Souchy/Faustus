use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashSet};

use market_core::economy::{Archive, Coverage, History, Line, Observation, Overview, Point};
use sqlx::Row;

use crate::{SqliteStore, StoreError};

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn history_preserves_observed_volume_and_missing_values_without_using_latest_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("volume.sqlite3");
        let store = SqliteStore::connect(&path).await.unwrap();
        for (date, volume) in [
            ("2026-01-01T00:00:00Z", Some("100.5")),
            ("2026-01-01T00:15:00Z", None),
            ("2026-01-01T00:30:00Z", Some("0")),
        ] {
            store
                .save_economy(&Overview {
                    league: "Standard".into(),
                    category: "Currency".into(),
                    source: "exchange".into(),
                    fetched_at: date.into(),
                    quote_currency: "Chaos Orb".into(),
                    rates: BTreeMap::new(),
                    lines: vec![Line {
                        id: "divine".into(),
                        price: Some("200".into()),
                        volume: volume.map(str::to_owned),
                        ..Line::default()
                    }],
                })
                .await
                .unwrap();
        }
        let reopened = SqliteStore::connect(&path).await.unwrap();
        let history = reopened
            .economy_history("Standard", "Currency", "exchange", "divine", None)
            .await
            .unwrap();
        assert_eq!(history.points.len(), 3);
        assert_eq!(history.points[0].volume.as_deref(), Some("100.5"));
        assert_eq!(history.points[1].volume, None);
        assert_eq!(history.points[2].volume.as_deref(), Some("0"));
    }

    #[tokio::test]
    async fn icon_metadata_prefers_one_feed_and_includes_dump_currency_fragments() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("icons.sqlite3"))
            .await
            .unwrap();
        for (category, source, name, icon) in [
            ("Currency", "stash", "Divine Orb", "/stash.png"),
            ("Currency", "exchange", "Divine Orb", "/exchange.png"),
            ("Fragment", "stash", "Fragment", "/fragment.png"),
        ] {
            store
                .save_economy(&Overview {
                    league: "Standard".into(),
                    category: category.into(),
                    source: source.into(),
                    fetched_at: "2026-01-01".into(),
                    quote_currency: "Chaos Orb".into(),
                    rates: BTreeMap::new(),
                    lines: if source == "exchange" {
                        vec![
                            Line {
                                id: name.into(),
                                name: name.into(),
                                icon: Some(icon.into()),
                                ..Line::default()
                            },
                            Line {
                                id: "Foulborn Exalted Orb".into(),
                                name: "Foulborn Exalted Orb".into(),
                                icon: Some("/foulborn.png".into()),
                                ..Line::default()
                            },
                        ]
                    } else {
                        vec![Line {
                            id: name.into(),
                            name: name.into(),
                            icon: Some(icon.into()),
                            ..Line::default()
                        }]
                    },
                })
                .await
                .unwrap();
        }
        let lines = store.economy_icon_metadata("Currency").await.unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().any(|line| line.name == "Foulborn Exalted Orb"));
        assert!(
            lines
                .iter()
                .any(|line| line.icon.as_deref() == Some("/stash.png"))
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.icon.as_deref() == Some("/exchange.png"))
        );
        assert_eq!(
            store.economy_icon_metadata("Fragment").await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn daily_trends_end_at_selected_date_preserve_gaps_and_require_endpoints() {
        let directory = tempfile::tempdir().unwrap();
        let store = SqliteStore::connect(directory.path().join("trend.sqlite3"))
            .await
            .unwrap();
        let archive = Archive {
            name: "Test".into(),
            status: "completed".into(),
            rows: 5,
            skipped: 0,
            error: None,
            start: None,
            end: None,
            league: Some("Test".into()),
            progress: String::new(),
            license: "CC BY-SA".into(),
            url: String::new(),
        };
        store.save_economy_import(&archive, None).await.unwrap();
        let observations =
            [(1, "20"), (2, "25"), (4, "30"), (8, "40"), (9, "1000")].map(|(day, price)| {
                Observation {
                    category: "SkillGem".into(),
                    line: Line {
                        id: "archive:test".into(),
                        name: "Fireball".into(),
                        price: Some(price.into()),
                        ..Line::default()
                    },
                    point: Point {
                        date: format!("2026-01-{day:02}"),
                        price: price.into(),
                        currency: "Chaos Orb".into(),
                        granularity: "daily".into(),
                        volume: None,
                        confidence: None,
                    },
                }
            });
        store
            .save_observations("Test", "stash", &observations, Some("Test"))
            .await
            .unwrap();
        let overview = store
            .historical_economy("Test", "SkillGem", "stash", "2026-01-08")
            .await
            .unwrap();
        assert_eq!(overview.lines[0].price.as_deref(), Some("40"));
        assert_eq!(overview.lines[0].change.as_deref(), Some("100"));
        assert_eq!(overview.lines[0].sparkline.len(), 8);
        assert_eq!(overview.lines[0].sparkline[2], None);
        let overview = store
            .historical_economy("Test", "SkillGem", "stash", "2026-01-04")
            .await
            .unwrap();
        assert!(overview.lines[0].change.is_none());
        assert!(overview.lines[0].sparkline.iter().any(Option::is_some));
        assert!(overview.lines[0].listings.is_none());
        let history = store
            .economy_history("Test", "SkillGem", "stash", "archive:test", None)
            .await
            .unwrap();
        assert!(history.points.iter().all(|point| point.volume.is_none()));
    }
}

async fn save_confidence(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    league: &str,
    source: &str,
    observations: &[Observation],
    archive: Option<&str>,
) -> Result<u64, StoreError> {
    let rows = observations
        .iter()
        .filter(|row| row.point.confidence.is_some())
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Ok(0);
    }
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "WITH incoming(league,category,source,item,observed_at,currency,archive,confidence,price) AS (",
    );
    query.push_values(rows, |mut values, row| {
        values
            .push_bind(league)
            .push_bind(&row.category)
            .push_bind(source)
            .push_bind(&row.line.id)
            .push_bind(&row.point.date)
            .push_bind(&row.point.currency)
            .push_bind(archive.unwrap_or(""))
            .push_bind(row.point.confidence.as_deref().unwrap())
            .push_bind(&row.point.price);
    });
    // Catalog rowids stay stable under item upserts and avoid repeating long variant IDs for every day.
    // Guard confidence by its original price at read time, avoiding millions of random price-table reads.
    query.push(
        ") INSERT INTO economy_observation_confidence(item,observed_at,currency,archive,confidence,price)
        SELECT i.rowid,n.observed_at,n.currency,n.archive,n.confidence,n.price FROM incoming n
        CROSS JOIN economy_items i USING(league,category,source,item)
        WHERE true
        ON CONFLICT DO UPDATE SET confidence=excluded.confidence,price=excluded.price",
    );
    Ok(query
        .build()
        .execute(&mut **transaction)
        .await?
        .rows_affected())
}

impl SqliteStore {
    pub fn try_economy_import_lock(&self, name: &str) -> std::io::Result<Option<std::fs::File>> {
        let path = self
            .pool
            .connect_options()
            .get_filename()
            .with_extension(format!(
                "{}.import.lock",
                crate::content_hash(name.as_bytes())
            ));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    pub async fn stored_economy_categories(&self) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT category FROM economy_categories
             WHERE archive='' OR archive IN
             (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed')",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn save_economy(&self, overview: &Overview) -> Result<(), StoreError> {
        let observations = overview
            .lines
            .iter()
            .filter_map(|line| {
                line.price.as_ref().map(|price| Observation {
                    category: overview.category.clone(),
                    line: line.clone(),
                    point: Point {
                        date: overview.fetched_at.clone(),
                        price: price.clone(),
                        currency: overview.quote_currency.clone(),
                        granularity: "snapshot".into(),
                        volume: line.volume.clone(),
                        confidence: None,
                    },
                })
            })
            .collect::<Vec<_>>();
        self.save_observations(&overview.league, &overview.source, &observations, None)
            .await?;
        sqlx::query("INSERT INTO economy_current VALUES (?, ?, ?, ?) ON CONFLICT(league,category,source) DO UPDATE SET overview_json=excluded.overview_json")
            .bind(&overview.league).bind(&overview.category).bind(&overview.source)
            .bind(serde_json::to_string(overview)?).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn save_observations(
        &self,
        league: &str,
        source: &str,
        observations: &[Observation],
        archive: Option<&str>,
    ) -> Result<(), StoreError> {
        let mut transaction = self.pool.begin().await?;
        let times = observations
            .iter()
            .map(|observation| (&observation.point.date, &observation.point.granularity))
            .collect::<HashSet<_>>();
        for batch in times.into_iter().collect::<Vec<_>>().chunks(1000) {
            let mut query =
                sqlx::QueryBuilder::<sqlx::Sqlite>::new("INSERT INTO economy_observation_times ");
            query.push_values(batch, |mut row, (date, granularity)| {
                row.push_bind(league)
                    .push_bind(date)
                    .push_bind(granularity)
                    .push_bind(archive.unwrap_or(""));
            });
            query
                .push(" ON CONFLICT DO NOTHING")
                .build()
                .execute(&mut *transaction)
                .await?;
        }
        let categories: HashSet<&str> = observations.iter().map(|o| o.category.as_str()).collect();
        if !categories.is_empty() {
            let mut query =
                sqlx::QueryBuilder::<sqlx::Sqlite>::new("INSERT INTO economy_categories ");
            query.push_values(categories, |mut row, category| {
                row.push_bind(league)
                    .push_bind(category)
                    .push_bind(source)
                    .push_bind(archive.unwrap_or(""));
            });
            query
                .push(" ON CONFLICT DO NOTHING")
                .build()
                .execute(&mut *transaction)
                .await?;
        }
        for batch in observations.chunks(1000) {
            let mut seen = HashSet::new();
            let items = batch
                .iter()
                .filter(|o| seen.insert((&o.category, &o.line.id)))
                .map(|o| Ok((&o.category, &o.line.id, serde_json::to_string(&o.line)?)))
                .collect::<Result<Vec<_>, StoreError>>()?;
            if items.is_empty() {
                continue;
            }
            let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("INSERT INTO economy_items ");
            query.push_values(&items, |mut row, (category, id, json)| {
                row.push_bind(league)
                    .push_bind(category)
                    .push_bind(source)
                    .push_bind(id)
                    .push_bind(json);
            });
            query.push(" ON CONFLICT(league,category,source,item) DO UPDATE SET line_json=excluded.line_json")
                .build().execute(&mut *transaction).await?;
            let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "INSERT INTO economy_observations (league,category,source,item,observed_at,granularity,price,currency,archive,volume) ",
            );
            query.push_values(batch, |mut row, observation| {
                row.push_bind(league)
                    .push_bind(&observation.category)
                    .push_bind(source)
                    .push_bind(&observation.line.id)
                    .push_bind(&observation.point.date)
                    .push_bind(&observation.point.granularity)
                    .push_bind(&observation.point.price)
                    .push_bind(&observation.point.currency)
                    .push_bind(archive)
                    .push_bind(&observation.point.volume);
            });
            query
                .push(" ON CONFLICT DO NOTHING")
                .build()
                .execute(&mut *transaction)
                .await?;
            save_confidence(&mut transaction, league, source, batch, archive).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn backfill_observation_confidence(
        &self,
        league: &str,
        source: &str,
        observations: &[Observation],
        archive: &str,
    ) -> Result<u64, StoreError> {
        let mut transaction = self.pool.begin().await?;
        let mut count = 0;
        for batch in observations.chunks(1000) {
            count +=
                save_confidence(&mut transaction, league, source, batch, Some(archive)).await?;
        }
        transaction.commit().await?;
        Ok(count)
    }

    pub async fn current_economy(
        &self,
        league: &str,
        category: &str,
        source: &str,
    ) -> Result<Option<Overview>, StoreError> {
        let json: Option<String> = sqlx::query_scalar(
            "SELECT overview_json FROM economy_current WHERE league=? AND category=? AND source=?",
        )
        .bind(league)
        .bind(category)
        .bind(source)
        .fetch_optional(&self.pool)
        .await?;
        json.map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .transpose()
    }

    pub async fn economy_dates(
        &self,
        league: &str,
        category: &str,
        source: &str,
    ) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT substr(observed_at,1,10) AS day FROM economy_observations o
             WHERE league=? AND category=? AND source=? AND
             (archive IS NULL OR archive IN (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed'))
             ORDER BY day DESC LIMIT 2000")
            .bind(league).bind(category).bind(source).fetch_all(&self.pool).await?)
    }

    pub async fn historical_economy(
        &self,
        league: &str,
        category: &str,
        source: &str,
        date: &str,
    ) -> Result<Overview, StoreError> {
        let rows = sqlx::query(
            "WITH ranked AS (
               SELECT o.*, row_number() OVER (PARTITION BY item,currency ORDER BY observed_at DESC) AS rank
               FROM economy_observations o WHERE league=? AND category=? AND source=?
               AND observed_at>=? AND observed_at<? AND
               (archive IS NULL OR archive IN (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed')))
             SELECT r.price,r.currency,r.observed_at,r.archive,i.line_json FROM ranked r
             JOIN economy_items i USING(league,category,source,item) WHERE r.rank=1")
            .bind(league).bind(category).bind(source).bind(date).bind(format!("{date}~"))
            .fetch_all(&self.pool).await?;
        let mut lines = Vec::new();
        let mut fetched_at = date.to_owned();
        let mut quote_currency = "Chaos Orb".to_owned();
        for row in rows {
            let mut line: Line = serde_json::from_str(row.get::<&str, _>("line_json"))?;
            line.price = Some(row.get("price"));
            // These attributes belong to a live response, not a saved daily estimate.
            line.divine_price = None;
            line.change = None;
            line.sparkline.clear();
            line.volume = None;
            line.pair = None;
            line.pair_rate = None;
            line.listings = None;
            line.samples = None;
            quote_currency = row.get("currency");
            let observed: String = row.get("observed_at");
            if observed > fetched_at {
                fetched_at = observed;
            }
            lines.push(line);
        }
        let end = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
        let start = end
            .checked_sub_signed(chrono::Duration::days(7))
            .unwrap_or(end);
        let trends = sqlx::query(
            "SELECT item,observed_at,price,currency FROM economy_observations
             WHERE league=? AND category=? AND source=? AND granularity='daily'
             AND observed_at>=? AND observed_at<=?
             AND archive IN (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed')
             ORDER BY observed_at")
            .bind(league).bind(category).bind(source).bind(start.to_string()).bind(date)
            .fetch_all(&self.pool).await?;
        let mut daily = BTreeMap::<String, BTreeMap<String, (String, String)>>::new();
        for row in trends {
            daily.entry(row.get("item")).or_default().insert(
                row.get("observed_at"),
                (row.get("price"), row.get("currency")),
            );
        }
        for line in &mut lines {
            let Some(points) = daily.get(&line.id) else {
                continue;
            };
            let Some((_, (base, currency))) = points.first_key_value() else {
                continue;
            };
            let base = base
                .parse::<Decimal>()
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
            if base <= Decimal::ZERO {
                continue;
            }
            line.sparkline = (0..=7)
                .map(|offset| {
                    let day = start + chrono::Duration::days(offset);
                    let (price, unit) = points.get(&day.to_string())?;
                    if unit != currency {
                        return None;
                    }
                    price
                        .parse::<Decimal>()
                        .ok()?
                        .checked_div(base)?
                        .checked_sub(Decimal::ONE)?
                        .checked_mul(Decimal::from(100))
                        .map(|value| value.normalize().to_string())
                })
                .collect();
            line.change = if points.contains_key(&start.to_string()) {
                line.sparkline.last().cloned().flatten()
            } else {
                None
            };
            line.metadata.insert(
                "trend_period".into(),
                "7-day daily history ending on selected date".into(),
            );
        }
        let mut rates = BTreeMap::new();
        rates.insert(quote_currency.clone(), "1".into());
        // Historical conversion is only possible with an observed rate on this same day.
        let currencies = sqlx::query(
            "SELECT i.line_json,o.price FROM economy_observations o JOIN economy_items i USING(league,category,source,item)
             WHERE o.league=? AND o.category='Currency' AND o.source=? AND o.currency=?
             AND o.observed_at>=? AND o.observed_at<? AND
             (o.archive IS NULL OR o.archive IN (SELECT name FROM economy_imports WHERE json_extract(status_json,'$.status')='completed'))
             ORDER BY o.observed_at")
            .bind(league).bind(source).bind(&quote_currency).bind(date).bind(format!("{date}~"))
            .fetch_all(&self.pool).await?;
        for row in currencies {
            let line: Line = serde_json::from_str(row.get::<&str, _>("line_json"))?;
            if matches!(line.name.as_str(), "Chaos Orb" | "Divine Orb") {
                rates.insert(line.name, row.get("price"));
            }
        }
        Ok(Overview {
            league: league.into(),
            category: category.into(),
            source: source.into(),
            fetched_at,
            quote_currency,
            rates,
            lines,
        })
    }

    pub async fn economy_icon_metadata(&self, category: &str) -> Result<Vec<Line>, StoreError> {
        let rows: Vec<String> = sqlx::query_scalar(
            "WITH preferred AS (
               SELECT overview_json,source,row_number() OVER (PARTITION BY category,source ORDER BY
                 CASE WHEN league='Standard' THEN 0 ELSE 1 END,league) AS rank
               FROM economy_current WHERE category=? OR
                 (?='Currency' AND (category='Fragment' OR source='exchange')))
             SELECT overview_json FROM preferred WHERE rank=1
             ORDER BY CASE WHEN source='stash' THEN 0 ELSE 1 END",
        )
        .bind(category)
        .bind(category)
        .fetch_all(&self.pool)
        .await?;
        let mut lines = Vec::new();
        let mut preferred_names = HashSet::new();
        for json in rows {
            let overview: Overview = serde_json::from_str(&json)?;
            let candidates: Vec<Line> = overview
                .lines
                .into_iter()
                .filter(|line| line.icon.is_some() && !preferred_names.contains(&line.name))
                .collect();
            preferred_names.extend(candidates.iter().map(|line| line.name.clone()));
            lines.extend(candidates);
        }
        Ok(lines)
    }

    pub async fn economy_history(
        &self,
        league: &str,
        category: &str,
        source: &str,
        item: &str,
        days: Option<i64>,
    ) -> Result<History, StoreError> {
        let latest: Option<String> = sqlx::query_scalar(
            "SELECT max(observed_at) FROM economy_observations o WHERE league=? AND category=? AND source=? AND item=?
             AND (archive IS NULL OR EXISTS (SELECT 1 FROM economy_imports a WHERE a.name=o.archive AND json_extract(a.status_json,'$.status')='completed'))")
            .bind(league).bind(category).bind(source).bind(item).fetch_one(&self.pool).await?;
        let since = days
            .and_then(|days| {
                let date = latest.as_deref()?.get(..10)?;
                chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .ok()?
                    .checked_sub_signed(chrono::Duration::days(days))
                    .map(|date| date.to_string())
            })
            .unwrap_or_default();
        let rows = sqlx::query(
            "SELECT r.*,(SELECT c.confidence FROM economy_observation_confidence c
               WHERE c.item=(SELECT rowid FROM economy_items i WHERE i.league=r.league
                   AND i.category=r.category AND i.source=r.source AND i.item=r.item)
                 AND c.observed_at=r.observed_at AND c.currency=r.currency
                 AND (c.price IS NULL OR c.price=r.price)
                 AND c.archive=coalesce(r.archive,'')) AS confidence FROM (
               SELECT league,category,source,item,observed_at,price,currency,granularity,volume,archive FROM economy_observations o
               WHERE league=? AND category=? AND source=? AND item=? AND observed_at>=?
               AND (archive IS NULL OR EXISTS (SELECT 1 FROM economy_imports a WHERE a.name=o.archive AND json_extract(a.status_json,'$.status')='completed'))
               ORDER BY observed_at DESC LIMIT 20000) r ORDER BY observed_at")
            .bind(league).bind(category).bind(source).bind(item).bind(since).fetch_all(&self.pool).await?;
        let points: Vec<Point> = rows
            .into_iter()
            .map(|row| Point {
                date: row.get("observed_at"),
                price: row.get("price"),
                currency: row.get("currency"),
                granularity: row.get("granularity"),
                volume: row.get("volume"),
                confidence: row.get("confidence"),
            })
            .collect();
        Ok(History {
            coverage: Coverage {
                start: points.first().map(|p| p.date.clone()),
                end: points.last().map(|p| p.date.clone()),
            },
            points,
        })
    }

    pub async fn imported_leagues(&self) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT json_extract(status_json,'$.league') FROM economy_imports
             WHERE json_extract(status_json,'$.status')='completed' ORDER BY name DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn economy_imports(&self) -> Result<Vec<Archive>, StoreError> {
        let json: Vec<String> =
            sqlx::query_scalar("SELECT status_json FROM economy_imports ORDER BY name")
                .fetch_all(&self.pool)
                .await?;
        json.into_iter()
            .map(|json| serde_json::from_str(&json).map_err(StoreError::from))
            .collect()
    }

    pub async fn save_economy_import(
        &self,
        archive: &Archive,
        checksum: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query("INSERT INTO economy_imports VALUES (?, ?, ?) ON CONFLICT(name) DO UPDATE SET status_json=excluded.status_json,checksum=coalesce(excluded.checksum,economy_imports.checksum)")
            .bind(&archive.name).bind(serde_json::to_string(archive)?).bind(checksum)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn economy_import_checksum(&self, name: &str) -> Result<Option<String>, StoreError> {
        Ok(
            sqlx::query_scalar("SELECT checksum FROM economy_imports WHERE name=?")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?
                .flatten(),
        )
    }

    pub async fn clear_incomplete_import(&self, name: &str) -> Result<(), StoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM economy_observations WHERE archive=?")
            .bind(name)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM economy_categories WHERE archive=?")
            .bind(name)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM economy_observation_times WHERE archive=?")
            .bind(name)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM economy_observation_confidence WHERE archive=?")
            .bind(name)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}
