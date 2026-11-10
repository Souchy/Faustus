CREATE TABLE gem_analysis_snapshots (
    id INTEGER PRIMARY KEY,
    league_id TEXT NOT NULL,
    captured_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    data_json TEXT NOT NULL,
    UNIQUE (league_id, content_hash)
);

CREATE INDEX gem_analysis_snapshots_by_league
    ON gem_analysis_snapshots (league_id, captured_at DESC, id DESC);
