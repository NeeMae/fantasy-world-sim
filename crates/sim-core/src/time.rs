use std::fmt;

use serde::{Deserialize, Serialize};

pub const MONTHS_PER_YEAR: u64 = 12;

/// A calendar date. One simulation tick is one month; tick 0 is month 1 of year 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Date {
    pub year: u64,
    /// 1-based month.
    pub month: u8,
}

impl Date {
    pub fn from_tick(tick: u64) -> Self {
        Date { year: tick / MONTHS_PER_YEAR + 1, month: (tick % MONTHS_PER_YEAR) as u8 + 1 }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Year {}, month {}", self.year, self.month)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_months() {
        assert_eq!(Date::from_tick(0), Date { year: 1, month: 1 });
        assert_eq!(Date::from_tick(11), Date { year: 1, month: 12 });
        assert_eq!(Date::from_tick(12), Date { year: 2, month: 1 });
    }
}
