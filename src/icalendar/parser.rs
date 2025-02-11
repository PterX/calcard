use std::{borrow::Cow, iter::Peekable, slice::Iter};

use mail_parser::decoders::{
    base64::base64_decode, charsets::map::charset_decoder,
    quoted_printable::quoted_printable_decode,
};

use crate::{
    common::{
        parser::{parse_digits, Integer, Timestamp},
        CalendarScale, Encoding, PartialDateTime,
    },
    icalendar::{ICalendarDay, ICalendarWeekday},
    Entry, Parser, StopChar, Token,
};

use super::*;

struct Params {
    params: Vec<ICalendarParameter>,
    stop_char: StopChar,
    data_type: Option<ICalendarValueType>,
    charset: Option<String>,
    encoding: Option<Encoding>,
}

impl Parser<'_> {
    pub fn icalendar(&mut self) -> Entry {
        let mut ical = ICalendar::default();
        let mut ical_stack = Vec::new();

        loop {
            // Fetch property name
            self.expect_iana_token();
            let token = match self.token() {
                Some(token) => token,
                None => break,
            };

            let mut params = Params {
                params: Vec::new(),
                stop_char: token.stop_char,
                data_type: None,
                encoding: None,
                charset: None,
            };

            // Parse parameters
            let name = token.text;
            match params.stop_char {
                StopChar::Semicolon => {
                    self.ical_parameters(&mut params);
                }
                StopChar::Colon => {}
                StopChar::Lf => {
                    // Invalid line
                    if name.is_empty() || !self.strict {
                        continue;
                    } else {
                        return Entry::InvalidLine(Token::new(name).into_string());
                    }
                }
                _ => {}
            }

            // Invalid stop char, try seeking colon
            if !matches!(params.stop_char, StopChar::Colon | StopChar::Lf) {
                params.stop_char = self.seek_value_or_eol();
            }

            // Parse property
            let name = match ICalendarProperty::try_from(name.as_ref()) {
                Ok(ICalendarProperty::Begin) => {
                    if params.stop_char == StopChar::Colon {
                        self.expect_single_value();
                        if let Some(token) = self.token() {
                            if let Ok(component_type) =
                                ICalendarComponentType::try_from(token.text.as_ref())
                            {
                                ical_stack.push(ical);
                                ical = ICalendar {
                                    component_type,
                                    ..Default::default()
                                };
                            } else if self.strict {
                                return Entry::InvalidComponentType(token.into_string());
                            }
                        }
                    }

                    if !self.strict {
                        continue;
                    } else {
                        return Entry::InvalidLine("BEGIN".to_string());
                    }
                }
                Ok(ICalendarProperty::End) => {
                    if params.stop_char == StopChar::Colon {
                        self.expect_single_value();
                        if let Some(token) = self.token() {
                            if let Ok(component_type) =
                                ICalendarComponentType::try_from(token.text.as_ref())
                            {
                                if ical.component_type == component_type || !self.strict {
                                    if let Some(mut parent) = ical_stack.pop() {
                                        parent.components.push(ical);
                                        ical = parent;
                                    } else {
                                        break;
                                    }
                                } else {
                                    return Entry::UnexpectedComponentEnd {
                                        expected: ical.component_type,
                                        found: component_type,
                                    };
                                }
                            } else if self.strict {
                                return Entry::InvalidComponentType(token.into_string());
                            }
                        }
                    }

                    if !self.strict {
                        continue;
                    } else {
                        return Entry::InvalidLine("END".to_string());
                    }
                }
                Ok(name) => name,
                Err(_) => {
                    if !name.is_empty() {
                        ICalendarProperty::Other(Token::new(name).into_string())
                    } else {
                        // Invalid line, skip
                        if params.stop_char != StopChar::Lf {
                            self.seek_lf();
                        }
                        continue;
                    }
                }
            };
            let mut entry = ICalendarEntry {
                name,
                params: params.params,
                values: Vec::new(),
            };

            // Parse value
            if params.stop_char != StopChar::Lf {
                // Obtain default type and separator
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

                // Decode
                match params.encoding {
                    Some(Encoding::Base64) if multi_value != ValueSeparator::None => {
                        self.expect_single_value();
                    }
                    Some(Encoding::QuotedPrintable) => {
                        self.unfold_qp = true;
                    }
                    _ => {}
                }

                if matches!(
                    (&params.data_type, &default_type),
                    (Some(ICalendarValueType::Recur), _)
                        | (None, ValueType::Ical(ICalendarValueType::Recur))
                ) {
                    match self.rrule() {
                        Ok(rrule) => {
                            entry
                                .values
                                .push(ICalendarValue::RecurrenceRule(Box::new(rrule)));
                        }
                        Err(other) => {
                            entry.values.push(ICalendarValue::Text(other));
                        }
                    }
                } else {
                    while let Some(mut token) = self.token() {
                        let eol = token.stop_char == StopChar::Lf;

                        // Decode binary parts
                        if let Some(encoding) = params.encoding {
                            let (bytes, default_encoding) = match encoding {
                                Encoding::Base64 => (base64_decode(&token.text), None),
                                Encoding::QuotedPrintable => {
                                    (quoted_printable_decode(&token.text), "iso-8859-1".into())
                                }
                            };
                            if let Some(bytes) = bytes {
                                if let Some(decoded) =
                                    params.charset.as_deref().or(default_encoding).and_then(
                                        |charset| {
                                            charset_decoder(charset.as_bytes())
                                                .map(|decoder| decoder(&bytes))
                                        },
                                    )
                                {
                                    token.text = Cow::Owned(decoded.into_bytes());
                                } else if std::str::from_utf8(&bytes).is_ok() {
                                    token.text = Cow::Owned(bytes);
                                } else {
                                    entry.values.push(ICalendarValue::Binary(bytes));
                                    if eol {
                                        break;
                                    } else {
                                        continue;
                                    }
                                }
                            }
                        }

                        match &default_type {
                            ValueType::Ical(default_type) => {
                                let value = match params.data_type.as_ref().unwrap_or(default_type)
                                {
                                    ICalendarValueType::Date => token
                                        .into_ical_date()
                                        .map(|data| ICalendarValue::PartialDateTime(Box::new(data)))
                                        .unwrap_or_else(ICalendarValue::Text),
                                    ICalendarValueType::DateTime => token
                                        .into_timestamp()
                                        .map(|data| ICalendarValue::PartialDateTime(Box::new(data)))
                                        .unwrap_or_else(ICalendarValue::Text),
                                    ICalendarValueType::Time => token
                                        .into_ical_time()
                                        .map(|data| ICalendarValue::PartialDateTime(Box::new(data)))
                                        .unwrap_or_else(ICalendarValue::Text),
                                    ICalendarValueType::UtcOffset => token
                                        .into_offset()
                                        .map(|data| ICalendarValue::PartialDateTime(Box::new(data)))
                                        .unwrap_or_else(ICalendarValue::Text),
                                    ICalendarValueType::Boolean => {
                                        ICalendarValue::Boolean(token.into_boolean())
                                    }
                                    ICalendarValueType::Float => token
                                        .into_float()
                                        .map(ICalendarValue::Float)
                                        .unwrap_or_else(ICalendarValue::Text),
                                    ICalendarValueType::Integer => token
                                        .into_integer()
                                        .map(ICalendarValue::Integer)
                                        .unwrap_or_else(ICalendarValue::Text),
                                    ICalendarValueType::Text
                                    | ICalendarValueType::Binary
                                    | ICalendarValueType::Unknown
                                    | ICalendarValueType::XmlReference
                                    | ICalendarValueType::Uid => {
                                        ICalendarValue::Text(token.into_string())
                                    }
                                    ICalendarValueType::Uri | ICalendarValueType::CalAddress => {
                                        token
                                            .into_uri_bytes()
                                            .map(|data| ICalendarValue::Uri(Uri::Data(data)))
                                            .unwrap_or_else(|uri| {
                                                ICalendarValue::Uri(Uri::Location(uri))
                                            })
                                    }
                                    ICalendarValueType::Other(_) => {
                                        ICalendarValue::Text(token.into_string())
                                    }
                                    ICalendarValueType::Duration => {
                                        if let Ok(duration) =
                                            ICalendarDuration::try_from(token.text.as_ref())
                                        {
                                            ICalendarValue::Duration(duration)
                                        } else {
                                            ICalendarValue::Text(token.into_string())
                                        }
                                    }
                                    ICalendarValueType::Period => {
                                        if let Ok(period) =
                                            ICalendarPeriod::try_from(token.text.as_ref())
                                        {
                                            ICalendarValue::Period(period)
                                        } else {
                                            ICalendarValue::Text(token.into_string())
                                        }
                                    }
                                    ICalendarValueType::Recur => unreachable!(),
                                };

                                entry.values.push(value);
                            }
                            ValueType::CalendarScale => {
                                entry.values.push(ICalendarValue::CalendarScale(
                                    CalendarScale::from(token),
                                ));
                            }
                            ValueType::Method => {
                                entry
                                    .values
                                    .push(ICalendarValue::Method(ICalendarMethod::from(token)));
                            }
                            ValueType::Classification => {
                                entry.values.push(ICalendarValue::Classification(
                                    ICalendarClassification::from(token),
                                ));
                            }
                            ValueType::Status => {
                                entry.values.push(ICalendarValue::Status(
                                    ICalendarParticipationStatus::from(token),
                                ));
                            }
                            ValueType::Transparency => {
                                entry.values.push(ICalendarValue::Transparency(
                                    ICalendarTransparency::from(token),
                                ));
                            }
                            ValueType::Action => {
                                entry
                                    .values
                                    .push(ICalendarValue::Action(ICalendarAction::from(token)));
                            }
                            ValueType::BusyType => {
                                entry.values.push(ICalendarValue::BusyType(
                                    ICalendarFreeBusyType::from(token),
                                ));
                            }
                            ValueType::ParticipantType => {
                                entry.values.push(ICalendarValue::ParticipantType(
                                    ICalendarParticipantType::from(token),
                                ));
                            }
                            ValueType::ResourceType => {
                                entry.values.push(ICalendarValue::ResourceType(
                                    ICalendarResourceType::from(token),
                                ));
                            }
                            ValueType::Proximity => {
                                entry.values.push(ICalendarValue::Proximity(
                                    ICalendarProximityValue::from(token),
                                ));
                            }
                        }

                        if eol {
                            break;
                        }
                    }
                }
            }

            // Add types
            if let Some(data_type) = params.data_type {
                entry.params.push(ICalendarParameter::Value(data_type));
            }

            ical.entries.push(entry);
        }

        if !ical_stack.is_empty() {
            if !self.strict {
                while let Some(mut parent) = ical_stack.pop() {
                    parent.components.push(ical);
                    ical = parent;
                }
            } else {
                return Entry::UnterminatedComponent(ical.component_type);
            }
        }

        Entry::ICalendar(ical)
    }

    fn ical_parameters(&mut self, params: &mut Params) {
        while params.stop_char == StopChar::Semicolon {
            self.expect_iana_token();
            let token = match self.token() {
                Some(token) => token,
                None => {
                    params.stop_char = StopChar::Lf;
                    break;
                }
            };

            // Obtain parameter values
            let param_name = token.text;
            params.stop_char = token.stop_char;
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
                                self.token_buf.push(token);
                            }
                            None => {
                                params.stop_char = StopChar::Lf;
                                break;
                            }
                        }
                    }
                }
            }

            let param_values = &mut params.params;

            hashify::fnc_map_ignore_case!(param_name.as_ref(),
                b"ALTREP" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Altrep(value));
                    }
                },
                b"CN" => {
                    param_values.push(ICalendarParameter::Cn(self.buf_to_string()));
                },
                b"CUTYPE" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Cutype(value));
                    }
                },
                b"DELEGATED-FROM" => {
                    param_values.push(ICalendarParameter::DelegatedFrom(self.buf_parse_many()));
                },
                b"DELEGATED-TO" => {
                    param_values.push(ICalendarParameter::DelegatedTo(self.buf_parse_many()));
                },
                b"DIR" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Dir(value));
                    }
                },
                b"FMTTYPE" => {
                    param_values.push(ICalendarParameter::Fmttype(self.buf_to_string()));
                },
                b"FBTYPE" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Fbtype(value));
                    }
                },
                b"LANGUAGE" => {
                    param_values.push(ICalendarParameter::Language(self.buf_to_string()));
                },
                b"MEMBER" => {
                    param_values.push(ICalendarParameter::Member(self.buf_parse_many()));
                },
                b"PARTSTAT" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Partstat(value));
                    }
                },
                b"RANGE" => {
                    if self.token_buf.first().is_some_and(|token| {
                        token.text.as_ref().eq_ignore_ascii_case(b"THISANDFUTURE")
                    }) {
                        param_values.push(ICalendarParameter::Range);
                    }
                    self.token_buf.clear();
                },
                b"RELATED" => {
                    if let Some(gap) = self.buf_try_parse_one() {
                        param_values.push(ICalendarParameter::Related(gap));
                    }
                },
                b"RELTYPE" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Reltype(value));
                    }
                },
                b"ROLE" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Role(value));
                    }
                },
                b"RSVP" => {
                    param_values.push(ICalendarParameter::Rsvp(self.buf_to_bool()));
                },
                b"SCHEDULE-AGENT" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::ScheduleAgent(value));
                    }
                },
                b"SCHEDULE-FORCE-SEND" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::ScheduleForceSend(value));
                    }
                },
                b"SCHEDULE-STATUS" => {
                    param_values.push(ICalendarParameter::ScheduleStatus(self.buf_to_string()));
                },
                b"SENT-BY" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::SentBy(value));
                    }
                },
                b"TZID" => {
                    param_values.push(ICalendarParameter::Tzid(self.buf_to_string()));
                },
                b"VALUE" => {
                    if let Some(value) = self.buf_parse_one::<ICalendarValueType>() {
                        params.data_type = value.into();
                    }
                },
                b"DISPLAY" => {
                    param_values.push(ICalendarParameter::Display(self.buf_parse_many()));
                },
                b"EMAIL" => {
                    param_values.push(ICalendarParameter::Email(self.buf_to_string()));
                },
                b"FEATURE" => {
                    param_values.push(ICalendarParameter::Feature(self.buf_parse_many()));
                },
                b"LABEL" => {
                    param_values.push(ICalendarParameter::Label(self.buf_to_string()));
                },
                b"SIZE" => {
                    param_values.push(ICalendarParameter::Size(self.buf_to_other().unwrap_or_default()));
                },
                b"FILENAME" => {
                    param_values.push(ICalendarParameter::Filename(self.buf_to_string()));
                },
                b"MANAGED-ID" => {
                    param_values.push(ICalendarParameter::ManagedId(self.buf_to_string()));
                },
                b"ORDER" => {
                    param_values.push(ICalendarParameter::Order(self.buf_to_other().unwrap_or_default()));
                },
                b"SCHEMA" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Schema(value));
                    }
                },
                b"DERIVED" => {
                    param_values.push(ICalendarParameter::Derived(self.buf_to_bool()));
                },
                b"GAP" => {
                    if let Some(gap) = self.buf_try_parse_one() {
                        param_values.push(ICalendarParameter::Gap(gap));
                    }
                },
                b"LINKREL" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(ICalendarParameter::Linkrel(value));
                    }
                },
                b"CHARSET" => {
                    for token in self.token_buf.drain(..) {
                        params.charset = token.into_string().into();
                    }
                },
                b"ENCODING" => {
                    for token in self.token_buf.drain(..) {
                        params.encoding = Encoding::parse(token.text.as_ref());
                    }
                },
                _ => {
                    if !param_name.is_empty() {
                        if params.encoding.is_none() && param_name.eq_ignore_ascii_case(b"base64") {
                            params.encoding = Some(Encoding::Base64);
                        } else {
                            param_values.push(ICalendarParameter::Other(
                                [Token::new(param_name).into_string()]
                                    .into_iter()
                                    .chain(self.token_buf.drain(..).map(|token| token.into_string()))
                                    .collect(),
                            ));
                        }
                    }
                }
            );
        }
    }

    fn rrule(&mut self) -> Result<ICalendarRecurrenceRule, String> {
        self.expect_rrule_value();

        let mut last_stop_char = StopChar::Equal;
        let mut is_valid = true;
        let mut has_freq = false;
        let mut rrule = ICalendarRecurrenceRule::default();

        let mut token_start = usize::MAX;
        let mut token_end = usize::MAX;

        while let Some(mut token) = self.token_until_lf(&mut last_stop_char) {
            if token_start == usize::MAX {
                token_start = token.start;
            }
            token_end = token.end;
            if !is_valid {
                continue;
            }
            if token.stop_char != StopChar::Equal {
                if !self.strict {
                    // Ignore unknown tokens
                    while let Some(token_) = self.token_until_lf(&mut last_stop_char) {
                        token_end = token.end;
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
                    while let Some(value) = self.parse_value_until_lf(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.freq = value;
                            has_freq = true;
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"UNTIL" => {
                    while let Some(value) = self.parse_value_until_lf::<Timestamp>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.until = Some(value.0);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"COUNT" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.count = Some(value.0.unsigned_abs() as u32);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"INTERVAL" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.interval = Some(value.0.unsigned_abs() as u32);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYSECOND" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.bysecond.push(value.0.unsigned_abs() as u16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYMINUTE" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.byminute.push(value.0.unsigned_abs() as u16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYHOUR" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.byhour.push(value.0.unsigned_abs() as u16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYDAY" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarDay>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.byday.push(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYMONTHDAY" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.bymonthday.push(value.0 as i16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYYEARDAY" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.byyearday.push(value.0 as i16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYWEEKNO" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.byweekno.push(value.0 as i16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYMONTH" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.bymonth.push(value.0 as i16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"BYSETPOS" => {
                    while let Some(value) = self.parse_value_until_lf::<Integer>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.bysetpos.push(value.0 as i16);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                b"WKST" => {
                    while let Some(value) = self.parse_value_until_lf::<ICalendarWeekday>(StopChar::Semicolon, &mut last_stop_char) {
                        token_end = token.end;
                        if let Ok(value) = value {
                            rrule.wkst = Some(value);
                        } else if !self.strict {
                            is_valid = false;
                        }
                    }
                },
                _ => {
                    if !self.strict {
                        // Ignore unknown tokens
                        while let Some(token) = self.token_until_lf(&mut last_stop_char) {
                            token_end = token.end;
                            if token.stop_char == StopChar::Semicolon{
                                break;
                            }
                        }
                    } else {
                        is_valid = false;
                    }
                }
            );
        }

        if has_freq {
            Ok(rrule)
        } else if token_start != usize::MAX {
            Err(self
                .input
                .get(token_start..=token_end)
                .map(|slice| String::from_utf8_lossy(slice).into_owned())
                .unwrap_or_default())
        } else {
            Err("".to_string())
        }
    }
}

impl Token<'_> {
    pub(crate) fn into_ical_date(self) -> std::result::Result<PartialDateTime, String> {
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
    fn parse_ical_date(&mut self, iter: &mut Peekable<Iter<u8>>) -> bool {
        parse_digits(iter, &mut self.year, 4, false)
            && parse_digits(iter, &mut self.month, 2, false)
            && parse_digits(iter, &mut self.day, 2, false)
    }

    fn parse_ical_time(&mut self, iter: &mut Peekable<Iter<u8>>) -> bool {
        if parse_digits(iter, &mut self.hour, 2, false)
            && parse_digits(iter, &mut self.minute, 2, false)
            && parse_digits(iter, &mut self.second, 2, false)
        {
            self.parse_zone(iter);
            true
        } else {
            false
        }
    }
}

impl TryFrom<&[u8]> for ICalendarPeriod {
    type Error = ();

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let mut iter = value.iter().peekable();
        let mut start = PartialDateTime::default();
        if start.parse_timestamp(&mut iter) {
            if let Some(duration) = ICalendarDuration::try_parse(&mut iter) {
                Ok(ICalendarPeriod::Duration {
                    start: start.to_timestamp().unwrap_or_default(),
                    duration,
                })
            } else {
                let mut end = PartialDateTime::default();
                if end.parse_timestamp(&mut iter) {
                    Ok(ICalendarPeriod::Range {
                        start: start.to_timestamp().unwrap_or_default(),
                        end: end.to_timestamp().unwrap_or_default(),
                    })
                } else {
                    Err(())
                }
            }
        } else {
            Err(())
        }
    }
}

impl ICalendarDuration {
    fn try_parse(iter: &mut Peekable<Iter<u8>>) -> Option<Self> {
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
        for ch in iter {
            match ch {
                b'0'..=b'9' => {
                    num = num.saturating_mul(10).saturating_add((ch - b'0') as u32);
                }
                b'T' | b't' => {}
                b'W' | b'w' => {
                    dur.weeks = num;
                    num = 0;
                }
                b'D' | b'd' => {
                    dur.days = num;
                    num = 0;
                }
                b'H' | b'h' => {
                    dur.hours = num;
                    num = 0;
                }
                b'M' | b'm' => {
                    dur.minutes = num;
                    num = 0;
                }
                b'S' | b's' => {
                    dur.seconds = num;
                    num = 0;
                }
                _ => {
                    if !ch.is_ascii_whitespace() {
                        return None;
                    }
                }
            }
        }

        if !dur.is_empty() {
            Some(dur)
        } else {
            None
        }
    }
}

impl TryFrom<&[u8]> for ICalendarDuration {
    type Error = ();

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if let Some(duration) = ICalendarDuration::try_parse(&mut value.iter().peekable()) {
            Ok(duration)
        } else {
            Err(())
        }
    }
}

impl ICalendarDuration {
    pub fn is_empty(&self) -> bool {
        self.weeks == 0
            && self.days == 0
            && self.hours == 0
            && self.minutes == 0
            && self.seconds == 0
    }
}

impl From<Token<'_>> for Uri {
    fn from(token: Token<'_>) -> Self {
        token
            .into_uri_bytes()
            .map(Uri::Data)
            .unwrap_or_else(Uri::Location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    bymonth: vec![1, 2],
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
                    bymonth: vec![1],
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
                    until: Some(104914800),
                    byday: vec![ICalendarDay {
                        ordwk: Some(-1),
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![4],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU;UNTIL=20061029T060000Z",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    until: Some(1162101600),
                    byday: vec![ICalendarDay {
                        ordwk: Some(-1),
                        weekday: ICalendarWeekday::Sunday,
                    }],
                    bymonth: vec![10],
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
                    bymonth: vec![3],
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
                    bymonth: vec![10],
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
                    until: Some(949327200),
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
                    bymonth: vec![1],
                    ..Default::default()
                },
            ),
            (
                "FREQ=DAILY;UNTIL=20000131T140000Z;BYMONTH=1",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Daily,
                    until: Some(949327200),
                    bymonth: vec![1],
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
                    until: Some(876182400),
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
                    until: Some(882921600),
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
                    bymonth: vec![6, 7],
                    ..Default::default()
                },
            ),
            (
                "FREQ=YEARLY;INTERVAL=2;COUNT=10;BYMONTH=1,2,3",
                ICalendarRecurrenceRule {
                    freq: ICalendarFrequency::Yearly,
                    count: Some(10),
                    interval: Some(2),
                    bymonth: vec![1, 2, 3],
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
                    bymonth: vec![3],
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
                    bymonth: vec![6, 7, 8],
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
                    bymonth: vec![11],
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
                    until: Some(873219600),
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
                Parser::new(rule.as_bytes()).strict().rrule().unwrap(),
                expected
            );
        }
    }

    #[test]
    fn test_parse_period() {
        for (rule, expected) in [
            (
                "19970308T160000Z/PT8H30M",
                ICalendarPeriod::Duration {
                    start: 857836800,
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
                "19970308T160000Z/PT3H",
                ICalendarPeriod::Duration {
                    start: 857836800,
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
                    start: 857851200,
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
                    start: 857862000,
                    end: 857865600,
                },
            ),
            (
                "19970101T180000Z/PT5H30M",
                ICalendarPeriod::Duration {
                    start: 852141600,
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
                    start: 876891600,
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
                    start: 889918200,
                    end: 889921800,
                },
            ),
        ] {
            let mut parser = Parser::new(rule.as_bytes());
            let token = parser.token().unwrap();

            assert_eq!(
                ICalendarPeriod::try_from(token.text.as_ref()).unwrap(),
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
        ] {
            let mut parser = Parser::new(rule.as_bytes());
            let token = parser.token().unwrap();

            assert_eq!(
                ICalendarDuration::try_from(token.text.as_ref()).expect(rule),
                expected,
                "Failed to parse: {rule}",
            );
        }
    }
}
