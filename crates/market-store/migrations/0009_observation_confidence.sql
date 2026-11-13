CREATE TABLE economy_observation_confidence (
    item INTEGER NOT NULL,
    observed_at TEXT NOT NULL,
    currency TEXT NOT NULL,
    archive TEXT NOT NULL,
    confidence TEXT NOT NULL CHECK(confidence IN ('High', 'Medium', 'Low')),
    PRIMARY KEY (item, observed_at, currency, archive)
) WITHOUT ROWID;
