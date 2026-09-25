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
        format::{AsciiPush, BufferedWrite, DurationParts},
        stack::ComponentStack,
        writer::{
            DOCUMENT_BUFFER, ENTRY_BUFFER, FoldingWriter, LineWriter, VisitedComponents,
            write_param_text, write_param_value, write_quoted_param_value, write_text, write_uri,
        },
    },
    icalendar::{
        ICalendarMonth, ICalendarParameterName, ICalendarParameterValue, ICalendarProperty,
        ICalendarValue, Uri, ValueSeparator,
    },
};
use std::{
    fmt::{self, Display, Write},
    slice::Iter,
};

pub(crate) const RRULE_BUFFER: usize = 256;
pub(crate) const DURATION_BUFFER: usize = 64;
pub(crate) const DATE_BUFFER: usize = 32;
pub(crate) const PERIOD_BUFFER: usize = 96;

impl ICalendar {
    pub fn write_to(&self, out: &mut impl Write) -> fmt::Result {
        out.write_buffered::<DOCUMENT_BUFFER>(|buf| {
            self.write_components(&mut FoldingWriter::new(buf))
        })
    }

    fn write_components<W: Write + AsciiPush + ?Sized>(
        &self,
        out: &mut FoldingWriter<'_, W>,
    ) -> fmt::Result {
        let mut component_iter: Iter<'_, u32> = [0].iter();
        let mut component_stack = ComponentStack::new();
        let mut visited = VisitedComponents::new(self.components.len());

        loop {
            if let Some(component_id) = component_iter.next() {
                let component_id = *component_id as usize;
                let Some(component) = self
                    .components
                    .get(component_id)
                    .filter(|_| visited.insert(component_id))
                else {
                    continue;
                };
                out.write_boundary("BEGIN:", component.component_type.as_str())?;

                for entry in &component.entries {
                    if !matches!(
                        entry.name,
                        ICalendarProperty::Begin | ICalendarProperty::End
                    ) {
                        entry.write_line(out, true)?;
                    }
                }

                if !component.component_ids.is_empty() {
                    component_stack.push((component, component_iter));
                    component_iter = component.component_ids.iter();
                } else {
                    out.write_boundary("END:", component.component_type.as_str())?;
                }
            } else if let Some((component, iter)) = component_stack.pop() {
                out.write_boundary("END:", component.component_type.as_str())?;
                component_iter = iter;
            } else {
                break;
            }
        }

        Ok(())
    }
}

impl ICalendarEntry {
    pub fn write_to(&self, out: &mut impl Write) -> fmt::Result {
        self.write_with_value(out, true)
    }

    pub fn write_with_value(&self, out: &mut impl Write, with_value: bool) -> fmt::Result {
        out.write_buffered::<ENTRY_BUFFER>(|buf| {
            self.write_line(&mut FoldingWriter::new(buf), with_value)
        })
    }

    pub(crate) fn write_line<W: Write + AsciiPush + ?Sized>(
        &self,
        out: &mut FoldingWriter<'_, W>,
        with_value: bool,
    ) -> fmt::Result {
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
                    out.push_u64(*i)?;
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
                    out.push_duration(v.parts())?;
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
                    out.write_base64(v)?;
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
                    v.push_ical(out, types.unwrap_or(&default_type))?;
                    continue;
                }
                ICalendarValue::Duration(v) => {
                    out.push_duration(v.parts())?;
                    continue;
                }
                ICalendarValue::RecurrenceRule(v) => {
                    v.push_to(out)?;
                    continue;
                }
                ICalendarValue::Period(v) => {
                    v.push_to(out)?;
                    continue;
                }
                ICalendarValue::Float(v) => {
                    write!(out, "{v}")?;
                    continue;
                }
                ICalendarValue::Integer(v) => {
                    out.push_i64(*v)?;
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
    fn write_value(&self, out: &mut impl LineWriter) -> fmt::Result {
        match self {
            Uri::Data(v) => {
                out.write_str("data:")?;
                out.write_str(v.content_type.as_deref().unwrap_or_default())?;
                out.write_str(";")?;
                out.write_atomic("base64,")?;
                out.write_base64(&v.data)
            }
            Uri::Location(v) => write_uri(out, v),
        }
    }

    fn write_param_text(&self, out: &mut impl LineWriter) -> fmt::Result {
        match self {
            Uri::Data(v) => {
                out.write_str("data:")?;
                write_param_text(out, v.content_type.as_deref().unwrap_or_default(), true)?;
                out.write_str(";")?;
                out.write_atomic("base64,")?;
                out.write_base64(&v.data)
            }
            Uri::Location(v) => write_param_text(out, v, true),
        }
    }
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

impl ICalendarRecurrenceRule {
    pub(crate) fn push_to(&self, buf: &mut impl AsciiPush) -> fmt::Result {
        buf.push_ascii_str("FREQ=")?;
        buf.push_ascii_str(self.freq.as_str())?;
        if let Some(until) = &self.until {
            buf.push_ascii_str(";UNTIL=")?;
            until.push_ical(
                buf,
                if until.has_date_and_time() {
                    &ICalendarValueType::DateTime
                } else {
                    &ICalendarValueType::Date
                },
            )?;
        }
        if let Some(count) = self.count.filter(|c| *c > 0) {
            buf.push_ascii_str(";COUNT=")?;
            buf.push_u64(count.into())?;
        }
        if let Some(interval) = self.interval {
            buf.push_ascii_str(";INTERVAL=")?;
            buf.push_u64(interval.into())?;
        }
        buf.push_list(";BYSECOND=", &self.bysecond, |buf, item| {
            buf.push_u64((*item).into())
        })?;
        buf.push_list(";BYMINUTE=", &self.byminute, |buf, item| {
            buf.push_u64((*item).into())
        })?;
        buf.push_list(";BYHOUR=", &self.byhour, |buf, item| {
            buf.push_u64((*item).into())
        })?;
        buf.push_list(";BYDAY=", &self.byday, |buf, item| item.push_to(buf))?;
        buf.push_list(";BYMONTHDAY=", &self.bymonthday, |buf, item| {
            buf.push_i64((*item).into())
        })?;
        buf.push_list(";BYYEARDAY=", &self.byyearday, |buf, item| {
            buf.push_i64((*item).into())
        })?;
        buf.push_list(";BYWEEKNO=", &self.byweekno, |buf, item| {
            buf.push_i64((*item).into())
        })?;
        buf.push_list(";BYMONTH=", &self.bymonth, |buf, item| item.push_to(buf))?;
        buf.push_list(";BYSETPOS=", &self.bysetpos, |buf, item| {
            buf.push_i64((*item).into())
        })?;
        if let Some(wkst) = self.wkst {
            buf.push_ascii_str(";WKST=")?;
            buf.push_ascii_str(wkst.as_str())?;
        }
        if let Some(rscale) = &self.rscale {
            buf.push_ascii_str(";RSCALE=")?;
            buf.push_ascii_str(rscale.as_str())?;
        } else if self.skip.is_some() {
            buf.push_ascii_str(";RSCALE=")?;
            buf.push_ascii_str(CalendarScale::Gregorian.as_str())?;
        }
        if let Some(skip) = &self.skip {
            buf.push_ascii_str(";SKIP=")?;
            buf.push_ascii_str(skip.as_str())?;
        }

        Ok(())
    }
}

impl Display for ICalendarRecurrenceRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_buffered::<RRULE_BUFFER>(|buf| self.push_to(buf))
    }
}

impl ICalendarDay {
    fn push_to(&self, buf: &mut impl AsciiPush) -> fmt::Result {
        if let Some(ordwk) = self.ordwk {
            buf.push_i64(ordwk.into())?;
        }
        buf.push_ascii_str(self.weekday.as_str())
    }
}

impl Display for ICalendarDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_buffered::<DATE_BUFFER>(|buf| self.push_to(buf))
    }
}

impl ICalendarMonth {
    fn push_to(&self, buf: &mut impl AsciiPush) -> fmt::Result {
        buf.push_u64(self.month().into())?;
        if self.is_leap() {
            buf.push_byte(b'L')?;
        }
        Ok(())
    }
}

impl Display for ICalendarMonth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_buffered::<DATE_BUFFER>(|buf| self.push_to(buf))
    }
}

impl ICalendarPeriod {
    fn push_to(&self, buf: &mut impl AsciiPush) -> fmt::Result {
        match self {
            ICalendarPeriod::Range { start, end } => {
                start.push_ical(buf, &ICalendarValueType::DateTime)?;
                buf.push_byte(b'/')?;
                end.push_ical(buf, &ICalendarValueType::DateTime)
            }
            ICalendarPeriod::Duration { start, duration } => {
                start.push_ical(buf, &ICalendarValueType::DateTime)?;
                buf.push_byte(b'/')?;
                buf.push_duration(duration.parts())
            }
        }
    }
}

impl Display for ICalendarPeriod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_buffered::<PERIOD_BUFFER>(|buf| self.push_to(buf))
    }
}

impl ICalendarDuration {
    pub(crate) fn parts(&self) -> DurationParts {
        DurationParts {
            neg: self.neg,
            weeks: self.weeks,
            days: self.days,
            hours: self.hours,
            minutes: self.minutes,
            seconds: self.seconds,
        }
    }
}

impl Display for ICalendarDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_buffered::<DURATION_BUFFER>(|buf| buf.push_duration(self.parts()))
    }
}

impl PartialDateTime {
    pub fn format_as_ical(&self, out: &mut impl Write, fmt: &ICalendarValueType) -> fmt::Result {
        out.write_buffered::<DATE_BUFFER>(|buf| self.push_ical(buf, fmt))
    }

    pub(crate) fn push_ical(
        &self,
        buf: &mut impl AsciiPush,
        fmt: &ICalendarValueType,
    ) -> fmt::Result {
        if matches!(fmt, ICalendarValueType::Date | ICalendarValueType::DateTime) {
            buf.push_4_digits(self.year.unwrap_or_default())?;
            buf.push_2_digits(self.month.unwrap_or_default())?;
            buf.push_2_digits(self.day.unwrap_or_default())?;
        }

        if matches!(fmt, ICalendarValueType::DateTime) {
            buf.push_byte(b'T')?;
        }

        if matches!(fmt, ICalendarValueType::DateTime | ICalendarValueType::Time) {
            buf.push_2_digits(self.hour.unwrap_or_default())?;
            buf.push_2_digits(self.minute.unwrap_or_default())?;
            buf.push_2_digits(self.second.unwrap_or_default())?;

            if matches!((self.tz_hour, self.tz_minute), (Some(0), Some(0))) {
                buf.push_byte(b'Z')?;
            }
        }

        if matches!(fmt, ICalendarValueType::UtcOffset) {
            buf.push_byte(if self.tz_minus { b'-' } else { b'+' })?;
            buf.push_2_digits(self.tz_hour.unwrap_or_default())?;
            buf.push_2_digits(self.tz_minute.unwrap_or_default())?;
        }

        Ok(())
    }
}

impl Display for ICalendar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    fn fold(line: &str) -> String {
        let (first, rest) = line.split_at(line.len().min(75));
        rest.as_bytes()
            .chunks(74)
            .fold(first.to_string(), |mut out, chunk| {
                out.push_str("\r\n ");
                out.push_str(std::str::from_utf8(chunk).unwrap());
                out
            })
            + "\r\n"
    }

    #[test]
    fn test_write_escapes_at_every_word_lane() {
        for offset in 0..=17 {
            let (head, tail) = ("x".repeat(offset), "y".repeat(17 - offset));
            for escaped in [r"\,", r"\;", r"\\", r"\n", r"\r"] {
                let line = format!("SUMMARY:{head}{escaped}{tail}\r\n");
                let out = write(&event(&line));
                assert!(out.contains(&line), "{line:?}\n{out}");
            }
            for encoded in ["\t", "^^", "^'", "^n"] {
                let param = format!("CN=\"{head}{encoded}{tail} z\":");
                let out = write(&event(&format!("ATTENDEE;{param}mailto:a@example.com\r\n")));
                assert!(out.contains(&param), "{param:?}\n{out}");
            }
        }
    }

    #[test]
    fn test_write_fold_points() {
        let x = |len: usize| "x".repeat(len);
        let y = |len: usize| "y".repeat(len);
        for (line, expected) in [
            (
                format!("DESCRIPTION:{}\\,z", x(61)),
                format!("DESCRIPTION:{}\\,\r\n z\r\n", x(61)),
            ),
            (
                format!("DESCRIPTION:{}\\,z", x(62)),
                format!("DESCRIPTION:{}\r\n \\,z\r\n", x(62)),
            ),
            (
                format!("DESCRIPTION:{}\\,z", x(63)),
                format!("DESCRIPTION:{}\r\n \\,z\r\n", x(63)),
            ),
            (
                format!("DESCRIPTION:{}\u{65e5}\u{672c}\u{8a9e}", x(62)),
                format!("DESCRIPTION:{}\r\n \u{65e5}\u{672c}\u{8a9e}\r\n", x(62)),
            ),
            (
                format!("DESCRIPTION:{}\u{e9}{}", x(61), y(80)),
                format!("DESCRIPTION:{}\u{e9}\r\n {}\r\n {}\r\n", x(61), y(74), y(6)),
            ),
            (
                format!(r"DESCRIPTION:{}\;{}", x(135), y(10)),
                format!(
                    "DESCRIPTION:{}\r\n {}{}\r\n {}\r\n",
                    x(63),
                    x(72),
                    r"\;",
                    y(10)
                ),
            ),
            (
                format!("ATTENDEE;CN=\"{}^'{}\":mailto:a@example.com", x(58), y(20)),
                format!(
                    "ATTENDEE;CN=\"{}^'yy\r\n {}\":mailto:a@example.com\r\n",
                    x(58),
                    y(18)
                ),
            ),
        ] {
            let out = write(&event(&format!("{line}\r\n")));
            assert!(out.contains(&expected), "{line:?}\n{out}");
        }
    }

    #[test]
    fn test_write_long_values_fold_every_75_octets() {
        for line in [
            format!("SUMMARY:{}", "0123456789".repeat(30)),
            format!("ATTACH;ENCODING=BASE64;VALUE=BINARY:{}", "QUJD".repeat(257)),
            format!(
                "ATTACH;ENCODING=BASE64;VALUE=BINARY:{}",
                "QUJD".repeat(1100)
            ),
        ] {
            let out = write(&event(&format!("{line}\r\n")));
            assert!(out.contains(&fold(&line)), "{line:?}\n{out}");
        }
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
    fn test_write_skips_component_boundary_entries() {
        let text = |name, value: &str| {
            ICalendarEntry::new(name).with_value(ICalendarValue::Text(value.to_string()))
        };
        let mut event = component(ICalendarComponentType::VEvent, &[]);
        event.entries = vec![
            text(ICalendarProperty::End, "VEVENT"),
            text(ICalendarProperty::Begin, "VTODO"),
            text(ICalendarProperty::Summary, "injected"),
        ];
        let ical = ICalendar {
            components: vec![component(ICalendarComponentType::VCalendar, &[1]), event],
        };

        let out = write(&ical);
        assert_eq!(
            out,
            concat!(
                "BEGIN:VCALENDAR\r\n",
                "BEGIN:VEVENT\r\n",
                "SUMMARY:injected\r\n",
                "END:VEVENT\r\n",
                "END:VCALENDAR\r\n"
            )
        );
        assert_eq!(parse(&out).components.len(), ical.components.len());
    }

    #[cfg(feature = "jmap")]
    #[test]
    fn jscalendar_properties_cannot_inject_component_boundaries() {
        use crate::jscalendar::JSCalendar;

        let ical = JSCalendar::<String, String>::parse(concat!(
            r#"{"@type":"Event","uid":"a","start":"2024-01-01T10:00:00","iCalendar":{"name":"vevent","#,
            r#""properties":[["end",{},"text","VEVENT"],["begin",{},"text","VTODO"],"#,
            r#"["summary",{},"text","injected"]]}}"#
        ))
        .expect("the event parses")
        .into_icalendar()
        .expect("the event exports");
        let reparsed = ICalendar::parse(write(&ical)).expect("the export parses");
        assert_eq!(reparsed.components.len(), ical.components.len());
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
