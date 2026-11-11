use std::collections::{BTreeMap, HashSet};

use market_core::economy::{Archive, Coverage, History, Line, Observation, Overview, Point};
use sqlx::Row;

use crate::{SqliteStore, StoreError};

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
            let mut query =
                sqlx::QueryBuilder::<sqlx::Sqlite>::new("INSERT INTO economy_observations ");
            query.push_values(batch, |mut row, observation| {
                row.push_bind(league)
                    .push_bind(&observation.category)
                    .push_bind(source)
                    .push_bind(&observation.line.id)
                    .push_bind(&observation.point.date)
                    .push_bind(&observation.point.granularity)
                    .push_bind(&observation.point.price)
                    .push_bind(&observation.point.currency)
                    .push_bind(archive);
            });
            query
                .push(" ON CONFLICT DO NOTHING")
                .build()
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
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
            "SELECT observed_at,price,currency,granularity FROM (
               SELECT observed_at,price,currency,granularity FROM economy_observations o
               WHERE league=? AND category=? AND source=? AND item=? AND observed_at>=?
               AND (archive IS NULL OR EXISTS (SELECT 1 FROM economy_imports a WHERE a.name=o.archive AND json_extract(a.status_json,'$.status')='completed'))
               ORDER BY observed_at DESC LIMIT 20000) ORDER BY observed_at")
            .bind(league).bind(category).bind(source).bind(item).bind(since).fetch_all(&self.pool).await?;
        let points: Vec<Point> = rows
            .into_iter()
            .map(|row| Point {
                date: row.get("observed_at"),
                price: row.get("price"),
                currency: row.get("currency"),
                granularity: row.get("granularity"),
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
        transaction.commit().await?;
        Ok(())
    }
}
