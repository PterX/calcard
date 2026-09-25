/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

#![no_main]

use calcard::{
    jscalendar::{JSCalendarProperty, JSCalendarValue},
    jscontact::{JSContactProperty, JSContactValue},
};
use calcard_fuzz::Id;
use jmap_tools::{
    Element, JsonPointer, JsonPointerHandler, JsonPointerItem, Key, Null, Property, Value,
};
use libfuzzer_sys::fuzz_target;
use std::{
    borrow::Cow,
    fmt::Write,
    hash::{BuildHasher, BuildHasherDefault, DefaultHasher},
};

const MAX_POINTER_LINES: usize = 16;
const MAX_DERIVED: usize = 24;
const MAX_DEPTH: usize = 6;
const MAX_KEYS: usize = 64;
const PATCH_MARK: &str = "fuzz-patch";

fuzz_target!(|data: &[u8]| {
    let mut parts = data.splitn(2, |byte| *byte == 0);
    let raw = parts.next().unwrap_or_default();
    let pointers = String::from_utf8_lossy(parts.next().unwrap_or_default());
    let document = String::from_utf8_lossy(raw);
    let case = JsonCase {
        raw,
        document: &document,
        pointers: &pointers,
    };
    case.check::<JSCalendarProperty<Id>, JSCalendarValue<Id, Id>>();
    case.check::<JSContactProperty<Id>, JSContactValue<Id, Id>>();
    case.check::<Null, Null>();
});

struct JsonCase<'x> {
    raw: &'x [u8],
    document: &'x str,
    pointers: &'x str,
}

impl JsonCase<'_> {
    fn check<P: Property, E: Element<Property = P>>(&self) {
        let direct = Value::<P, E>::parse_json(self.document);
        let via_serde =
            serde_json::from_str::<Value<'_, P, E>>(self.document).map_err(|err| err.to_string());
        match (&direct, &via_serde) {
            (Ok(direct), Ok(via_serde)) => {
                assert_eq!(
                    format!("{direct:?}"),
                    format!("{via_serde:?}"),
                    "parse_json and serde_json build different values"
                );
                assert_eq!(
                    direct.layout(),
                    via_serde.layout(),
                    "parse_json and serde_json borrow different strings"
                );
            }
            (Err(direct), Err(via_serde)) => assert_eq!(
                direct, via_serde,
                "parse_json and serde_json report different errors"
            ),
            _ => panic!("parse_json and serde_json disagree: {direct:?} against {via_serde:?}"),
        }

        let from_slice =
            serde_json::from_slice::<Value<'_, P, E>>(self.raw).map_err(|err| err.to_string());
        if std::str::from_utf8(self.raw).is_ok() {
            assert_eq!(
                format!("{from_slice:?}"),
                format!("{via_serde:?}"),
                "serde_json from_slice and from_str disagree on valid UTF-8"
            );
        }

        let Ok(value) = direct else {
            return;
        };
        let json = serde_json::to_string(&value).expect("parsed value serializes");
        let reparsed = Value::<P, E>::parse_json(&json)
            .unwrap_or_else(|err| panic!("serialized value does not parse: {err}: {json}"));
        let json_again = serde_json::to_string(&reparsed).expect("reparsed value serializes");
        let stable = !value.has_float() && !value.has_slash_key();
        if stable {
            assert_eq!(json_again, json, "JSON serialization is not a fixed point");
        }
        let pretty = serde_json::to_string_pretty(&value).expect("parsed value serializes pretty");
        let from_pretty = Value::<P, E>::parse_json(&pretty)
            .unwrap_or_else(|err| panic!("pretty JSON does not parse: {err}: {pretty}"));
        if stable {
            assert_eq!(
                serde_json::to_string(&from_pretty).expect("value serializes"),
                json,
                "pretty and compact JSON describe different values"
            );
        }
        assert!(
            value.clone().into_owned() == value,
            "into_owned changed the value"
        );

        KeyContract(value.keys()).check();

        let derived = value.pointer_paths();
        for pointer in self
            .pointers
            .lines()
            .take(MAX_POINTER_LINES)
            .chain(derived.iter().map(String::as_str))
        {
            PointerCase {
                value: &value,
                text: pointer,
            }
            .check();
        }
    }
}

trait Inspect<P: Property> {
    fn keys(&self) -> Vec<Key<'static, P>>;
    fn walk_keys(&self, out: &mut Vec<Key<'static, P>>);
    fn has_float(&self) -> bool;
    fn has_slash_key(&self) -> bool;
    fn layout(&self) -> String;
    fn pointer_paths(&self) -> Vec<String>;
    fn walk_layout(&self, out: &mut String);
    fn walk_paths(&self, prefix: &mut Vec<String>, out: &mut Vec<String>);
}

impl<P: Property, E: Element<Property = P>> Inspect<P> for Value<'_, P, E> {
    fn keys(&self) -> Vec<Key<'static, P>> {
        let mut keys = Vec::new();
        self.walk_keys(&mut keys);
        keys
    }

    fn walk_keys(&self, out: &mut Vec<Key<'static, P>>) {
        match self {
            Value::Array(items) => items.iter().for_each(|item| item.walk_keys(out)),
            Value::Object(map) => {
                for (key, item) in map.iter() {
                    if out.len() >= MAX_KEYS {
                        return;
                    }
                    out.push(key.to_owned());
                    item.walk_keys(out);
                }
            }
            _ => {}
        }
    }

    fn has_float(&self) -> bool {
        match self {
            Value::Number(_) => self.is_f64(),
            Value::Array(items) => items.iter().any(|item| item.has_float()),
            Value::Object(map) => map.values().any(|item| item.has_float()),
            _ => false,
        }
    }

    fn has_slash_key(&self) -> bool {
        match self {
            Value::Array(items) => items.iter().any(|item| item.has_slash_key()),
            Value::Object(map) => map
                .iter()
                .any(|(key, item)| key.to_string().starts_with('/') || item.has_slash_key()),
            _ => false,
        }
    }

    fn layout(&self) -> String {
        let mut layout = String::new();
        self.walk_layout(&mut layout);
        layout
    }

    fn pointer_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        self.walk_paths(&mut Vec::new(), &mut paths);
        paths
    }

    fn walk_layout(&self, out: &mut String) {
        match self {
            Value::Str(Cow::Borrowed(_)) => out.push('b'),
            Value::Str(Cow::Owned(_)) => out.push('o'),
            Value::Array(items) => {
                out.push('[');
                items.iter().for_each(|item| item.walk_layout(out));
                out.push(']');
            }
            Value::Object(map) => {
                out.push('{');
                for (key, item) in map.iter() {
                    let _ = write!(out, "{key:?}");
                    item.walk_layout(out);
                }
                out.push('}');
            }
            _ => out.push('.'),
        }
    }

    fn walk_paths(&self, prefix: &mut Vec<String>, out: &mut Vec<String>) {
        if out.len() >= MAX_DERIVED || prefix.len() >= MAX_DEPTH {
            return;
        }
        let null = Value::Null;
        let children: Vec<(String, &Self)> = match self {
            Value::Object(map) => map
                .iter()
                .map(|(key, item)| (key.to_string().into_owned(), item))
                .collect(),
            Value::Array(items) => items
                .iter()
                .chain([&null])
                .enumerate()
                .map(|(index, item)| (index.to_string(), item))
                .collect(),
            _ => return,
        };
        for (segment, item) in children {
            prefix.push(segment);
            if out.len() < MAX_DERIVED {
                let pointer = JsonPointer::<P>::encode(prefix.iter());
                out.push(format!("/{pointer}"));
                if let Some((_, parent)) = prefix.split_last() {
                    let mut wildcard = JsonPointer::<P>::encode(parent);
                    wildcard.push_str(if parent.is_empty() { "*" } else { "/*" });
                    out.push(wildcard);
                }
                out.push(pointer);
            }
            item.walk_paths(prefix, out);
            prefix.pop();
        }
    }
}

struct KeyContract<P: Property>(Vec<Key<'static, P>>);

impl<P: Property> KeyContract<P> {
    fn check(&self) {
        let hasher = BuildHasherDefault::<DefaultHasher>::default();
        for key in &self.0 {
            let text = key.to_string();
            let borrowed = Key::<P>::Borrowed(&text);
            let key_first = *key == borrowed;
            let text_first = borrowed == *key;
            assert!(
                key_first && text_first && *key == text.as_ref(),
                "key {key:?} does not equal its own text {text:?}"
            );
            assert_eq!(
                hasher.hash_one(key),
                hasher.hash_one(&borrowed),
                "key {key:?} hashes unlike its text {text:?}"
            );
            for other in &self.0 {
                assert_eq!(
                    key == other,
                    text == other.to_string(),
                    "key equality of {key:?} and {other:?} disagrees with their text"
                );
            }
        }
    }
}

struct PointerCase<'x, 'y, P: Property, E: Element<Property = P>> {
    value: &'x Value<'y, P, E>,
    text: &'x str,
}

impl<P: Property, E: Element<Property = P>> PointerCase<'_, '_, P, E> {
    fn check(&self) {
        let pointer = JsonPointer::<P>::parse(self.text);
        let shown = pointer.to_string();
        let reparsed = JsonPointer::<P>::parse(&shown);
        let leading_empty_key = matches!(
            pointer.first(),
            Some(JsonPointerItem::Key(key)) if key.to_string().is_empty()
        ) || pointer.iter().any(
            |item| matches!(item, JsonPointerItem::Key(key) if key.to_string().starts_with('/')),
        );
        assert!(
            leading_empty_key || reparsed.to_string() == shown,
            "pointer display is not a fixed point of parse for {:?}: {shown:?} became {reparsed}",
            self.text
        );
        let encoded = JsonPointer::<P>::encode(self.text.split('/'));
        let _ = JsonPointer::<P>::parse(&encoded).to_string();

        let mut results = Vec::new();
        self.value.eval_jptr(pointer.iter(), &mut results);

        let mark = Value::Str(Cow::Borrowed(PATCH_MARK));
        let mut patched = self.value.clone();
        if patched.patch_jptr(pointer.iter(), mark.clone()) && Self::is_concrete(&pointer) {
            let mut found = Vec::new();
            patched.eval_jptr(pointer.iter(), &mut found);
            assert!(
                found.iter().any(|item| item.as_ref() == &mark),
                "a successful patch at {:?} is not visible to eval: {found:?}",
                self.text
            );
        }
        let mut removed = self.value.clone();
        removed.patch_jptr(pointer.iter(), Value::Null);
    }

    fn is_concrete(pointer: &JsonPointer<P>) -> bool {
        !pointer.is_empty()
            && pointer
                .iter()
                .all(|item| matches!(item, JsonPointerItem::Key(_) | JsonPointerItem::Number(_)))
    }
}
