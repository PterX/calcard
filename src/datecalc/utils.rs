use super::duration_from_midnight;
use super::rrule_iter::WasLimited;
use super::rruleset::RRuleResult;
use super::timezone::Tz;
use chrono::{NaiveDate, NaiveTime, Utc};
use std::ops;
use std::ops::{
    Bound::{Excluded, Unbounded},
    RangeBounds,
};

const DAY_SECS: i64 = 24 * 60 * 60;

/// Helper function to collect dates given some filters.
///
/// In the case where the iterator ended with errors, the error will be included,
/// otherwise the second value of the return tuple will be `None`.
pub(super) fn collect_with_error<T>(
    mut iterator: T,
    start: &Option<chrono::DateTime<Tz>>,
    end: &Option<chrono::DateTime<Tz>>,
    inclusive: bool,
    limit: Option<u16>,
) -> RRuleResult
where
    T: Iterator<Item = chrono::DateTime<Tz>> + WasLimited,
{
    let mut list = vec![];
    let mut was_limited = false;
    // This loop should always end because `.next()` has build in limits
    // Once a limit is tripped it will break in the `None` case.
    while limit.is_none() || matches!(limit, Some(limit) if usize::from(limit) > list.len()) {
        if let Some(value) = iterator.next() {
            if is_in_range(&value, start, end, inclusive) {
                list.push(value);
            }
            if has_reached_the_end(&value, end, inclusive) {
                // Date is after end date, so can stop iterating
                break;
            }
        } else {
            was_limited = iterator.was_limited();
            break;
        }
    }

    was_limited = was_limited || matches!(limit, Some(limit) if usize::from(limit) == list.len());

    RRuleResult {
        dates: list,
        limited: was_limited,
    }
}

/// Checks if `date` is after `end`.
fn has_reached_the_end(
    date: &chrono::DateTime<Tz>,
    end: &Option<chrono::DateTime<Tz>>,
    inclusive: bool,
) -> bool {
    if inclusive {
        match end {
            Some(end) => !(..=end).contains(&date),
            None => false,
        }
    } else {
        match end {
            Some(end) => !(Unbounded, Excluded(end)).contains(date),
            None => false,
        }
    }
}

/// Helper function to determine if a date is within a given range.
pub(super) fn is_in_range(
    date: &chrono::DateTime<Tz>,
    start: &Option<chrono::DateTime<Tz>>,
    end: &Option<chrono::DateTime<Tz>>,
    inclusive: bool,
) -> bool {
    // Should it include or not include the start and/or end date?
    if inclusive {
        match (start, end) {
            (Some(start), Some(end)) => (start..=end).contains(&date),
            (Some(start), None) => (start..).contains(&date),
            (None, Some(end)) => (..=end).contains(&date),
            (None, None) => true,
        }
    } else {
        match (start, end) {
            (Some(start), Some(end)) => (Excluded(start), Excluded(end)).contains(date),
            (Some(start), None) => (Excluded(start), Unbounded).contains(date),
            (None, Some(end)) => (Unbounded, Excluded(end)).contains(date),
            (None, None) => true,
        }
    }
}

#[inline(always)]
pub(crate) fn date_from_ordinal(ordinal: i64) -> NaiveDate {
    chrono::DateTime::<Utc>::from_timestamp(ordinal * DAY_SECS, 0)
        .unwrap_or_default()
        .date_naive()
}

#[inline(always)]
pub(crate) fn days_since_unix_epoch(date: &chrono::DateTime<Utc>) -> i64 {
    date.timestamp() / DAY_SECS
}

#[inline(always)]
pub(crate) fn is_leap_year(year: i32) -> bool {
    year.trailing_zeros() >= 2 && (year % 25 != 0 || year.trailing_zeros() >= 4)
}

#[inline(always)]
pub(crate) fn get_year_len(year: i32) -> u16 {
    if is_leap_year(year) {
        366
    } else {
        365
    }
}

pub(crate) trait DifferentSigns {
    fn different_sign(a: Self, b: Self) -> bool;
}

macro_rules! impl_different_signs {
    ($($ty:ty),*) => {
        $(
        impl DifferentSigns for $ty {
            fn different_sign(a: Self, b: Self) -> bool {
                a > 0 && b < 0 || a < 0 && b > 0
            }
        })*
    };
    (@unsigned, $($ty:ty),*) => {
        $(
        impl DifferentSigns for $ty {
            fn different_sign(_: Self, _: Self) -> bool {
                false
            }
        })*
    };
}
impl_different_signs!(isize, i32, i16);
impl_different_signs!(@unsigned, usize, u32, u16, u8);

pub(crate) fn pymod<T>(a: T, b: T) -> T
where
    T: DifferentSigns + Copy + ops::Rem<Output = T> + ops::Add<Output = T>,
{
    let r = a % b;
    // If r and b differ in sign, add b to wrap the result to the correct sign.
    if T::different_sign(r, b) {
        r + b
    } else {
        r
    }
}

pub(crate) fn add_time_to_date(
    tz: Tz,
    date: NaiveDate,
    time: NaiveTime,
) -> Option<chrono::DateTime<Tz>> {
    if let Some(dt) = date.and_time(time).and_local_timezone(tz).single() {
        return Some(dt);
    }
    // If the day is a daylight saving time, the above code might not work, and we
    // can try to get a valid datetime by adding the `time` as a duration instead.
    let dt = date.and_hms_opt(0, 0, 0)?.and_local_timezone(tz).single()?;
    let day_duration = duration_from_midnight(time);
    dt.checked_add_signed(day_duration)
}

#[cfg(test)]
mod test {
    use chrono::{Duration, TimeZone};
    const UTC: Tz = Tz::Tz(chrono_tz::Tz::UTC);
    use super::*;

    #[test]
    fn naive_date_from_ordinal() {
        let tests = [
            (-1, NaiveDate::from_ymd_opt(1969, 12, 31).unwrap()),
            (0, NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
            (1, NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()),
            (10, NaiveDate::from_ymd_opt(1970, 1, 11).unwrap()),
            (365, NaiveDate::from_ymd_opt(1971, 1, 1).unwrap()),
            (19877, NaiveDate::from_ymd_opt(2024, 6, 3).unwrap()),
        ];

        for (days, expected) in tests {
            assert_eq!(date_from_ordinal(days), expected, "seconds: {}", days);
        }
    }

    #[test]
    fn python_mod() {
        assert_eq!(pymod(2, -3), -1);
        assert_eq!(pymod(-2, 3), 1);
        assert_eq!(pymod(-2, -3), -2);
        assert_eq!(pymod(-3, -3), 0);
        assert_eq!(pymod(0, 3), 0);
        assert_eq!(pymod(1, 3), 1);
        assert_eq!(pymod(2, 3), 2);
        assert_eq!(pymod(3, 3), 0);
        assert_eq!(pymod(4, 3), 1);
        assert_eq!(pymod(6, 3), 0);
        assert_eq!(pymod(-6, 3), 0);
        assert_eq!(pymod(-6, -3), 0);
        assert_eq!(pymod(6, -3), 0);
    }

    #[test]
    fn leap_year() {
        let tests = [
            (2015, false),
            (2016, true),
            (2017, false),
            (2018, false),
            (2019, false),
            (2020, true),
            (2021, false),
        ];

        for (year, expected_output) in tests {
            let res = is_leap_year(year);
            assert_eq!(res, expected_output);
        }
    }

    #[test]
    fn year_length() {
        let tests = [(2015, 365), (2016, 366)];

        for (year, expected_output) in tests {
            let res = get_year_len(year);
            assert_eq!(res, expected_output);
        }
    }

    #[test]
    fn adds_time_to_date() {
        let tests = [
            (
                Tz::Tz(chrono_tz::Tz::UTC),
                NaiveDate::from_ymd_opt(2017, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(1, 15, 30).unwrap(),
                Some(
                    Tz::Tz(chrono_tz::Tz::UTC)
                        .with_ymd_and_hms(2017, 1, 1, 1, 15, 30)
                        .unwrap(),
                ),
            ),
            (
                Tz::Tz(chrono_tz::Tz::America__Vancouver),
                NaiveDate::from_ymd_opt(2021, 3, 14).unwrap(),
                NaiveTime::from_hms_opt(2, 22, 10).unwrap(),
                Some(
                    Tz::Tz(chrono_tz::Tz::America__Vancouver)
                        .with_ymd_and_hms(2021, 3, 14, 0, 0, 0)
                        .unwrap()
                        + Duration::hours(2)
                        + Duration::minutes(22)
                        + Duration::seconds(10),
                ),
            ),
            (
                Tz::Tz(chrono_tz::Tz::America__New_York),
                NaiveDate::from_ymd_opt(1997, 10, 26).unwrap(),
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                Some(
                    Tz::Tz(chrono_tz::Tz::America__New_York)
                        .with_ymd_and_hms(1997, 10, 26, 9, 0, 0)
                        .unwrap(),
                ),
            ),
        ];

        for (tz, date, time, expected_output) in tests {
            let res = add_time_to_date(tz, date, time);
            assert_eq!(res, expected_output);
        }
    }

    #[test]
    fn in_range_exclusive_start_to_end() {
        let inclusive = false;
        let start = UTC.with_ymd_and_hms(2021, 10, 1, 8, 0, 0).unwrap();
        let end = UTC.with_ymd_and_hms(2021, 10, 1, 10, 0, 0).unwrap();

        // In middle
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &Some(start),
            &Some(end),
            inclusive,
        ));
        // To small
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 7, 0, 0).unwrap(),
            &Some(start),
            &Some(end),
            inclusive,
        ));
        // To big
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 11, 0, 0).unwrap(),
            &Some(start),
            &Some(end),
            inclusive,
        ));
        // Equal to end
        assert!(!is_in_range(&end, &Some(start), &Some(end), inclusive));
        // Equal to start
        assert!(!is_in_range(&start, &Some(start), &Some(end), inclusive));
    }

    #[test]
    fn in_range_exclusive_start() {
        let inclusive = false;
        let start = UTC.with_ymd_and_hms(2021, 10, 1, 8, 0, 0).unwrap();

        // Just after
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &Some(start),
            &None,
            inclusive,
        ));
        // To small
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 7, 0, 0).unwrap(),
            &Some(start),
            &None,
            inclusive,
        ));
        // Bigger
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 2, 8, 0, 0).unwrap(),
            &Some(start),
            &None,
            inclusive,
        ));
        // Equal to start
        assert!(!is_in_range(&start, &Some(start), &None, inclusive));
    }

    #[test]
    fn in_range_exclusive_end() {
        let inclusive = false;
        let end = UTC.with_ymd_and_hms(2021, 10, 1, 10, 0, 0).unwrap();

        // Just before
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &None,
            &Some(end),
            inclusive,
        ));
        // Smaller
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 9, 20, 10, 0, 0).unwrap(),
            &None,
            &Some(end),
            inclusive,
        ));
        // Bigger
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 2, 8, 0, 0).unwrap(),
            &None,
            &Some(end),
            inclusive,
        ));
        // Equal to end
        assert!(!is_in_range(&end, &None, &Some(end), inclusive));
    }

    #[test]
    fn in_range_exclusive_all() {
        let inclusive = false;

        // Some date
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &None,
            &None,
            inclusive,
        ));
        // Smaller
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 9, 20, 10, 0, 0).unwrap(),
            &None,
            &None,
            inclusive,
        ));
        // Bigger
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 2, 8, 0, 0).unwrap(),
            &None,
            &None,
            inclusive,
        ));
    }

    // ---------------- inclusive -----------------------

    #[test]
    fn in_range_inclusive_start_to_end() {
        let inclusive = true;
        let start = UTC.with_ymd_and_hms(2021, 10, 1, 8, 0, 0).unwrap();
        let end = UTC.with_ymd_and_hms(2021, 10, 1, 10, 0, 0).unwrap();

        // In middle
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &Some(start),
            &Some(end),
            inclusive,
        ));
        // To small
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 7, 0, 0).unwrap(),
            &Some(start),
            &Some(end),
            inclusive,
        ));
        // To big
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 11, 0, 0).unwrap(),
            &Some(start),
            &Some(end),
            inclusive,
        ));
        // Equal to end
        assert!(is_in_range(&end, &Some(start), &Some(end), inclusive));
        // Equal to start
        assert!(is_in_range(&start, &Some(start), &Some(end), inclusive));
    }

    #[test]
    fn in_range_inclusive_start() {
        let inclusive = true;
        let start = UTC.with_ymd_and_hms(2021, 10, 1, 8, 0, 0).unwrap();

        // Just after
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &Some(start),
            &None,
            inclusive,
        ));
        // To small
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 7, 0, 0).unwrap(),
            &Some(start),
            &None,
            inclusive,
        ));
        // Bigger
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 2, 8, 0, 0).unwrap(),
            &Some(start),
            &None,
            inclusive,
        ));
        // Equal to start
        assert!(is_in_range(&start, &Some(start), &None, inclusive));
    }

    #[test]
    fn in_range_inclusive_end() {
        let inclusive = true;
        let end = UTC.with_ymd_and_hms(2021, 10, 1, 10, 0, 0).unwrap();

        // Just before
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &None,
            &Some(end),
            inclusive,
        ));
        // Smaller
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 9, 20, 10, 0, 0).unwrap(),
            &None,
            &Some(end),
            inclusive,
        ));
        // Bigger
        assert!(!is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 2, 8, 0, 0).unwrap(),
            &None,
            &Some(end),
            inclusive,
        ));
        // Equal to end
        assert!(is_in_range(&end, &None, &Some(end), inclusive));
    }

    #[test]
    fn in_range_inclusive_all() {
        let inclusive = true;

        // Some date
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 1, 9, 0, 0).unwrap(),
            &None,
            &None,
            inclusive,
        ));
        // Smaller
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 9, 20, 10, 0, 0).unwrap(),
            &None,
            &None,
            inclusive,
        ));
        // Bigger
        assert!(is_in_range(
            &UTC.with_ymd_and_hms(2021, 10, 2, 8, 0, 0).unwrap(),
            &None,
            &None,
            inclusive,
        ));
    }
}
