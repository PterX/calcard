/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::common::format::AsciiPush;
use jmap_tools::{JsonPointer, JsonPointerItem, Key, Property};
use std::{
    borrow::Cow,
    fmt::{self, Display, Write},
};

const POINTER_CAPACITY: usize = 48;
const CONVERTED_KEYS_CAPACITY: usize = 4;

pub(crate) trait PointerText: Write {
    fn reserve_escapes(&mut self, _escapes: usize) {}

    fn push_pointer_token(&mut self, token: &str) -> fmt::Result {
        let escapes = token
            .bytes()
            .filter(|byte| matches!(byte, b'~' | b'/'))
            .count();
        if escapes == 0 {
            return self.write_str(token);
        }
        self.reserve_escapes(escapes);
        for piece in token.split_inclusive(['~', '/']) {
            match piece.strip_suffix('~') {
                Some(head) => {
                    self.write_str(head)?;
                    self.write_str("~0")?;
                }
                None => match piece.strip_suffix('/') {
                    Some(head) => {
                        self.write_str(head)?;
                        self.write_str("~1")?;
                    }
                    None => self.write_str(piece)?,
                },
            }
        }
        Ok(())
    }
}

pub(crate) trait PointerString: Sized {
    fn from_pointer<'x>(parts: impl IntoIterator<Item = &'x str> + Clone) -> Self;
}

pub(crate) trait PointerTextExt<P: Property> {
    fn to_text(&self) -> String;
    fn text_eq(&self, text: &str) -> bool;
}

pub(crate) trait DisplayEq {
    fn display_eq(&self, text: &str) -> bool;
}

pub(crate) trait IdReferenceText {
    fn to_id_reference(&self) -> String;
}

pub(crate) trait IntoAsciiLowercase {
    fn into_ascii_lowercase(self) -> String;
}

pub(crate) trait PointerProperty: Property {
    fn into_pointer(self) -> Result<JsonPointer<Self>, Self>;
}

pub(crate) trait ConvertedKeys<P: Property> {
    fn into_converted_keys(self) -> Vec<Key<'static, P>>;
}

pub(crate) struct Expect<'x>(&'x [u8]);

pub(crate) struct AsciiString(String);

impl PointerText for String {
    fn reserve_escapes(&mut self, escapes: usize) {
        self.reserve_exact(self.capacity() - self.len() + escapes);
    }
}

impl PointerString for String {
    fn from_pointer<'x>(parts: impl IntoIterator<Item = &'x str> + Clone) -> Self {
        let len = parts
            .clone()
            .into_iter()
            .map(|part| part.len() + 1)
            .sum::<usize>();
        let mut text = String::with_capacity(len.saturating_sub(1));
        for (position, part) in parts.into_iter().enumerate() {
            if position > 0 {
                text.push('/');
            }
            let _ = text.push_pointer_token(part);
        }
        text
    }
}

impl<P: Property> PointerTextExt<P> for JsonPointer<P> {
    fn to_text(&self) -> String {
        let mut text = String::with_capacity(POINTER_CAPACITY);
        let _ = write!(text, "{self}");
        text
    }

    fn text_eq(&self, text: &str) -> bool {
        if text.bytes().any(|byte| matches!(byte, b'/' | b'~')) {
            return self.display_eq(text);
        }
        match self.as_slice() {
            [] | [JsonPointerItem::Root] => text.is_empty(),
            [JsonPointerItem::Wildcard] => text == "*",
            [JsonPointerItem::Invalid(invalid)] => invalid == text,
            [JsonPointerItem::Key(key)] => *key == text,
            [JsonPointerItem::Number(number)] => number.display_eq(text),
            _ => false,
        }
    }
}

impl<T: Display + ?Sized> DisplayEq for T {
    fn display_eq(&self, text: &str) -> bool {
        let mut expect = Expect::new(text);
        write!(expect, "{self}").is_ok() && expect.is_complete()
    }
}

impl IdReferenceText for str {
    fn to_id_reference(&self) -> String {
        let mut text = String::with_capacity(self.len() + 1);
        text.push('#');
        text.push_str(self);
        text
    }
}

impl IntoAsciiLowercase for Cow<'_, str> {
    fn into_ascii_lowercase(self) -> String {
        let mut text = self.into_owned();
        text.make_ascii_lowercase();
        text
    }
}

impl AsciiString {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        AsciiString(String::with_capacity(capacity))
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for AsciiString {
    fn from(text: String) -> Self {
        AsciiString(text)
    }
}

impl AsciiPush for AsciiString {
    #[inline(always)]
    fn push_ascii(&mut self, bytes: &[u8]) -> fmt::Result {
        self.0.extend(bytes.iter().copied().map(char::from));
        Ok(())
    }
}

impl<'x> Expect<'x> {
    pub(crate) fn new(text: &'x str) -> Self {
        Expect(text.as_bytes())
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsciiPush for Expect<'_> {
    #[inline(always)]
    fn push_ascii(&mut self, bytes: &[u8]) -> fmt::Result {
        self.0 = self.0.strip_prefix(bytes).ok_or(fmt::Error)?;
        Ok(())
    }
}

impl Write for Expect<'_> {
    #[inline(always)]
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.push_ascii(text.as_bytes())
    }
}

impl<P: PointerProperty> ConvertedKeys<P> for Key<'_, P> {
    fn into_converted_keys(self) -> Vec<Key<'static, P>> {
        match self {
            Key::Property(property) => match property.into_pointer() {
                Ok(pointer) => pointer.into_converted_keys(),
                Err(property) => property.to_cow().as_ref().into_converted_keys(),
            },
            Key::Borrowed(text) => text.into_converted_keys(),
            Key::Owned(text) => text.as_str().into_converted_keys(),
        }
    }
}

impl<P: PointerProperty> ConvertedKeys<P> for JsonPointer<P> {
    fn into_converted_keys(self) -> Vec<Key<'static, P>> {
        let mut keys = Vec::with_capacity(self.len());
        for item in self.into_iter() {
            match item {
                JsonPointerItem::Key(key) => {
                    match key.as_string_key() {
                        Some(text) if text.contains('/') => {
                            keys.extend(JsonPointer::<P>::parse(text).into_iter().filter_map(
                                |item| match item {
                                    JsonPointerItem::Key(key) => Some(key),
                                    JsonPointerItem::Number(number) => {
                                        Some(Key::Owned(number.to_string()))
                                    }
                                    JsonPointerItem::Root
                                    | JsonPointerItem::Wildcard
                                    | JsonPointerItem::Invalid(_) => None,
                                },
                            ));
                        }
                        _ => keys.push(key),
                    }
                }
                JsonPointerItem::Number(number) => keys.push(Key::Owned(number.to_string())),
                JsonPointerItem::Root | JsonPointerItem::Wildcard | JsonPointerItem::Invalid(_) => {
                }
            }
        }
        keys
    }
}

impl<P: PointerProperty> ConvertedKeys<P> for &str {
    fn into_converted_keys(self) -> Vec<Key<'static, P>> {
        if self.contains('~') {
            return JsonPointer::<P>::parse(self).into_converted_keys();
        }
        let mut keys: Vec<Key<'static, P>> = Vec::with_capacity(CONVERTED_KEYS_CAPACITY);
        let mut parent_is_key = false;
        for (position, segment) in self.split('/').enumerate() {
            let parent = if parent_is_key { keys.last() } else { None };
            let (key, is_key) = match segment {
                "" if position == 0 => continue,
                "" => (Key::from(""), true),
                "*" => {
                    parent_is_key = false;
                    continue;
                }
                _ => match P::try_parse(parent, segment) {
                    Some(property) => (Key::Property(property), true),
                    None => (
                        Key::Owned(segment.to_string()),
                        !segment.bytes().all(|byte| byte.is_ascii_digit())
                            || (segment != "0" && segment.starts_with('0'))
                            || segment.parse::<u64>().is_err(),
                    ),
                },
            };
            keys.push(key);
            parent_is_key = is_key;
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::{ConvertedKeys, DisplayEq, Expect, PointerProperty, PointerString};
    use crate::{
        common::{format::AsciiPush, xorshift::XorShift},
        jscalendar::JSCalendarProperty,
        jscontact::JSContactProperty,
    };
    use jmap_tools::{JsonPointer, Key};

    const SPLIT_ROUNDS: usize = 10_000;
    const SEGMENTS: &[&str] = &[
        "",
        "*",
        "0",
        "12",
        "007",
        "18446744073709551616",
        "~0",
        "~1",
        "a~1b",
        "x",
        "\u{e9}",
        "participants",
        "recurrenceOverrides",
        "2025-03-05T09:00:00",
        "calendarIds",
        "alerts",
        "roles",
        "links",
        "title",
        "convertedProperties",
        "addressBookIds",
        "phones",
        "features",
        "contexts",
        "private",
        "sortAs",
        "surname",
        "localizations",
        "name",
        "components",
    ];

    impl XorShift {
        fn pointer_text(&mut self) -> String {
            let mut text = String::new();
            if self.one_in(4) {
                text.push('/');
            }
            for position in 0..self.below(6) {
                if position > 0 {
                    text.push('/');
                }
                text.push_str(self.pick(SEGMENTS));
            }
            text
        }
    }

    fn split_matches_parse<P: PointerProperty>(text: &str) {
        let split = format!("{:?}", ConvertedKeys::<P>::into_converted_keys(text));
        let parsed = format!("{:?}", JsonPointer::<P>::parse(text).into_converted_keys());
        assert_eq!(split, parsed, "{text:?}");
        let owned = format!(
            "{:?}",
            Key::<P>::Owned(text.to_string()).into_converted_keys()
        );
        assert_eq!(owned, parsed, "{text:?}");
    }

    #[test]
    fn converted_key_split_matches_json_pointer_parse() {
        let mut rng = XorShift::new(0x6b65_7973);
        for _ in 0..SPLIT_ROUNDS {
            let text = rng.pointer_text();
            split_matches_parse::<JSCalendarProperty<String>>(&text);
            split_matches_parse::<JSContactProperty<String>>(&text);
        }
    }

    const PIECES: &[&str] = &[
        "a",
        "b",
        "k1",
        "~",
        "/",
        "0",
        "1",
        "~0",
        "~1",
        "\u{e9}",
        "\u{65e5}\u{672c}",
        " ",
        "//",
        "~~",
        "",
    ];

    #[test]
    fn from_pointer_matches_json_pointer_encode_with_exact_capacity() {
        let mut rng = XorShift::new(0x7e57_5eed);
        for _ in 0..10_000 {
            let parts: Vec<String> = (0..rng.below(7))
                .map(|_| (0..rng.below(12)).map(|_| *rng.pick(PIECES)).collect())
                .collect();
            let encoded = String::from_pointer(parts.iter().map(String::as_str));
            assert_eq!(
                encoded,
                JsonPointer::<JSContactProperty<String>>::encode(&parts),
                "{parts:?}"
            );
            assert_eq!(encoded.capacity(), encoded.len(), "{parts:?}");
        }
    }

    #[test]
    fn expect_compares_pushed_text() {
        for (pushed, text, expected) in [
            ("2025-03-05", "2025-03-05", true),
            ("2025-03-05", "2025-03-0", false),
            ("2025-03-0", "2025-03-05", false),
            ("", "", true),
            ("a", "", false),
            ("", "a", false),
        ] {
            let mut expect = Expect::new(text);
            let matched = expect.push_ascii(pushed.as_bytes()).is_ok() && expect.is_complete();
            assert_eq!(matched, expected, "{pushed:?} {text:?}");
            assert_eq!(pushed.display_eq(text), expected, "{pushed:?} {text:?}");
        }
    }
}
