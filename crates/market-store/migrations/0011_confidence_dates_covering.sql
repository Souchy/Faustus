CREATE INDEX economy_confidence_dates ON economy_observation_confidence (
    archive, observed_at, item, currency, confidence, price
);
