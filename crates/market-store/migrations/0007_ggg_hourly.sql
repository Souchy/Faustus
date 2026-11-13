CREATE TABLE ggg_items (
    id INTEGER PRIMARY KEY,
    metadata TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    category TEXT NOT NULL,
    icon TEXT
);
CREATE TABLE ggg_leagues (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE ggg_pairs (
    id INTEGER PRIMARY KEY,
    market TEXT NOT NULL UNIQUE,
    a INTEGER NOT NULL REFERENCES ggg_items(id),
    b INTEGER NOT NULL REFERENCES ggg_items(id)
);
CREATE TABLE ggg_hours (
    league INTEGER NOT NULL REFERENCES ggg_leagues(id),
    hour INTEGER NOT NULL CHECK(hour >= 0 AND hour % 3600 = 0),
    complete INTEGER NOT NULL CHECK(complete IN (0, 1)),
    error TEXT,
    PRIMARY KEY (league, hour)
) WITHOUT ROWID;
CREATE TABLE ggg_observations (
    league INTEGER NOT NULL,
    hour INTEGER NOT NULL,
    pair INTEGER NOT NULL REFERENCES ggg_pairs(id),
    volume_a INTEGER NOT NULL CHECK(volume_a >= 0),
    volume_b INTEGER NOT NULL CHECK(volume_b >= 0),
    low_stock_a INTEGER NOT NULL CHECK(low_stock_a >= 0),
    low_stock_b INTEGER NOT NULL CHECK(low_stock_b >= 0),
    high_stock_a INTEGER NOT NULL CHECK(high_stock_a >= 0),
    high_stock_b INTEGER NOT NULL CHECK(high_stock_b >= 0),
    low_ratio_a INTEGER NOT NULL CHECK(low_ratio_a >= 0),
    low_ratio_b INTEGER NOT NULL CHECK(low_ratio_b >= 0),
    high_ratio_a INTEGER NOT NULL CHECK(high_ratio_a >= 0),
    high_ratio_b INTEGER NOT NULL CHECK(high_ratio_b >= 0),
    PRIMARY KEY (league, hour, pair),
    FOREIGN KEY (league, hour) REFERENCES ggg_hours(league, hour)
) WITHOUT ROWID;
CREATE INDEX ggg_history ON ggg_observations(league, pair, hour);
CREATE TABLE ggg_catalog_state (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    fetched_at INTEGER NOT NULL
);
