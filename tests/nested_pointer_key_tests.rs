/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    jscalendar::{JSCalendar, JSCalendarProperty, JSCalendarValue},
    jscontact::{JSContact, JSContactProperty},
};
use jmap_tools::{Element, JsonPointer, JsonPointerItem, Key, Null, PointerDepth, Property, Value};
use std::{iter::successors, thread};

const SMALL_STACK: usize = 256 * 1024;
const DEEP_LEVELS: usize = 200;
const LEVELS_PAST_LIMIT: usize = 8;
const SHALLOW_LEVELS: usize = PointerDepth::LIMIT as usize + LEVELS_PAST_LIMIT;
const CALENDAR_PATHS: [&[&str]; 3] = [
    &[],
    &["convertedProperties"],
    &["recurrenceOverrides", "2025-01-01T00:00:00"],
];
const CONTACT_PATHS: [&[&str]; 3] = [&[], &["localizations"], &["convertedProperties"]];

type CalendarProperty = JSCalendarProperty<String>;
type ContactProperty = JSContactProperty<String>;

struct Chain {
    prefix: &'static [&'static str],
    suffix: &'static [&'static str],
}

const TRAILING_TOKEN: Chain = Chain {
    prefix: &[],
    suffix: &["x"],
};
const CONVERTED_PROPERTIES: Chain = Chain {
    prefix: &["convertedProperties"],
    suffix: &[],
};
const RECURRENCE_OVERRIDE: Chain = Chain {
    prefix: &["recurrenceOverrides", "2025-01-01T00:00:00"],
    suffix: &[],
};
const LOCALIZATIONS: Chain = Chain {
    prefix: &["localizations"],
    suffix: &[],
};

impl Chain {
    fn keys(&self, levels: usize) -> Vec<String> {
        successors(Some(String::from("a/b")), |key| {
            Some(JsonPointer::<Null>::encode(
                self.prefix
                    .iter()
                    .copied()
                    .chain([key.as_str()])
                    .chain(self.suffix.iter().copied()),
            ))
        })
        .take(levels)
        .collect()
    }
}

fn tested_levels(keys: &[String]) -> impl Iterator<Item = (usize, &String)> {
    (1..).zip(keys)
}

fn innermost<P: Property>(
    property: &P,
    as_pointer: fn(&P) -> Option<&JsonPointer<P>>,
) -> (usize, String) {
    let (levels, innermost) = successors(as_pointer(property), |pointer| {
        pointer.iter().find_map(|item| match item {
            JsonPointerItem::Key(Key::Property(inner)) => as_pointer(inner),
            _ => None,
        })
    })
    .fold((0, None), |(levels, _), pointer| {
        (levels + 1, Some(pointer))
    });
    (
        levels,
        innermost.map(ToString::to_string).unwrap_or_default(),
    )
}

fn calendar_pointer(property: &CalendarProperty) -> Option<&JsonPointer<CalendarProperty>> {
    match property {
        JSCalendarProperty::Pointer(pointer) => Some(pointer),
        _ => None,
    }
}

fn contact_pointer(property: &ContactProperty) -> Option<&JsonPointer<ContactProperty>> {
    match property {
        JSContactProperty::Pointer(pointer) => Some(pointer),
        _ => None,
    }
}

fn check_nesting<P: Property>(
    keys: &[String],
    (levels, text): (usize, &str),
    key: &Key<'_, P>,
    as_pointer: fn(&P) -> Option<&JsonPointer<P>>,
) {
    let limit = usize::from(PointerDepth::LIMIT);
    let Key::Property(property) = key else {
        panic!("{levels} levels: {key:?} is not a property key");
    };
    let innermost_text = levels
        .checked_sub(limit)
        .and_then(|untyped| keys.get(untyped))
        .map_or("a/b", String::as_str);
    assert_eq!(
        innermost(property, as_pointer),
        (levels.min(limit), innermost_text.to_string()),
        "{levels} levels"
    );
    assert_eq!(key.to_string(), text, "{levels} levels");
}

fn only_key<'x, P: Property, E: Element<Property = P>>(
    value: &'x Value<'_, P, E>,
    path: &[&str],
) -> &'x Key<'x, P> {
    let object = path
        .iter()
        .try_fold(value, |value, member| {
            value
                .as_object()
                .and_then(|object| object.iter().find(|(key, _)| key.to_string() == *member))
                .map(|(_, value)| value)
        })
        .and_then(Value::as_object)
        .expect("path leads to an object");
    let mut keys = object.keys();
    let key = keys.next().expect("one member");
    assert!(keys.next().is_none());
    key
}

fn wrapped(key: &str, path: &[&str]) -> String {
    path.iter()
        .rev()
        .fold(format!("{{\"{key}\":1}}"), |json, member| {
            format!("{{\"{member}\":{json}}}")
        })
}

fn on_small_stack(test: impl FnOnce() + Send + 'static) {
    thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(test)
        .expect("spawns")
        .join()
        .expect("finishes");
}

fn check_calendar(keys: &[String], (levels, key): (usize, &String), path: &[&str]) {
    let json = wrapped(key, path);
    let case = (levels, key.as_str());
    let parsed = JSCalendar::<String, String>::parse(&json).expect("parses");
    check_nesting(keys, case, only_key(&parsed.0, path), calendar_pointer);
    let parsed: Value<'_, CalendarProperty, JSCalendarValue<String, String>> =
        serde_json::from_str(&json).expect("parses");
    check_nesting(keys, case, only_key(&parsed, path), calendar_pointer);
}

fn check_contact(keys: &[String], (levels, key): (usize, &String), path: &[&str]) {
    let json = wrapped(key, path);
    let parsed = JSContact::<String, String>::parse(&json).expect("parses");
    let case = (levels, key.as_str());
    check_nesting(keys, case, only_key(&parsed.0, path), contact_pointer);
}

#[test]
fn nested_jscalendar_pointer_keys_stop_at_the_depth_limit() {
    on_small_stack(|| {
        for chain in [TRAILING_TOKEN, CONVERTED_PROPERTIES, RECURRENCE_OVERRIDE] {
            let keys = chain.keys(SHALLOW_LEVELS);
            for case in tested_levels(&keys) {
                for path in CALENDAR_PATHS {
                    check_calendar(&keys, case, path);
                }
            }
        }
        let keys = TRAILING_TOKEN.keys(DEEP_LEVELS);
        let deepest = tested_levels(&keys).last().expect("deep key");
        check_calendar(&keys, deepest, &[]);
    });
}

#[test]
fn nested_jscontact_pointer_keys_stop_at_the_depth_limit() {
    on_small_stack(|| {
        for chain in [TRAILING_TOKEN, CONVERTED_PROPERTIES, LOCALIZATIONS] {
            let keys = chain.keys(SHALLOW_LEVELS);
            for case in tested_levels(&keys) {
                for path in CONTACT_PATHS {
                    check_contact(&keys, case, path);
                }
            }
        }
        let keys = TRAILING_TOKEN.keys(DEEP_LEVELS);
        let deepest = tested_levels(&keys).last().expect("deep key");
        check_contact(&keys, deepest, &[]);
    });
}
