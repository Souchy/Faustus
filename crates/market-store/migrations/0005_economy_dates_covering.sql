DROP INDEX economy_dates;
CREATE INDEX economy_dates ON economy_observations (
    league, category, source, observed_at, item, granularity, price, currency, archive
);
