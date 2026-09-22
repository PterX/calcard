/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use super::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarPeriod, ICalendarProperty,
    ICalendarValue, timezone::TzResolver,
};
use crate::{
    common::{DateTimeResult, timezone::Tz},
    datecalc::{RRuleIter, error::RRuleError, rrule::RRule},
    icalendar::ICalendarParameterName,
};
use ahash::{AHashMap, AHashSet};
use chrono::{DateTime, NaiveDateTime, TimeDelta, TimeZone, Timelike};
use std::{
    collections::hash_map::Entry,
    fmt::{Display, Formatter},
    hash::Hash,
};

#[allow(clippy::type_complexity)]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "serde"), derive(serde::Serialize))]
pub struct CalendarExpand {
    pub events: Vec<CalendarEvent<DateTime<Tz>, TimeOrDelta<DateTime<Tz>, TimeDelta>>>,
    pub errors: Vec<CalendarError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "serde"), derive(serde::Serialize))]
pub struct CalendarEvent<S, E> {
    pub comp_id: u32,
    pub start: S,
    pub end: E,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    any(test, feature = "serde"),
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(any(test, feature = "serde"), serde(tag = "type", content = "data"))]
pub enum TimeOrDelta<T, D> {
    Time(T),
    Delta(D),
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "serde"), derive(serde::Serialize))]
pub struct CalendarError {
    pub comp_id: u32,
    pub error: CalendarErrorType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "serde"), derive(serde::Serialize))]
pub enum CalendarErrorType {
    MissingDtStart,
    InvalidDtStart,
    InvalidDtEnd,
    InvalidDuration,
    RRule(RRuleError),
}

impl ICalendar {
    pub fn expand_dates(&self, default_tz: impl Into<Tz>, mut limit: usize) -> CalendarExpand {
        let tz_resolver = self.build_tz_resolver().with_default(default_tz);
        let mut expand = CalendarExpand::default();
        let mut recurrences = Vec::new();
        let mut overridden = OverriddenInstances::default();

        for (comp_id, comp) in self.components.iter().enumerate() {
            if comp.component_type.has_time_ranges() {
                match comp.build_calendar_date(comp_id as u32, &tz_resolver) {
                    Ok(Some(mut event)) => {
                        if event.rid.is_some() {
                            expand.events.append(&mut event.rdates);
                            overridden.insert(comp_id as u32, event);
                        } else if event.rrule.is_some()
                            || !event.rdates.is_empty()
                            || !event.exdates.is_empty()
                        {
                            recurrences.push((comp_id as u32, event));
                        } else if let Some(cal_event) = event.event {
                            expand.events.push(cal_event);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        expand.errors.push(CalendarError {
                            comp_id: comp_id as u32,
                            error,
                        });
                    }
                }
            }
        }

        // Expand recurrences
        for (mut comp_id, mut event) in recurrences {
            let exdates = event
                .exdates
                .iter()
                .filter_map(|(tz_id, dt)| {
                    dt.to_date_time_with_tz(tz_id.map_or(event.start_tz, |tz_id| {
                        tz_resolver.resolve_or_default(Some(tz_id))
                    }))
                })
                .collect::<AHashSet<_>>();
            for instance in std::mem::take(&mut event.rdates)
                .into_iter()
                .chain(event.rrule.is_none().then(|| event.event.take()).flatten())
            {
                match overridden.take(&event, &instance.start) {
                    Some((_, overridden_event)) => expand.events.extend(overridden_event.event),
                    None if !exdates.contains(&instance.start) => expand.events.push(instance),
                    None => {}
                }
            }
            let Some(rrule) = event.rrule.take() else {
                continue;
            };
            let floating_start = if let Some(floating_start) = Tz::Floating
                .from_local_datetime(&event.dt_start.date_time)
                .single()
            {
                floating_start
            } else {
                expand.errors.push(CalendarError {
                    comp_id,
                    error: CalendarErrorType::InvalidDtStart,
                });
                continue;
            };
            let rrule = match rrule
                .with_floating_until(event.until_tz())
                .validate(floating_start)
            {
                Ok(rrule) => rrule,
                Err(err) => {
                    expand.errors.push(CalendarError {
                        comp_id,
                        error: CalendarErrorType::RRule(err),
                    });
                    continue;
                }
            };
            let mut override_offset = None;
            let mut override_duration = None;

            for date in RRuleIter::new(&rrule, &floating_start, true) {
                if limit != 0 {
                    limit -= 1;
                } else {
                    break;
                }
                let date = if date.timezone().is_floating() {
                    event
                        .start_tz
                        .from_local_datetime(&date.naive_local())
                        .single()
                        .unwrap_or(date)
                } else {
                    date
                };
                if event.rdate_starts.contains(&date) {
                    continue;
                }
                match overridden.take(&event, &date) {
                    Some((new_comp_id, overridden_event)) => {
                        if let Some(new_event) = overridden_event.event {
                            if overridden_event.rid_this_and_future {
                                comp_id = new_comp_id;
                                override_offset = Some(new_event.start - date);
                                override_duration = Some(overridden_event.default_duration);
                            }
                            expand.events.push(new_event);
                        }
                    }
                    None if !exdates.contains(&date) => {
                        expand.events.push(CalendarEvent {
                            start: override_offset.map_or(date, |offset| date + offset),
                            end: TimeOrDelta::Delta(
                                override_duration.unwrap_or(event.default_duration),
                            ),
                            comp_id,
                        });
                    }
                    None => {}
                }
            }
        }

        // Add missing overridden events (this should not occur unless the iCalendar is malformed)
        expand.events.extend(overridden.into_events());

        expand
    }
}

type ExpandedEvent = CalendarEvent<DateTime<Tz>, TimeOrDelta<DateTime<Tz>, TimeDelta>>;

struct CalendarEventBuilder<'x> {
    event: Option<ExpandedEvent>,
    dt_start: DateTimeResult,
    dt_start_tzid: Option<&'x str>,
    start_tz: Tz,
    default_duration: TimeDelta,
    rrule: Option<RRule>,
    uid: Option<&'x str>,
    sequence: i64,
    rdates: Vec<ExpandedEvent>,
    rdate_starts: AHashSet<DateTime<Tz>>,
    exdates: Vec<(Option<&'x str>, DateTimeResult)>,
    rid: Option<DateTime<Tz>>,
    rid_this_and_future: bool,
}

impl CalendarEventBuilder<'_> {
    fn until_tz(&self) -> Tz {
        self.dt_start
            .tz()
            .or(self.dt_start_tzid.map(|_| self.start_tz))
            .unwrap_or(Tz::Floating)
    }
}

type OverriddenInstance<'x> = (u32, CalendarEventBuilder<'x>);

#[derive(Default)]
struct OverriddenInstances<'x> {
    zoned: AHashMap<(Option<&'x str>, DateTime<Tz>), OverriddenInstance<'x>>,
    floating: AHashMap<(Option<&'x str>, NaiveDateTime), OverriddenInstance<'x>>,
}

impl<'x> OverriddenInstances<'x> {
    fn insert(&mut self, comp_id: u32, instance: CalendarEventBuilder<'x>) {
        match instance.rid {
            Some(rid) if rid.timezone().is_floating() => {
                Self::insert_latest(
                    &mut self.floating,
                    (instance.uid, rid.naive_local()),
                    (comp_id, instance),
                );
            }
            Some(rid) => {
                Self::insert_latest(&mut self.zoned, (instance.uid, rid), (comp_id, instance));
            }
            None => {}
        }
    }

    fn insert_latest<K: Eq + Hash>(
        instances: &mut AHashMap<K, OverriddenInstance<'x>>,
        key: K,
        instance: OverriddenInstance<'x>,
    ) {
        match instances.entry(key) {
            Entry::Occupied(current) if current.get().1.sequence > instance.1.sequence => {}
            Entry::Occupied(mut current) => {
                current.insert(instance);
            }
            Entry::Vacant(slot) => {
                slot.insert(instance);
            }
        }
    }

    fn take(
        &mut self,
        series: &CalendarEventBuilder<'x>,
        start: &DateTime<Tz>,
    ) -> Option<OverriddenInstance<'x>> {
        self.zoned.remove(&(series.uid, *start)).or_else(|| {
            (!self.floating.is_empty())
                .then(|| self.floating.remove(&(series.uid, start.naive_local())))
                .flatten()
        })
    }

    fn into_events(self) -> impl Iterator<Item = ExpandedEvent> {
        self.zoned
            .into_values()
            .chain(self.floating.into_values())
            .filter_map(|(_, instance)| instance.event)
    }
}

impl ICalendarComponent {
    fn build_calendar_date(
        &self,
        comp_id: u32,
        tz_resolver: &TzResolver<&'_ str>,
    ) -> Result<Option<CalendarEventBuilder<'_>>, CalendarErrorType> {
        let mut dt_start = None;
        let mut dt_start_tzid = None;
        let mut dt_start_has_time = false;
        let mut dt_end: Option<DateTimeResult> = None;
        let mut dt_end_tzid = None;
        let mut todo_dates = vec![];
        let mut rid: Option<DateTimeResult> = None;
        let mut rid_tzid = None;
        let mut rid_this_and_future = false;
        let mut duration = None;
        let mut rrule = None;
        let mut uid = None;
        let mut sequence = 0;
        let mut rdates = vec![];
        let mut rdates_periods = vec![];
        let mut exdates = vec![];

        for entry in &self.entries {
            match (&entry.name, entry.values.first()) {
                (ICalendarProperty::Dtstart, Some(ICalendarValue::PartialDateTime(dt))) => {
                    dt_start = dt.to_date_time();
                    dt_start_tzid = entry.tz_id();
                    dt_start_has_time = dt.has_time();
                }
                (ICalendarProperty::Dtend, Some(ICalendarValue::PartialDateTime(dt))) => {
                    if let Some(dt) = dt.to_date_time() {
                        dt_end = Some(dt);
                        dt_end_tzid = entry.tz_id();
                    }
                }
                (
                    ICalendarProperty::Due
                    | ICalendarProperty::Completed
                    | ICalendarProperty::Created,
                    Some(ICalendarValue::PartialDateTime(dt)),
                ) if self.component_type == ICalendarComponentType::VTodo => {
                    todo_dates.push((&entry.name, dt.to_date_time(), entry.tz_id()));
                }
                (ICalendarProperty::RecurrenceId, Some(ICalendarValue::PartialDateTime(dt))) => {
                    if let Some(dt) = dt.to_date_time() {
                        for param in &entry.params {
                            match &param.name {
                                ICalendarParameterName::Tzid => {
                                    rid_tzid = param.value.as_text();
                                }
                                ICalendarParameterName::Range => {
                                    rid_this_and_future = true;
                                }
                                _ => (),
                            }
                        }

                        rid = Some(dt);
                    }
                }
                (ICalendarProperty::Duration, Some(ICalendarValue::Duration(dur))) => {
                    duration = Some(dur);
                }
                (ICalendarProperty::Rrule, Some(ICalendarValue::RecurrenceRule(rule))) => {
                    rrule = RRule::from_floating_ical(rule);
                }
                (ICalendarProperty::Uid, Some(ICalendarValue::Text(value))) => {
                    uid = Some(value.as_str());
                }
                (ICalendarProperty::Sequence, Some(ICalendarValue::Integer(value))) => {
                    sequence = *value;
                }
                (ICalendarProperty::Rdate, _) => {
                    let tz_id = entry.tz_id();
                    for value in &entry.values {
                        match value {
                            ICalendarValue::PartialDateTime(dt) => {
                                if let Some(dt) = dt.to_date_time() {
                                    rdates.push((tz_id, dt));
                                }
                            }
                            ICalendarValue::Period(period) => match period {
                                ICalendarPeriod::Range { start, end } => {
                                    if let (Some(start), Some(end)) =
                                        (start.to_date_time(), end.to_date_time())
                                    {
                                        rdates_periods.push((tz_id, start, TimeOrDelta::Time(end)));
                                    }
                                }
                                ICalendarPeriod::Duration { start, duration } => {
                                    if let (Some(start), Some(duration)) =
                                        (start.to_date_time(), duration.to_time_delta())
                                    {
                                        rdates_periods.push((
                                            tz_id,
                                            start,
                                            TimeOrDelta::Delta(duration),
                                        ));
                                    }
                                }
                            },
                            _ => (),
                        }
                    }
                }
                (ICalendarProperty::Exdate, _) => {
                    let tz_id = entry.tz_id();
                    for value in &entry.values {
                        if let ICalendarValue::PartialDateTime(dt) = value
                            && let Some(dt) = dt.to_date_time()
                        {
                            exdates.push((tz_id, dt));
                        }
                    }
                }
                _ => (),
            }
        }

        let dt_start = match dt_start {
            Some(dt_start) => dt_start,
            None => match &rid {
                Some(rid) => {
                    dt_start_tzid = rid_tzid;
                    rid.clone()
                }
                None => match self.component_type {
                    ICalendarComponentType::VEvent => {
                        return Err(CalendarErrorType::MissingDtStart);
                    }
                    ICalendarComponentType::VTodo => {
                        let mut due_idx = None;
                        let mut completed_idx = None;
                        let mut created_idx = None;

                        for (idx, (prop, dt, _)) in todo_dates.iter().enumerate() {
                            if dt.is_some() {
                                match prop {
                                    ICalendarProperty::Due => due_idx = Some(idx),
                                    ICalendarProperty::Completed => completed_idx = Some(idx),
                                    ICalendarProperty::Created => created_idx = Some(idx),
                                    _ => (),
                                }
                            }
                        }

                        match (due_idx, completed_idx, created_idx) {
                            (Some(due_idx), _, _) => {
                                let due = &mut todo_dates[due_idx];
                                dt_start_tzid = due.2;
                                dt_start_has_time = true;
                                due.1.take().unwrap()
                            }
                            (_, Some(completed_idx), Some(created_idx)) => {
                                let completed = &mut todo_dates[completed_idx];
                                dt_end = completed.1.take();
                                dt_end_tzid = completed.2;

                                let created = &mut todo_dates[created_idx];
                                dt_start_tzid = created.2;
                                created.1.take().unwrap()
                            }
                            (_, Some(date_idx), _) | (_, _, Some(date_idx)) => {
                                let date = &mut todo_dates[date_idx];
                                dt_start_tzid = date.2;
                                dt_start_has_time = true;
                                date.1.take().unwrap()
                            }
                            _ => {
                                return Ok(None);
                            }
                        }
                    }
                    _ => {
                        return Ok(None);
                    }
                },
            },
        };
        let mut event = None;
        let is_instance = rid.is_some();
        let start_tz = dt_start
            .tz()
            .unwrap_or_else(|| tz_resolver.resolve_or_default(dt_start_tzid));
        let value_tz = |tz_id: Option<&str>| {
            tz_id.map_or(start_tz, |tz_id| {
                tz_resolver.resolve_or_default(Some(tz_id))
            })
        };
        let dt_start_tz = dt_start
            .to_date_time_with_tz(start_tz)
            .ok_or(CalendarErrorType::InvalidDtStart)?;
        let default_duration = if let Some(dt_end) = dt_end {
            let end = dt_end
                .to_date_time_with_tz(tz_resolver.resolve_or_default(dt_end_tzid.or(dt_start_tzid)))
                .ok_or(CalendarErrorType::InvalidDtEnd)?;

            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end: TimeOrDelta::Time(end),
                    comp_id,
                });
            }
            dt_end.date_time - dt_start.date_time
        } else if let Some(duration) = duration {
            let duration = duration
                .to_time_delta()
                .ok_or(CalendarErrorType::InvalidDuration)?;
            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end: TimeOrDelta::Delta(duration),
                    comp_id,
                });
            }
            duration
        } else if let Some((due, due_tzid)) = todo_dates
            .into_iter()
            .filter_map(|(prop, dt, tz_id)| {
                if prop == &ICalendarProperty::Due {
                    dt.map(|dt| (dt, tz_id))
                } else {
                    None
                }
            })
            .next()
        {
            let end = due
                .to_date_time_with_tz(tz_resolver.resolve_or_default(due_tzid.or(dt_start_tzid)))
                .ok_or(CalendarErrorType::InvalidDtEnd)?;

            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end: TimeOrDelta::Time(end),
                    comp_id,
                });
            }
            due.date_time - dt_start.date_time
        } else {
            /*
               For cases where a "VEVENT" calendar component
               specifies a "DTSTART" property with a DATE value type but no
               "DTEND" nor "DURATION" property, the event's duration is taken to
               be one day.  For cases where a "VEVENT" calendar component
               specifies a "DTSTART" property with a DATE-TIME value type but no
               "DTEND" property, the event ends on the same calendar date and
               time of day specified by the "DTSTART" property.
            */

            let duration = if dt_start_has_time {
                // If the start has time, we use the same time for the end
                dt_start
                    .date_time
                    .with_hour(23)
                    .and_then(|dt| dt.with_minute(59))
                    .and_then(|dt| dt.with_second(59))
                    .map(|dt| dt - dt_start.date_time)
                    .unwrap_or_else(|| TimeDelta::days(1))
            } else {
                TimeDelta::days(1)
            };
            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end: TimeOrDelta::Delta(duration),
                    comp_id,
                });
            }
            duration
        };
        let rid = rid.and_then(|rid| {
            rid.to_date_time_with_tz(rid_tzid.map_or(Tz::Floating, |tz_id| {
                tz_resolver.resolve_or_default(Some(tz_id))
            }))
        });

        // Add rdates
        let mut rdate_events = rdates
            .into_iter()
            .filter_map(|(tz_id, rdate)| {
                rdate
                    .to_date_time_with_tz(value_tz(tz_id))
                    .map(|start| CalendarEvent {
                        start,
                        end: TimeOrDelta::Delta(default_duration),
                        comp_id,
                    })
            })
            .chain(
                rdates_periods
                    .into_iter()
                    .filter_map(|(tz_id, start, end)| {
                        let tz = value_tz(tz_id);
                        Some(CalendarEvent {
                            start: start.to_date_time_with_tz(tz)?,
                            end: end.into_date_time_with_tz(tz)?,
                            comp_id,
                        })
                    }),
            )
            .collect::<Vec<_>>();
        let mut rdate_starts = AHashSet::new();
        if !rdate_events.is_empty() {
            rdate_events.retain(|rdate| rdate_starts.insert(rdate.start));
            if event
                .as_ref()
                .is_some_and(|event| rdate_starts.contains(&event.start))
            {
                event = None;
            }
        }

        Ok(Some(CalendarEventBuilder {
            event,
            dt_start_tzid,
            default_duration,
            uid,
            sequence,
            rrule,
            rdates: rdate_events,
            rdate_starts,
            exdates,
            start_tz,
            dt_start,
            rid,
            rid_this_and_future,
        }))
    }
}

impl TimeOrDelta<DateTimeResult, TimeDelta> {
    pub fn into_date_time_with_tz(self, tz: Tz) -> Option<TimeOrDelta<DateTime<Tz>, TimeDelta>> {
        match self {
            TimeOrDelta::Time(time) => time.to_date_time_with_tz(tz).map(TimeOrDelta::Time),
            TimeOrDelta::Delta(delta) => Some(TimeOrDelta::Delta(delta)),
        }
    }
}

impl CalendarEvent<DateTime<Tz>, TimeOrDelta<DateTime<Tz>, TimeDelta>> {
    pub fn timestamps(&self) -> (i64, i64) {
        let timestamp = self.start.timestamp();
        let end_timestamp = match self.end {
            TimeOrDelta::Time(time) => time.timestamp(),
            TimeOrDelta::Delta(delta) => timestamp + delta.num_seconds(),
        };

        (timestamp, end_timestamp)
    }

    pub fn try_into_date_time(self) -> Option<CalendarEvent<DateTime<Tz>, DateTime<Tz>>> {
        match self.end {
            TimeOrDelta::Time(time) => Some(time),
            TimeOrDelta::Delta(delta) => self
                .start
                .naive_local()
                .checked_add_signed(delta)
                .and_then(|end| end.and_local_timezone(self.start.timezone()).single()),
        }
        .map(|end| CalendarEvent {
            start: self.start,
            end,
            comp_id: self.comp_id,
        })
    }
}

impl Display for CalendarErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarErrorType::MissingDtStart => write!(f, "Missing DTSTART property"),
            CalendarErrorType::InvalidDtStart => write!(f, "Invalid DTSTART property"),
            CalendarErrorType::InvalidDtEnd => write!(f, "Invalid DTEND property"),
            CalendarErrorType::InvalidDuration => write!(f, "Invalid DURATION property"),
            CalendarErrorType::RRule(err) => write!(f, "RRule error: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Entry, Parser,
        common::timezone::Tz,
        icalendar::{
            ICalendar,
            dates::{CalendarError, CalendarEvent},
        },
    };
    use chrono::DateTime;
    use serde::Serialize;
    use std::{io::Write, time::Instant};

    fn expanded(events: &[&str]) -> Vec<String> {
        expanded_in(
            Tz::UTC,
            &events
                .iter()
                .map(|event| format!("UID:expand\r\n{event}"))
                .collect::<Vec<_>>(),
        )
    }

    fn expanded_in(default_tz: impl Into<Tz>, events: &[String]) -> Vec<String> {
        let mut ical = String::from("BEGIN:VCALENDAR\r\n");
        for event in events {
            ical.push_str("BEGIN:VEVENT\r\n");
            ical.push_str(event);
            ical.push_str("END:VEVENT\r\n");
        }
        ical.push_str("END:VCALENDAR\r\n");
        let mut instances = ICalendar::parse(&ical)
            .unwrap()
            .expand_dates(default_tz, 100)
            .events
            .into_iter()
            .filter_map(|event| event.try_into_date_time())
            .map(|event| {
                format!(
                    "{}/{}#{}",
                    event.start.to_rfc3339(),
                    event.end.to_rfc3339(),
                    event.comp_id
                )
            })
            .collect::<Vec<_>>();
        instances.sort();
        instances
    }

    #[test]
    fn rfc5545_3_8_5_2_duplicate_instances_are_ignored() {
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=Europe/Berlin:20250106T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\nRDATE;TZID=Europe/Berlin:20250107T090000,20250107T090000\r\n"
            ]),
            [
                "2025-01-06T09:00:00+01:00/2025-01-06T10:00:00+01:00#1",
                "2025-01-07T09:00:00+01:00/2025-01-07T10:00:00+01:00#1",
                "2025-01-08T09:00:00+01:00/2025-01-08T10:00:00+01:00#1",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20150219T133000Z\r\nRDATE;VALUE=PERIOD:20150219T133000Z/PT10H\r\n"
            ]),
            ["2015-02-19T13:30:00+00:00/2015-02-19T23:30:00+00:00#1"],
            "RFC 5545 Section 3.8.5.2: the RDATE PERIOD duration applies"
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_overrides_replace_rdate_instances() {
        for master in [
            "DTSTART;TZID=Europe/Berlin:20250106T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\nRDATE;TZID=Europe/Berlin:20250110T200000\r\n",
            "DTSTART;TZID=Europe/Berlin:20250106T090000\r\nDURATION:PT1H\r\nRDATE;TZID=Europe/Berlin:20250107T090000,20250110T200000\r\n",
        ] {
            let instances = expanded(&[
                master,
                "RECURRENCE-ID;TZID=Europe/Berlin:20250110T200000\r\nDTSTART;TZID=Europe/Berlin:20250110T210000\r\nDURATION:PT1H\r\n",
                "RECURRENCE-ID;TZID=Europe/Berlin:20250106T090000\r\nDTSTART;TZID=Europe/Berlin:20250106T100000\r\nDURATION:PT1H\r\n",
            ]);
            assert_eq!(
                instances,
                [
                    "2025-01-06T10:00:00+01:00/2025-01-06T11:00:00+01:00#3",
                    "2025-01-07T09:00:00+01:00/2025-01-07T10:00:00+01:00#1",
                    "2025-01-10T21:00:00+01:00/2025-01-10T22:00:00+01:00#2",
                ],
                "{master}"
            );
        }
    }

    #[test]
    fn zone_less_recurrence_id_matches_the_local_instance() {
        assert_eq!(
            expanded(&[
                "DTSTART;VALUE=DATE:20250106\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID;VALUE=DATE:20250107\r\nDTSTART;TZID=Europe/Berlin:20250107T100000\r\nDURATION:PT1H\r\n",
            ]),
            [
                "2025-01-06T00:00:00+00:00/2025-01-07T00:00:00+00:00#1",
                "2025-01-07T10:00:00+01:00/2025-01-07T11:00:00+01:00#2",
                "2025-01-08T00:00:00+00:00/2025-01-09T00:00:00+00:00#1",
            ],
            "RFC 5545 Section 3.8.4.4: a DATE RECURRENCE-ID is the calendar date of the instance"
        );
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=Europe/Berlin:20250106T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\n",
                "RECURRENCE-ID:20250107T090000\r\nDTSTART;TZID=Asia/Tokyo:20250107T120000\r\nDURATION:PT1H\r\n",
            ]),
            [
                "2025-01-06T09:00:00+01:00/2025-01-06T10:00:00+01:00#1",
                "2025-01-07T12:00:00+09:00/2025-01-07T13:00:00+09:00#2",
            ]
        );
    }

    #[test]
    fn rfc5545_3_3_10_utc_until_is_compared_in_the_start_time_zone() {
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=Europe/Berlin:20250106T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;UNTIL=20250108T080000Z\r\n"
            ]),
            [
                "2025-01-06T09:00:00+01:00/2025-01-06T10:00:00+01:00#1",
                "2025-01-07T09:00:00+01:00/2025-01-07T10:00:00+01:00#1",
                "2025-01-08T09:00:00+01:00/2025-01-08T10:00:00+01:00#1",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=America/New_York:20250106T220000\r\nDURATION:PT30M\r\nRRULE:FREQ=HOURLY;UNTIL=20250107T040000Z\r\n"
            ]),
            [
                "2025-01-06T22:00:00-05:00/2025-01-06T22:30:00-05:00#1",
                "2025-01-06T23:00:00-05:00/2025-01-06T23:30:00-05:00#1",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;UNTIL=20250107T090000Z\r\n"
            ]),
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
                "2025-01-07T09:00:00+00:00/2025-01-07T10:00:00+00:00#1",
            ]
        );
    }

    #[test]
    fn rfc5545_3_8_5_3_exdate_excludes_rdate_and_start() {
        assert_eq!(
            expanded(&[
                "DTSTART;VALUE=DATE:20090401\r\nDTEND;VALUE=DATE:20090402\r\nRDATE;VALUE=DATE:20090401\r\nEXDATE;VALUE=DATE:20090401\r\n"
            ]),
            Vec::<String>::new()
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRDATE:20250107T090000Z,20250108T090000Z\r\nEXDATE:20250106T090000Z,20250108T090000Z\r\n"
            ]),
            ["2025-01-07T09:00:00+00:00/2025-01-07T10:00:00+00:00#1"]
        );
        assert_eq!(
            expanded(&[
                "DTSTART;VALUE=DATE:20040224\r\nDTEND;VALUE=DATE:20040225\r\nRRULE:FREQ=MONTHLY;COUNT=3\r\nEXDATE;VALUE=DATE:20040324\r\n",
                "RECURRENCE-ID;VALUE=DATE:20040324\r\nDTSTART;VALUE=DATE:20040325\r\nDTEND;VALUE=DATE:20040326\r\n",
            ]),
            [
                "2004-02-24T00:00:00+00:00/2004-02-25T00:00:00+00:00#1",
                "2004-03-25T00:00:00+00:00/2004-03-26T00:00:00+00:00#2",
                "2004-04-24T00:00:00+00:00/2004-04-25T00:00:00+00:00#1",
            ],
            "an override component is not hidden by an EXDATE for its instance"
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_overrides_match_uid_and_recurrence_id() {
        assert_eq!(
            expanded_in(
                Tz::UTC,
                &[
                    "UID:a\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\n".into(),
                    "UID:b\r\nRECURRENCE-ID:20250107T090000Z\r\nDTSTART:20250107T120000Z\r\nDURATION:PT1H\r\n".into(),
                ]
            ),
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
                "2025-01-07T09:00:00+00:00/2025-01-07T10:00:00+00:00#1",
                "2025-01-07T12:00:00+00:00/2025-01-07T13:00:00+00:00#2",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID:20250107T090000Z\r\nSEQUENCE:3\r\nDTSTART:20250107T140000Z\r\nDURATION:PT1H\r\n",
                "RECURRENCE-ID:20250107T090000Z\r\nSEQUENCE:1\r\nDTSTART:20250107T110000Z\r\nDURATION:PT1H\r\n",
                "RECURRENCE-ID:20250108T090000Z\r\nSEQUENCE:2\r\nDTSTART:20250108T110000Z\r\nDURATION:PT1H\r\n",
            ]),
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
                "2025-01-07T14:00:00+00:00/2025-01-07T15:00:00+00:00#2",
                "2025-01-08T11:00:00+00:00/2025-01-08T12:00:00+00:00#4",
            ],
            "RFC 5545 Section 3.8.7.4: instances MAY have different sequence numbers; the highest revision wins"
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_this_and_future_matches_by_recurrence_id() {
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=5\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20250107T120000Z\r\nDTSTART:20250107T090000Z\r\nDURATION:PT1H\r\n",
                "RECURRENCE-ID:20250108T120000Z\r\nDTSTART:20250108T170000Z\r\nDURATION:PT1H\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T13:00:00+00:00#1",
                "2025-01-07T09:00:00+00:00/2025-01-07T10:00:00+00:00#2",
                "2025-01-08T17:00:00+00:00/2025-01-08T18:00:00+00:00#3",
                "2025-01-09T09:00:00+00:00/2025-01-09T10:00:00+00:00#2",
                "2025-01-10T09:00:00+00:00/2025-01-10T10:00:00+00:00#2",
            ]
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_this_and_future_duration_applies_to_later_instances() {
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=4\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20250107T120000Z\r\nDTSTART:20250107T120000Z\r\nDURATION:PT3H\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T13:00:00+00:00#1",
                "2025-01-07T12:00:00+00:00/2025-01-07T15:00:00+00:00#2",
                "2025-01-08T12:00:00+00:00/2025-01-08T15:00:00+00:00#2",
                "2025-01-09T12:00:00+00:00/2025-01-09T15:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: the overriding component governs this and all subsequent instances"
        );

        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT3H\r\nRRULE:FREQ=DAILY;COUNT=4\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20250107T120000Z\r\nDTSTART:20250107T140000Z\r\nDTEND:20250107T143000Z\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T15:00:00+00:00#1",
                "2025-01-07T14:00:00+00:00/2025-01-07T14:30:00+00:00#2",
                "2025-01-08T14:00:00+00:00/2025-01-08T14:30:00+00:00#2",
                "2025-01-09T14:00:00+00:00/2025-01-09T14:30:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: subsequent instances are rescheduled by the same time difference"
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_an_instance_with_an_rrule_is_not_a_series() {
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20250107T120000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nDTSTART:20250107T140000Z\r\nDURATION:PT2H\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T13:00:00+00:00#1",
                "2025-01-07T14:00:00+00:00/2025-01-07T16:00:00+00:00#2",
                "2025-01-08T14:00:00+00:00/2025-01-08T16:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: RECURRENCE-ID references an individual instance, so the component is an override and not a recurrence set of its own"
        );

        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID:20250107T120000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nDTSTART:20250107T140000Z\r\nDURATION:PT2H\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T13:00:00+00:00#1",
                "2025-01-07T14:00:00+00:00/2025-01-07T16:00:00+00:00#2",
                "2025-01-08T12:00:00+00:00/2025-01-08T13:00:00+00:00#1",
            ],
            "RFC 5545 Section 3.8.5.3: RRULE belongs to a recurring component, not to an instance of one"
        );
    }

    #[test]
    fn rfc5545_3_3_5_utc_start_is_expanded_in_utc() {
        assert_eq!(
            expanded_in(
                chrono_tz::Tz::Pacific__Auckland,
                &["UID:utc\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEXDATE:20250107T090000\r\n".into()]
            ),
            ["2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1"]
        );
    }

    #[test]
    fn expand_rrule() {
        // Read all .ics files in the test directory
        for entry in std::fs::read_dir("resources/ical").unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "ics") {
                let input = match String::from_utf8(std::fs::read(&path).unwrap()) {
                    Ok(input) => input,
                    Err(err) => {
                        // ISO-8859-1
                        err.as_bytes()
                            .iter()
                            .map(|&b| b as char)
                            .collect::<String>()
                    }
                };
                let mut parser = Parser::new(&input);
                let mut output = None;
                //let mut output_debug =
                //    std::fs::File::create(path.with_extension("ics.debug")).unwrap();
                let file_name = path.as_path().to_str().unwrap();

                /*if file_name != "resources/ical/246.ics" {
                    continue;
                }*/

                #[derive(Serialize)]
                struct TestResult {
                    errors: Vec<CalendarError>,
                    events: Vec<CalendarEvent<DateTime<Tz>, DateTime<Tz>>>,
                }

                print!("Expanding recurrences for {file_name}... ");
                let now = Instant::now();
                loop {
                    match parser.entry() {
                        Entry::ICalendar(ical) => {
                            let expanded = ical.expand_dates(chrono_tz::Tz::Pacific__Auckland, 100);
                            let mut events = expanded
                                .events
                                .into_iter()
                                .filter_map(|event| event.try_into_date_time())
                                .collect::<Vec<_>>();
                            events.sort_by_key(|a| (a.start, a.comp_id));

                            for err in &expanded.errors {
                                print!("[{}: {:?}] ", err.comp_id, err.error);
                            }

                            if !events.is_empty() || !expanded.errors.is_empty() {
                                writeln!(
                                    output.get_or_insert_with(|| std::fs::File::create(
                                        path.with_extension("json")
                                    )
                                    .unwrap()),
                                    "{}",
                                    serde_json::to_string_pretty(&TestResult {
                                        errors: expanded.errors,
                                        events,
                                    })
                                    .unwrap()
                                )
                                .unwrap();
                            }
                        }
                        Entry::InvalidLine(_) => {}
                        Entry::Eof => {
                            println!(" (done in {:?})", now.elapsed());
                            break;
                        }
                        other => {
                            panic!("Expected iCal, got {other:?} for {file_name}");
                        }
                    }
                }
            }
        }
    }
}
