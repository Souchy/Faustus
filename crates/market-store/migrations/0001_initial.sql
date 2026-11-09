CREATE TABLE exchange_pages (
    id INTEGER PRIMARY KEY,
    requested_cursor_key TEXT NOT NULL,
    next_change_id TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    raw_json BLOB NOT NULL,
    UNIQUE (requested_cursor_key, content_hash)
);

CREATE TABLE exchange_markets (
    id INTEGER PRIMARY KEY,
    page_id INTEGER NOT NULL REFERENCES exchange_pages(id) ON DELETE CASCADE,
    league TEXT NOT NULL,
    market_id TEXT NOT NULL,
    market_pair_json TEXT NOT NULL,
    volume_traded_json TEXT NOT NULL,
    lowest_stock_json TEXT NOT NULL,
    highest_stock_json TEXT NOT NULL,
    lowest_ratio_json TEXT NOT NULL,
    highest_ratio_json TEXT NOT NULL,
    UNIQUE (page_id, league, market_id)
);

CREATE INDEX exchange_markets_league_idx ON exchange_markets(league);

CREATE TABLE ninja_snapshots (
    id INTEGER PRIMARY KEY,
    league_id TEXT NOT NULL,
    category TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    etag TEXT,
    cache_control TEXT,
    raw_json BLOB NOT NULL,
    UNIQUE (league_id, category, content_hash)
);

CREATE INDEX ninja_snapshots_lookup_idx
    ON ninja_snapshots(league_id, category, fetched_at DESC);

CREATE TABLE collector_state (
    provider_key TEXT PRIMARY KEY,
    cursor TEXT,
    last_checked_at TEXT,
    last_success_at TEXT,
    last_error TEXT,
    etag TEXT,
    cache_control TEXT
);
