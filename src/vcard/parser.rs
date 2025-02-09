use std::{borrow::Cow, iter::Peekable, slice::Iter, str::FromStr};

use mail_parser::{
    decoders::{
        base64::base64_decode, charsets::map::charset_decoder,
        quoted_printable::quoted_printable_decode,
    },
    DateTime,
};

use crate::{
    common::{tokenizer::StopChar, Data, Encoding},
    vcard::VCardProperty,
    Parser, Token,
};

use super::{
    VCard, VCardEntry, VCardParameter, VCardPartialDateTime, VCardType, VCardValue, VCardValueType,
    ValueSeparator, ValueType,
};

struct Params {
    params: Vec<VCardParameter>,
    stop_char: StopChar,
    data_types: Vec<VCardValueType>,
    charset: Option<String>,
    encoding: Option<Encoding>,
    group_name: Option<String>,
}

impl Parser<'_> {
    pub fn vcard(&mut self) -> VCard {
        let mut vcard = VCard::default();
        let mut is_v4 = true;

        'outer: loop {
            // Fetch property name
            self.expect_iana_token();
            let mut token = match self.token() {
                Some(token) => token,
                None => break,
            };

            let mut params = Params {
                params: Vec::new(),
                stop_char: token.stop_char,
                data_types: Vec::new(),
                group_name: None,
                encoding: None,
                charset: None,
            };

            // Parse group name
            if matches!(token.stop_char, StopChar::Dot) {
                params.group_name = token.into_string().into();
                token = match self.token() {
                    Some(token) => token,
                    None => break,
                };
            }

            // Parse parameters
            let name = token.text;
            match params.stop_char {
                StopChar::Semicolon => {
                    self.vcard_parameters(&mut params);
                }
                StopChar::Colon => {}
                StopChar::Lf => {
                    // Invalid line
                    continue;
                }
                _ => {}
            }

            // Invalid stop char, try seeking colon
            if !matches!(params.stop_char, StopChar::Colon | StopChar::Lf) {
                params.stop_char = self.seek_value_or_eol();
            }

            // Parse property
            let name = match VCardProperty::try_from(name.as_ref()) {
                Ok(name) => name,
                Err(_) => {
                    if !name.is_empty() {
                        VCardProperty::Other(Token::new(name).into_string())
                    } else {
                        // Invalid line, skip
                        if params.stop_char != StopChar::Lf {
                            self.seek_lf();
                        }
                        continue;
                    }
                }
            };
            let mut entry = VCardEntry {
                group: params.group_name,
                name,
                params: params.params,
                values: Vec::new(),
            };

            // Parse value
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
                    ValueSeparator::Skip => {
                        self.expect_single_value();
                        self.token();
                        break 'outer;
                    }
                }
                match params.encoding {
                    Some(Encoding::Base64) if multi_value != ValueSeparator::None => {
                        self.expect_single_value();
                    }
                    Some(Encoding::QuotedPrintable) => {
                        self.unfold_qp = true;
                    }
                    _ => {}
                }

                let mut data_types = params.data_types.iter();
                let mut token_idx = 0;
                while let Some(mut token) = self.token() {
                    let eol = token.stop_char == StopChar::Lf;

                    // Decode old vCard
                    if let Some(encoding) = params.encoding {
                        let (bytes, default_encoding) = match encoding {
                            Encoding::Base64 => (base64_decode(&token.text), None),
                            Encoding::QuotedPrintable => {
                                (quoted_printable_decode(&token.text), "iso-8859-1".into())
                            }
                        };
                        if let Some(bytes) = bytes {
                            if let Some(decoded) = params
                                .charset
                                .as_deref()
                                .or(default_encoding)
                                .and_then(|charset| {
                                    charset_decoder(charset.as_bytes())
                                        .map(|decoder| decoder(&bytes))
                                })
                            {
                                token.text = Cow::Owned(decoded.into_bytes());
                            } else if std::str::from_utf8(&bytes).is_ok() {
                                token.text = Cow::Owned(bytes);
                            } else {
                                entry.values.push(VCardValue::Binary(Data {
                                    data: bytes,
                                    content_type: None,
                                }));
                                if eol {
                                    break;
                                } else {
                                    continue;
                                }
                            }
                        }
                    }

                    let default_type = match &default_type {
                        ValueType::Vcard(default_type) => default_type,
                        ValueType::Kind if token_idx == 0 => {
                            if let Ok(gram_gender) = token.text.as_ref().try_into() {
                                entry.values.push(VCardValue::Kind(gram_gender));
                                if eol {
                                    break;
                                } else {
                                    continue;
                                }
                            }
                            &VCardValueType::Text
                        }
                        ValueType::Sex if token_idx == 0 => {
                            if let Ok(gram_gender) = token.text.as_ref().try_into() {
                                entry.values.push(VCardValue::Sex(gram_gender));
                                if eol {
                                    break;
                                } else {
                                    continue;
                                }
                            }
                            &VCardValueType::Text
                        }
                        ValueType::GramGender if token_idx == 0 => {
                            if let Ok(gram_gender) = token.text.as_ref().try_into() {
                                entry.values.push(VCardValue::GramGender(gram_gender));
                                if eol {
                                    break;
                                } else {
                                    continue;
                                }
                            }
                            &VCardValueType::Text
                        }
                        _ => &VCardValueType::Text,
                    };

                    let value = match data_types.next().unwrap_or(default_type) {
                        VCardValueType::Date if is_v4 => token
                            .into_date()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::DateAndOrTime if is_v4 => token
                            .into_date_and_or_datetime()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::DateTime if is_v4 => token
                            .into_date_time()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::Time if is_v4 => token
                            .into_time()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::Timestamp if is_v4 => token
                            .into_timestamp()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::UtcOffset if is_v4 => token
                            .into_offset()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::Boolean => VCardValue::Boolean(token.into_boolean()),
                        VCardValueType::Float => token
                            .into_float()
                            .map(VCardValue::Float)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::Integer => token
                            .into_integer()
                            .map(VCardValue::Integer)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::LanguageTag => VCardValue::Text(token.into_string()),
                        VCardValueType::Text => {
                            if is_v4
                                && matches!(
                                    (&entry.name, token.text.first()),
                                    (VCardProperty::Version, Some(b'1'..=b'3'))
                                )
                            {
                                is_v4 = false;
                            }

                            VCardValue::Text(token.into_string())
                        }
                        VCardValueType::Uri => token
                            .into_uri_bytes()
                            .map(VCardValue::Binary)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::Other(_) => VCardValue::Text(token.into_string()),
                        // VCard 3.0 and older
                        VCardValueType::Date
                        | VCardValueType::DateAndOrTime
                        | VCardValueType::DateTime
                        | VCardValueType::Time => token
                            .into_datetime_or_legacy()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::Timestamp => token
                            .into_timestamp_or_legacy()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                        VCardValueType::UtcOffset => token
                            .into_offset_or_legacy()
                            .map(VCardValue::PartialDateTime)
                            .unwrap_or_else(VCardValue::Text),
                    };

                    entry.values.push(value);

                    if eol {
                        break;
                    }

                    token_idx += 1;
                }
            }

            // Add types
            if !params.data_types.is_empty() {
                entry.params.push(VCardParameter::Value(params.data_types));
            }

            vcard.entries.push(entry);
        }

        vcard
    }

    fn vcard_parameters(&mut self, params: &mut Params) {
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
                b"LANGUAGE" => {
                    param_values.push(VCardParameter::Language(self.buf_to_string()));
                },
                b"VALUE" => {
                    params.data_types.extend(
                        self.token_buf
                            .drain(..)
                            .map(Into::into),
                    );
                },
                b"PREF" => {
                    param_values.push(VCardParameter::Pref(self.buf_to_other().unwrap_or_default()));
                },
                b"ALTID" => {
                    param_values.push(VCardParameter::Altid(self.buf_to_string()));
                },
                b"PID" => {
                    param_values.push(VCardParameter::Pid(
                        self.token_buf
                            .drain(..)
                            .map(|token| token.into_string())
                            .collect(),
                    ));
                },
                b"TYPE" => {
                    let mut types = self.buf_parse_many();

                    // RFC6350 has many mistakes, this is a workaround for the "TYPE" values
                    // which in the examples sometimes appears between quotes.
                    match types.first() {
                        Some(VCardType::Other(text)) if types.len() == 1 && text.contains(",") => {
                            let mut types_ = Vec::with_capacity(2);
                            for text in text.split(',') {
                                if let Ok(typ) = VCardType::try_from(text.as_bytes()) {
                                    types_.push(typ);
                                }
                            }
                            types = types_;
                        }
                        _ => {}
                    }

                    if let Some(types_) = param_values.iter_mut().find_map(|param| {
                        if let VCardParameter::Type(types) = param {
                            Some(types)
                        } else {
                            None
                        }
                    }) {
                        types_.extend(types);
                    } else {
                        param_values.push(VCardParameter::Type(types));
                    }
                },
                b"MEDIATYPE" => {
                    param_values.push(VCardParameter::Mediatype(self.buf_to_string()));
                },
                b"CALSCALE" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(VCardParameter::Calscale(value));
                    }
                },
                b"SORT-AS" => {
                    param_values.push(VCardParameter::SortAs(self.buf_to_string()));
                },
                b"GEO" => {
                    param_values.push(VCardParameter::Geo(self.buf_to_string()));
                },
                b"TZ" => {
                    param_values.push(VCardParameter::Tz(self.buf_to_string()));
                },
                b"INDEX" => {
                    param_values.push(VCardParameter::Index(self.buf_to_other().unwrap_or_default()));
                },
                b"LEVEL" => {
                    if let Some(value) = self.buf_try_parse_one() {
                        param_values.push(VCardParameter::Level(value));
                    }
                },
                b"GROUP" => {
                    param_values.push(VCardParameter::Group(self.buf_to_string()));
                },
                b"CC" => {
                    param_values.push(VCardParameter::Cc(self.buf_to_string()));
                },
                b"AUTHOR" => {
                    param_values.push(VCardParameter::Author(self.buf_to_string()));
                },
                b"AUTHOR-NAME" => {
                    param_values.push(VCardParameter::AuthorName(self.buf_to_string()));
                },
                b"CREATED" => {
                    param_values.push(VCardParameter::Created(self.buf_to_other::<Timestamp>().unwrap_or_default().0));
                },
                b"DERIVED" => {
                    param_values.push(VCardParameter::Derived(self.buf_to_bool()));
                },
                b"LABEL" => {
                    param_values.push(VCardParameter::Label(self.buf_to_string()));
                },
                b"PHONETIC" => {
                    if let Some(value) = self.buf_parse_one() {
                        param_values.push(VCardParameter::Phonetic(value));
                    }
                },
                b"PROP-ID" => {
                    param_values.push(VCardParameter::PropId(self.buf_to_string()));
                },
                b"SCRIPT" => {
                    param_values.push(VCardParameter::Script(self.buf_to_string()));
                },
                b"SERVICE-TYPE" => {
                    param_values.push(VCardParameter::ServiceType(self.buf_to_string()));
                },
                b"USERNAME" => {
                    param_values.push(VCardParameter::Username(self.buf_to_string()));
                },
                b"JSPTR" => {
                    param_values.push(VCardParameter::Jsptr(self.buf_to_string()));
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
                    match VCardType::try_from(param_name.as_ref()) {
                        Ok(typ) if self.token_buf.is_empty() => {
                            if let Some(types) = param_values.iter_mut().find_map(|param| {
                                if let VCardParameter::Type(types) = param {
                                    Some(types)
                                } else {
                                    None
                                }
                            }) {
                                types.push(typ);
                            } else {
                                param_values.push(VCardParameter::Type(vec![typ]));
                            }
                        },
                        _ => {
                            if !param_name.is_empty() {
                                if params.encoding.is_none() && param_name.eq_ignore_ascii_case(b"base64") {
                                    params.encoding = Some(Encoding::Base64);
                                } else {
                                    param_values.push(VCardParameter::Other(
                                        [Token::new(param_name).into_string()]
                                            .into_iter()
                                            .chain(self.token_buf.drain(..).map(|token| token.into_string()))
                                            .collect(),
                                    ));
                                }
                            }
                        }
                    }
                }
            );
        }
    }
}

impl Token<'_> {
    pub(crate) fn into_date(self) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        dt.parse_date(&mut self.text.iter().peekable());
        if !dt.is_null() {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_date_and_or_datetime(
        self,
    ) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        dt.parse_date_and_or_time(&mut self.text.iter().peekable());
        if !dt.is_null() {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_date_time(self) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        dt.parse_date_time(&mut self.text.iter().peekable());
        if !dt.is_null() {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_time(self) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        dt.parse_time(&mut self.text.iter().peekable(), false);
        if !dt.is_null() {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_offset(self) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        dt.parse_zone(&mut self.text.iter().peekable());
        if !dt.is_null() {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_float(self) -> std::result::Result<f64, String> {
        if let Ok(text) = std::str::from_utf8(self.text.as_ref()) {
            if let Ok(float) = text.parse::<f64>() {
                return Ok(float);
            }
        }

        Err(self.into_string())
    }

    pub(crate) fn into_integer(self) -> std::result::Result<i64, String> {
        if let Ok(text) = std::str::from_utf8(self.text.as_ref()) {
            if let Ok(float) = text.parse::<i64>() {
                return Ok(float);
            }
        }

        Err(self.into_string())
    }

    pub(crate) fn into_timestamp(self) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        if dt.parse_timestamp(&mut self.text.iter().peekable()) {
            Ok(dt)
        } else {
            Err(self.into_string())
        }
    }

    pub(crate) fn into_timestamp_or_legacy(
        self,
    ) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        if dt.parse_timestamp(&mut self.text.iter().peekable()) {
            Ok(dt)
        } else {
            let mut dt = VCardPartialDateTime::default();
            if dt.parse_date_legacy(&mut self.text.iter().peekable()) {
                Ok(dt)
            } else {
                Err(self.into_string())
            }
        }
    }

    pub(crate) fn into_datetime_or_legacy(
        self,
    ) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        if dt.parse_date_legacy(&mut self.text.iter().peekable()) {
            Ok(dt)
        } else {
            self.into_date_and_or_datetime()
        }
    }

    pub(crate) fn into_offset_or_legacy(self) -> std::result::Result<VCardPartialDateTime, String> {
        let mut dt = VCardPartialDateTime::default();
        if dt.parse_zone_legacy(&mut self.text.iter().peekable()) {
            Ok(dt)
        } else {
            self.into_offset()
        }
    }

    pub(crate) fn into_boolean(self) -> bool {
        self.text.as_ref().eq_ignore_ascii_case(b"true")
    }
}

impl VCardPartialDateTime {
    fn parse_timestamp(&mut self, iter: &mut Peekable<Iter<u8>>) -> bool {
        let mut idx = 0;
        for ch in iter {
            match ch {
                b'0'..=b'9' => {
                    let value = match idx {
                        0..=3 => &mut self.year,
                        4..=5 => &mut self.month,
                        6..=7 => &mut self.day,
                        9..=10 => &mut self.hour,
                        11..=12 => &mut self.minute,
                        13..=14 => &mut self.second,
                        16..=17 => &mut self.tz_hour,
                        18..=19 => &mut self.tz_minute,
                        _ => return false,
                    };

                    if let Some(value) = value {
                        *value = value.saturating_mul(10).saturating_add((ch - b'0') as u16);
                    } else {
                        *value = Some((ch - b'0') as u16);
                    }
                }
                b'T' | b't' if idx == 8 => {}
                b'+' if idx == 15 => {}
                b'Z' | b'z' if idx == 15 => {
                    self.tz_hour = Some(0);
                    self.tz_minute = Some(0);
                }
                b'-' if idx == 15 => {
                    self.tz_minus = true;
                }
                b' ' | b'\t' | b'\r' | b'\n' => {
                    continue;
                }
                _ => return false,
            }
            idx += 1;
        }

        true
    }

    fn parse_date_legacy(&mut self, iter: &mut Peekable<Iter<u8>>) -> bool {
        let mut idx = 0;

        for ch in iter {
            match ch {
                b'0'..=b'9' => {
                    let value = match idx {
                        0 => &mut self.year,
                        1 => &mut self.month,
                        2 => &mut self.day,
                        3 => &mut self.hour,
                        4 => &mut self.minute,
                        5 => &mut self.second,
                        6 => &mut self.tz_hour,
                        7 => &mut self.tz_minute,
                        _ => return false,
                    };

                    if let Some(value) = value {
                        *value = value.saturating_mul(10).saturating_add((ch - b'0') as u16);
                    } else {
                        *value = Some((ch - b'0') as u16);
                    }
                }
                b'T' | b't' if idx < 3 => {
                    idx = 3;
                }
                b'+' if idx <= 5 => {
                    idx = 6;
                }
                b'Z' | b'z' if idx == 5 => {
                    self.tz_hour = Some(0);
                    self.tz_minute = Some(0);
                    break;
                }
                b'-' if idx <= 2 => {
                    idx += 1;
                }
                b'-' if idx <= 5 => {
                    self.tz_minus = true;
                    idx = 6;
                }
                b':' if (3..=6).contains(&idx) => {
                    idx += 1;
                }
                b' ' | b'\t' | b'\r' | b'\n' => {
                    continue;
                }
                _ => return false,
            }
        }

        self.has_date() || self.has_zone()
    }

    fn parse_zone_legacy(&mut self, iter: &mut Peekable<Iter<u8>>) -> bool {
        let mut idx = 0;

        for ch in iter {
            match ch {
                b'0'..=b'9' => {
                    let value = match idx {
                        0 => &mut self.tz_hour,
                        1 => &mut self.tz_minute,
                        _ => return false,
                    };

                    if let Some(value) = value {
                        *value = value.saturating_mul(10).saturating_add((ch - b'0') as u16);
                    } else {
                        *value = Some((ch - b'0') as u16);
                    }
                }
                b'+' if self.tz_hour.is_none() => {}
                b'-' if self.tz_hour.is_none() => {
                    self.tz_minus = true;
                }
                b'Z' | b'z' if self.tz_hour.is_none() => {
                    self.tz_hour = Some(0);
                    self.tz_minute = Some(0);
                    break;
                }
                b':' => {
                    idx += 1;
                }
                b' ' | b'\t' | b'\r' | b'\n' => {
                    continue;
                }
                _ => return false,
            }
        }

        self.tz_hour.is_some() && self.tz_minute.is_some()
    }

    fn parse_date_time(&mut self, iter: &mut Peekable<Iter<u8>>) {
        self.parse_date_noreduc(iter);
        if matches!(iter.peek(), Some(&&b'T' | &&b't')) {
            iter.next();
            self.parse_time(iter, true);
        }
    }

    fn parse_date_and_or_time(&mut self, iter: &mut Peekable<Iter<u8>>) {
        self.parse_date(iter);
        if matches!(iter.peek(), Some(&&b'T' | &&b't')) {
            iter.next();
            self.parse_time(iter, false);
        }
    }

    fn parse_date(&mut self, iter: &mut Peekable<Iter<u8>>) {
        parse_digits(iter, &mut self.year, 4, true);
        if self.year.is_some() && iter.peek() == Some(&&b'-') {
            iter.next();
            parse_digits(iter, &mut self.month, 2, true);
        } else {
            parse_digits(iter, &mut self.month, 2, true);
            parse_digits(iter, &mut self.day, 2, false);
        }
    }

    fn parse_date_noreduc(&mut self, iter: &mut Peekable<Iter<u8>>) {
        parse_digits(iter, &mut self.year, 4, true);
        parse_digits(iter, &mut self.month, 2, true);
        parse_digits(iter, &mut self.day, 2, false);
    }

    fn parse_time(&mut self, iter: &mut Peekable<Iter<u8>>, mut notrunc: bool) {
        for part in [&mut self.hour, &mut self.minute, &mut self.second] {
            match iter.peek() {
                Some(b'0'..=b'9') => {
                    notrunc = true;
                    parse_digits(iter, part, 2, false);
                }
                Some(b'-') if !notrunc => {
                    iter.next();
                }
                _ => break,
            }
        }
        self.parse_zone(iter);
    }

    fn parse_zone(&mut self, iter: &mut Peekable<Iter<u8>>) -> bool {
        self.tz_minus = match iter.peek() {
            Some(b'-') => true,
            Some(b'+') => false,
            Some(b'Z') | Some(b'z') => {
                self.tz_hour = Some(0);
                self.tz_minute = Some(0);
                iter.next();
                return true;
            }
            _ => return false,
        };

        iter.next();
        let mut idx = 0;
        for ch in iter {
            match ch {
                b'0'..=b'9' => {
                    idx += 1;
                    let value = match idx {
                        1 | 2 => &mut self.tz_hour,
                        3 | 4 => &mut self.tz_minute,
                        _ => return false,
                    };

                    if let Some(value) = value {
                        *value = value.saturating_mul(10).saturating_add((ch - b'0') as u16);
                    } else {
                        *value = Some((ch - b'0') as u16);
                    }
                }
                _ => {
                    if !ch.is_ascii_whitespace() {
                        return false;
                    }
                }
            }
        }

        self.tz_hour.is_some()
    }

    pub fn is_null(&self) -> bool {
        self.year.is_none()
            && self.month.is_none()
            && self.day.is_none()
            && self.hour.is_none()
            && self.minute.is_none()
            && self.second.is_none()
            && self.tz_hour.is_none()
            && self.tz_minute.is_none()
    }

    pub fn has_date(&self) -> bool {
        self.year.is_some() && self.month.is_some() && self.day.is_some()
    }

    pub fn has_time(&self) -> bool {
        self.hour.is_some() && self.minute.is_some()
    }

    pub fn has_zone(&self) -> bool {
        self.tz_hour.is_some()
    }

    pub fn to_timestamp(&self) -> Option<i64> {
        if self.has_date() && self.has_time() {
            DateTime {
                year: self.year.unwrap(),
                month: self.month.unwrap() as u8,
                day: self.day.unwrap() as u8,
                hour: self.hour.unwrap() as u8,
                minute: self.minute.unwrap() as u8,
                second: self.second.unwrap_or_default() as u8,
                tz_before_gmt: self.tz_minus,
                tz_hour: self.tz_hour.unwrap_or_default() as u8,
                tz_minute: self.tz_minute.unwrap_or_default() as u8,
            }
            .to_timestamp()
            .into()
        } else {
            None
        }
    }
}

fn parse_digits(
    iter: &mut Peekable<Iter<u8>>,
    target: &mut Option<u16>,
    num: usize,
    nullable: bool,
) {
    let mut idx = 0;
    while let Some(ch) = iter.peek() {
        match ch {
            b'0'..=b'9' => {
                let ch = (*ch - b'0') as u16;
                idx += 1;
                iter.next();

                if let Some(target) = target {
                    *target = target.saturating_mul(10).saturating_add(ch);

                    if idx == num {
                        return;
                    }
                } else {
                    *target = Some(ch);
                }
            }
            b'-' if nullable => {
                idx += 1;
                iter.next();
                if idx == num / 2 {
                    return;
                }
            }
            _ => {
                if !ch.is_ascii_whitespace() {
                    return;
                } else {
                    iter.next();
                }
            }
        };
    }
}

#[derive(Default)]
pub(crate) struct Timestamp(pub i64);

impl FromStr for Timestamp {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let mut dt = VCardPartialDateTime::default();
        dt.parse_timestamp(&mut s.as_bytes().iter().peekable());
        dt.to_timestamp().map(Timestamp).ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Entry;

    use super::*;
    use std::io::Write;

    #[test]
    fn parse_vcard() {
        // Read all .vcf files in the test directory
        for entry in std::fs::read_dir("resources/vcard").unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "vcf") {
                let input = std::fs::read_to_string(&path).unwrap();
                let mut parser = Parser::new(input.as_bytes());
                let mut output = std::fs::File::create(path.with_extension("vcf.out")).unwrap();
                let file_name = path.as_path().to_str().unwrap();

                loop {
                    match parser.entry() {
                        Entry::VCard(mut vcard) => {
                            let vcard_text = vcard.to_string();
                            writeln!(output, "{}", vcard_text).unwrap();

                            // Roundtrip parsing
                            let mut parser = Parser::new(vcard_text.as_bytes());
                            match parser.entry() {
                                Entry::VCard(mut vcard_) => {
                                    vcard.entries.retain(|entry| {
                                        !matches!(entry.name, VCardProperty::Version)
                                    });
                                    vcard_.entries.retain(|entry| {
                                        !matches!(entry.name, VCardProperty::Version)
                                    });
                                    assert_eq!(vcard.entries.len(), vcard_.entries.len());

                                    if !file_name.contains("003.vcf") {
                                        for (entry, entry_) in
                                            vcard.entries.iter().zip(vcard_.entries.iter())
                                        {
                                            if entry != entry_
                                                && matches!(
                                                    (entry.values.first(), entry_.values.first()),
                                                    (
                                                        Some(VCardValue::Binary(_),),
                                                        Some(VCardValue::Text(_))
                                                    )
                                                )
                                            {
                                                continue;
                                            }
                                            assert_eq!(entry, entry_, "failed for {file_name}");
                                        }
                                    }
                                }
                                other => panic!("Expected VCard, got {other:?} for {file_name}"),
                            }
                        }
                        Entry::InvalidLine(text) => {
                            println!("Invalid line in {file_name}: {text}");
                        }
                        Entry::ICalendar(_) => {
                            panic!("Expected VCard, got ICalendar for {file_name}");
                        }
                        Entry::Eof => break,
                    }
                }
            }
        }
    }

    #[test]
    fn test_parse_dates() {
        for (input, typ, expected) in [
            (
                "19850412",
                VCardValueType::Date,
                VCardPartialDateTime {
                    year: Some(1985),
                    month: Some(4),
                    day: Some(12),
                    ..Default::default()
                },
            ),
            (
                "1985-04",
                VCardValueType::Date,
                VCardPartialDateTime {
                    year: Some(1985),
                    month: Some(4),
                    ..Default::default()
                },
            ),
            (
                "1985",
                VCardValueType::Date,
                VCardPartialDateTime {
                    year: Some(1985),
                    ..Default::default()
                },
            ),
            (
                "--0412",
                VCardValueType::Date,
                VCardPartialDateTime {
                    month: Some(4),
                    day: Some(12),
                    ..Default::default()
                },
            ),
            (
                "---12",
                VCardValueType::Date,
                VCardPartialDateTime {
                    day: Some(12),
                    ..Default::default()
                },
            ),
            (
                "102200",
                VCardValueType::Time,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "1022",
                VCardValueType::Time,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    ..Default::default()
                },
            ),
            (
                "10",
                VCardValueType::Time,
                VCardPartialDateTime {
                    hour: Some(10),
                    ..Default::default()
                },
            ),
            (
                "-2200",
                VCardValueType::Time,
                VCardPartialDateTime {
                    minute: Some(22),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "--00",
                VCardValueType::Time,
                VCardPartialDateTime {
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "102200Z",
                VCardValueType::Time,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    second: Some(0),
                    tz_hour: Some(0),
                    tz_minute: Some(0),
                    ..Default::default()
                },
            ),
            (
                "102200-0800",
                VCardValueType::Time,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    second: Some(0),
                    tz_hour: Some(8),
                    tz_minute: Some(0),
                    tz_minus: true,
                    ..Default::default()
                },
            ),
            (
                "19961022T140000",
                VCardValueType::DateTime,
                VCardPartialDateTime {
                    year: Some(1996),
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "--1022T1400",
                VCardValueType::DateTime,
                VCardPartialDateTime {
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    ..Default::default()
                },
            ),
            (
                "---22T14",
                VCardValueType::DateTime,
                VCardPartialDateTime {
                    day: Some(22),
                    hour: Some(14),
                    ..Default::default()
                },
            ),
            (
                "19961022T140000",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    year: Some(1996),
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "--1022T1400",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    ..Default::default()
                },
            ),
            (
                "---22T14",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    day: Some(22),
                    hour: Some(14),
                    ..Default::default()
                },
            ),
            (
                "19850412",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    year: Some(1985),
                    month: Some(4),
                    day: Some(12),
                    ..Default::default()
                },
            ),
            (
                "1985-04",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    year: Some(1985),
                    month: Some(4),
                    ..Default::default()
                },
            ),
            (
                "1985",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    year: Some(1985),
                    ..Default::default()
                },
            ),
            (
                "--0412",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    month: Some(4),
                    day: Some(12),
                    ..Default::default()
                },
            ),
            (
                "---12",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    day: Some(12),
                    ..Default::default()
                },
            ),
            (
                "T102200",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "T1022",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    ..Default::default()
                },
            ),
            (
                "T10",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    hour: Some(10),
                    ..Default::default()
                },
            ),
            (
                "T-2200",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    minute: Some(22),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "T--00",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "T102200Z",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    second: Some(0),
                    tz_hour: Some(0),
                    tz_minute: Some(0),
                    ..Default::default()
                },
            ),
            (
                "T102200-0800",
                VCardValueType::DateAndOrTime,
                VCardPartialDateTime {
                    hour: Some(10),
                    minute: Some(22),
                    second: Some(0),
                    tz_hour: Some(8),
                    tz_minute: Some(0),
                    tz_minus: true,
                    ..Default::default()
                },
            ),
            (
                "19961022T140000",
                VCardValueType::Timestamp,
                VCardPartialDateTime {
                    year: Some(1996),
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    second: Some(0),
                    ..Default::default()
                },
            ),
            (
                "19961022T140000Z",
                VCardValueType::Timestamp,
                VCardPartialDateTime {
                    year: Some(1996),
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    second: Some(0),
                    tz_hour: Some(0),
                    tz_minute: Some(0),
                    ..Default::default()
                },
            ),
            (
                "19961022T140000-05",
                VCardValueType::Timestamp,
                VCardPartialDateTime {
                    year: Some(1996),
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    second: Some(0),
                    tz_hour: Some(5),
                    tz_minus: true,
                    ..Default::default()
                },
            ),
            (
                "19961022T140000-0500",
                VCardValueType::Timestamp,
                VCardPartialDateTime {
                    year: Some(1996),
                    month: Some(10),
                    day: Some(22),
                    hour: Some(14),
                    minute: Some(0),
                    second: Some(0),
                    tz_hour: Some(5),
                    tz_minute: Some(0),
                    tz_minus: true,
                },
            ),
            (
                "-0500",
                VCardValueType::UtcOffset,
                VCardPartialDateTime {
                    tz_hour: Some(5),
                    tz_minute: Some(0),
                    tz_minus: true,
                    ..Default::default()
                },
            ),
        ] {
            let mut iter = input.as_bytes().iter().peekable();
            let mut dt = VCardPartialDateTime::default();

            match typ {
                VCardValueType::Date => dt.parse_date(&mut iter),
                VCardValueType::DateAndOrTime => dt.parse_date_and_or_time(&mut iter),
                VCardValueType::DateTime => dt.parse_date_time(&mut iter),
                VCardValueType::Time => dt.parse_time(&mut iter, false),
                VCardValueType::Timestamp => {
                    dt.parse_timestamp(&mut iter);
                }
                VCardValueType::UtcOffset => {
                    dt.parse_zone(&mut iter);
                }
                _ => unreachable!(),
            }

            assert_eq!(dt, expected, "failed for {input:?}");
            assert!(
                dt.to_string() == input
                    || dt.to_string() == input.strip_prefix("T").unwrap_or(input),
                "roundtrip failed: {input} != {dt}"
            );
        }
    }
}
