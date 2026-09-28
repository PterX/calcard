/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::common::{Encoding, tokenizer::StopChar};
use encodify::{
    base64,
    qp::{self, QuotedPrintable},
};
use mail_parser::Charset;
use std::borrow::Cow;

const QUOTED_PRINTABLE_CHARSET: &str = "iso-8859-1";
const QUOTED_PRINTABLE: QuotedPrintable = qp::BODY.strict();

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decoded {
    Binary(Vec<u8>),
    Text(String),
    Undecodable,
}

pub(crate) struct ValueDecoder<'x> {
    pub(crate) encoding: Encoding,
    pub(crate) charset: Option<&'x str>,
    pub(crate) binary: bool,
}

impl ValueDecoder<'_> {
    #[inline(never)]
    pub(crate) fn decode(
        &self,
        input: &[u8],
        stop_char: StopChar,
        payload: Option<Vec<u8>>,
    ) -> Decoded {
        let Some(bytes) = payload
            .map(Cow::Owned)
            .or_else(|| self.encoding.decode_value(input, stop_char))
        else {
            return Decoded::Undecodable;
        };
        if self.binary {
            return Decoded::Binary(bytes.into_owned());
        }
        let default_charset = match self.encoding {
            Encoding::QuotedPrintable => Some(QUOTED_PRINTABLE_CHARSET),
            Encoding::Base64 => None,
        };
        match self
            .charset
            .or(default_charset)
            .and_then(|charset| Charset::from_label(charset.as_bytes()))
            .filter(|charset| *charset != Charset::Utf8)
        {
            Some(charset) => Decoded::Text(match bytes {
                Cow::Borrowed(bytes) => charset.decode(bytes).into_owned(),
                Cow::Owned(bytes) => charset.decode_owned(bytes),
            }),
            None => match String::from_utf8(bytes.into_owned()) {
                Ok(text) => Decoded::Text(text),
                Err(err) => Decoded::Binary(err.into_bytes()),
            },
        }
    }
}

impl Encoding {
    pub(crate) fn decode(self, input: &[u8]) -> Option<Cow<'_, [u8]>> {
        match self {
            Encoding::Base64 => base64::LENIENT.decode(input).ok().map(Cow::Owned),
            Encoding::QuotedPrintable => QUOTED_PRINTABLE.decode(input).ok(),
        }
    }

    fn decode_value(self, input: &[u8], stop_char: StopChar) -> Option<Cow<'_, [u8]>> {
        match (self, stop_char) {
            (Encoding::QuotedPrintable, StopChar::Lf) | (Encoding::Base64, _) => self.decode(input),
            (Encoding::QuotedPrintable, _) => Self::decode_quoted_printable_component(input),
        }
    }

    fn decode_quoted_printable_component(input: &[u8]) -> Option<Cow<'_, [u8]>> {
        let mut text = input;
        while let [head @ .., b' ' | b'\t'] = text {
            text = head;
        }
        match input.get(text.len()..) {
            Some(blanks @ [_, ..]) => {
                let mut bytes = Vec::new();
                QUOTED_PRINTABLE.decode_append(text, &mut bytes).ok()?;
                bytes.extend_from_slice(blanks);
                Some(Cow::Owned(bytes))
            }
            _ => QUOTED_PRINTABLE.decode(input).ok(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoded, ValueDecoder};
    use crate::{
        common::{Data, Encoding, tokenizer::StopChar},
        icalendar::{ICalendar, ICalendarValue},
        vcard::{VCard, VCardProperty, VCardValue},
    };

    fn calendar_value(value: &str) -> Option<ICalendarValue> {
        ICalendar::parse(format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nATTACH;ENCODING=BASE64;VALUE=BINARY:{value}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        ))
        .expect("the calendar parses")
        .components
        .get(1)
        .and_then(|component| component.entries.first())
        .and_then(|entry| entry.values.first())
        .cloned()
    }

    fn card_bytes(line: &str) -> Option<Vec<u8>> {
        VCard::parse(format!(
            "BEGIN:VCARD\r\nVERSION:4.0\r\n{line}\r\nEND:VCARD\r\n"
        ))
        .expect("the card parses")
        .entries
        .iter()
        .find_map(|entry| match entry.values.first() {
            Some(VCardValue::Binary(data)) => Some(data.data.clone()),
            _ => None,
        })
    }

    #[test]
    fn base64_value_without_padding_keeps_its_last_bytes() {
        for (value, expected) in [
            ("QUJDRA", &b"ABCD"[..]),
            ("QUJD\r\n RA", b"ABCD"),
            ("QU\r\n I", b"AB"),
        ] {
            assert_eq!(
                calendar_value(value),
                Some(ICalendarValue::Binary(expected.to_vec())),
                "{value:?}"
            );
        }
        assert_eq!(
            card_bytes("PHOTO;ENCODING=b:QUI").as_deref(),
            Some(&b"AB"[..])
        );
        assert_eq!(
            card_bytes("O;ENCODING=BASE64:/9").as_deref(),
            Some(&b"\xff"[..])
        );
        assert_eq!(
            Data::try_parse(b"data:;base64,QUJDRA")
                .map(|data| data.data)
                .as_deref(),
            Some(&b"ABCD"[..])
        );
    }

    #[test]
    fn base64_padding_after_a_single_character_adds_no_byte() {
        for value in ["QUJDQ=", "QUJD\r\n Q=", "QUJDQ\r\n ="] {
            assert_eq!(
                calendar_value(value),
                Some(ICalendarValue::Binary(b"ABC".to_vec())),
                "{value:?}"
            );
        }
    }

    #[test]
    fn value_decoder_applies_the_charset_and_falls_back_to_binary() {
        let text = |text: &str| Decoded::Text(text.to_string());
        let binary = |bytes: &[u8]| Decoded::Binary(bytes.to_vec());
        for (encoding, charset, is_binary, input, payload, expected) in [
            (Encoding::Base64, None, true, "QUJD", None, binary(b"ABC")),
            (Encoding::Base64, None, false, "QUJD", None, text("ABC")),
            (Encoding::Base64, None, false, "/w==", None, binary(&[0xff])),
            (
                Encoding::Base64,
                Some("iso-8859-1"),
                false,
                "Y2Fm6Q==",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::Base64,
                Some("x-unknown"),
                false,
                "Y2Fmw6k=",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::Base64,
                None,
                true,
                "QU*D",
                None,
                Decoded::Undecodable,
            ),
            (
                Encoding::Base64,
                None,
                true,
                "",
                Some(vec![1, 2, 3]),
                binary(&[1, 2, 3]),
            ),
            (
                Encoding::Base64,
                None,
                false,
                "QU*D",
                Some(b"ABC".to_vec()),
                text("ABC"),
            ),
            (
                Encoding::QuotedPrintable,
                None,
                false,
                "caf=E9",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::QuotedPrintable,
                Some("UTF-8"),
                false,
                "caf=C3=\r\n=A9",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::QuotedPrintable,
                Some("utf-8"),
                false,
                "caf=C3",
                None,
                binary(b"caf\xc3"),
            ),
            (
                Encoding::QuotedPrintable,
                None,
                true,
                "=00=FF",
                None,
                binary(&[0, 0xff]),
            ),
            (
                Encoding::QuotedPrintable,
                None,
                false,
                "=G1",
                None,
                Decoded::Undecodable,
            ),
        ] {
            let decoder = ValueDecoder {
                encoding,
                charset,
                binary: is_binary,
            };
            assert_eq!(
                decoder.decode(input.as_bytes(), StopChar::Lf, payload),
                expected,
                "{encoding:?} {charset:?} {input:?}"
            );
        }
    }

    #[test]
    fn quoted_printable_keeps_blanks_before_a_value_separator() {
        let decoder = ValueDecoder {
            encoding: Encoding::QuotedPrintable,
            charset: Some("utf-8"),
            binary: false,
        };
        for (input, stop_char, expected) in [
            ("Doe ", StopChar::Semicolon, "Doe "),
            ("Doe \t", StopChar::Comma, "Doe \t"),
            ("caf=C3=A9 ", StopChar::Semicolon, "caf\u{e9} "),
            ("a=\r\nb ", StopChar::Semicolon, "ab "),
            (" ", StopChar::Semicolon, " "),
            ("Doe", StopChar::Semicolon, "Doe"),
            ("Doe ", StopChar::Lf, "Doe"),
            ("a=20 ", StopChar::Lf, "a "),
        ] {
            assert_eq!(
                decoder.decode(input.as_bytes(), stop_char, None),
                Decoded::Text(expected.to_string()),
                "{input:?} {stop_char:?}"
            );
        }
        assert_eq!(
            VCard::parse(
                "BEGIN:VCARD\r\nVERSION:2.1\r\nN;ENCODING=QUOTED-PRINTABLE:Doe ;John ;=E9 \r\nEND:VCARD\r\n"
            )
            .expect("the card parses")
            .entries
            .iter()
            .find(|entry| entry.name == VCardProperty::N)
            .map(|entry| entry.values.to_vec()),
            Some(vec![
                VCardValue::Text("Doe ".to_string()),
                VCardValue::Text("John ".to_string()),
                VCardValue::Text("\u{e9}".to_string()),
            ])
        );
    }
}
