/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use super::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarPeriod, ICalendarProperty,
    ICalendarRecurrenceRule, ICalendarValue, timezone::TzResolver,
};
use crate::{
    common::{
        DateTimeResult,
        timezone::{NominalDuration, Tz, ZonedDateTime},
    },
    datecalc::{MAX_UNPRODUCTIVE_WORK, error::RRuleError, rrule::RRule},
    icalendar::ICalendarParameterName,
};
use ahash::{AHashMap, AHashSet};
use jiff::{SignedDuration, civil};
use std::{
    collections::hash_map::Entry,
    fmt::{Display, Formatter},
    hash::Hash,
};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "serde"), derive(serde::Serialize))]
pub struct CalendarExpand {
    pub events: Vec<CalendarEvent>,
    pub errors: Vec<CalendarError>,
}

/// One instance of a calendar component, with its end already resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "serde"), derive(serde::Serialize))]
pub struct CalendarEvent {
    pub comp_id: u32,
    pub start: ZonedDateTime,
    pub end: ZonedDateTime,
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
    ExpansionLimitReached,
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
                            expand
                                .events
                                .extend(event.rdates.drain(..).map(|rdate| rdate.event));
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
        let mut unproductive_budget = MAX_UNPRODUCTIVE_WORK;
        for (series_id, event) in &mut recurrences {
            let mut comp_id = *series_id;
            let length = event.length();
            let exdates = event
                .exdates
                .iter()
                .filter_map(|(tz_id, dt)| {
                    dt.to_date_time_with_tz(tz_id.map_or(event.start_tz, |tz_id| {
                        tz_resolver.resolve_or_default(Some(tz_id))
                    }))
                })
                .collect::<AHashSet<_>>();
            let start_instance = event
                .rrule
                .is_none()
                .then(|| event.event.take())
                .flatten()
                .map(|start| RecurrenceInstance {
                    event: start,
                    length,
                });
            for instance in std::mem::take(&mut event.rdates)
                .into_iter()
                .chain(start_instance)
            {
                match overridden.take(event, &instance.event.start) {
                    Some((_, overridden_event)) => expand.events.extend(
                        overridden_event
                            .replacing(instance.event.start, instance.length)
                            .map(|(replacement, _)| replacement),
                    ),
                    None if !exdates.contains(&instance.event.start) => {
                        expand.events.push(instance.event)
                    }
                    None => {}
                }
            }
            let Some(rrule) = event.rrule.take() else {
                continue;
            };
            let rrule = match RRule::from_ical(rrule, event.dt_start_zoned, event.is_date) {
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
            let mut instances = rrule.iter().with_unproductive_budget(unproductive_budget);

            while limit != 0 {
                let Some(date) = instances.next() else {
                    break;
                };
                limit -= 1;
                if event.rdate_starts.contains(&date) {
                    continue;
                }
                match overridden.take(event, &date) {
                    Some((new_comp_id, overridden_event)) => {
                        if let Some((new_event, new_duration)) =
                            overridden_event.replacing(date, length)
                        {
                            if overridden_event.rid_this_and_future {
                                comp_id = new_comp_id;
                                override_offset =
                                    Some(DefaultDuration::shift(date, new_event.start));
                                override_duration = Some(new_duration);
                            }
                            expand.events.push(new_event);
                        }
                    }
                    None if !exdates.contains(&date) => {
                        let start = match override_offset {
                            Some(offset) => match offset.after(date) {
                                Some(start) => start,
                                None => continue,
                            },
                            None => date,
                        };
                        let duration = override_duration.unwrap_or(event.default_duration);
                        if let Some(end) = duration.after(start) {
                            expand.events.push(CalendarEvent {
                                start,
                                end,
                                comp_id,
                            });
                        }
                    }
                    None => {}
                }
            }

            unproductive_budget = instances.unproductive_budget();
            if instances.is_exhausted() {
                expand.errors.push(CalendarError {
                    comp_id,
                    error: CalendarErrorType::ExpansionLimitReached,
                });
            }
        }

        // Add missing overridden events (this should not occur unless the iCalendar is malformed)
        expand.events.extend(overridden.into_events(&recurrences));

        expand
    }
}

/// How long an instance of a component lasts.
#[derive(Debug, Clone, Copy)]
enum DefaultDuration {
    Exact(SignedDuration),
    Nominal(NominalDuration),
}

impl DefaultDuration {
    /// Returns the duration between the start and end of a series, as
    /// RFC 5545 section 3.8.5.3 says every instance of it should last.
    fn between(start: ZonedDateTime, end: ZonedDateTime, has_time: bool) -> Self {
        if has_time {
            Self::Exact(SignedDuration::from_secs(
                end.timestamp() - start.timestamp(),
            ))
        } else {
            Self::Nominal(NominalDuration::new(end.days_since(start), 0))
        }
    }

    /// Returns the end of an instance that starts at `start`.
    fn shift(from: ZonedDateTime, to: ZonedDateTime) -> Self {
        if from.timezone() == to.timezone() {
            Self::Nominal(NominalDuration::between(from, to))
        } else {
            Self::Exact(to.signed_duration_since(from))
        }
    }

    fn after(&self, start: ZonedDateTime) -> Option<ZonedDateTime> {
        match self {
            Self::Exact(duration) => start.checked_add(*duration),
            Self::Nominal(duration) => start.checked_add_nominal(*duration),
        }
    }
}

struct CalendarEventBuilder<'x> {
    event: Option<CalendarEvent>,
    component_type: &'x ICalendarComponentType,
    start_tz: Tz,
    default_duration: DefaultDuration,
    inherits_start: bool,
    instance_end: InstanceEnd,
    dt_start_zoned: ZonedDateTime,
    is_date: bool,
    rrule: Option<&'x ICalendarRecurrenceRule>,
    uid: Option<&'x str>,
    sequence: i64,
    rdates: Vec<RecurrenceInstance>,
    rdate_starts: AHashSet<ZonedDateTime>,
    exdates: Vec<(Option<&'x str>, DateTimeResult)>,
    rid: Option<ZonedDateTime>,
    rid_this_and_future: bool,
}

#[derive(Debug, Clone, Copy)]
struct RecurrenceInstance {
    event: CalendarEvent,
    length: InstanceLength,
}

#[derive(Debug, Clone, Copy)]
struct InstanceLength {
    duration: DefaultDuration,
    is_date: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstanceEnd {
    Inherited,
    Relative,
    Fixed,
}

impl InstanceEnd {
    fn of(tz_id: Option<&str>, end: &DateTimeResult) -> Self {
        if tz_id.is_some() || end.tz().is_some() {
            InstanceEnd::Fixed
        } else {
            InstanceEnd::Relative
        }
    }
}

impl CalendarEventBuilder<'_> {
    fn length(&self) -> InstanceLength {
        InstanceLength {
            duration: self.default_duration,
            is_date: self.is_date,
        }
    }

    fn anchor(&self, recurrence_id: ZonedDateTime) -> Option<ZonedDateTime> {
        if recurrence_id.timezone().is_floating() {
            self.start_tz.from_local(recurrence_id.naive_local())
        } else {
            Some(recurrence_id.with_timezone(self.start_tz))
        }
    }

    fn replacing(
        &self,
        start: ZonedDateTime,
        replaced: InstanceLength,
    ) -> Option<(CalendarEvent, DefaultDuration)> {
        let event = self.event?;
        if !self.inherits_start || self.is_date != replaced.is_date {
            return Some((event, self.default_duration));
        }
        let (end, duration) = match self.instance_end {
            InstanceEnd::Fixed => (
                event.end,
                DefaultDuration::Exact(event.end.signed_duration_since(start)),
            ),
            InstanceEnd::Relative => (
                self.default_duration.after(start).unwrap_or(event.end),
                self.default_duration,
            ),
            InstanceEnd::Inherited => (
                replaced.duration.after(start).unwrap_or(event.end),
                replaced.duration,
            ),
        };
        Some((
            CalendarEvent {
                start,
                end,
                ..event
            },
            duration,
        ))
    }
}

type OverriddenInstance<'x> = (u32, CalendarEventBuilder<'x>);

type OverrideKey<'x, T> = (Option<&'x str>, &'x ICalendarComponentType, T);

#[derive(Default)]
struct OverriddenInstances<'x> {
    zoned: AHashMap<OverrideKey<'x, ZonedDateTime>, OverriddenInstance<'x>>,
    floating: AHashMap<OverrideKey<'x, civil::DateTime>, OverriddenInstance<'x>>,
}

impl<'x> OverriddenInstances<'x> {
    fn insert(&mut self, comp_id: u32, instance: CalendarEventBuilder<'x>) {
        match instance.rid {
            Some(rid) if rid.timezone().is_floating() => {
                Self::insert_latest(
                    &mut self.floating,
                    (instance.uid, instance.component_type, rid.naive_local()),
                    (comp_id, instance),
                );
            }
            Some(rid) => {
                Self::insert_latest(
                    &mut self.zoned,
                    (instance.uid, instance.component_type, rid),
                    (comp_id, instance),
                );
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
        start: &ZonedDateTime,
    ) -> Option<OverriddenInstance<'x>> {
        (!self.zoned.is_empty())
            .then(|| {
                self.zoned
                    .remove(&(series.uid, series.component_type, *start))
            })
            .flatten()
            .or_else(|| {
                (!self.floating.is_empty())
                    .then(|| {
                        self.floating.remove(&(
                            series.uid,
                            series.component_type,
                            start.naive_local(),
                        ))
                    })
                    .flatten()
            })
    }

    fn into_events<'y>(
        self,
        recurrences: &'y [(u32, CalendarEventBuilder<'x>)],
    ) -> impl Iterator<Item = CalendarEvent> + use<'x, 'y> {
        let series = if self.zoned.is_empty() && self.floating.is_empty() {
            AHashMap::new()
        } else {
            recurrences
                .iter()
                .rev()
                .map(|(_, series)| ((series.uid, series.component_type), series))
                .collect::<AHashMap<_, _>>()
        };
        self.zoned
            .into_values()
            .chain(self.floating.into_values())
            .filter_map(move |(_, instance)| {
                series
                    .get(&(instance.uid, instance.component_type))
                    .and_then(|series| {
                        instance.replacing(series.anchor(instance.rid?)?, series.length())
                    })
                    .map(|(replacement, _)| replacement)
                    .or(instance.event)
            })
    }
}

/// How an `RDATE` of PERIOD value type names the end of its instance.
enum PeriodEnd {
    Time(DateTimeResult),
    Duration(NominalDuration),
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
        let mut rid_has_time = false;
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
                (ICalendarProperty::RecurrenceId, Some(ICalendarValue::PartialDateTime(value))) => {
                    if let Some(dt) = value.to_date_time() {
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

                        rid_has_time = value.has_time();
                        rid = Some(dt);
                    }
                }
                (ICalendarProperty::Duration, Some(ICalendarValue::Duration(dur))) => {
                    duration = Some(dur);
                }
                (ICalendarProperty::Rrule, Some(ICalendarValue::RecurrenceRule(rule))) => {
                    rrule = Some(rule);
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
                            ICalendarValue::Period(period) => match period.as_ref() {
                                ICalendarPeriod::Range { start, end } => {
                                    if let (Some(start), Some(end)) =
                                        (start.to_date_time(), end.to_date_time())
                                    {
                                        rdates_periods.push((tz_id, start, PeriodEnd::Time(end)));
                                    }
                                }
                                ICalendarPeriod::Duration { start, duration } => {
                                    if let (Some(start), Some(duration)) =
                                        (start.to_date_time(), duration.to_nominal())
                                    {
                                        rdates_periods.push((
                                            tz_id,
                                            start,
                                            PeriodEnd::Duration(duration),
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

        let has_start = dt_start.is_some();
        let dt_start = match dt_start {
            Some(dt_start) => dt_start,
            None => match &rid {
                Some(rid) => {
                    dt_start_tzid = rid_tzid;
                    dt_start_has_time = rid_has_time;
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
        let mut instance_end = InstanceEnd::Relative;
        let default_duration = if let Some(dt_end) = dt_end {
            let end = dt_end
                .to_date_time_with_tz(tz_resolver.resolve_or_default(dt_end_tzid.or(dt_start_tzid)))
                .ok_or(CalendarErrorType::InvalidDtEnd)?;

            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end,
                    comp_id,
                });
            }
            instance_end = InstanceEnd::of(dt_end_tzid, &dt_end);
            DefaultDuration::between(dt_start_tz, end, dt_start_has_time)
        } else if let Some(duration) = duration {
            let duration = duration
                .to_nominal()
                .ok_or(CalendarErrorType::InvalidDuration)?;
            let duration = DefaultDuration::Nominal(duration);
            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end: duration
                        .after(dt_start_tz)
                        .ok_or(CalendarErrorType::InvalidDuration)?,
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
                    end,
                    comp_id,
                });
            }
            instance_end = InstanceEnd::of(due_tzid, &due);
            DefaultDuration::between(dt_start_tz, end, dt_start_has_time)
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

            instance_end = InstanceEnd::Inherited;
            let duration =
                if dt_start_has_time || self.component_type == ICalendarComponentType::VTodo {
                    DefaultDuration::Exact(SignedDuration::ZERO)
                } else {
                    DefaultDuration::Nominal(NominalDuration::DAY)
                };
            if rrule.is_none() || is_instance {
                event = Some(CalendarEvent {
                    start: dt_start_tz,
                    end: duration
                        .after(dt_start_tz)
                        .ok_or(CalendarErrorType::InvalidDuration)?,
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
        let is_date = !dt_start_has_time;
        let series_length = InstanceLength {
            duration: default_duration,
            is_date,
        };
        let mut rdate_events = rdates
            .into_iter()
            .filter_map(|(tz_id, rdate)| {
                let start = rdate.to_date_time_with_tz(value_tz(tz_id))?;
                Some(RecurrenceInstance {
                    event: CalendarEvent {
                        start,
                        end: default_duration.after(start)?,
                        comp_id,
                    },
                    length: series_length,
                })
            })
            .chain(
                rdates_periods
                    .into_iter()
                    .filter_map(|(tz_id, start, end)| {
                        let tz = value_tz(tz_id);
                        let start = start.to_date_time_with_tz(tz)?;
                        let (end, duration) = match end {
                            PeriodEnd::Time(end) => {
                                let end = end.to_date_time_with_tz(tz)?;
                                (
                                    end,
                                    DefaultDuration::Exact(end.signed_duration_since(start)),
                                )
                            }
                            PeriodEnd::Duration(duration) => {
                                let duration = DefaultDuration::Nominal(duration);
                                (duration.after(start)?, duration)
                            }
                        };
                        Some(RecurrenceInstance {
                            event: CalendarEvent {
                                start,
                                end,
                                comp_id,
                            },
                            length: InstanceLength {
                                duration,
                                is_date: false,
                            },
                        })
                    }),
            )
            .collect::<Vec<_>>();
        let mut rdate_starts = AHashSet::new();
        if !rdate_events.is_empty() {
            rdate_events.retain(|rdate| rdate_starts.insert(rdate.event.start));
            if event
                .as_ref()
                .is_some_and(|event| rdate_starts.contains(&event.start))
            {
                event = None;
            }
        }

        Ok(Some(CalendarEventBuilder {
            event,
            component_type: &self.component_type,
            default_duration,
            inherits_start: is_instance && !has_start,
            instance_end,
            dt_start_zoned: dt_start_tz,
            is_date,
            uid,
            sequence,
            rrule: rrule.map(|rule| &**rule),
            rdates: rdate_events,
            rdate_starts,
            exdates,
            start_tz,
            rid,
            rid_this_and_future,
        }))
    }
}

impl CalendarEvent {
    /// Returns the instant the instance starts and the instant it ends.
    pub fn timestamps(&self) -> (i64, i64) {
        (self.start.timestamp(), self.end.timestamp())
    }

    /// Returns the wall clock readings of the start and end, as seconds since
    /// the Unix epoch, for callers that index instances by displayed time.
    pub fn naive_timestamps(&self) -> (i64, i64) {
        (self.start.naive_timestamp(), self.end.naive_timestamp())
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
            CalendarErrorType::ExpansionLimitReached => {
                write!(f, "Recurrence expansion gave up before the rule finished")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Entry, Parser,
        common::timezone::Tz,
        datecalc::{MAX_ITER_LOOP, MAX_UNPRODUCTIVE_WORK},
        icalendar::{
            ICalendar,
            dates::{CalendarError, CalendarErrorType, CalendarEvent},
        },
    };
    use serde::Serialize;
    use std::{io::Write, time::Instant};

    fn auckland() -> Tz {
        Tz::iana("Pacific/Auckland").unwrap()
    }

    fn expanded(events: &[&str]) -> Vec<String> {
        expanded_in(
            Tz::UTC,
            &events
                .iter()
                .map(|event| format!("UID:expand\r\n{event}"))
                .collect::<Vec<_>>(),
        )
    }

    fn calendar<E: AsRef<str>>(events: impl IntoIterator<Item = E>) -> ICalendar {
        let mut ical = String::from("BEGIN:VCALENDAR\r\n");
        for event in events {
            ical.push_str("BEGIN:VEVENT\r\n");
            ical.push_str(event.as_ref());
            ical.push_str("END:VEVENT\r\n");
        }
        ical.push_str("END:VCALENDAR\r\n");
        ICalendar::parse(&ical).unwrap()
    }

    fn expanded_in(default_tz: impl Into<Tz>, events: &[String]) -> Vec<String> {
        let mut instances = calendar(events)
            .expand_dates(default_tz, 100)
            .events
            .into_iter()
            .map(|event| format!("{}/{}#{}", event.start, event.end, event.comp_id))
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
                auckland(),
                &["UID:utc\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEXDATE:20250107T090000\r\n".into()]
            ),
            ["2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1"]
        );
    }

    #[test]
    fn rfc5545_3_3_10_all_day_until_in_utc_names_its_last_day() {
        assert_eq!(
            expanded_in(
                Tz::iana("Europe/Berlin").unwrap(),
                &["UID:end-of-day\r\nDTSTART;VALUE=DATE:20150919\r\nDTEND;VALUE=DATE:20150920\r\nRRULE:FREQ=YEARLY;UNTIL=20160918T235959Z\r\n".into()]
            ),
            ["2015-09-19T00:00:00+02:00/2015-09-20T00:00:00+02:00#1"],
            "an UNTIL at the end of a UTC day ends the series on that day"
        );
        assert_eq!(
            expanded_in(
                Tz::iana("America/New_York").unwrap(),
                &["UID:start-of-day\r\nDTSTART;VALUE=DATE:20200903\r\nDTEND;VALUE=DATE:20200904\r\nRRULE:FREQ=WEEKLY;UNTIL=20200916T230000Z;INTERVAL=2;BYDAY=TH\r\n".into()]
            ),
            [
                "2020-09-03T00:00:00-04:00/2020-09-04T00:00:00-04:00#1",
                "2020-09-17T00:00:00-04:00/2020-09-18T00:00:00-04:00#1",
            ],
            "an UNTIL at the UTC instant a day starts somewhere ends the series on that day"
        );
    }

    #[test]
    fn rfc5545_3_3_10_set_position_without_another_rule_selects_the_start() {
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=MONTHLY;COUNT=3;BYSETPOS=-1\r\n"
            ]),
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
                "2025-02-06T09:00:00+00:00/2025-02-06T10:00:00+00:00#1",
                "2025-03-06T09:00:00+00:00/2025-03-06T10:00:00+00:00#1",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=MONTHLY;COUNT=3;BYSETPOS=2\r\n"
            ]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn rfc5545_3_3_10_parts_forbidden_at_the_frequency_are_ignored() {
        let mondays = [
            "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
            "2025-01-13T09:00:00+00:00/2025-01-13T10:00:00+00:00#1",
            "2025-01-20T09:00:00+00:00/2025-01-20T10:00:00+00:00#1",
        ];
        for rule in [
            "FREQ=DAILY;COUNT=3;BYDAY=1MO",
            "FREQ=WEEKLY;COUNT=3;BYMONTHDAY=-1",
            "FREQ=WEEKLY;COUNT=3;BYYEARDAY=10",
            "FREQ=WEEKLY;COUNT=3;BYMONTHDAY=5;BYSETPOS=1",
        ] {
            assert_eq!(
                expanded(&[&format!(
                    "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:{rule}\r\n"
                )]),
                mondays,
                "{rule}"
            );
        }
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=MONTHLY;COUNT=2;BYWEEKNO=1\r\n"
            ]),
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
                "2025-02-06T09:00:00+00:00/2025-02-06T10:00:00+00:00#1",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2;INTERVAL=0\r\n"
            ]),
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T10:00:00+00:00#1",
                "2025-01-07T09:00:00+00:00/2025-01-07T10:00:00+00:00#1",
            ]
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_this_and_future_shift_keeps_the_wall_clock_across_dst() {
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=Europe/Berlin:20250328T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=4\r\n",
                "RECURRENCE-ID;TZID=Europe/Berlin;RANGE=THISANDFUTURE:20250329T090000\r\nDTSTART;TZID=Europe/Berlin:20250330T090000\r\nDURATION:PT1H\r\n",
            ]),
            [
                "2025-03-28T09:00:00+01:00/2025-03-28T10:00:00+01:00#1",
                "2025-03-30T09:00:00+02:00/2025-03-30T10:00:00+02:00#2",
                "2025-03-31T09:00:00+02:00/2025-03-31T10:00:00+02:00#2",
                "2025-04-01T09:00:00+02:00/2025-04-01T10:00:00+02:00#2",
            ]
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_this_and_future_shift_moves_the_wall_clock_into_a_gap() {
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=Europe/Berlin:20250326T220000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=5\r\n",
                "RECURRENCE-ID;TZID=Europe/Berlin;RANGE=THISANDFUTURE:20250327T220000\r\nDTSTART;TZID=Europe/Berlin:20250328T023000\r\nDURATION:PT1H\r\n",
            ]),
            [
                "2025-03-26T22:00:00+01:00/2025-03-26T23:00:00+01:00#1",
                "2025-03-28T02:30:00+01:00/2025-03-28T03:30:00+01:00#2",
                "2025-03-29T02:30:00+01:00/2025-03-29T03:30:00+01:00#2",
                "2025-03-30T02:30:00+01:00/2025-03-30T04:30:00+02:00#2",
                "2025-03-31T02:30:00+02:00/2025-03-31T03:30:00+02:00#2",
            ]
        );
    }

    #[test]
    fn rfc5545_3_6_1_date_time_start_without_end_is_zero_length() {
        assert_eq!(
            expanded(&["DTSTART;TZID=America/New_York:20060102T120000\r\n"]),
            ["2006-01-02T12:00:00-05:00/2006-01-02T12:00:00-05:00#1"],
            "RFC 5545 Section 3.6.1: the event ends on the same calendar date and time of day as DTSTART"
        );
        assert_eq!(
            expanded(&["DTSTART:20060102T120000Z\r\nRRULE:FREQ=DAILY;COUNT=2\r\n"]),
            [
                "2006-01-02T12:00:00+00:00/2006-01-02T12:00:00+00:00#1",
                "2006-01-03T12:00:00+00:00/2006-01-03T12:00:00+00:00#1",
            ]
        );
        assert_eq!(
            expanded(&["DTSTART;VALUE=DATE:20060102\r\nRRULE:FREQ=DAILY;COUNT=2\r\n"]),
            [
                "2006-01-02T00:00:00+00:00/2006-01-03T00:00:00+00:00#1",
                "2006-01-03T00:00:00+00:00/2006-01-04T00:00:00+00:00#1",
            ],
            "RFC 5545 Section 3.6.1: a DATE DTSTART without DTEND or DURATION lasts one day"
        );

        let ical = ICalendar::parse(concat!(
            "BEGIN:VCALENDAR\r\n",
            "BEGIN:VTODO\r\nUID:todo\r\nDTSTART:20060102T120000Z\r\nEND:VTODO\r\n",
            "BEGIN:VTODO\r\nUID:due\r\nDUE:20060103T120000Z\r\nEND:VTODO\r\n",
            "BEGIN:VJOURNAL\r\nUID:journal\r\nDTSTART:20060104T120000Z\r\nEND:VJOURNAL\r\n",
            "BEGIN:VTODO\r\nUID:date-todo\r\nDTSTART;VALUE=DATE:20060105\r\nEND:VTODO\r\n",
            "BEGIN:VJOURNAL\r\nUID:date-journal\r\nDTSTART;VALUE=DATE:20060106\r\nEND:VJOURNAL\r\n",
            "END:VCALENDAR\r\n"
        ))
        .unwrap();
        let mut instances = ical
            .expand_dates(Tz::UTC, 10)
            .events
            .into_iter()
            .map(|event| format!("{}/{}#{}", event.start, event.end, event.comp_id))
            .collect::<Vec<_>>();
        instances.sort();
        assert_eq!(
            instances,
            [
                "2006-01-02T12:00:00+00:00/2006-01-02T12:00:00+00:00#1",
                "2006-01-03T12:00:00+00:00/2006-01-03T12:00:00+00:00#2",
                "2006-01-04T12:00:00+00:00/2006-01-04T12:00:00+00:00#3",
                "2006-01-05T00:00:00+00:00/2006-01-05T00:00:00+00:00#4",
                "2006-01-06T00:00:00+00:00/2006-01-07T00:00:00+00:00#5",
            ],
            "RFC 4791 Section 9.9: a VTODO with only DTSTART or DUE and a VJOURNAL with a DATE-TIME DTSTART are points in time, a VJOURNAL with a DATE DTSTART lasts one day"
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_an_override_without_dtstart_keeps_the_replaced_instance() {
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=America/New_York:20060102T120000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID;TZID=America/New_York:20060103T120000\r\nSUMMARY:Only title\r\n",
            ]),
            [
                "2006-01-02T12:00:00-05:00/2006-01-02T13:00:00-05:00#1",
                "2006-01-03T12:00:00-05:00/2006-01-03T13:00:00-05:00#2",
                "2006-01-04T12:00:00-05:00/2006-01-04T13:00:00-05:00#1",
            ],
            "the start and its value type come from RECURRENCE-ID, the length from the instance it replaces"
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20060102T120000Z\r\nDTEND:20060102T133000Z\r\nRRULE:FREQ=DAILY;COUNT=2\r\n",
                "RECURRENCE-ID:20060103T120000Z\r\nSUMMARY:Renamed\r\n",
            ]),
            [
                "2006-01-02T12:00:00+00:00/2006-01-02T13:30:00+00:00#1",
                "2006-01-03T12:00:00+00:00/2006-01-03T13:30:00+00:00#2",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART;VALUE=DATE:20060102\r\nDTEND;VALUE=DATE:20060104\r\nRRULE:FREQ=WEEKLY;COUNT=2\r\n",
                "RECURRENCE-ID;VALUE=DATE:20060109\r\nSUMMARY:Renamed\r\n",
            ]),
            [
                "2006-01-02T00:00:00+00:00/2006-01-04T00:00:00+00:00#1",
                "2006-01-09T00:00:00+00:00/2006-01-11T00:00:00+00:00#2",
            ]
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20060102T120000Z\r\nDURATION:PT1H\r\nRDATE;VALUE=PERIOD:20060105T090000Z/PT3H\r\n",
                "RECURRENCE-ID:20060105T090000Z\r\nSUMMARY:Renamed\r\n",
            ]),
            [
                "2006-01-02T12:00:00+00:00/2006-01-02T13:00:00+00:00#1",
                "2006-01-05T09:00:00+00:00/2006-01-05T12:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.5.2: the replaced instance lasts as long as its PERIOD"
        );
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20250107T120000Z\r\nSUMMARY:Renamed\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T13:00:00+00:00#1",
                "2025-01-07T12:00:00+00:00/2025-01-07T13:00:00+00:00#2",
                "2025-01-08T12:00:00+00:00/2025-01-08T13:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: the subsequent instances keep the same duration"
        );
    }

    #[test]
    fn rfc5545_3_8_4_4_an_override_without_dtstart_starts_in_the_series_time_zone() {
        for recurrence_id in [
            "RECURRENCE-ID:20060103T170000Z",
            "RECURRENCE-ID;TZID=Europe/Berlin:20060103T180000",
            "RECURRENCE-ID:20060103T120000",
        ] {
            assert_eq!(
                expanded(&[
                    "DTSTART;TZID=America/New_York:20060102T120000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\n",
                    &format!("{recurrence_id}\r\nSUMMARY:Renamed\r\n"),
                ]),
                [
                    "2006-01-02T12:00:00-05:00/2006-01-02T13:00:00-05:00#1",
                    "2006-01-03T12:00:00-05:00/2006-01-03T13:00:00-05:00#2",
                ],
                "the occurrence starts as the instance it replaces: {recurrence_id}"
            );
        }
    }

    #[test]
    fn rfc5545_3_8_4_4_separate_overrides_are_not_impacted_by_this_and_future() {
        assert_eq!(
            expanded(&[
                "DTSTART:20250106T120000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=4\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20250107T120000Z\r\nDTSTART:20250107T140000Z\r\nDURATION:PT3H\r\n",
                "RECURRENCE-ID:20250108T120000Z\r\nSUMMARY:Separate\r\n",
            ]),
            [
                "2025-01-06T12:00:00+00:00/2025-01-06T13:00:00+00:00#1",
                "2025-01-07T14:00:00+00:00/2025-01-07T17:00:00+00:00#2",
                "2025-01-08T12:00:00+00:00/2025-01-08T13:00:00+00:00#3",
                "2025-01-09T14:00:00+00:00/2025-01-09T17:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: subsequent instances defined in separate components are not impacted by the given recurrence instance"
        );
    }

    #[test]
    fn rfc5545_3_6_1_an_override_with_dtstart_but_no_end_is_zero_length() {
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=America/New_York:20060102T120000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\n",
                "RECURRENCE-ID;TZID=America/New_York:20060103T120000\r\nDTSTART;TZID=America/New_York:20060103T150000\r\n",
            ]),
            [
                "2006-01-02T12:00:00-05:00/2006-01-02T13:00:00-05:00#1",
                "2006-01-03T15:00:00-05:00/2006-01-03T15:00:00-05:00#2",
            ],
            "RFC 5545 Section 3.6.1 applies to every VEVENT, and an override does not inherit from its series"
        );
    }

    #[test]
    fn an_override_past_the_expansion_limit_keeps_the_series_length() {
        let ical = calendar([
            "UID:limit\r\nDTSTART:20250106T090000Z\r\nDURATION:PT2H\r\nRRULE:FREQ=DAILY;COUNT=10\r\n",
            "UID:limit\r\nRECURRENCE-ID:20250110T090000Z\r\nSUMMARY:Late\r\n",
        ]);
        let expanded = ical.expand_dates(Tz::UTC, 2);
        assert_eq!(expanded.events.len(), 3);
        assert_eq!(
            expanded
                .events
                .iter()
                .filter(|event| event.comp_id == 2)
                .map(|event| format!("{}/{}", event.start, event.end))
                .collect::<Vec<_>>(),
            ["2025-01-10T09:00:00+00:00/2025-01-10T11:00:00+00:00"]
        );
    }

    fn override_instances(ical: &ICalendar, limit: usize) -> Vec<String> {
        let mut instances = ical
            .expand_dates(Tz::UTC, limit)
            .events
            .into_iter()
            .filter(|event| event.comp_id == 2)
            .map(|event| format!("{}/{}", event.start, event.end))
            .collect::<Vec<_>>();
        instances.sort();
        instances
    }

    #[test]
    fn an_override_past_the_expansion_limit_starts_like_the_instance_it_replaces() {
        for (series, recurrence_id) in [
            (
                "UID:limit\r\nDTSTART;TZID=America/New_York:20250106T090000\r\nDURATION:PT2H\r\nRRULE:FREQ=DAILY;COUNT=10\r\n",
                "UID:limit\r\nRECURRENCE-ID:20250110T090000\r\n",
            ),
            (
                "UID:limit\r\nDTSTART;TZID=America/New_York:20250106T090000\r\nDURATION:PT2H\r\nRRULE:FREQ=DAILY;COUNT=10\r\n",
                "UID:limit\r\nRECURRENCE-ID:20250110T140000Z\r\n",
            ),
            (
                "UID:limit\r\nDTSTART;TZID=America/New_York:20250328T200000\r\nDURATION:P1D\r\nRRULE:FREQ=DAILY;COUNT=5\r\n",
                "UID:limit\r\nRECURRENCE-ID;TZID=Europe/Berlin:20250330T010000\r\n",
            ),
            (
                "DTSTART:20250106T090000Z\r\nDURATION:PT2H\r\nRRULE:FREQ=DAILY;COUNT=10\r\n",
                "RECURRENCE-ID:20250110T090000Z\r\n",
            ),
        ] {
            let ical = calendar([series, &format!("{recurrence_id}SUMMARY:Late\r\n")]);
            let matched = override_instances(&ical, 100);
            assert_eq!(matched.len(), 1, "{recurrence_id}");
            assert_eq!(
                override_instances(&ical, 1),
                matched,
                "the expansion limit does not change the occurrence: {recurrence_id}"
            );
        }
    }

    #[test]
    fn rfc5545_3_8_4_4_an_override_without_dtstart_keeps_an_end_that_names_an_instant() {
        for (dtend, end) in [
            (
                "DTEND;TZID=America/New_York:20060103T140000",
                "2006-01-03T14:00:00-05:00",
            ),
            ("DTEND:20060103T190000Z", "2006-01-03T19:00:00+00:00"),
            ("DTEND:20060103T140000", "2006-01-03T14:00:00-05:00"),
        ] {
            assert_eq!(
                expanded(&[
                    "DTSTART;TZID=America/New_York:20060102T120000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                    &format!("RECURRENCE-ID:20060103T120000\r\n{dtend}\r\n"),
                ]),
                [
                    "2006-01-02T12:00:00-05:00/2006-01-02T13:00:00-05:00#1".to_string(),
                    format!("2006-01-03T12:00:00-05:00/{end}#2"),
                    "2006-01-04T12:00:00-05:00/2006-01-04T13:00:00-05:00#1".to_string(),
                ],
                "{dtend}"
            );
        }
        assert_eq!(
            expanded(&[
                "DTSTART;TZID=America/New_York:20060102T120000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\n",
                "RECURRENCE-ID;RANGE=THISANDFUTURE:20060103T120000\r\nDTEND;TZID=America/New_York:20060103T140000\r\n",
            ]),
            [
                "2006-01-02T12:00:00-05:00/2006-01-02T13:00:00-05:00#1",
                "2006-01-03T12:00:00-05:00/2006-01-03T14:00:00-05:00#2",
                "2006-01-04T12:00:00-05:00/2006-01-04T14:00:00-05:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: the subsequent instances take the length of the given one"
        );

        let ical = ICalendar::parse(concat!(
            "BEGIN:VCALENDAR\r\n",
            "BEGIN:VTODO\r\nUID:todo\r\nDTSTART;TZID=America/New_York:20060102T120000\r\n",
            "DUE;TZID=America/New_York:20060102T130000\r\nRRULE:FREQ=DAILY;COUNT=2\r\nEND:VTODO\r\n",
            "BEGIN:VTODO\r\nUID:todo\r\nRECURRENCE-ID:20060103T120000\r\nDUE:20060103T190000Z\r\nEND:VTODO\r\n",
            "END:VCALENDAR\r\n"
        ))
        .unwrap();
        assert_eq!(
            override_instances(&ical, 10),
            ["2006-01-03T12:00:00-05:00/2006-01-03T19:00:00+00:00"]
        );
    }

    #[test]
    fn overrides_only_replace_instances_of_the_same_component_type() {
        let ical = ICalendar::parse(concat!(
            "BEGIN:VCALENDAR\r\n",
            "BEGIN:VTODO\r\nUID:x\r\nDTSTART:20250106T090000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VTODO\r\n",
            "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20250106T090000Z\r\nDURATION:PT2H\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:x\r\nRECURRENCE-ID:20250107T090000Z\r\nSUMMARY:Moved\r\nEND:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ))
        .unwrap();
        let mut instances = ical
            .expand_dates(Tz::UTC, 10)
            .events
            .into_iter()
            .map(|event| format!("{}/{}#{}", event.start, event.end, event.comp_id))
            .collect::<Vec<_>>();
        instances.sort();
        assert_eq!(
            instances,
            [
                "2025-01-06T09:00:00+00:00/2025-01-06T09:00:00+00:00#1",
                "2025-01-06T09:00:00+00:00/2025-01-06T11:00:00+00:00#2",
                "2025-01-07T09:00:00+00:00/2025-01-07T09:00:00+00:00#1",
                "2025-01-07T09:00:00+00:00/2025-01-07T11:00:00+00:00#3",
                "2025-01-08T09:00:00+00:00/2025-01-08T09:00:00+00:00#1",
                "2025-01-08T09:00:00+00:00/2025-01-08T11:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: RECURRENCE-ID identifies an instance of a VEVENT, VTODO or VJOURNAL with the same UID"
        );
    }

    #[test]
    fn an_override_of_another_value_type_keeps_its_own_dates() {
        assert_eq!(
            expanded(&[
                "DTSTART:20060102T000000\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=2\r\n",
                "RECURRENCE-ID;VALUE=DATE:20060103\r\nSUMMARY:Renamed\r\n",
            ]),
            [
                "2006-01-02T00:00:00+00:00/2006-01-02T01:00:00+00:00#1",
                "2006-01-03T00:00:00+00:00/2006-01-04T00:00:00+00:00#2",
            ],
            "RFC 5545 Section 3.8.4.4: RECURRENCE-ID MUST have the same value type as DTSTART"
        );
    }

    #[test]
    fn expansion_stops_pulling_instances_once_the_limit_is_reached() {
        let ical = ICalendar::parse(concat!(
            "BEGIN:VCALENDAR\r\n",
            "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=5\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1S\r\nRRULE:FREQ=SECONDLY;BYMONTH=2;BYMONTHDAY=31\r\nEND:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ))
        .unwrap();

        let limited = ical.expand_dates(Tz::UTC, 3);
        assert_eq!(limited.events.len(), 3);
        assert_eq!(limited.errors, []);

        let unlimited = ical.expand_dates(Tz::UTC, 100);
        assert_eq!(unlimited.events.len(), 5);
        assert_eq!(
            unlimited.errors,
            [CalendarError {
                comp_id: 2,
                error: CalendarErrorType::ExpansionLimitReached,
            }]
        );
    }

    #[test]
    fn rules_that_match_nothing_share_one_budget() {
        let impossible_rules = MAX_UNPRODUCTIVE_WORK / MAX_ITER_LOOP as usize + 1;
        let impossible = (0..impossible_rules).map(|uid| {
            format!(
                "UID:{uid}\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;BYMONTH=2;BYMONTHDAY=31\r\n"
            )
        });
        let ical = calendar(impossible.chain([
            "UID:series\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\nRDATE:20250110T090000Z\r\n".into(),
            "UID:single\r\nDTSTART:20250107T120000Z\r\nDURATION:PT1H\r\n".into(),
        ]));
        let series = impossible_rules as u32 + 1;

        let expanded = ical.expand_dates(Tz::UTC, 3000);
        let limited = (1..=series)
            .map(|comp_id| CalendarError {
                comp_id,
                error: CalendarErrorType::ExpansionLimitReached,
            })
            .collect::<Vec<_>>();
        assert_eq!(expanded.errors, limited);
        let mut events = expanded
            .events
            .iter()
            .map(|event| format!("{}#{}", event.start, event.comp_id))
            .collect::<Vec<_>>();
        events.sort();
        assert_eq!(
            events,
            [
                format!("2025-01-07T12:00:00+00:00#{}", series + 1),
                format!("2025-01-10T09:00:00+00:00#{series}"),
            ],
            "the series after the spent budget keeps its RDATE, and events without a rule cost nothing"
        );
    }

    #[test]
    fn a_shared_budget_leaves_productive_calendars_alone() {
        let weekly = (0..200).map(|uid| {
            format!(
                "UID:{uid}\r\nDTSTART:20250106T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=4\r\n"
            )
        });
        let ical = calendar(
            std::iter::once(
                "UID:sparse\r\nDTSTART:20250106T000000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=SECONDLY;BYHOUR=0;BYMINUTE=0;BYSECOND=0;COUNT=2000\r\n".to_string(),
            )
            .chain(weekly),
        );

        let expanded = ical.expand_dates(Tz::UTC, 3000);
        assert_eq!(expanded.errors, []);
        assert_eq!(expanded.events.len(), 2000 + 200 * 4);
    }

    #[test]
    fn rfc5545_3_3_14_offsets_of_a_day_or_more_are_rejected() {
        for offset in ["-2400", "+2400", "+2500"] {
            let expanded = ICalendar::parse(format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20250101T090000{offset}\r\nDURATION:PT1H\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            ))
            .unwrap()
            .expand_dates(Tz::UTC, 10);
            assert_eq!(expanded.events, [], "{offset}");
            assert_eq!(expanded.errors.len(), 1, "{offset}");
        }
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
                    events: Vec<CalendarEvent>,
                }

                print!("Expanding recurrences for {file_name}... ");
                let now = Instant::now();
                loop {
                    match parser.entry() {
                        Entry::ICalendar(ical) => {
                            let expanded = ical.expand_dates(auckland(), 100);
                            let mut events = expanded.events;
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
