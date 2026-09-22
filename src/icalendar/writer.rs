/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use super::{
    ICalendar, ICalendarDay, ICalendarDuration, ICalendarEntry, ICalendarPeriod,
    ICalendarRecurrenceRule, ICalendarValueType,
};
use crate::{
    common::{
        CalendarScale, IanaString, PartialDateTime,
        writer::{
            FoldingWriter, LineWriter, write_bytes, write_param_text, write_param_value,
            write_quoted_param_value, write_text, write_uri,
        },
    },
    icalendar::{
        ICalendarMonth, ICalendarParameterName, ICalendarParameterValue, ICalendarValue, Uri,
        ValueSeparator,
    },
};
use std::{
    fmt::{Display, Write},
    slice::Iter,
};

impl ICalendar {
    pub fn write_to(&self, out: &mut impl Write) -> std::fmt::Result {
        let mut component_iter: Iter<'_, u32> = [0].iter();
        let mut component_stack = Vec::with_capacity(4);
        let mut visited = vec![false; self.components.len()];

        loop {
            if let Some(component_id) = component_iter.next() {
                let Some((component, visited)) = self
                    .components
                    .get(*component_id as usize)
                    .zip(visited.get_mut(*component_id as usize))
                    .filter(|(_, visited)| !**visited)
                else {
                    continue;
                };
                *visited = true;
                write_boundary(out, "BEGIN:", component.component_type.as_str())?;

                for entry in &component.entries {
                    entry.write_to(out)?;
                }

                if !component.component_ids.is_empty() {
                    component_stack.push((component, component_iter));
                    component_iter = component.component_ids.iter();
                } else {
                    write_boundary(out, "END:", component.component_type.as_str())?;
                }
            } else if let Some((component, iter)) = component_stack.pop() {
                write_boundary(out, "END:", component.component_type.as_str())?;
                component_iter = iter;
            } else {
                break;
            }
        }

        Ok(())
    }
}

impl ICalendarEntry {
    pub fn write_to(&self, out: &mut impl Write) -> std::fmt::Result {
        self.write_with_value(out, true)
    }

    pub fn write_with_value(&self, out: &mut impl Write, with_value: bool) -> std::fmt::Result {
        let mut folded = FoldingWriter::new(out);
        let out = &mut folded;

        out.write_atomic(self.name.as_str())?;

        if matches!(self.values.first(), Some(ICalendarValue::Binary(_))) {
            out.write_atomic(";ENCODING=BASE64")?;
            if !self
                .params
                .iter()
                .any(|param| param.name == ICalendarParameterName::Value)
            {
                out.write_atomic(";VALUE=BINARY")?;
            }
        }

        let mut types = None;
        let mut last_param: Option<&ICalendarParameterName> = None;

        for param in &self.params {
            if last_param.is_some_and(|last_param| last_param == &param.name) {
                out.write_atomic(",")?;
            } else {
                out.write_atomic(";")?;
                out.write_atomic(param.name.as_str())?;
                if !matches!(param.value, ICalendarParameterValue::Null) {
                    out.write_atomic("=")?;
                }
                last_param = Some(&param.name);
            }

            match &param.value {
                ICalendarParameterValue::Text(v)
                    if matches!(param.name, ICalendarParameterName::Jsptr) =>
                {
                    write_quoted_param_value(out, v, true)?;
                }
                ICalendarParameterValue::Text(v) => {
                    write_param_value(out, v, true)?;
                }
                ICalendarParameterValue::Integer(i) => {
                    write!(out, "{i}")?;
                }
                ICalendarParameterValue::Bool(v) => {
                    let v = if !matches!(param.name, ICalendarParameterName::Range) {
                        if *v { "TRUE" } else { "FALSE" }
                    } else {
                        "THISANDFUTURE"
                    };
                    out.write_atomic(v)?;
                }
                ICalendarParameterValue::Uri(uri) => {
                    out.write_atomic("\"")?;
                    uri.write_param_text(out)?;
                    out.write_atomic("\"")?;
                }
                ICalendarParameterValue::Cutype(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Fbtype(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Partstat(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Related(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Reltype(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Role(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::ScheduleAgent(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::ScheduleForceSend(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Value(v) => {
                    types = Some(v);
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Display(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Feature(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Duration(v) => {
                    write!(out, "{v}")?;
                }
                ICalendarParameterValue::Linkrel(v) => {
                    write_param_value(out, v.as_str(), true)?;
                }
                ICalendarParameterValue::Null => {
                    last_param = None;
                }
            }
        }

        out.write_atomic(":")?;

        if !with_value {
            return out.end_line();
        }

        let (default_type, separator) = self.name.default_types();
        let separator = if !matches!(separator, ValueSeparator::Comma) {
            ";"
        } else {
            ","
        };
        let default_type = default_type.unwrap_ical();

        for (pos, value) in self.values.iter().enumerate() {
            if pos > 0 {
                out.write_atomic(separator)?;
            }

            let text = match value {
                ICalendarValue::Binary(v) => {
                    write_bytes(out, v)?;
                    continue;
                }
                ICalendarValue::Boolean(v) => {
                    out.write_atomic(if *v { "TRUE" } else { "FALSE" })?;
                    continue;
                }
                ICalendarValue::Uri(v) => {
                    v.write_value(out)?;
                    continue;
                }
                ICalendarValue::PartialDateTime(v) => {
                    v.format_as_ical(out, types.unwrap_or(&default_type))?;
                    continue;
                }
                ICalendarValue::Duration(v) => {
                    write!(out, "{}", v)?;
                    continue;
                }
                ICalendarValue::RecurrenceRule(v) => {
                    write!(out, "{}", v)?;
                    continue;
                }
                ICalendarValue::Period(v) => {
                    write!(out, "{}", v)?;
                    continue;
                }
                ICalendarValue::Float(v) => {
                    write!(out, "{v}")?;
                    continue;
                }
                ICalendarValue::Integer(v) => {
                    write!(out, "{v}")?;
                    continue;
                }
                ICalendarValue::Text(v) => {
                    match types.unwrap_or(&default_type) {
                        ICalendarValueType::Uri | ICalendarValueType::CalAddress => {
                            write_uri(out, v)?;
                        }
                        ICalendarValueType::Recur => write_text(out, v, false, false)?,
                        _ => write_text(out, v, true, true)?,
                    }
                    continue;
                }
                ICalendarValue::CalendarScale(v) => v.as_str(),
                ICalendarValue::Method(v) => v.as_str(),
                ICalendarValue::Classification(v) => v.as_str(),
                ICalendarValue::Status(v) => v.as_str(),
                ICalendarValue::Transparency(v) => v.as_str(),
                ICalendarValue::Action(v) => v.as_str(),
                ICalendarValue::BusyType(v) => v.as_str(),
                ICalendarValue::ParticipantType(v) => v.as_str(),
                ICalendarValue::ResourceType(v) => v.as_str(),
                ICalendarValue::Proximity(v) => v.as_str(),
            };

            out.write_atomic(text)?;
        }

        out.end_line()
    }
}

impl Uri {
    fn write_value(&self, out: &mut impl LineWriter) -> std::fmt::Result {
        match self {
            Uri::Data(v) => {
                out.write_str("data:")?;
                out.write_str(v.content_type.as_deref().unwrap_or_default())?;
                out.write_str(";")?;
                out.write_atomic("base64,")?;
                write_bytes(out, &v.data)
            }
            Uri::Location(v) => write_uri(out, v),
        }
    }

    fn write_param_text(&self, out: &mut impl LineWriter) -> std::fmt::Result {
        match self {
            Uri::Data(v) => {
                out.write_str("data:")?;
                write_param_text(out, v.content_type.as_deref().unwrap_or_default(), true)?;
                out.write_str(";")?;
                out.write_atomic("base64,")?;
                write_bytes(out, &v.data)
            }
            Uri::Location(v) => write_param_text(out, v, true),
        }
    }
}

fn write_boundary(out: &mut impl Write, keyword: &str, name: &str) -> std::fmt::Result {
    let mut folded = FoldingWriter::new(out);
    folded.write_atomic(keyword)?;
    folded.write_atomic(name)?;
    folded.end_line()
}

#[cfg(feature = "rkyv")]
pub(crate) fn write_component_begin(out: &mut impl Write, name: &str) -> std::fmt::Result {
    write_boundary(out, "BEGIN:", name)
}

#[cfg(feature = "rkyv")]
pub(crate) fn write_component_end(out: &mut impl Write, name: &str) -> std::fmt::Result {
    write_boundary(out, "END:", name)
}

impl Uri {
    pub fn to_unwrapped_string(&self) -> String {
        match self {
            Uri::Data(v) => v.to_unwrapped_string(),
            Uri::Location(v) => v.to_string(),
        }
    }

    pub fn into_unwrapped_string(self) -> String {
        match self {
            Uri::Data(v) => v.to_unwrapped_string(),
            Uri::Location(v) => v,
        }
    }
}

impl Display for ICalendarRecurrenceRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FREQ={}", self.freq.as_str())?;
        if let Some(until) = &self.until {
            write!(f, ";UNTIL=")?;
            until.format_as_ical(
                f,
                if until.has_date_and_time() {
                    &ICalendarValueType::DateTime
                } else {
                    &ICalendarValueType::Date
                },
            )?;
        }
        if let Some(count) = self.count.filter(|c| *c > 0) {
            write!(f, ";COUNT={}", count)?;
        }
        if let Some(interval) = self.interval {
            write!(f, ";INTERVAL={}", interval)?;
        }
        if !self.bysecond.is_empty() {
            write!(f, ";BYSECOND=")?;
            for (pos, item) in self.bysecond.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.byminute.is_empty() {
            write!(f, ";BYMINUTE=")?;
            for (pos, item) in self.byminute.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.byhour.is_empty() {
            write!(f, ";BYHOUR=")?;
            for (pos, item) in self.byhour.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.byday.is_empty() {
            write!(f, ";BYDAY=")?;
            for (pos, item) in self.byday.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.bymonthday.is_empty() {
            write!(f, ";BYMONTHDAY=")?;
            for (pos, item) in self.bymonthday.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.byyearday.is_empty() {
            write!(f, ";BYYEARDAY=")?;
            for (pos, item) in self.byyearday.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.byweekno.is_empty() {
            write!(f, ";BYWEEKNO=")?;
            for (pos, item) in self.byweekno.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.bymonth.is_empty() {
            write!(f, ";BYMONTH=")?;
            for (pos, item) in self.bymonth.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if !self.bysetpos.is_empty() {
            write!(f, ";BYSETPOS=")?;
            for (pos, item) in self.bysetpos.iter().enumerate() {
                if pos > 0 {
                    write!(f, ",")?;
                }
                write!(f, "{}", item)?;
            }
        }
        if let Some(wkst) = self.wkst {
            write!(f, ";WKST={}", wkst.as_str())?;
        }
        if let Some(rscale) = &self.rscale {
            write!(f, ";RSCALE={}", rscale.as_str())?;
        } else if self.skip.is_some() {
            write!(f, ";RSCALE={}", CalendarScale::Gregorian.as_str())?;
        }
        if let Some(skip) = &self.skip {
            write!(f, ";SKIP={}", skip.as_str())?;
        }

        Ok(())
    }
}

impl Display for ICalendarDay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ordwk) = self.ordwk {
            write!(f, "{}", ordwk)?;
        }
        write!(f, "{}", self.weekday.as_str())
    }
}

impl Display for ICalendarMonth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.is_leap() {
            write!(f, "{}", self.month())
        } else {
            write!(f, "{}L", self.month())
        }
    }
}

impl Display for ICalendarPeriod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ICalendarPeriod::Range { start, end } => {
                start.format_as_ical(f, &ICalendarValueType::DateTime)?;
                write!(f, "/")?;
                end.format_as_ical(f, &ICalendarValueType::DateTime)
            }
            ICalendarPeriod::Duration { start, duration } => {
                start.format_as_ical(f, &ICalendarValueType::DateTime)?;
                write!(f, "/{}", duration)
            }
        }
    }
}

impl Display for ICalendarDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.neg {
            write!(f, "-")?;
        }
        write!(f, "P")?;
        if self.is_empty() {
            return write!(f, "T0S");
        }
        if self.weeks != 0 {
            write!(f, "{}W", self.weeks)?;
        }
        if self.days != 0 {
            write!(f, "{}D", self.days)?;
        }
        if self.hours != 0 || self.minutes != 0 || self.seconds != 0 {
            write!(f, "T")?;
            if self.hours != 0 {
                write!(f, "{}H", self.hours)?;
            }
            if self.minutes != 0 {
                write!(f, "{}M", self.minutes)?;
            }
            if self.seconds != 0 {
                write!(f, "{}S", self.seconds)?;
            }
        }

        Ok(())
    }
}

impl PartialDateTime {
    pub fn format_as_ical(
        &self,
        out: &mut impl Write,
        fmt: &ICalendarValueType,
    ) -> std::fmt::Result {
        if matches!(fmt, ICalendarValueType::Date | ICalendarValueType::DateTime) {
            write!(
                out,
                "{:04}{:02}{:02}",
                self.year.unwrap_or_default(),
                self.month.unwrap_or_default(),
                self.day.unwrap_or_default()
            )?;
        }

        if matches!(fmt, ICalendarValueType::DateTime) {
            write!(out, "T")?;
        }

        if matches!(fmt, ICalendarValueType::DateTime | ICalendarValueType::Time) {
            write!(
                out,
                "{:02}{:02}{:02}",
                self.hour.unwrap_or_default(),
                self.minute.unwrap_or_default(),
                self.second.unwrap_or_default()
            )?;

            if matches!((self.tz_hour, self.tz_minute), (Some(0), Some(0))) {
                write!(out, "Z")?;
            }
        }

        if matches!(fmt, ICalendarValueType::UtcOffset) {
            if self.tz_minus {
                write!(out, "-")?;
            } else {
                write!(out, "+")?;
            }

            write!(
                out,
                "{:02}{:02}",
                self.tz_hour.unwrap_or_default(),
                self.tz_minute.unwrap_or_default(),
            )?;
        }

        Ok(())
    }
}

impl Display for ICalendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write_to(f)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Entry, Parser,
        icalendar::{
            ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry,
            ICalendarParameterName, ICalendarProperty, ICalendarValue,
        },
    };

    fn parse(input: &str) -> ICalendar {
        let mut parser = Parser::new(input);
        let Entry::ICalendar(ical) = parser.entry() else {
            panic!("expected iCalendar for {input}");
        };
        ical
    }

    fn write(ical: &ICalendar) -> String {
        let out = ical.to_string();

        #[cfg(feature = "rkyv")]
        {
            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(ical).unwrap();
            let archived =
                rkyv::access::<crate::icalendar::ArchivedICalendar, rkyv::rancor::Error>(&bytes)
                    .unwrap();
            assert_eq!(out, archived.to_string(), "archived writer diverged");
        }

        out
    }

    #[test]
    fn test_write_fold_width() {
        for input in [
            "RRULE:FREQ=WEEKLY;UNTIL=20210309T080000Z;INTERVAL=1;BYDAY=MO,TU,WE,TH,FR,SA;WKST=MO",
            "RDATE;VALUE=PERIOD:19970101T180000Z/19970102T070000Z,19970109T180000Z/PT5H30M",
            "FREEBUSY;FBTYPE=BUSY:20120103T091500Z/20120103T101500Z,20120113T130000Z/20120113T150000Z",
            "EXDATE;TZID=US/Central:20170706T090000,20170713T090000,20170720T090000,20170803T090000",
            "DTSTART;TZID=/softwarestudio.org/Olson_20011030_5/America/Chicago:20030515T183000",
            "REQUEST-STATUS:EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE;;",
        ] {
            let ical = parse(&format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\n\
                 BEGIN:VEVENT\r\nUID:1\r\n{input}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            ));

            let out = write(&ical);
            crate::common::writer::assert_fold_width(&out, input);

            assert_eq!(write(&parse(&out)), out, "not idempotent for {input}");
        }
    }

    #[test]
    fn test_write_component_boundary_fold_width() {
        let name = "X".repeat(90);
        let ical = parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//x//EN\r\n\
             BEGIN:{name}\r\nEND:{name}\r\nEND:VCALENDAR\r\n"
        ));

        let out = write(&ical);
        crate::common::writer::assert_fold_width(&out, &name);

        assert_eq!(write(&parse(&out)), out, "not idempotent for {name}");
    }

    fn component(component_type: ICalendarComponentType, ids: &[u32]) -> ICalendarComponent {
        ICalendarComponent {
            component_type,
            entries: Vec::new(),
            component_ids: ids.to_vec(),
        }
    }

    #[test]
    fn test_write_empty_icalendar() {
        let ical = ICalendar::default();

        assert_eq!(write(&ical), "");

        let mut out = String::new();
        assert!(ical.write_to(&mut out).is_ok());
        assert_eq!(out, "");
    }

    #[test]
    fn test_write_skips_missing_component_ids() {
        let ical = ICalendar {
            components: vec![
                component(ICalendarComponentType::VCalendar, &[7, 1]),
                component(ICalendarComponentType::VEvent, &[u32::MAX]),
            ],
        };

        assert_eq!(
            write(&ical),
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
    }

    fn event(lines: &str) -> ICalendar {
        parse(&format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:writer\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n"
        ))
    }

    #[test]
    fn rfc5545_3_3_13_uri_values_are_written_verbatim() {
        let ical = event(concat!(
            "ATTACH:data:text/plain;base64,aGVsbG8=\r\n",
            "URL:https://example.com/a,b;c\r\n",
            "ORGANIZER:mailto:a,b;c@example.com\r\n",
            "BEGIN:VLOCATION\r\nCOORDINATES;VALUE=URI:geo:48.198634,16.371648;crs=wgs84;u=40\r\nEND:VLOCATION\r\n",
        ));
        let out = write(&ical);

        for line in [
            "ATTACH:data:text/plain;base64,aGVsbG8=\r\n",
            "URL:https://example.com/a,b;c\r\n",
            "ORGANIZER:mailto:a,b;c@example.com\r\n",
            "COORDINATES;VALUE=URI:geo:48.198634,16.371648;crs=wgs84;u=40\r\n",
        ] {
            assert!(
                out.contains(line),
                "RFC 5545 Section 3.3.13: no escaping is defined for URI values, missing {line:?}\n{out}"
            );
        }
        assert_eq!(parse(&out), ical, "{out}");
    }

    #[test]
    fn rfc5545_3_3_13_uri_values_keep_their_backslashes() {
        for (line, values) in [
            ("CATEGORIES;VALUE=URI:a\\,b\r\n", 2),
            ("RESOURCES;VALUE=URI:a\\,b\r\n", 2),
            ("X-FOO;VALUE=URI:a\\;b\r\n", 2),
            ("CATEGORIES;VALUE=URI:data:text/plain;base64\\,aGk=\r\n", 2),
            ("CATEGORIES;VALUE=URI:a,b\r\n", 2),
            ("CATEGORIES:a\\,b\r\n", 1),
            ("ORGANIZER:mailto:a\\,b@example.com\r\n", 1),
        ] {
            let ical = event(line);
            let entry = &ical.components[1].entries[1];
            assert_eq!(entry.values.len(), values, "{line:?}: {ical}");

            let out = write(&ical);
            assert!(
                out.contains(line),
                "RFC 5545 Section 3.3.13: a BACKSLASH is not an escape in a URI value, missing {line:?}\n{out}"
            );
            assert_eq!(parse(&out), ical, "{out}");
        }
    }

    #[test]
    fn rfc6868_parameter_values_use_caret_encoding() {
        let ical = event(
            "ATTENDEE;CN=\"Jane ^'JJ^' Doe^nSales ^^ Marketing\";X-TOKEN=a^b:mailto:jane@example.com\r\n",
        );
        let attendee = ical
            .components
            .iter()
            .flat_map(|component| component.entries.iter())
            .find(|entry| entry.name == ICalendarProperty::Attendee)
            .expect("attendee");
        assert_eq!(
            attendee
                .parameter(&ICalendarParameterName::Cn)
                .and_then(|value| value.as_text()),
            Some("Jane \"JJ\" Doe\nSales ^ Marketing"),
            "RFC 6868 Section 3.2: ^' decodes to a quote, ^n to a line break and ^^ to ^"
        );
        assert_eq!(
            attendee
                .parameter(&ICalendarParameterName::Other("X-TOKEN".into()))
                .and_then(|value| value.as_text()),
            Some("a^b"),
            "RFC 6868 Section 3.2: other characters after ^ are left in place"
        );

        let out = write(&ical);
        assert!(
            out.contains("CN=\"Jane ^'JJ^' Doe^nSales ^^ Marketing\""),
            "RFC 6868 Section 3.1\n{out}"
        );
        assert!(out.contains("X-TOKEN=a^^b"), "RFC 6868 Section 3.1\n{out}");
        assert_eq!(parse(&out), ical, "{out}");

        let folded = event("ATTENDEE;CN=\"a^\r\n 'b^\r\n\tn\":mailto:a@example.com\r\n");
        assert!(
            write(&folded).contains("CN=\"a^'b^n\""),
            "caret sequences survive line folding"
        );

        let bare_cr = event("ATTENDEE;X-TOKEN=a^\rn:mailto:a@example.com\r\n");
        assert_eq!(
            bare_cr
                .components
                .iter()
                .flat_map(|component| component.entries.iter())
                .find(|entry| entry.name == ICalendarProperty::Attendee)
                .and_then(|entry| entry
                    .parameter(&ICalendarParameterName::Other("X-TOKEN".into()))
                    .and_then(|value| value.as_text())),
            Some("a^\rn"),
            "RFC 6868 Section 3: a bare CR is not a line fold, so ^ and the following character stay in place"
        );
    }

    #[test]
    fn jscalendar_icalendar_4_2_2_jsptr_is_always_quoted() {
        let out = write(&event("JSPROP;JSPTR=title:\"Meeting\"\r\n"));
        assert!(
            out.contains("JSPROP;JSPTR=\"title\":"),
            "draft-ietf-calext-jscalendar-icalendar-28 Section 4.2.2: the parameter value MUST be quoted\n{out}"
        );
    }

    #[test]
    fn rfc5545_3_2_7_base64_attach_without_value_binary_stays_binary() {
        let ical = event(concat!(
            "ATTACH;ENCODING=BASE64;FMTTYPE=text/plain:aGVsbG8gd29ybGQ=\r\n",
            "IMAGE;ENCODING=BASE64;FMTTYPE=image/png:aGVsbG8=\r\n",
            "X-TEXT;ENCODING=BASE64:aGVsbG8=\r\n",
        ));
        let values = ical
            .components
            .iter()
            .flat_map(|component| component.entries.iter())
            .filter_map(|entry| Some((entry.name.as_str(), entry.values.first()?)))
            .collect::<Vec<_>>();
        assert!(
            values.contains(&("ATTACH", &ICalendarValue::Binary(b"hello world".to_vec()))),
            "RFC 5545 Section 3.2.7: ENCODING=BASE64 applies to BINARY values\n{values:?}"
        );
        assert!(
            values.contains(&("IMAGE", &ICalendarValue::Binary(b"hello".to_vec()))),
            "{values:?}"
        );
        assert!(
            values.contains(&("X-TEXT", &ICalendarValue::Text("hello".into()))),
            "{values:?}"
        );

        let out = write(&ical);
        assert!(
            out.contains(
                "ATTACH;ENCODING=BASE64;FMTTYPE=text/plain;VALUE=BINARY:aGVsbG8gd29ybGQ=\r\n"
            ),
            "RFC 5545 Sections 3.2.7 and 3.8.1.1: inline binary is marked with VALUE=BINARY\n{out}"
        );
        assert!(
            out.contains("IMAGE;ENCODING=BASE64;FMTTYPE=image/png;VALUE=BINARY:aGVsbG8=\r\n"),
            "{out}"
        );
        assert_eq!(parse(&out), ical, "{out}");
        assert_eq!(out.matches("VALUE=BINARY").count(), 2, "{out}");

        // A binary value that was built rather than parsed is marked too.
        let mut built = ICalendar::default();
        built.components.push(ICalendarComponent {
            component_type: ICalendarComponentType::VCalendar,
            entries: vec![
                ICalendarEntry::new(ICalendarProperty::Attach)
                    .with_value(ICalendarValue::Binary(b"hello".to_vec())),
            ],
            component_ids: vec![],
        });
        let out = write(&built);
        assert!(
            out.contains("ATTACH;ENCODING=BASE64;VALUE=BINARY:aGVsbG8=\r\n"),
            "{out}"
        );
    }

    #[test]
    fn test_write_cyclic_component_ids_terminates() {
        let ical = ICalendar {
            components: vec![
                component(ICalendarComponentType::VCalendar, &[1]),
                component(ICalendarComponentType::VEvent, &[0]),
            ],
        };

        assert_eq!(
            write(&ical),
            concat!(
                "BEGIN:VCALENDAR\r\n",
                "BEGIN:VEVENT\r\n",
                "END:VEVENT\r\n",
                "END:VCALENDAR\r\n"
            )
        );
    }
}
