CREATE TABLE economy_items (
    league TEXT NOT NULL, category TEXT NOT NULL, source TEXT NOT NULL,
    item TEXT NOT NULL, line_json TEXT NOT NULL,
    PRIMARY KEY (league, category, source, item)
);
CREATE TABLE economy_observations (
    league TEXT NOT NULL, category TEXT NOT NULL, source TEXT NOT NULL,
    item TEXT NOT NULL, observed_at TEXT NOT NULL, granularity TEXT NOT NULL,
    price TEXT NOT NULL, currency TEXT NOT NULL, archive TEXT,
    PRIMARY KEY (league, category, source, item, observed_at, currency)
);
CREATE INDEX economy_dates ON economy_observations(league, category, source, observed_at);
CREATE TABLE economy_current (
    league TEXT NOT NULL, category TEXT NOT NULL, source TEXT NOT NULL,
    overview_json TEXT NOT NULL, PRIMARY KEY (league, category, source)
);
CREATE TABLE economy_imports (
    name TEXT PRIMARY KEY, status_json TEXT NOT NULL, checksum TEXT
);
