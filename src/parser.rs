use std::{
    iter::{Enumerate, Peekable},
    slice::Iter,
    str::FromStr,
};

use mail_parser::{
    decoders::{base64::base64_decode, hex::decode_hex},
    DateTime,
};

use crate::{
    tokenizer::{StopChar, Token},
    vcard::{VCard, VCardBinary, VCardPartialDateTime},
};

pub enum Entry {
    VCard(VCard),
    InvalidLine(String),
    Eof,
}

pub struct Parser<'x> {
    pub(crate) input: &'x [u8],
    pub(crate) iter: Peekable<Enumerate<Iter<'x, u8>>>,
    pub(crate) strict: bool,
    pub(crate) stop_colon: bool,
    pub(crate) stop_semicolon: bool,
    pub(crate) stop_comma: bool,
    pub(crate) stop_equal: bool,
    pub(crate) stop_slash: bool,
    pub(crate) stop_dot: bool,
    pub(crate) unfold_qp: bool,
    pub(crate) unquote: bool,
    pub(crate) token_buf: Vec<Token<'x>>,
}

impl<'x> Parser<'x> {
    pub fn new(input: &'x [u8]) -> Self {
        Self {
            input,
            iter: input.iter().enumerate().peekable(),
            strict: false,
            stop_colon: true,
            stop_semicolon: true,
            stop_comma: true,
            stop_equal: true,
            stop_slash: false,
            stop_dot: false,
            unfold_qp: false,
            unquote: true,
            token_buf: Vec::with_capacity(10),
        }
    }

    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    pub fn entry(&mut self) -> Entry {
        self.expect_iana_token();

        loop {
            if let Some(token) = self.token() {
                if token.text.eq_ignore_ascii_case(b"BEGIN") && token.stop_char == StopChar::Colon {
                    if let Some(token) = self.token() {
                        if token.stop_char == StopChar::Lf {
                            hashify::fnc_map_ignore_case!(token.text.as_ref(),
                                b"VCARD" => {
                                    return Entry::VCard(self.vcard());
                                },
                                b"VCALENDAR" => {
                                    //self.vcalendar()
                                    todo!()
                                }
                                _ => {
                                    return Entry::InvalidLine(token.into_string());
                                }
                            )
                        }
                    } else {
                        return Entry::Eof;
                    }
                }

                let token_start = token.start;
                let mut token_end = token.end;

                if token.stop_char != StopChar::Lf {
                    self.expect_single_value();
                    while let Some(token) = self.token() {
                        token_end = token.end;
                        if token.stop_char == StopChar::Lf {
                            break;
                        }
                    }
                } else if token.text.is_empty() {
                    continue;
                }

                return Entry::InvalidLine(
                    std::str::from_utf8(self.input.get(token_start..token_end).unwrap_or_default())
                        .unwrap_or_default()
                        .to_string(),
                );
            } else {
                return Entry::Eof;
            }
        }
    }
}

impl Token<'_> {
    pub(crate) fn into_uri_bytes(self) -> std::result::Result<VCardBinary, String> {
        if self
            .text
            .as_ref()
            .get(0..5)
            .unwrap_or_default()
            .eq_ignore_ascii_case(b"data:")
        {
            let mut bin = VCardBinary::default();
            let text = self.text.as_ref().get(5..).unwrap_or_default();
            let mut offset_start = 0;
            let mut is_base64 = false;

            for (idx, ch) in text.iter().enumerate() {
                match ch {
                    b';' => {
                        if idx > 0 {
                            bin.content_type = Some(
                                std::str::from_utf8(&text[offset_start..idx])
                                    .unwrap_or_default()
                                    .to_string(),
                            );
                        }
                        offset_start = idx + 1;
                    }
                    b',' => {
                        if idx != offset_start {
                            let text = text.get(offset_start..idx).unwrap_or_default();
                            if text.eq_ignore_ascii_case(b"base64") {
                                is_base64 = true;
                            } else if bin.content_type.is_none() {
                                bin.content_type =
                                    Some(std::str::from_utf8(text).unwrap_or_default().to_string());
                            }
                        }

                        offset_start = idx + 1;
                        break;
                    }
                    _ => {}
                }
            }

            let text = text.get(offset_start..).unwrap_or_default();
            if !text.is_empty() {
                if is_base64 {
                    if let Some(bytes) = base64_decode(text) {
                        bin.data = bytes;
                        return Ok(bin);
                    }
                } else {
                    let (success, bytes) = decode_hex(text);
                    if success {
                        bin.data = bytes;
                        return Ok(bin);
                    }
                }
            }
        }

        Err(self.into_string())
    }

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
                b'Z' | b'z' | b'+' if idx == 15 => {}
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

    pub fn to_timestamp(&self) -> Option<i64> {
        if self.year.is_some()
            && self.month.is_some()
            && self.day.is_some()
            && self.hour.is_some()
            && self.minute.is_some()
        {
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
    use crate::vcard::VCardValueType;

    use super::*;

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
        }
    }

    #[test]
    fn test_parse_uri() {
        for (uri, expected) in [
            (
                concat!(
                    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAsAAAALCAQAAAADpb+\n",
                    "tAAAAQklEQVQI122PQQ4AMAjCKv//Mzs4M0zmRYKkamEwWQVoRJogk4PuRoOoMC/EK8nYb+",
                    "l08WGvSxKlNHO5kxnp/WXrAzsSERN1N6q5AAAAAElFTkSuQmCC"
                ),
                VCardBinary {
                    content_type: Some("image/png".to_string()),
                    data: vec![
                        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 11,
                        0, 0, 0, 11, 8, 4, 0, 0, 0, 3, 165, 191, 173, 0, 0, 0, 66, 73, 68, 65, 84,
                        8, 215, 109, 143, 65, 14, 0, 48, 8, 194, 42, 255, 255, 51, 59, 56, 51, 76,
                        230, 69, 130, 164, 106, 97, 48, 89, 5, 104, 68, 154, 32, 147, 131, 238, 70,
                        131, 168, 48, 47, 196, 43, 201, 216, 111, 233, 116, 241, 97, 175, 75, 18,
                        165, 52, 115, 185, 147, 25, 233, 253, 101, 235, 3, 59, 18, 17, 19, 117, 55,
                        170, 185, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
                    ],
                },
            ),
            (
                concat!(
                    "data:text/html,%3Cbody%3E%3Cp%3EToller%20%3Cstron",
                    "g%20class%3D%22text-bold%22%3ETermin%3C%2Fstrong%3E%20f%C3%BCr%3C%2Fp",
                    "%3E%3Cblockquote%3E%3Cp%3Emal%20%3Cem%20class%3D%22text-italic%22%3Ez",
                    "u%22gucken%22%3C%2Fem%3E%3C%2Fp%3E%3C%2Fblockquote%3E%3Cp%3E%3Cu%20cl",
                    "ass%3D%22text-underline%22%3Eund%3C%2Fu%3E%20%3Cdel%3Eso%3C%2Fdel%3E%",
                    "3Cbr%2F%3E%3C%2Fp%3E%3C%2Fbody%3E"
                ),
                VCardBinary {
                    content_type: Some("text/html".to_string()),
                    data: vec![
                        60, 98, 111, 100, 121, 62, 60, 112, 62, 84, 111, 108, 108, 101, 114, 32,
                        60, 115, 116, 114, 111, 110, 103, 32, 99, 108, 97, 115, 115, 61, 34, 116,
                        101, 120, 116, 45, 98, 111, 108, 100, 34, 62, 84, 101, 114, 109, 105, 110,
                        60, 47, 115, 116, 114, 111, 110, 103, 62, 32, 102, 195, 188, 114, 60, 47,
                        112, 62, 60, 98, 108, 111, 99, 107, 113, 117, 111, 116, 101, 62, 60, 112,
                        62, 109, 97, 108, 32, 60, 101, 109, 32, 99, 108, 97, 115, 115, 61, 34, 116,
                        101, 120, 116, 45, 105, 116, 97, 108, 105, 99, 34, 62, 122, 117, 34, 103,
                        117, 99, 107, 101, 110, 34, 60, 47, 101, 109, 62, 60, 47, 112, 62, 60, 47,
                        98, 108, 111, 99, 107, 113, 117, 111, 116, 101, 62, 60, 112, 62, 60, 117,
                        32, 99, 108, 97, 115, 115, 61, 34, 116, 101, 120, 116, 45, 117, 110, 100,
                        101, 114, 108, 105, 110, 101, 34, 62, 117, 110, 100, 60, 47, 117, 62, 32,
                        60, 100, 101, 108, 62, 115, 111, 60, 47, 100, 101, 108, 62, 60, 98, 114,
                        47, 62, 60, 47, 112, 62, 60, 47, 98, 111, 100, 121, 62,
                    ],
                },
            ),
        ] {
            assert_eq!(
                Token {
                    text: uri.as_bytes().into(),
                    start: 0,
                    end: 0,
                    stop_char: StopChar::Lf
                }
                .into_uri_bytes(),
                Ok(expected)
            );
        }
    }
}
