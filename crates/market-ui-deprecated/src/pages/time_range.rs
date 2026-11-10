#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimeRangeSelection {
    start_days_ago: u8,
    end_days_ago: u8,
}

impl TimeRangeSelection {
    pub(crate) const MAX_DAYS_AGO: u8 = 6;

    pub(crate) fn new(start_days_ago: u8, end_days_ago: u8) -> Self {
        let end_days_ago = end_days_ago.min(Self::MAX_DAYS_AGO);
        Self {
            start_days_ago: start_days_ago.clamp(end_days_ago, Self::MAX_DAYS_AGO),
            end_days_ago,
        }
    }

    pub(crate) fn start_days_ago(self) -> u8 {
        self.start_days_ago
    }

    pub(crate) fn end_days_ago(self) -> u8 {
        self.end_days_ago
    }

    pub(crate) fn with_start_days_ago(self, days_ago: u8) -> Self {
        Self::new(days_ago, self.end_days_ago)
    }

    pub(crate) fn point_index(self, point_count: usize, days_ago: u8) -> Option<usize> {
        point_count
            .checked_sub(1)?
            .checked_sub(usize::from(days_ago))
    }
}

impl Default for TimeRangeSelection {
    fn default() -> Self {
        Self::new(Self::MAX_DAYS_AGO, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_keeps_start_older_than_end_and_clamps_to_sparkline() {
        let range = TimeRangeSelection::new(3, 5);

        assert_eq!(range.start_days_ago(), 5);
        assert_eq!(range.end_days_ago(), 5);
        assert_eq!(TimeRangeSelection::new(9, 0).start_days_ago(), 6);
    }

    #[test]
    fn range_maps_day_offsets_to_sparkline_indices() {
        let range = TimeRangeSelection::default();

        assert_eq!(range.point_index(7, range.start_days_ago()), Some(0));
        assert_eq!(range.point_index(7, range.end_days_ago()), Some(6));
        assert_eq!(range.point_index(3, range.start_days_ago()), None);
    }
}
