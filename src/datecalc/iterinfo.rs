/*
 * SPDX-FileCopyrightText: 2021 Fredrik Meringdal, Ralph Bisschops <https://github.com/fmeringdal/rust-rrule>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use super::counter_date::DateTimeIter;
use super::easter::easter;
use super::get_month;
use super::rrule::{NWeekday, RRule};
use super::{monthinfo::MonthInfo, yearinfo::YearInfo};
use crate::common::timezone::Tz;
use crate::icalendar::ICalendarFrequency;
use chrono::{Datelike, NaiveTime, TimeZone};

#[derive(Debug, Clone)]
pub(crate) struct IterInfo {
    year_info: YearInfo,
    month_info: Option<MonthInfo>,
    easter_mask: Option<Vec<i32>>,
    rrule: RRule,
}

impl IterInfo {
    pub fn new(rrule: &RRule, dt_start: &chrono::DateTime<Tz>) -> Self {
        let year = dt_start.year();
        let month = get_month(dt_start);

        let year_info = YearInfo::new(year, rrule);
        let mut ii = Self {
            rrule: rrule.clone(),
            year_info,
            month_info: None,
            easter_mask: None,
        };
        ii.rebuild_inner(year, month, true);

        ii
    }

    fn rebuild_inner(&mut self, year: i32, month: u8, skip_year_info: bool) {
        if !skip_year_info
            && !matches!(&self.month_info, Some(month_info) if month_info.last_year == year)
        {
            self.year_info = YearInfo::new(year, &self.rrule);
        }

        let contains_nth_by_weekday = self
            .rrule
            .by_weekday
            .iter()
            .any(|by_weekday| matches!(by_weekday, NWeekday::Nth(_, _)));

        if contains_nth_by_weekday
            && !(matches!(&self.month_info, Some(month_info) if month_info.last_month == month && month_info.last_year == year))
        {
            let new_month_info = MonthInfo::new(&self.year_info, month, &self.rrule);
            self.month_info = Some(new_month_info);
        }

        self.easter_mask = self
            .rrule
            .by_easter
            .map(|by_easter| vec![easter(year, by_easter)]);
    }

    pub fn rebuild(&mut self, counter_date: &DateTimeIter) {
        self.rebuild_inner(counter_date.year, counter_date.month as u8, false);
    }

    pub fn year_len(&self) -> u16 {
        self.year_info.year_len
    }

    pub fn year_ordinal(&self) -> i64 {
        self.year_info.year_ordinal
    }

    pub fn month_range(&self) -> &[u16] {
        self.year_info.month_range
    }

    pub fn easter_mask(&self) -> Option<&Vec<i32>> {
        self.easter_mask.as_ref()
    }

    pub fn weekday_mask(&self) -> &[u32] {
        self.year_info.weekday_mask
    }

    pub fn month_mask(&self) -> &[u8] {
        self.year_info.month_mask
    }

    pub fn week_no_mask(&self) -> Option<&Vec<u8>> {
        self.year_info.week_no_mask.as_ref()
    }

    pub fn neg_weekday_mask(&self) -> Option<&Vec<u8>> {
        self.month_info.as_ref().map(|info| &info.neg_weekday_mask)
    }

    pub fn next_year_len(&self) -> u16 {
        self.year_info.next_year_len
    }

    pub fn month_day_mask(&self) -> &[i8] {
        self.year_info.month_day_mask
    }

    pub fn neg_month_day_mask(&self) -> &[i8] {
        self.year_info.neg_month_day_mask
    }

    pub fn year_dayset(&self) -> Vec<usize> {
        let year_len = usize::from(self.year_len());
        (0..year_len).collect()
    }

    pub fn month_dayset(&self, month: u32) -> Vec<usize> {
        let month_range = self.month_range();
        let month = month as usize;
        let start = usize::from(month_range[month - 1]);
        let end = usize::from(month_range[month]);
        (start..end).collect()
    }

    pub fn weekday_set(&self, year: i32, month: u32, day: u32) -> Vec<usize> {
        let set_len = usize::from(self.year_len() + 7);

        let mut date_ordinal = chrono::Utc
            .with_ymd_and_hms(year, month, day, 0, 0, 0)
            .unwrap()
            .ordinal0() as usize;

        let mut set = vec![];

        let week_start_num_days_from_monday = self.rrule.week_start.num_days_from_monday();

        for _ in 0..7 {
            if date_ordinal >= set_len {
                break;
            }
            set.push(date_ordinal);
            date_ordinal += 1;
            if self.weekday_mask()[date_ordinal] == week_start_num_days_from_monday {
                break;
            }
        }

        set
    }

    pub fn day_dayset(year: i32, month: u32, day: u32) -> Vec<usize> {
        let date_ordinal = chrono::Utc
            .with_ymd_and_hms(year, month, day, 0, 0, 0)
            .unwrap()
            .ordinal0();

        vec![date_ordinal as usize]
    }

    pub fn hour_timeset(&self, hour: u8) -> Vec<NaiveTime> {
        self.rrule
            .by_minute
            .iter()
            .flat_map(|minute| self.min_timeset(hour, *minute))
            .collect()
    }

    pub fn min_timeset(&self, hour: u8, minute: u8) -> Vec<NaiveTime> {
        self.rrule
            .by_second
            .iter()
            .filter_map(|second| {
                NaiveTime::from_hms_opt(u32::from(hour), u32::from(minute), u32::from(*second))
            })
            .collect()
    }

    pub fn sec_timeset(hour: u8, minute: u8, second: u8) -> Vec<NaiveTime> {
        if let Some(time) =
            NaiveTime::from_hms_opt(u32::from(hour), u32::from(minute), u32::from(second))
        {
            vec![time]
        } else {
            vec![]
        }
    }

    pub fn get_dayset(
        &self,
        freq: ICalendarFrequency,
        year: i32,
        month: u32,
        day: u32,
    ) -> Vec<usize> {
        let mut dayset = match freq {
            ICalendarFrequency::Yearly => self.year_dayset(),
            ICalendarFrequency::Monthly => self.month_dayset(month),
            ICalendarFrequency::Weekly => self.weekday_set(year, month, day),
            _ => Self::day_dayset(year, month, day),
        };

        // Filter out days according to the RRule filters.
        dayset.retain(|day| !super::filters::is_filtered(self, *day));

        dayset
    }

    /// Gets a timeset without checking if the hour, minute and second are valid, according
    /// to the `RRule`.
    ///
    /// This is usually called after calling the `increment_counter_date` where we know
    /// that we get a valid `DateTime` back, and there is no need to do any duplicate
    /// validation.
    pub fn get_timeset_unchecked(&self, hour: u8, minute: u8, second: u8) -> Vec<NaiveTime> {
        match self.rrule.freq {
            ICalendarFrequency::Hourly => self.hour_timeset(hour),
            ICalendarFrequency::Minutely => self.min_timeset(hour, minute),
            ICalendarFrequency::Secondly => Self::sec_timeset(hour, minute, second),
            _ => unreachable!(
                "This method is never called with an invalid frequency and is not publicly exposed"
            ),
        }
    }

    /// Gets a timeset.
    ///
    /// An empty set is returned if the hour, minute and second aren't valid,
    /// according to the `RRule`.
    pub fn get_timeset(&self, hour: u8, minute: u8, second: u8) -> Vec<NaiveTime> {
        match self.rrule.freq {
            ICalendarFrequency::Hourly
            | ICalendarFrequency::Minutely
            | ICalendarFrequency::Secondly => {
                let incorrect_hour = self.rrule.freq >= ICalendarFrequency::Hourly
                    && !self.rrule.by_hour.is_empty()
                    && !self.rrule.by_hour.contains(&hour);
                let incorrect_minute = self.rrule.freq >= ICalendarFrequency::Minutely
                    && !self.rrule.by_minute.is_empty()
                    && !self.rrule.by_minute.contains(&minute);
                let incorrect_second = self.rrule.freq >= ICalendarFrequency::Secondly
                    && !self.rrule.by_second.is_empty()
                    && !self.rrule.by_second.contains(&second);
                let date_is_not_a_candidate =
                    incorrect_hour || incorrect_minute || incorrect_second;

                // If date is not a potential candidate, then we return an empty timeset.
                if date_is_not_a_candidate {
                    return vec![];
                }

                self.get_timeset_unchecked(hour, minute, second)
            }
            _ => {
                let timeset = self
                    .rrule
                    .by_hour
                    .iter()
                    .flat_map(|hour| {
                        self.rrule.by_minute.iter().flat_map(move |minute| {
                            self.rrule.by_second.iter().filter_map(move |second| {
                                NaiveTime::from_hms_opt(
                                    u32::from(*hour),
                                    u32::from(*minute),
                                    u32::from(*second),
                                )
                            })
                        })
                    })
                    .collect();

                timeset
            }
        }
    }

    pub fn rrule(&self) -> &RRule {
        &self.rrule
    }
}
