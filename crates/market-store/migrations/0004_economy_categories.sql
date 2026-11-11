CREATE TABLE economy_categories (
    league TEXT NOT NULL, category TEXT NOT NULL, source TEXT NOT NULL,
    archive TEXT NOT NULL,
    PRIMARY KEY (league, category, source, archive)
);
INSERT OR IGNORE INTO economy_categories
SELECT DISTINCT league, category, source, coalesce(archive, '')
-- Avoid millions of random table lookups through the date index.
FROM economy_observations NOT INDEXED;
