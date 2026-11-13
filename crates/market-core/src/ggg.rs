use serde::{Deserialize, Serialize};

pub const HOUR: i64 = 3600;
pub const MAX_HOURS: i64 = 180 * 24;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub category: String,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Market {
    pub id: String,
    pub a: Item,
    pub b: Item,
    pub volume_a: String,
    pub volume_b: String,
    pub low_stock_a: String,
    pub low_stock_b: String,
    pub high_stock_a: String,
    pub high_stock_b: String,
    pub low_ratio_a: String,
    pub low_ratio_b: String,
    pub high_ratio_a: String,
    pub high_ratio_b: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HourFailure {
    pub hour: i64,
    pub error: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Coverage {
    pub league: String,
    pub start: i64,
    pub end: i64,
    pub expected_hours: u64,
    pub completed_hours: Vec<i64>,
    pub failed_hours: Vec<HourFailure>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Overview {
    pub league: String,
    pub hour: i64,
    pub complete: bool,
    pub markets: Vec<Market>,
    pub platform: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HistoryPoint {
    pub hour: i64,
    pub market: Market,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct History {
    pub league: String,
    pub market: String,
    pub points: Vec<HistoryPoint>,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ImportJob {
    pub league: String,
    pub start: i64,
    pub end: i64,
    pub status: String,
    pub completed_hours: u64,
    pub total_hours: u64,
    pub error: Option<String>,
    pub warning: Option<String>,
    pub platform: String,
}

pub fn validate_league(league: &str) -> Result<(), String> {
    if league.trim().is_empty()
        || league.chars().count() > 100
        || league.chars().any(char::is_control)
    {
        return Err("League must contain 1–100 non-control characters.".into());
    }
    Ok(())
}

pub fn validate_range(league: &str, start: i64, end: i64, now: i64) -> Result<(), String> {
    validate_league(league)?;
    if start < 0 || end < start || start % HOUR != 0 || end % HOUR != 0 {
        return Err("Range must be nonnegative, inclusive, ordered hourly Unix seconds.".into());
    }
    if end >= now.div_euclid(HOUR) * HOUR {
        return Err("Only completed past hours are available.".into());
    }
    if (end - start) / HOUR + 1 > MAX_HOURS {
        return Err("Range cannot exceed 180 days (4320 hours).".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_inclusive_completed_hour_ranges() {
        assert!(validate_range("Standard", 0, 3600, 7200).is_ok());
        for (start, end) in [(-3600, 0), (1, 3600), (3600, 0), (0, 7200)] {
            assert!(validate_range("Standard", start, end, 7200).is_err());
        }
        assert!(validate_range("x", 0, (MAX_HOURS - 1) * HOUR, i64::MAX).is_ok());
        assert!(validate_range("x", 0, MAX_HOURS * HOUR, i64::MAX).is_err());
        for league in ["", "  ", "x\n", &"x".repeat(101)] {
            assert!(validate_league(league).is_err());
        }
    }
}
