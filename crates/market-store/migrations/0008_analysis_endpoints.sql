CREATE INDEX economy_analysis_metadata ON economy_items (
    league, category, source, item,
    json_extract(line_json, '$.name'),
    json_extract(line_json, '$.variant'),
    json_extract(line_json, '$.base_type'),
    json_extract(line_json, '$.links'),
    json_extract(line_json, '$.gem_level'),
    json_extract(line_json, '$.gem_quality'),
    json_extract(line_json, '$.corrupted'),
    json_extract(line_json, '$.map_tier')
);

CREATE TABLE economy_observation_times (
    league TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    granularity TEXT NOT NULL,
    archive TEXT NOT NULL,
    PRIMARY KEY (league, observed_at, granularity, archive)
) WITHOUT ROWID;

-- Seek between distinct times instead of scanning every item price in old dumps.
WITH RECURSIVE times(league, category, source, archive, observed_at) AS (
    SELECT c.league, c.category, c.source, c.archive, (
        SELECT min(o.observed_at) FROM economy_observations o INDEXED BY economy_dates
        WHERE o.league=c.league AND o.category=c.category AND o.source=c.source
          AND coalesce(o.archive, '')=c.archive
    )
    FROM economy_categories c
    UNION ALL
    SELECT t.league, t.category, t.source, t.archive, (
        SELECT min(o.observed_at) FROM economy_observations o INDEXED BY economy_dates
        WHERE o.league=t.league AND o.category=t.category AND o.source=t.source
          AND coalesce(o.archive, '')=t.archive AND o.observed_at>t.observed_at
    )
    FROM times t WHERE t.observed_at IS NOT NULL
)
INSERT OR IGNORE INTO economy_observation_times
SELECT league, observed_at,
       CASE WHEN length(observed_at)=10 THEN 'daily' ELSE 'snapshot' END, archive
FROM times WHERE observed_at IS NOT NULL;
