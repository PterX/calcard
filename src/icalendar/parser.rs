/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use super::*;
use crate::{
    Entry, Parser, StopChar, Token,
    common::{
        CalendarScale, Encoding, PartialDateTime,
        decode::{Decoded, ValueDecoder},
        parser::{Boolean, Integer, parse_digits, parse_small_digits},
        stack::ComponentStack,
        tokenizer::Mode,
    },
    icalendar::{ICalendarDay, ICalendarWeekday},
};
use smallvec::{SmallVec, smallvec};
use std::{iter::Peekable, mem::take, slice::Iter};

const EMPTY_CALENDAR_LINE: &str = "BEGIN:VCALENDAR";
const COMPONENT_CAPACITY: usize = 4;

impl ICalendar {
    fn into_entry(self, strict: bool) -> Entry {
        if strict
            && self
                .calendar_root()
                .is_some_and(|root| root.component_ids.is_empty())
        {
            Entry::InvalidLine(EMPTY_CALENDAR_LINE.to_string())
        } else {
            Entry::ICalendar(self)
        }
    }
}

struct Params {
    params: Vec<ICalendarParameter>,
    stop_char: StopChar,
    data_type: Option<IanaType<ICalendarValueType, String>>,
    charset: Option<String>,
    encoding: Option<Encoding>,
}

impl Params {
    fn decode_value(
        &self,
        token: &mut Token<'_>,
        payload: Option<Vec<u8>>,
        encoding: Encoding,
        name: &ICalendarProperty,
    ) -> Option<Vec<u8>> {
        let binary = match &self.data_type {
            Some(data_type) => data_type == &IanaType::Iana(ICalendarValueType::Binary),
            None => {
                encoding == Encoding::Base64
                    && matches!(name, ICalendarProperty::Attach | ICalendarProperty::Image)
            }
        };
        let decoder = ValueDecoder {
            encoding,
            charset: self.charset.as_deref(),
            binary,
        };
        match decoder.decode(&token.text, payload) {
            Decoded::Text(text) => {
                token.text = text.into();
                None
            }
            Decoded::Binary(bytes) => Some(bytes),
            Decoded::Undecodable => None,
        }
    }

    fn value(&mut self, token: Token<'_>, default_type: &ValueType) -> ICalendarValue {
        let value_type = match default_type {
            ValueType::Ical(default_type) => self
                .data_type
                .as_ref()
                .map(|v| v.iana().unwrap_or(&ICalendarValueType::Text))
                .unwrap_or(default_type),
            ValueType::CalendarScale => {
                return IanaType::from(token).map_or_text(ICalendarValue::CalendarScale);
            }
            ValueType::Method => return IanaType::from(token).map_or_text(ICalendarValue::Method),
            ValueType::Classification => {
                return IanaType::from(token).map_or_text(ICalendarValue::Classification);
            }
            ValueType::Status => return IanaType::from(token).map_or_text(ICalendarValue::Status),
            ValueType::Transparency => {
                return IanaType::from(token).map_or_text(ICalendarValue::Transparency);
            }
            ValueType::Action => return IanaType::from(token).map_or_text(ICalendarValue::Action),
            ValueType::BusyType => {
                return IanaType::from(token).map_or_text(ICalendarValue::BusyType);
            }
            ValueType::ParticipantType => {
                return IanaType::from(token).map_or_text(ICalendarValue::ParticipantType);
            }
            ValueType::ResourceType => {
                return IanaType::from(token).map_or_text(ICalendarValue::ResourceType);
            }
            ValueType::Proximity => {
                return IanaType::from(token).map_or_text(ICalendarValue::Proximity);
            }
        };
        match value_type {
            ICalendarValueType::Date => token
                .into_ical_date()
                .map(ICalendarValue::PartialDateTime)
                .unwrap_or_else(ICalendarValue::Text),
            ICalendarValueType::DateTime => match token.into_timestamp(false) {
                Ok(timestamp) => {
                    if !timestamp.has_time() {
                        self.data_type = Some(IanaType::Iana(ICalendarValueType::Date));
                    }
                    ICalendarValue::PartialDateTime(timestamp)
                }
                Err(other) => ICalendarValue::Text(other),
            },
            ICalendarValueType::Time => token
                .into_ical_time()
                .map(ICalendarValue::PartialDateTime)
                .unwrap_or_else(ICalendarValue::Text),
            ICalendarValueType::UtcOffset => token
                .into_offset()
                .map(ICalendarValue::PartialDateTime)
                .unwrap_or_else(ICalendarValue::Text),
            ICalendarValueType::Boolean => ICalendarValue::Boolean(token.into_boolean()),
            ICalendarValueType::Float => token
                .into_float()
                .map(ICalendarValue::Float)
                .unwrap_or_else(ICalendarValue::Text),
            ICalendarValueType::Integer => token
                .into_integer()
                .map(ICalendarValue::Integer)
                .unwrap_or_else(ICalendarValue::Text),
            ICalendarValueType::Uri | ICalendarValueType::CalAddress => token
                .into_uri_bytes()
                .map(|data| ICalendarValue::Uri(Uri::from(data)))
                .unwrap_or_else(|uri| ICalendarValue::Uri(Uri::Location(uri))),
            ICalendarValueType::Duration => match ICalendarDuration::parse(token.text.as_ref()) {
                Some(duration) => ICalendarValue::Duration(duration),
                None => ICalendarValue::Text(token.into_string()),
            },
            ICalendarValueType::Period => match ICalendarPeriod::parse(token.text.as_ref()) {
                Some(period) => ICalendarValue::Period(Box::new(period)),
                None => ICalendarValue::Text(token.into_string()),
            },
            ICalendarValueType::Text
            | ICalendarValueType::Binary
            | ICalendarValueType::Unknown
            | ICalendarValueType::XmlReference
            | ICalendarValueType::Uid
            | ICalendarValueType::Recur => ICalendarValue::Text(token.into_string()),
        }
    }

    fn push_known(&mut self, name: &ICalendarParameterName, token: Token<'_>) {
        let value = match name {
            ICalendarParameterName::Altrep
            | ICalendarParameterName::DelegatedFrom
            | ICalendarParameterName::DelegatedTo
            | ICalendarParameterName::Dir
            | ICalendarParameterName::Member
            | ICalendarParameterName::SentBy
            | ICalendarParameterName::Schema => ICalendarParameterValue::Uri(Uri::from(token)),
            ICalendarParameterName::Rsvp | ICalendarParameterName::Derived => {
                IanaType::<Boolean, String>::from(token).into()
            }
            ICalendarParameterName::Range => {
                if token.text.as_ref().eq_ignore_ascii_case(b"THISANDFUTURE") {
                    ICalendarParameterValue::Bool(true)
                } else {
                    return;
                }
            }
            ICalendarParameterName::Size | ICalendarParameterName::Order => {
                IanaType::<Integer, String>::from(token).into()
            }
            ICalendarParameterName::Gap => {
                IanaType::<ICalendarDuration, String>::from(token).into()
            }
            ICalendarParameterName::Cutype => {
                IanaType::<ICalendarUserTypes, String>::from(token).into()
            }
            ICalendarParameterName::Fbtype => {
                IanaType::<ICalendarFreeBusyType, String>::from(token).into()
            }
            ICalendarParameterName::Partstat => {
                IanaType::<ICalendarParticipationStatus, String>::from(token).into()
            }
            ICalendarParameterName::Related => {
                IanaType::<ICalendarRelated, String>::from(token).into()
            }
            ICalendarParameterName::Reltype => {
                IanaType::<ICalendarRelationshipType, String>::from(token).into()
            }
            ICalendarParameterName::Role => {
                IanaType::<ICalendarParticipationRole, String>::from(token).into()
            }
            ICalendarParameterName::ScheduleAgent => {
                IanaType::<ICalendarScheduleAgentValue, String>::from(token).into()
            }
            ICalendarParameterName::ScheduleForceSend => {
                IanaType::<ICalendarScheduleForceSendValue, String>::from(token).into()
            }
            ICalendarParameterName::Value => {
                self.data_type = Some(token.into());
                return;
            }
            ICalendarParameterName::Display => {
                IanaType::<ICalendarDisplayType, String>::from(token).into()
            }
            ICalendarParameterName::Feature => {
                IanaType::<ICalendarFeatureType, String>::from(token).into()
            }
            ICalendarParameterName::Linkrel => IanaType::<LinkRelation, String>::from(token).into(),
            _ => ICalendarParameterValue::Text(token.into_string()),
        };
        self.params
            .push(ICalendarParameter::new(name.clone(), value));
    }
}

trait IntoValue<I> {
    fn map_or_text(self, iana: impl FnOnce(I) -> ICalendarValue) -> ICalendarValue;
}

impl<I> IntoValue<I> for IanaType<I, String> {
    fn map_or_text(self, iana: impl FnOnce(I) -> ICalendarValue) -> ICalendarValue {
        match self {
            IanaType::Iana(value) => iana(value),
            IanaType::Other(value) => ICalendarValue::Text(value),
        }
    }
}

enum ParamKind {
    Known(ICalendarParameterName),
    Charset,
    Encoding,
    Other(ICalendarParameterName),
    Pending,
}

impl ParamKind {
    fn classify(token: Token<'_>, params: &mut Params) -> Self {
        let name = token.text.as_ref();
        if let Some(name) = ICalendarParameterName::try_parse(name) {
            return ParamKind::Known(name);
        }
        match name.first() {
            None => ParamKind::Pending,
            Some(b'c' | b'C') if name.eq_ignore_ascii_case(b"charset") => ParamKind::Charset,
            Some(b'e' | b'E') if name.eq_ignore_ascii_case(b"encoding") => ParamKind::Encoding,
            _ if params.encoding.is_none() && name.eq_ignore_ascii_case(b"base64") => {
                params.encoding = Some(Encoding::Base64);
                ParamKind::Pending
            }
            _ => ParamKind::Other(ICalendarParameterName::Other(token.into_string())),
        }
    }
}

#[derive(Default)]
struct ParamValues {
    count: usize,
    last_other: Option<String>,
}

impl ParamValues {
    fn push(&mut self, kind: &ParamKind, token: Token<'_>, params: &mut Params) {
        self.count += 1;
        match kind {
            ParamKind::Known(name) => params.push_known(name, token),
            ParamKind::Charset => params.charset = Some(token.into_string()),
            ParamKind::Encoding => params.encoding = Encoding::parse(token.text.as_ref()),
            ParamKind::Other(name) => {
                if let Some(value) = self.last_other.replace(token.into_string()) {
                    params.params.push(ICalendarParameter::new(
                        name.clone(),
                        ICalendarParameterValue::Text(value),
                    ));
                }
            }
            ParamKind::Pending => {}
        }
    }

    fn finish(self, kind: ParamKind, params: &mut Params) {
        match kind {
            ParamKind::Known(name) if self.count == 0 => {
                params
                    .params
                    .push(ICalendarParameter::new(name, ICalendarParameterValue::Null));
            }
            ParamKind::Other(name) => params.params.push(ICalendarParameter::new(
                name,
                self.last_other
                    .map_or(ICalendarParameterValue::Null, ICalendarParameterValue::Text),
            )),
            _ => {}
        }
    }
}

impl Parser<'_> {
    pub fn icalendar(&mut self, component_type: ICalendarComponentType) -> Entry {
        let mut components = Vec::with_capacity(COMPONENT_CAPACITY);
        components.push(ICalendarComponent {
            component_type,
            ..Default::default()
        });
        let mut parents = ComponentStack::new();
        let mut current = 0;
        let mut next_component_id: u32 = 1;

        loop {
            self.expect_iana_token();
            let Some(name) = self.token() else {
                break;
            };
            let property = ICalendarProperty::parse(name.text.as_ref());

            let mut params = Params {
                params: Vec::new(),
                stop_char: name.stop_char,
                data_type: None,
                encoding: None,
                charset: None,
            };

            match params.stop_char {
                StopChar::Semicolon => {
                    params.params = Vec::with_capacity(
                        property
                            .as_ref()
                            .map_or(0, ICalendarProperty::parameter_capacity),
                    );
                    self.ical_parameters(&mut params);
                }
                StopChar::Colon => {}
                StopChar::Lf => {
                    if name.text.is_empty() || !self.strict {
                        continue;
                    } else {
                        return Entry::InvalidLine(name.into_string());
                    }
                }
                StopChar::Comma | StopChar::Equal | StopChar::Dot => {
                    params.stop_char = self.seek_value_or_eol();
                }
            }

            let name = match property {
                Some(ICalendarProperty::Begin) => {
                    if params.stop_char == StopChar::Colon
                        && let Some(component_type) = self.component_type()
                    {
                        if let Some(parent) = components.get_mut(current) {
                            parent.component_ids.push(next_component_id);
                        }
                        components.push(ICalendarComponent {
                            component_type,
                            ..Default::default()
                        });
                        parents.push(current);
                        current = next_component_id as usize;
                        let Some(id) = next_component_id.checked_add(1) else {
                            return Entry::TooManyComponents;
                        };
                        next_component_id = id;
                        continue;
                    }

                    if !self.strict {
                        continue;
                    } else {
                        return Entry::InvalidLine("BEGIN".to_string());
                    }
                }
                Some(ICalendarProperty::End) => {
                    if params.stop_char == StopChar::Colon
                        && let Some(component_type) = self.component_type()
                    {
                        let open_type = components
                            .get(current)
                            .map(|component| &component.component_type);
                        if open_type == Some(&component_type) || !self.strict {
                            if let Some(parent) = parents.pop() {
                                current = parent;
                                continue;
                            } else {
                                break;
                            }
                        } else {
                            return Entry::UnexpectedComponentEnd {
                                expected: open_type.cloned().unwrap_or_default(),
                                found: component_type,
                            };
                        }
                    }

                    if !self.strict {
                        continue;
                    } else {
                        return Entry::InvalidLine("END".to_string());
                    }
                }
                Some(name) => name,
                None => {
                    if !name.text.is_empty() {
                        ICalendarProperty::Other(name.into_string())
                    } else {
                        if params.stop_char != StopChar::Lf {
                            self.seek_lf();
                        }
                        continue;
                    }
                }
            };

            let entry = self.ical_entry(name, params);
            if let Some(component) = components.get_mut(current) {
                if component.entries.capacity() == 0 {
                    self.presize(
                        &mut component.entries,
                        component.component_type.entry_capacity(),
                    );
                }
                component.entries.push(entry);
            }
        }

        if !parents.is_empty() && self.strict {
            return Entry::UnterminatedComponent(
                components
                    .get(current)
                    .map_or("", |component| component.component_type.as_str())
                    .to_string()
                    .into(),
            );
        }

        ICalendar { components }.into_entry(self.strict)
    }

    fn component_type(&mut self) -> Option<ICalendarComponentType> {
        self.expect_single_value();
        self.token().map(|token| {
            ICalendarComponentType::parse(token.text.as_ref())
                .unwrap_or_else(|| ICalendarComponentType::Other(token.into_string()))
        })
    }

    fn ical_entry(&mut self, name: ICalendarProperty, mut params: Params) -> ICalendarEntry {
        let mut entry = ICalendarEntry {
            name,
            params: take(&mut params.params),
            values: SmallVec::new(),
        };

        if params.stop_char != StopChar::Lf {
            let (default_type, multi_value) = entry.name.default_types();
            match multi_value {
                ValueSeparator::None => {
                    self.expect_single_value();
                }
                ValueSeparator::Comma => {
                    self.expect_multi_value_comma();
                }
                ValueSeparator::Semicolon => {
                    self.expect_multi_value_semicolon();
                }
            }

            match params.encoding {
                Some(Encoding::Base64) if multi_value != ValueSeparator::None => {
                    self.expect_single_value();
                }
                Some(Encoding::QuotedPrintable) => {
                    self.mode.set(Mode::UNFOLD_QP, true);
                }
                _ => {}
            }

            self.mode.set(
                Mode::UNESCAPE_BACKSLASH,
                !matches!(
                    params
                        .data_type
                        .as_ref()
                        .map(|data_type| data_type.iana().unwrap_or(&ICalendarValueType::Text))
                        .or(match &default_type {
                            ValueType::Ical(default_type) => Some(default_type),
                            _ => None,
                        }),
                    Some(ICalendarValueType::Uri | ICalendarValueType::CalAddress)
                ),
            );

            if matches!(
                (&params.data_type, &default_type),
                (Some(IanaType::Iana(ICalendarValueType::Recur)), _)
                    | (None, ValueType::Ical(ICalendarValueType::Recur))
            ) {
                entry.values = smallvec![match self.rrule() {
                    Ok(rrule) => ICalendarValue::RecurrenceRule(Box::new(rrule)),
                    Err(other) => ICalendarValue::Text(other),
                }];
            } else {
                let mut payload = None;
                while let Some(mut token) = self.value_token(params.encoding, &mut payload) {
                    let eol = token.stop_char == StopChar::Lf;

                    if token.text.is_empty()
                        && (matches!(multi_value, ValueSeparator::None)
                            || matches!(entry.name, ICalendarProperty::Other(_))
                                && entry.values.is_empty())
                    {
                        if eol {
                            break;
                        } else {
                            continue;
                        }
                    }

                    let value = if let Some(encoding) = params.encoding
                        && let Some(binary) =
                            params.decode_value(&mut token, payload.take(), encoding, &entry.name)
                    {
                        params.data_type = Some(IanaType::Iana(ICalendarValueType::Binary));
                        ICalendarValue::Binary(binary)
                    } else {
                        params.value(token, &default_type)
                    };
                    entry.values.push(value);

                    if eol {
                        break;
                    }
                }
            }
        }

        if let Some(data_type) = params.data_type {
            entry.params.push(ICalendarParameter {
                name: ICalendarParameterName::Value,
                value: match data_type {
                    IanaType::Iana(value) => ICalendarParameterValue::Value(value),
                    IanaType::Other(value) => ICalendarParameterValue::Text(value),
                },
            });
        }

        entry
    }

    fn ical_parameters(&mut self, params: &mut Params) {
        while params.stop_char == StopChar::Semicolon {
            self.expect_iana_token();
            let Some(token) = self.token() else {
                params.stop_char = StopChar::Lf;
                break;
            };
            params.stop_char = token.stop_char;
            let kind = ParamKind::classify(token, params);
            let buffered = matches!(kind, ParamKind::Pending) || !self.token_buf.is_empty();
            let mut values = ParamValues::default();

            if !matches!(
                params.stop_char,
                StopChar::Lf | StopChar::Colon | StopChar::Semicolon
            ) {
                if params.stop_char != StopChar::Equal {
                    params.stop_char = self.seek_param_value_or_eol();
                }
                if params.stop_char == StopChar::Equal {
                    self.expect_param_value();
                    while !matches!(
                        params.stop_char,
                        StopChar::Lf | StopChar::Colon | StopChar::Semicolon
                    ) {
                        match self.token() {
                            Some(token) => {
                                params.stop_char = token.stop_char;
                                if buffered {
                                    self.token_buf.push(token);
                                } else {
                                    values.push(&kind, token, params);
                                }
                            }
                            None => {
                                params.stop_char = StopChar::Lf;
                                break;
                            }
                        }
                    }
                }
            }

            if buffered {
                self.buffered_param(kind, params);
            } else {
                values.finish(kind, params);
            }
        }
    }

    fn buffered_param(&mut self, kind: ParamKind, params: &mut Params) {
        match kind {
            ParamKind::Known(name) => {
                if self.token_buf.is_empty() {
                    params
                        .params
                        .push(ICalendarParameter::new(name, ICalendarParameterValue::Null));
                } else {
                    for token in self.token_buf.drain(..) {
                        params.push_known(&name, token);
                    }
                }
            }
            ParamKind::Charset => {
                for token in self.token_buf.drain(..) {
                    params.charset = token.into_string().into();
                }
            }
            ParamKind::Encoding => {
                for token in self.token_buf.drain(..) {
                    params.encoding = Encoding::parse(token.text.as_ref());
                }
            }
            ParamKind::Other(name) => {
                if self.token_buf.is_empty() {
                    params
                        .params
                        .push(ICalendarParameter::new(name, ICalendarParameterValue::Null));
                } else {
                    params.params.extend(self.token_buf.drain(..).map(|token| {
                        ICalendarParameter::new(
                            name.clone(),
                            ICalendarParameterValue::Text(token.into_string()),
                        )
                    }));
                }
            }
            ParamKind::Pending => {}
        }
    }

    pub(crate) fn rrule(&mut self) -> Result<ICalendarRecurrenceRule, String> {
        self.expect_rrule_value();

        let mut last_stop_char = StopChar::Equal;
        let mut is_valid = true;
        let mut has_freq = false;
        let mut rrule = ICalendarRecurrenceRule::default();

        let mut token_start = usize::MAX;

        while let Some(mut token) = self.token_until_lf(&mut last_stop_char) {
            if token_start == usize::MAX && !token.text.is_empty() {
                token_start = token.start;
            }
            if !is_valid {
                continue;
            }
            if token.text.is_empty() && token.stop_char != StopChar::Equal {
                continue;
            }
            if token.stop_char != StopChar::Equal {
                if !self.strict {
                    // Ignore unknown tokens
                    while let Some(token_) = self.token_until_lf(&mut last_stop_char) {
                        if token_.stop_char == StopChar::Equal {
                            token = token_;
                            break;
                        }
                    }
                    if token.stop_char != StopChar::Equal {
                        is_valid = false;
                        continue;
                    }
                } else {
                    is_valid = false;
                    continue;
                }
            }

            hashify::fnc_map_ignore_case!(token.text.as_ref(),
                b"FREQ" => {
                    while let Some(value) = self.parse_value_until_lf(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.freq = value;
                            has_freq = true;
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"UNTIL" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarDateOrTime>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.until = Some(value.0);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"COUNT" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            let count = value.0.unsigned_abs() as u32;
                            if count > 0 {
                                rrule.count = Some(count);
                            }
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"INTERVAL" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            let interval = value.0.unsigned_abs() as u16;
                            if interval > 0 {
                                rrule.interval = Some(interval);
                            }
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYSECOND" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.bysecond.push(value.0.unsigned_abs() as u8);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYMINUTE" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.byminute.push(value.0.unsigned_abs() as u8);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYHOUR" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.byhour.push(value.0.unsigned_abs() as u8);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYDAY" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarDay>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.byday.push(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYMONTHDAY" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.bymonthday.push(value.0 as i8);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYYEARDAY" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.byyearday.push(value.0 as i16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYWEEKNO" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.byweekno.push(value.0 as i8);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYMONTH" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarMonth>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.bymonth.push(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYSETPOS" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.bysetpos.push(value.0 as i32);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"WKST" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarWeekday>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.wkst = Some(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"RSCALE" => {
                    while let Some(value) = self.parse_value_until_lf::<CalendarScale>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.rscale = Some(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"SKIP" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarSkip>(StopChar::Semicolon, &mut last_stop_char) {                        if let Some(value)= value {
                            rrule.skip = Some(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                _ => {
                    if !self.strict {
                        // Ignore unknown tokens
                        while let Some(token) = self.token_until_lf(&mut last_stop_char) {                            if token.stop_char == StopChar::Semicolon{
                                break;
                            }
                        }
                    } else {
                        is_valid = false;
                    }
                }
            );
        }

        if has_freq && is_valid {
            Ok(rrule)
        } else if token_start != usize::MAX && self.last_token_end >= token_start {
            Err(self
                .input
                .get(token_start..=self.last_token_end)
                .map(|slice| {
                    let mut buf = Vec::with_capacity(slice.len());
                    let mut iter = slice.iter().peekable();

                    while let Some(ch) = iter.next() {
                        match ch {
                            b'\r' => {}
                            b'\n' => {
                                iter.next_if(|ch| matches!(ch, b' ' | b'\t'));
                            }
                            _ => buf.push(*ch),
                        }
                    }

                    String::from_utf8(buf)
                        .unwrap_or_else(|err| String::from_utf8_lossy(err.as_bytes()).into_owned())
                })
                .unwrap_or_default())
        } else {
            Err("".to_string())
        }
    }
}

impl Token<'_> {
    pub(crate) fn into_ical_date(self) -> std::result::Result<PartialDateTime, String> {
        if let Some(dt) = self
            .text
            .first_chunk::<8>()
            .and_then(PartialDateTime::from_basic_date)
        {
            return Ok(dt);
        }
        let mut dt = PartialDateTime::default();
        if dt.parse_ical_date(&mut self.text.iter().peekable()) {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_ical_time(self) -> std::result::Result<PartialDateTime, String> {
        let mut dt = PartialDateTime::default();
        if dt.parse_ical_time(&mut self.text.iter().peekable()) {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }
}

impl PartialDateTime {
    pub fn parse_ical_date(&mut self, iter: &mut Peekable<Iter<'_, u8>>) -> bool {
        parse_digits(iter, &mut self.year, 4, false)
            && parse_small_digits(iter, &mut self.month, 2, false)
            && parse_small_digits(iter, &mut self.day, 2, false)
    }

    pub fn parse_ical_time(&mut self, iter: &mut Peekable<Iter<'_, u8>>) -> bool {
        if parse_small_digits(iter, &mut self.hour, 2, false)
            && parse_small_digits(iter, &mut self.minute, 2, false)
            && parse_small_digits(iter, &mut self.second, 2, false)
        {
            self.parse_zone(iter);
            true
        } else {
            false
        }
    }
}

struct ICalendarDateOrTime(PartialDateTime);

impl IanaParse for ICalendarDateOrTime {
    fn parse(value: &[u8]) -> Option<Self> {
        match PartialDateTime::from_timestamp_bytes(value, false) {
            (dt, true) => Some(ICalendarDateOrTime(dt)),
            (_, false) => None,
        }
    }
}

impl IanaParse for ICalendarPeriod {
    fn parse(value: &[u8]) -> Option<Self> {
        let mut iter = value.iter().peekable();
        let mut start = PartialDateTime::default();
        if start.parse_timestamp(&mut iter, true) {
            if let Some(duration) = ICalendarDuration::try_parse(&mut iter) {
                Some(ICalendarPeriod::Duration { start, duration })
            } else {
                let mut end = PartialDateTime::default();
                if end.parse_timestamp(&mut iter, true) {
                    Some(ICalendarPeriod::Range { start, end })
                } else {
                    None
                }
            }
        } else {
            None
        }
    }
}

impl IanaParse for ICalendarMonth {
    fn parse(value: &[u8]) -> Option<Self> {
        let mut num: i8 = 0;
        let mut is_leap = false;

        for (pos, ch) in value.iter().enumerate() {
            match ch {
                b'L' | b'l' if pos > 0 => {
                    is_leap = true;
                }
                b'0'..=b'9' => {
                    num = num.saturating_mul(10).saturating_add((*ch - b'0') as i8);
                }
                _ => {
                    if !ch.is_ascii_whitespace() {
                        return None;
                    }
                }
            }
        }

        Some(ICalendarMonth(if is_leap { -num } else { num }))
    }
}

impl ICalendarMonth {
    pub fn new(month: u8, is_leap: bool) -> Self {
        let month = i8::try_from(month).unwrap_or(i8::MAX);
        ICalendarMonth(if is_leap { -month } else { month })
    }

    pub fn is_leap(&self) -> bool {
        self.0 < 0
    }

    pub fn month(&self) -> u8 {
        self.0.unsigned_abs()
    }
}

impl From<ICalendarMonth> for u8 {
    fn from(value: ICalendarMonth) -> Self {
        value.month()
    }
}

impl ICalendarDuration {
    fn try_parse(iter: &mut Peekable<Iter<'_, u8>>) -> Option<Self> {
        let mut dur = ICalendarDuration::default();
        loop {
            match iter.peek() {
                Some(b'P' | b'p') => {
                    iter.next();
                    break;
                }
                Some(b'+') => {
                    iter.next();
                }
                Some(b'-') if !dur.neg => {
                    iter.next();
                    dur.neg = true;
                }
                Some(b' ' | b'\t') => {
                    iter.next();
                }
                _ => {
                    return None;
                }
            }
        }

        let mut num: u32 = 0;
        let mut saw_component = false;
        for ch in iter {
            match ch {
                b'0'..=b'9' => {
                    num = num.saturating_mul(10).saturating_add((ch - b'0') as u32);
                }
                b'T' | b't' => {}
                b'W' | b'w' => {
                    dur.weeks = num;
                    num = 0;
                    saw_component = true;
                }
                b'D' | b'd' => {
                    dur.days = num;
                    num = 0;
                    saw_component = true;
                }
                b'H' | b'h' => {
                    dur.hours = num;
                    num = 0;
                    saw_component = true;
                }
                b'M' | b'm' => {
                    dur.minutes = num;
                    num = 0;
                    saw_component = true;
                }
                b'S' | b's' => {
                    dur.seconds = num;
                    num = 0;
                    saw_component = true;
                }
                _ => {
                    if !ch.is_ascii_whitespace() {
                        return None;
                    }
                }
            }
        }

        if saw_component { Some(dur) } else { None }
    }
}

impl IanaParse for ICalendarDuration {
    fn parse(value: &[u8]) -> Option<Self> {
        ICalendarDuration::try_parse(&mut value.iter().peekable())
    }
}

impl From<Token<'_>> for Uri {
    fn from(token: Token<'_>) -> Self {
        token
            .into_uri_bytes()
            .map(Uri::from)
            .unwrap_or_else(Uri::Location)
    }
}

impl IanaParse for ICalendarDay {
    fn parse(value: &[u8]) -> Option<Self> {
        let mut iter = value.iter().enumerate();
        let mut is_negative = false;
        let mut has_ordwk = false;
        let mut ordwk: i16 = 0;

        loop {
            let (pos, ch) = iter.next()?;

            match ch {
                b'0'..=b'9' => {
                    ordwk = ordwk.saturating_mul(10).saturating_add((ch - b'0') as i16);
                    has_ordwk = true;
                }
                b'-' if pos == 0 => {
                    is_negative = true;
                }
                b'+' if pos == 0 => {}
                b'A'..=b'Z' | b'a'..=b'z' => {
                    return ICalendarWeekday::parse(value.get(pos..).unwrap_or_default()).map(
                        |weekday| ICalendarDay {
                            ordwk: has_ordwk.then_some(if is_negative { -ordwk } else { ordwk }),
                            weekday,
                        },
                    );
                }
                _ => return None,
            }
        }
    }
}

impl Uri {
    pub fn parse(value: impl Into<String>) -> Self {
        let uri = value.into();
        Data::try_parse(uri.as_bytes())
            .map(Uri::from)
            .unwrap_or_else(|| Uri::Location(uri))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::tokenizer::TokenText;
    use std::io::Write;

    #[test]
    fn parse_ical() {
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
                let mut output = std::fs::File::create(path.with_extension("ics.out")).unwrap();
                //let mut output_debug =
                //    std::fs::File::create(path.with_extension("ics.debug")).unwrap();
                let file_name = path.as_path().to_str().unwrap();

                loop {
                    match parser.entry() {
                        Entry::ICalendar(mut ical) => {
                            for item in &mut ical.components {
                                for item in &mut item.entries {
                                    if item.name == ICalendarProperty::Version {
                                        item.values = smallvec![ICalendarValue::Text("2.0".into())];
                                    }
                                }
                            }
                            let ical_text = ical.to_string();
                            crate::common::writer::assert_fold_width(&ical_text, file_name);
                            writeln!(output, "{}", ical_text).unwrap();
                            //writeln!(output_debug, "{:#?}", ical).unwrap();

                            // Roundtrip parsing
                            let mut parser = Parser::new(&ical_text);
                            match parser.entry() {
                                Entry::ICalendar(ical_) => {
                                    /*ical.components.iter_mut().for_each(|component| {
                                        component.entries.retain(|entry| {
                                            !matches!(entry.name, ICalendarProperty::Version)
                                        });
                                    });
                                    ical_.components.iter_mut().for_each(|component| {
                                        component.entries.retain(|entry| {
                                            !matches!(entry.name, ICalendarProperty::Version)
                                        });
                                    });*/

                                    compare_components(&ical, &ical_, file_name);
                                }
                                other => panic!("Expected iCal, got {other:?} for {file_name}"),
                            }

                            // Rkyv archiving tests
                            #[cfg(feature = "rkyv")]
                            {
                                let ical_bytes =
                                    rkyv::to_bytes::<rkyv::rancor::Error>(&ical).unwrap();
                                let ical_unarchived = rkyv::access::<
                                    crate::icalendar::ArchivedICalendar,
                                    rkyv::rancor::Error,
                                >(&ical_bytes)
                                .unwrap();
                                assert_eq!(ical_text, ical_unarchived.to_string());
                            }
                        }
                        Entry::InvalidLine(text) => {
                            println!("Invalid line in {file_name}: {text}");
                        }
                        Entry::Eof => break,
                        other => {
                            panic!("Expected iCal, got {other:?} for {file_name}");
                        }
                    }
                }
            }
        }
    }

    fn compare_components(a: &ICalendar, b: &ICalendar, file_name: &str) {
        assert_eq!(
            a.components.len(),
            b.components.len(),
            "failed for {file_name}"
        );

        for (a, b) in a.components.iter().zip(b.components.iter()) {
            assert_eq!(a, b, "failed for {file_name}");
        }
    }

    #[test]
    fn test_parse_rrule() {
        for (rule, expected) in [
            (
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Monday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Wednesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Friday,
                        },
                    ],
                    bysetpos: vec![-1],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYMONTH=1,2",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    bymonth: vec![ICalendarMonth(1), ICalendarMonth(2)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;INTERVAL=2;BYMONTH=1;BYDAY=SU;BYHOUR=8,9;BYMINUTE=30",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    interval: Some(2),
                    byminute: vec![30],
                    byhour: vec![8, 9],
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![ICalendarMonth(1)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=DAILY;COUNT=10;INTERVAL=2",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Daily,
                    count: Some(10),
                    interval: Some(2),
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYMONTH=4;BYDAY=-1SU;UNTIL=19730429T070000Z",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    until: Some(PartialDateTime {
                        year: Some(1973),
                        month: Some(4),
                        day: Some(29),
                        hour: Some(7),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    }),
                    byday: vec![ICalendarDay {
                        ordwk: Some(-1),
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![ICalendarMonth(4)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU;UNTIL=20061029T060000",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    until: Some(PartialDateTime {
                        year: Some(2006),
                        month: Some(10),
                        day: Some(29),
                        hour: Some(6),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: None,
                        tz_minute: None,
                        tz_minus: false,
                    }),
                    byday: vec![ICalendarDay {
                        ordwk: Some(-1),
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![ICalendarMonth(10)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYMONTH=3;BYDAY=2SU",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    byday: vec![ICalendarDay {
                        ordwk: Some(2),
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![ICalendarMonth(3)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    byday: vec![ICalendarDay {
                        ordwk: Some(-1),
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![ICalendarMonth(10)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=DAILY;COUNT=10",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Daily,
                    count: Some(10),
                    ..Default::default()
                },
            ),
            (
                "FREQ=DAILY;INTERVAL=10;COUNT=5",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Daily,
                    count: Some(5),
                    interval: Some(10),
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;UNTIL=20000131T140000Z;BYMONTH=1;BYDAY=SU,MO,TU,WE,TH,FR,SA",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    until: Some(PartialDateTime {
                        year: Some(2000),
                        month: Some(1),
                        day: Some(31),
                        hour: Some(14),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    }),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Sunday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Monday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Wednesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Friday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Saturday,
                        },
                    ],
                    bymonth: vec![ICalendarMonth(1)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=DAILY;UNTIL=20000131T140000Z;BYMONTH=1",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Daily,
                    until: Some(PartialDateTime {
                        year: Some(2000),
                        month: Some(1),
                        day: Some(31),
                        hour: Some(14),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    }),
                    bymonth: vec![ICalendarMonth(1)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;COUNT=10",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    count: Some(10),
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;INTERVAL=2;WKST=SU",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    interval: Some(2),
                    wkst: Some(ICalendarWeekday::Sunday),
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;UNTIL=19971007T000000Z;WKST=SU;BYDAY=TU,TH",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    until: Some(PartialDateTime {
                        year: Some(1997),
                        month: Some(10),
                        day: Some(7),
                        hour: Some(0),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    }),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                    ],
                    wkst: Some(ICalendarWeekday::Sunday),
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;COUNT=10;WKST=SU;BYDAY=TU,TH",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    count: Some(10),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                    ],
                    wkst: Some(ICalendarWeekday::Sunday),
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;INTERVAL=2;UNTIL=19971224T000000Z;WKST=SU;BYDAY=MO,WE,FR",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    until: Some(PartialDateTime {
                        year: Some(1997),
                        month: Some(12),
                        day: Some(24),
                        hour: Some(0),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    }),
                    interval: Some(2),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Monday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Wednesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Friday,
                        },
                    ],
                    wkst: Some(ICalendarWeekday::Sunday),
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;INTERVAL=2;COUNT=8;WKST=SU;BYDAY=TU,TH",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    count: Some(8),
                    interval: Some(2),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                    ],
                    wkst: Some(ICalendarWeekday::Sunday),
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;COUNT=10;BYDAY=1FR",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(10),
                    byday: vec![ICalendarDay {
                        ordwk: Some(1),
                        weekday: ICalendarWeekday::Friday,
                    }],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;INTERVAL=2;COUNT=10;BYDAY=1SU,-1SU",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(10),
                    interval: Some(2),
                    byday: vec![
                        ICalendarDay {
                            ordwk: Some(1),
                            weekday: ICalendarWeekday::Sunday,
                        },
                        ICalendarDay {
                            ordwk: Some(-1),
                            weekday: ICalendarWeekday::Sunday,
                        },
                    ],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;COUNT=6;BYDAY=-2MO",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(6),
                    byday: vec![ICalendarDay {
                        ordwk: Some(-2),
                        weekday: ICalendarWeekday::Monday,
                    }],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;BYMONTHDAY=-3",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    bymonthday: vec![-3],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;COUNT=10;BYMONTHDAY=2,15",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(10),
                    bymonthday: vec![2, 15],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;COUNT=10;BYMONTHDAY=1,-1",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(10),
                    bymonthday: vec![1, -1],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;INTERVAL=18;COUNT=10;BYMONTHDAY=10,11,12,13,14,15",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(10),
                    interval: Some(18),
                    bymonthday: vec![10, 11, 12, 13, 14, 15],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;INTERVAL=2;BYDAY=TU",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    interval: Some(2),
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Tuesday,
                    }],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;COUNT=10;BYMONTH=6,7",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    count: Some(10),
                    bymonth: vec![ICalendarMonth(6), ICalendarMonth(7)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;INTERVAL=2;COUNT=10;BYMONTH=1,2,3",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    count: Some(10),
                    interval: Some(2),
                    bymonth: vec![ICalendarMonth(1), ICalendarMonth(2), ICalendarMonth(3)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;INTERVAL=3;COUNT=10;BYYEARDAY=1,100,200",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    count: Some(10),
                    interval: Some(3),
                    byyearday: vec![1, 100, 200],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYDAY=20MO",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    byday: vec![ICalendarDay {
                        ordwk: Some(20),
                        weekday: ICalendarWeekday::Monday,
                    }],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYWEEKNO=20;BYDAY=MO",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Monday,
                    }],
                    byweekno: vec![20],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYMONTH=3;BYDAY=TH",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Thursday,
                    }],
                    bymonth: vec![ICalendarMonth(3)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYDAY=TH;BYMONTH=6,7,8",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Thursday,
                    }],
                    bymonth: vec![ICalendarMonth(6), ICalendarMonth(7), ICalendarMonth(8)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Friday,
                    }],
                    bymonthday: vec![13],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;BYDAY=SA;BYMONTHDAY=7,8,9,10,11,12,13",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Saturday,
                    }],
                    bymonthday: vec![7, 8, 9, 10, 11, 12, 13],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;INTERVAL=4;BYMONTH=11;BYDAY=TU;BYMONTHDAY=2,3,4,5,6,7,8",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    interval: Some(4),
                    byday: vec![ICalendarDay {
                        ordwk: None,
                        weekday: ICalendarWeekday::Tuesday,
                    }],
                    bymonthday: vec![2, 3, 4, 5, 6, 7, 8],
                    bymonth: vec![ICalendarMonth(11)],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;COUNT=3;BYDAY=TU,WE,TH;BYSETPOS=3",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(3),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Wednesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                    ],
                    bysetpos: vec![3],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-2",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Monday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Wednesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Thursday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Friday,
                        },
                    ],
                    bysetpos: vec![-2],
                    ..Default::default()
                },
            ),
            (
                "FREQ=HOURLY;INTERVAL=3;UNTIL=19970902T170000Z",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Hourly,
                    until: Some(PartialDateTime {
                        year: Some(1997),
                        month: Some(9),
                        day: Some(2),
                        hour: Some(17),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    }),
                    interval: Some(3),
                    ..Default::default()
                },
            ),
            (
                "FREQ=MINUTELY;INTERVAL=15;COUNT=6",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Minutely,
                    count: Some(6),
                    interval: Some(15),
                    ..Default::default()
                },
            ),
            (
                "FREQ=MINUTELY;INTERVAL=90;COUNT=4",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Minutely,
                    count: Some(4),
                    interval: Some(90),
                    ..Default::default()
                },
            ),
            (
                "FREQ=DAILY;BYHOUR=9,10,11,12,13,14,15,16;BYMINUTE=0,20,40",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Daily,
                    byminute: vec![0, 20, 40],
                    byhour: vec![9, 10, 11, 12, 13, 14, 15, 16],
                    ..Default::default()
                },
            ),
            (
                "FREQ=MINUTELY;INTERVAL=20;BYHOUR=9,10,11,12,13,14,15,16",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Minutely,
                    interval: Some(20),
                    byhour: vec![9, 10, 11, 12, 13, 14, 15, 16],
                    ..Default::default()
                },
            ),
            (
                "FREQ=WEEKLY;INTERVAL=2;COUNT=4;BYDAY=TU,SU;WKST=MO",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Weekly,
                    count: Some(4),
                    interval: Some(2),
                    byday: vec![
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Tuesday,
                        },
                        ICalendarDay {
                            ordwk: None,
                            weekday: ICalendarWeekday::Sunday,
                        },
                    ],
                    wkst: Some(ICalendarWeekday::Monday),
                    ..Default::default()
                },
            ),
            (
                "FREQ=MONTHLY;BYMONTHDAY=15,30;COUNT=5",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Monthly,
                    count: Some(5),
                    bymonthday: vec![15, 30],
                    ..Default::default()
                },
            ),
        ] {
            assert_eq!(
                Parser::new(rule).strict().rrule().unwrap(),
                expected,
                "failed for {rule}"
            );
        }
    }

    #[test]
    fn test_parse_period() {
        for (rule, expected) in [
            (
                "19970308T160000Z/PT8H30M",
                ICalendarPeriod::Duration {
                    start: PartialDateTime {
                        year: Some(1997),
                        month: Some(3),
                        day: Some(8),
                        hour: Some(16),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                    duration: ICalendarDuration {
                        neg: false,
                        weeks: 0,
                        days: 0,
                        hours: 8,
                        minutes: 30,
                        seconds: 0,
                    },
                },
            ),
            (
                "19970308T160000/PT3H",
                ICalendarPeriod::Duration {
                    start: PartialDateTime {
                        year: Some(1997),
                        month: Some(3),
                        day: Some(8),
                        hour: Some(16),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: None,
                        tz_minute: None,
                        tz_minus: false,
                    },
                    duration: ICalendarDuration {
                        neg: false,
                        weeks: 0,
                        days: 0,
                        hours: 3,
                        minutes: 0,
                        seconds: 0,
                    },
                },
            ),
            (
                "19970308T200000Z/PT1H",
                ICalendarPeriod::Duration {
                    start: PartialDateTime {
                        year: Some(1997),
                        month: Some(3),
                        day: Some(8),
                        hour: Some(20),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                    duration: ICalendarDuration {
                        neg: false,
                        weeks: 0,
                        days: 0,
                        hours: 1,
                        minutes: 0,
                        seconds: 0,
                    },
                },
            ),
            (
                "19970308T230000Z/19970309T000000Z",
                ICalendarPeriod::Range {
                    start: PartialDateTime {
                        year: Some(1997),
                        month: Some(3),
                        day: Some(8),
                        hour: Some(23),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                    end: PartialDateTime {
                        year: Some(1997),
                        month: Some(3),
                        day: Some(9),
                        hour: Some(0),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                },
            ),
            (
                "19970101T180000Z/PT5H30M",
                ICalendarPeriod::Duration {
                    start: PartialDateTime {
                        year: Some(1997),
                        month: Some(1),
                        day: Some(1),
                        hour: Some(18),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                    duration: ICalendarDuration {
                        neg: false,
                        weeks: 0,
                        days: 0,
                        hours: 5,
                        minutes: 30,
                        seconds: 0,
                    },
                },
            ),
            (
                "19971015T050000Z/PT8H30M",
                ICalendarPeriod::Duration {
                    start: PartialDateTime {
                        year: Some(1997),
                        month: Some(10),
                        day: Some(15),
                        hour: Some(5),
                        minute: Some(0),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                    duration: ICalendarDuration {
                        neg: false,
                        weeks: 0,
                        days: 0,
                        hours: 8,
                        minutes: 30,
                        seconds: 0,
                    },
                },
            ),
            (
                "19980314T233000Z/19980315T003000Z",
                ICalendarPeriod::Range {
                    start: PartialDateTime {
                        year: Some(1998),
                        month: Some(3),
                        day: Some(14),
                        hour: Some(23),
                        minute: Some(30),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                    end: PartialDateTime {
                        year: Some(1998),
                        month: Some(3),
                        day: Some(15),
                        hour: Some(0),
                        minute: Some(30),
                        second: Some(0),
                        tz_hour: Some(0),
                        tz_minute: Some(0),
                        tz_minus: false,
                    },
                },
            ),
        ] {
            let mut parser = Parser::new(rule);
            let token = parser.token().unwrap();

            assert_eq!(
                ICalendarPeriod::parse(token.text.as_ref()).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn test_parse_duration() {
        for (rule, expected) in [
            (
                "P1W3DT4H5M6S",
                ICalendarDuration {
                    neg: false,
                    weeks: 1,
                    days: 3,
                    hours: 4,
                    minutes: 5,
                    seconds: 6,
                },
            ),
            (
                "P15DT5H0M20S",
                ICalendarDuration {
                    neg: false,
                    weeks: 0,
                    days: 15,
                    hours: 5,
                    minutes: 0,
                    seconds: 20,
                },
            ),
            (
                "P7W",
                ICalendarDuration {
                    neg: false,
                    weeks: 7,
                    days: 0,
                    hours: 0,
                    minutes: 0,
                    seconds: 0,
                },
            ),
            (
                "-P7W",
                ICalendarDuration {
                    neg: true,
                    weeks: 7,
                    days: 0,
                    hours: 0,
                    minutes: 0,
                    seconds: 0,
                },
            ),
            (
                "PT0S",
                ICalendarDuration {
                    neg: false,
                    weeks: 0,
                    days: 0,
                    hours: 0,
                    minutes: 0,
                    seconds: 0,
                },
            ),
        ] {
            let mut parser = Parser::new(rule);
            let token = parser.token().unwrap();

            assert_eq!(
                ICalendarDuration::parse(token.text.as_ref()).expect(rule),
                expected,
                "Failed to parse: {rule}",
            );
        }

        assert!(ICalendarDuration::parse(b"P").is_none());

        // Duration zero
        let zero = ICalendarDuration::default();
        assert_eq!(zero.to_string(), "PT0S");

        let reparsed = ICalendarDuration::parse(zero.to_string().as_bytes()).unwrap();
        assert_eq!(reparsed, zero);
    }

    fn entries(input: &str) -> Vec<Entry> {
        let mut parser = Parser::new(input);
        let mut entries = Vec::new();
        loop {
            match parser.entry() {
                Entry::Eof => return entries,
                entry => entries.push(entry),
            }
        }
    }

    #[test]
    fn test_parse_ical_date_shapes() {
        for text in [
            "20250101",
            "20250101T090000Z",
            "202501011",
            "20251332",
            "00000000",
            "2025010",
            "2025010a",
            "2025-01-01",
            " 20250101",
            "2025 0101",
            "2025:101",
            "202501/1",
            "\u{661}0250101",
            "",
        ] {
            let token = Token {
                text: TokenText::Borrowed(text),
                start: 0,
                end: 0,
                stop_char: StopChar::Lf,
            };
            let mut expected = PartialDateTime::default();
            let expected = if expected.parse_ical_date(&mut text.as_bytes().iter().peekable()) {
                Ok(expected)
            } else {
                Err(text.to_string())
            };
            assert_eq!(token.into_ical_date(), expected, "{text:?}");
        }
    }

    #[test]
    fn test_parse_parameter_carry_over() {
        const BEGIN: &str = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n";
        const END: &str = "END:VEVENT\r\nEND:VCALENDAR\r\n";
        for (input, equivalent) in [
            (
                "ATTENDEE;=a,b;CN=c:mailto:x\r\n",
                "ATTENDEE;CN=a,b,c:mailto:x\r\n",
            ),
            (
                "ATTENDEE;=a;ROLE:mailto:x\r\n",
                "ATTENDEE;ROLE=a:mailto:x\r\n",
            ),
            (
                "ATTACH;BASE64=a;FMTTYPE=text/plain:SGVsbG8=\r\n",
                "ATTACH;ENCODING=BASE64;FMTTYPE=a,text/plain:SGVsbG8=\r\n",
            ),
            ("SUMMARY;=a;X-A=b:x\r\n", "SUMMARY;X-A=a,b:x\r\n"),
            (
                "SUMMARY;=a:x\r\nATTENDEE;CN=b:mailto:y\r\n",
                "SUMMARY:x\r\nATTENDEE;CN=a,b:mailto:y\r\n",
            ),
            (
                "DTSTART;=x;VALUE=DATE:20250101\r\n",
                "DTSTART;VALUE=x,DATE:20250101\r\n",
            ),
            (
                "DESCRIPTION;=x;ENCODING=QUOTED-PRINTABLE:a=3Db\r\n",
                "DESCRIPTION;ENCODING=x,QUOTED-PRINTABLE:a=3Db\r\n",
            ),
            (
                "DESCRIPTION;ENCODING=QUOTED-PRINTABLE;=x;CHARSET=utf-8:a=C3=A9\r\n",
                "DESCRIPTION;ENCODING=QUOTED-PRINTABLE;CHARSET=x,utf-8:a=C3=A9\r\n",
            ),
        ] {
            assert_eq!(
                entries(&format!("{BEGIN}{input}{END}")),
                entries(&format!("{BEGIN}{equivalent}{END}")),
                "{input:?}"
            );
        }
        assert_eq!(
            entries(concat!(
                "BEGIN:VCALENDAR\r\nSUMMARY;=a:x\r\nEND:VCALENDAR\r\n",
                "BEGIN:VCARD\r\nTEL;TYPE=home:1\r\nEND:VCARD\r\n"
            )),
            entries(concat!(
                "BEGIN:VCALENDAR\r\nSUMMARY:x\r\nEND:VCALENDAR\r\n",
                "BEGIN:VCARD\r\nTEL;TYPE=a,home:1\r\nEND:VCARD\r\n"
            ))
        );
    }

    #[test]
    fn test_parse_deep_nesting() {
        const DEPTH: usize = 11;
        let level_type = |level: usize| ICalendarComponentType::Other(format!("X-LEVEL-{level}"));
        let input = std::iter::once("BEGIN:VCALENDAR\r\n".to_string())
            .chain(
                (1..=DEPTH)
                    .map(|level| format!("BEGIN:X-LEVEL-{level}\r\nSUMMARY:open {level}\r\n")),
            )
            .chain(
                (1..=DEPTH)
                    .rev()
                    .map(|level| format!("END:X-LEVEL-{level}\r\nSUMMARY:closed {level}\r\n")),
            )
            .chain(std::iter::once("END:VCALENDAR\r\n".to_string()))
            .collect::<String>();
        let ical = ICalendar::parse(&input).expect("valid calendar");
        assert_eq!(ical.components.len(), DEPTH + 1);
        for (depth, component) in ical.components.iter().enumerate() {
            let summaries = component
                .entries
                .iter()
                .filter_map(|entry| entry.values.first().and_then(ICalendarValue::as_text))
                .collect::<Vec<_>>();
            let (component_type, expected_summaries, component_ids) = match depth {
                0 => (
                    ICalendarComponentType::VCalendar,
                    vec!["closed 1".to_string()],
                    vec![1],
                ),
                DEPTH => (level_type(depth), vec![format!("open {depth}")], vec![]),
                _ => (
                    level_type(depth),
                    vec![format!("open {depth}"), format!("closed {}", depth + 1)],
                    vec![depth as u32 + 1],
                ),
            };
            assert_eq!(component.component_type, component_type, "{depth}");
            assert_eq!(summaries, expected_summaries, "{depth}");
            assert_eq!(component.component_ids, component_ids, "{depth}");
        }

        let open = (1..DEPTH)
            .map(|level| format!("BEGIN:X-LEVEL-{level}\r\n"))
            .collect::<String>();
        let mismatched =
            format!("BEGIN:VCALENDAR\r\n{open}END:X-LEVEL-3\r\nSUMMARY:after\r\nEND:X-LEVEL-9\r\n");
        assert_eq!(
            Parser::new(&mismatched).strict().entry(),
            Entry::UnexpectedComponentEnd {
                expected: level_type(DEPTH - 1),
                found: level_type(3),
            }
        );
        let ical = ICalendar::parse(&mismatched).expect("lenient parse");
        assert_eq!(
            ical.components
                .iter()
                .map(|component| component.entries.len())
                .collect::<Vec<_>>(),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0]
        );
    }

    #[test]
    fn test_parse_encoded_values() {
        let binary = |bytes: &[u8]| ICalendarValue::Binary(bytes.to_vec());
        let text = |text: &str| ICalendarValue::Text(text.to_string());
        let binary_type = ICalendarParameter {
            name: ICalendarParameterName::Value,
            value: ICalendarParameterValue::Value(ICalendarValueType::Binary),
        };
        for (line, expected, typed_binary) in [
            (
                "ATTACH;ENCODING=BASE64;VALUE=BINARY:SGVs\r\n bG8=",
                binary(b"Hello"),
                true,
            ),
            ("ATTACH;ENCODING=BASE64:SGVsbG8=", binary(b"Hello"), true),
            (
                "DESCRIPTION;ENCODING=BASE64:SGVs\r\n\tbG8=",
                text("Hello"),
                false,
            ),
            ("DESCRIPTION;ENCODING=BASE64:/w==", binary(&[0xff]), true),
            (
                "DESCRIPTION;ENCODING=QUOTED-PRINTABLE:caf=E9",
                text("caf\u{e9}"),
                false,
            ),
            (
                "DESCRIPTION;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:caf=C3=\r\n=A9",
                text("caf\u{e9}"),
                false,
            ),
            ("X-DATA;ENCODING=BASE64:QU*D", text("QU*D"), false),
        ] {
            let input = format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n{line}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            );
            let ical = ICalendar::parse(&input).expect("valid calendar");
            let entry = ical
                .components
                .iter()
                .flat_map(|component| &component.entries)
                .next()
                .expect("one entry");
            assert_eq!(entry.values.as_slice(), [expected], "{line:?}");
            assert_eq!(
                entry.params.contains(&binary_type),
                typed_binary,
                "{line:?}"
            );
        }
    }

    #[test]
    fn test_month_new_saturates_out_of_range_months() {
        for month in 0..=u8::MAX {
            for is_leap in [false, true] {
                let value = ICalendarMonth::new(month, is_leap);
                assert_eq!(value.month(), month.min(127), "{month} {is_leap}");
                assert_eq!(value.is_leap(), is_leap && month != 0, "{month} {is_leap}");
                if let Ok(month) = i8::try_from(month) {
                    assert_eq!(value.0, if is_leap { -month } else { month });
                }
            }
        }
    }

    #[test]
    fn test_entry_presize_is_bounded_by_the_input() {
        const COMPONENTS: usize = 20_000;
        const FIRST_PUSH_CAPACITY: usize = 4;
        let events = "BEGIN:VEVENT\nX:1\nEND:VEVENT\n".repeat(COMPONENTS);
        let ical = ICalendar::parse(format!("BEGIN:VCALENDAR\n{events}END:VCALENDAR\n"))
            .expect("valid calendar");
        let events = ical.components.iter().skip(1);
        assert_eq!(
            events.clone().filter(|c| c.entries.len() == 1).count(),
            COMPONENTS
        );
        let capacity = events.map(|c| c.entries.capacity()).sum::<usize>();
        assert!(
            capacity <= 2 * FIRST_PUSH_CAPACITY * COMPONENTS,
            "{COMPONENTS} one-entry components reserve {capacity} entry slots"
        );

        let event = concat!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example Corp.//Example Client//EN\r\n",
            "BEGIN:VEVENT\r\nUID:6D3B9A2E-4C1F-4B8E-9E57-2A1C0F7D8B34\r\n",
            "DTSTAMP:20250113T091631Z\r\nDTSTART;TZID=Europe/Berlin:20250121T103000\r\n",
            "DTEND;TZID=Europe/Berlin:20250121T113000\r\nSUMMARY:Quarterly planning review\r\n",
            "LOCATION:Meeting Room 4.12\r\nDESCRIPTION:Bring the Q4 numbers and the hiring plan.\r\n",
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\n",
            "END:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let ical = ICalendar::parse(event).expect("valid calendar");
        assert_eq!(
            ical.components
                .iter()
                .map(|c| (c.entries.len(), c.entries.capacity()))
                .collect::<Vec<_>>(),
            [
                (2, ICalendarComponentType::VCalendar.entry_capacity()),
                (7, ICalendarComponentType::VEvent.entry_capacity()),
                (3, ICalendarComponentType::VAlarm.entry_capacity())
            ]
        );
    }
}
