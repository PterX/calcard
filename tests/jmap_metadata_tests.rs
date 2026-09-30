/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    icalendar::ICalendar,
    jscalendar::{JSCalendar, JSCalendarProperty, JSCalendarValue},
    jscontact::{JSContact, JSContactProperty, JSContactValue},
    vcard::VCard,
};
use jmap_tools::{Element, JsonPointer, JsonPointerHandler, JsonPointerItem, Key, Property, Value};
use serde_json::Value as JsonValue;
use std::str::FromStr;

const METADATA_NAMES: [&str; 2] = ["metadata", "privateMetadata"];

const CALENDAR_METADATA: &str = r#"{
    "@type": "Event",
    "metadata": {"title": {"k": "v"}, "start": {}},
    "privateMetadata": {"uid": {"k": "2025-01-01T09:00:00+00:00"}},
    "metadata/start": {"start": "2025-01-01T09:00:00+00:00", "blobId": "b1"},
    "privateMetadata/x.example/start": "2025-01-01T09:00:00+00:00"
}"#;

const CONTACT_METADATA: &str = r#"{
    "@type": "Card",
    "metadata": {"name": {"k": "v"}, "created": {}},
    "privateMetadata": {"uid": {"k": "2025-01-01T09:00:00+00:00"}},
    "metadata/created": {"created": "2025-01-01T09:00:00+00:00", "blobId": "b1"},
    "privateMetadata/x.example/updated": "2025-01-01T09:00:00+00:00"
}"#;

fn assert_selectors<P: Property + FromStr>(
    roots: [P; 2],
    as_pointer: fn(&P) -> Option<&JsonPointer<P>>,
) {
    for (name, root) in METADATA_NAMES.into_iter().zip(roots) {
        assert_eq!(P::from_str(name).ok(), Some(root.clone()), "{name}");
        assert_eq!(root.to_cow(), name);

        let selector = format!("{name}/x.example");
        let property = P::from_str(&selector).unwrap_or_else(|_| panic!("{selector}"));
        assert_eq!(property.to_cow(), selector);
        assert!(
            matches!(
                as_pointer(&property).map(JsonPointer::as_slice),
                Some([
                    JsonPointerItem::Key(Key::Property(first)),
                    JsonPointerItem::Key(Key::Owned(namespace)),
                ]) if *first == root && namespace == "x.example"
            ),
            "{property:?}"
        );
    }

    for invalid in ["metadataX/y", "/metadata/x", "uid/x", "x.example"] {
        assert!(P::from_str(invalid).is_err(), "{invalid}");
    }
}

fn assert_opaque<P: Property, E: Element<Property = P>>(value: &Value<'_, P, E>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object.iter() {
                assert!(!matches!(key, Key::Property(_)), "{key:?}");
                assert_opaque(value);
            }
        }
        Value::Array(items) => items.iter().for_each(assert_opaque),
        Value::Element(element) => panic!("typed metadata value {element:?}"),
        _ => {}
    }
}

fn assert_untyped_metadata<P: Property, E: Element<Property = P>>(
    value: &Value<'_, P, E>,
    json: &str,
) {
    let expected: JsonValue = serde_json::from_str(json).expect("valid JSON");
    assert_eq!(
        serde_json::to_value(value).expect("serializable value"),
        expected
    );

    let mut metadata_keys = 0;
    for (key, value) in value.as_object().expect("object").iter() {
        let name = key.to_string();
        if METADATA_NAMES.iter().any(|root| name.starts_with(root)) {
            assert!(matches!(key, Key::Property(_)), "{key:?}");
            assert_opaque(value);
            metadata_keys += 1;
        }
    }
    assert_eq!(metadata_keys, 4);
}

fn assert_no_metadata(text: &str) {
    for name in METADATA_NAMES {
        assert!(!text.contains(name), "{text}");
    }
}

#[test]
fn metadata_properties_parse_and_serialize() {
    assert_selectors::<JSCalendarProperty<String>>(
        [
            JSCalendarProperty::Metadata,
            JSCalendarProperty::PrivateMetadata,
        ],
        |property| match property {
            JSCalendarProperty::Pointer(pointer) => Some(pointer),
            _ => None,
        },
    );
    assert_selectors::<JSContactProperty<String>>(
        [
            JSContactProperty::Metadata,
            JSContactProperty::PrivateMetadata,
        ],
        |property| match property {
            JSContactProperty::Pointer(pointer) => Some(pointer),
            _ => None,
        },
    );

    let event = JSCalendar::<String, String>::parse(CALENDAR_METADATA).expect("valid JSCalendar");
    assert_untyped_metadata(&event.0, CALENDAR_METADATA);

    let card = JSContact::<String, String>::parse(CONTACT_METADATA).expect("valid JSContact");
    assert_untyped_metadata(&card.0, CONTACT_METADATA);
}

#[test]
fn jscalendar_export_drops_metadata() {
    for json in [
        r#"{
            "@type": "Event",
            "uid": "a",
            "title": "Base",
            "start": "2025-01-01T09:00:00",
            "duration": "PT1H",
            "recurrenceRule": {"frequency": "daily", "count": 3},
            "recurrenceOverrides": {
                "2025-01-02T09:00:00": {
                    "title": "Moved",
                    "metadata": {"x.example": {"k": "v"}},
                    "privateMetadata": {"y.example": {"k": "v"}}
                }
            },
            "metadata": {"x.example": {"k": "v"}},
            "privateMetadata": {"y.example": {"k": "v"}},
            "metadata/z.example": {"k": "v"},
            "example.com:foo": "bar"
        }"#,
        r#"{
            "@type": "Group",
            "metadata": {"x.example": {"k": "v"}},
            "privateMetadata": {"y.example": {"k": "v"}},
            "entries": [{
                "@type": "Task",
                "uid": "a",
                "title": "Base",
                "recurrenceRule": {"frequency": "daily", "count": 3},
                "recurrenceOverrides": {
                    "2025-01-02T09:00:00": {"title": "Moved", "metadata": {}}
                },
                "metadata": {"x.example": {"k": "v"}},
                "privateMetadata": {"y.example": {"k": "v"}},
                "example.com:foo": "bar"
            }]
        }"#,
    ] {
        let ical = JSCalendar::<String, String>::parse(json)
            .expect("valid JSCalendar")
            .into_icalendar()
            .expect("JSCalendar exports to iCalendar")
            .to_string();

        assert_no_metadata(&ical.to_ascii_lowercase());
        assert!(ical.contains("SUMMARY:Base"), "{ical}");
        assert!(ical.contains("SUMMARY:Moved"), "{ical}");
        assert!(ical.contains("example.com:foo"), "{ical}");
    }
}

#[test]
fn jscalendar_import_drops_metadata() {
    let jscal = ICalendar::parse(concat!(
        "BEGIN:VCALENDAR\r\n",
        "VERSION:2.0\r\n",
        "JSPROP;JSPTR=metadata:{\"x.example\":{\"k\":\"v\"}}\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:a\r\n",
        "SUMMARY:b\r\n",
        "JSPROP;JSPTR=metadata:{\"x.example\":{\"k\":\"v\"}}\r\n",
        "JSPROP;JSPTR=\"privateMetadata/y.example\":{\"k\":\"v\"}\r\n",
        "JSPROP;JSPTR=\"metadata/*\":{}\r\n",
        "JSPROP;JSPTR=\"metadata~1z.example\":{}\r\n",
        "JSPROP;JSPTR=\"example.com:foo\":\"bar\"\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    ))
    .expect("valid iCalendar")
    .into_jscalendar::<String, String>();
    let json = jscal.to_string_pretty();

    assert_no_metadata(&json);
    let value: JsonValue = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(value["entries"][0]["example.com:foo"], "bar", "{json}");
    assert_eq!(value["entries"][0]["title"], "b", "{json}");
}

#[test]
fn jscontact_export_drops_metadata() {
    let card = JSContact::<String, String>::parse(
        r#"{
            "@type": "Card",
            "version": "1.0",
            "uid": "u1",
            "name": {"full": "Jane Doe"},
            "localizations": {"de": {"metadata/x.example": {"k": "v"}}},
            "metadata": {"x.example": {"k": "v"}},
            "privateMetadata": {"y.example": {"k": "v"}},
            "metadata/z.example": {"k": "v"},
            "example.com:foo": "bar"
        }"#,
    )
    .expect("valid JSContact");
    let vcard = card
        .into_vcard()
        .expect("JSContact exports to vCard")
        .to_string();

    assert_no_metadata(&vcard.to_ascii_lowercase());
    assert!(vcard.contains("FN:Jane Doe"), "{vcard}");
    assert!(vcard.contains("example.com:foo"), "{vcard}");
}

#[test]
fn jscontact_import_drops_metadata() {
    let jscontact = VCard::parse(concat!(
        "BEGIN:VCARD\r\n",
        "VERSION:4.0\r\n",
        "FN:Jane Doe\r\n",
        "JSPROP;JSPTR=metadata:{\"x.example\":{\"k\":\"v\"}}\r\n",
        "JSPROP;JSPTR=\"privateMetadata/y.example\":{\"k\":\"v\"}\r\n",
        "JSPROP;JSPTR=\"metadata/*\":{}\r\n",
        "JSPROP;JSPTR=\"metadata~1z.example\":{}\r\n",
        "JSPROP;JSPTR=\"example.com:foo\":\"bar\"\r\n",
        "END:VCARD\r\n"
    ))
    .expect("valid vCard")
    .into_jscontact::<String, String>();
    let json = jscontact.to_string_pretty();

    assert_no_metadata(&json);
    let value: JsonValue = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(value["example.com:foo"], "bar", "{json}");
}

const VERBATIM_DATE: &str = "2025-01-01T09:00:00+00:00";

const VERBATIM_NAMESPACE: &str = concat!(
    r##"{"title":"t","start":"2025-01-01T09:00:00+00:00","created":"2025-01-01T09:00:00+00:00","##,
    r##""updated":"2025-01-01T09:00:00+00:00","id":"#ref","uid":"u","blobId":"#ref","##,
    r##""a/blobId":"b1","calendarIds":{"#ref":true},"addressBookIds":{"#ref":true},"##,
    r##""@type":"Event","kind":"individual","status":"confirmed","##,
    r##""0":["2025-01-01T09:00:00+00:00",{"blobId":"b1","@type":"Card"}],"##,
    r##""nested":{"duration":"PT1H","start":"2025-01-01T09:00:00+00:00","12":[{"id":"#ref"}]}}"##
);

type EventValue<'x> = Value<'x, JSCalendarProperty<String>, JSCalendarValue<String, String>>;
type CardValue<'x> = Value<'x, JSContactProperty<String>, JSContactValue<String, String>>;

fn verbatim_object(root_type: &str) -> String {
    format!(
        concat!(
            r#"{{"@type":"{root_type}","metadata":{{"x.example":{namespace},"title":{namespace}}},"#,
            r#""privateMetadata":{{"x.example":{namespace}}}}}"#
        ),
        root_type = root_type,
        namespace = VERBATIM_NAMESPACE,
    )
}

fn verbatim_patch() -> String {
    format!(
        concat!(
            r#"{{"metadata/x.example":{namespace},"metadata/x.example/start":"{date}","#,
            r##""metadata/x.example/a~1blobId":"b1","metadata/x.example/0":"#ref","##,
            r##""privateMetadata/title/12/id":"#ref","metadata/x.example/nested/start":"{date}"}}"##
        ),
        namespace = VERBATIM_NAMESPACE,
        date = VERBATIM_DATE,
    )
}

fn assert_verbatim<P: Property, E: Element<Property = P>>(
    json: &str,
    members: usize,
    as_pointer: fn(&P) -> Option<&JsonPointer<P>>,
) {
    let direct = Value::<P, E>::parse_json(json).expect("parse_json accepts it");
    let via_serde = serde_json::from_str::<Value<'_, P, E>>(json).expect("serde accepts it");
    for value in [&direct, &via_serde] {
        assert_eq!(serde_json::to_string(value).expect("serializes"), json);
        let mut found = 0;
        for (key, item) in value.as_object().expect("object").iter() {
            let Some(property) = key.as_property().filter(|property| property.is_opaque()) else {
                continue;
            };
            if let Some(pointer) = as_pointer(property) {
                let [_, segments @ ..] = pointer.as_slice() else {
                    panic!("empty pointer {pointer:?}");
                };
                assert!(
                    segments
                        .iter()
                        .all(|segment| matches!(segment, JsonPointerItem::Key(Key::Owned(_)))),
                    "{pointer:?}"
                );
            }
            assert_opaque(item);
            found += 1;
        }
        assert_eq!(found, members, "{json}");
    }
}

fn assert_patches_verbatim<P: Property, E: Element<Property = P>>(
    object: &str,
    as_pointer: fn(&P) -> Option<&JsonPointer<P>>,
) {
    let patch = concat!(
        r#"{"metadata/x.example/start":"2025-01-01T09:00:00+00:00","#,
        r##""metadata/x.example/a~1blobId":"b1","metadata/y.example":{"0":"#ref"},"##,
        r#""privateMetadata/z.example/12":["2025-01-01T09:00:00+00:00"]}"#
    );
    let mut target = Value::<P, E>::parse_json(object).expect("valid object");
    for (key, value) in Value::<P, E>::parse_json(patch)
        .expect("valid patch")
        .into_expanded_object()
    {
        let pointer = key
            .as_property()
            .and_then(as_pointer)
            .expect("pointer key")
            .clone();
        assert!(target.patch_jptr(pointer.iter(), value), "{pointer:?}");
    }
    let patched = serde_json::to_string(&target).expect("serializes");
    let expected = object.replace(
        r#""metadata":{"x.example":{"k":"v"}},"privateMetadata":{"z.example":{}}"#,
        concat!(
            r#""metadata":{"x.example":{"k":"v","start":"2025-01-01T09:00:00+00:00","#,
            r##""a/blobId":"b1"},"y.example":{"0":"#ref"}},"##,
            r#""privateMetadata":{"z.example":{"12":["2025-01-01T09:00:00+00:00"]}}"#
        ),
    );
    assert_eq!(patched, expected);
    assert_verbatim::<P, E>(&patched, 2, as_pointer);
}

fn event_pointer(
    property: &JSCalendarProperty<String>,
) -> Option<&JsonPointer<JSCalendarProperty<String>>> {
    match property {
        JSCalendarProperty::Pointer(pointer) => Some(pointer),
        _ => None,
    }
}

fn card_pointer(
    property: &JSContactProperty<String>,
) -> Option<&JsonPointer<JSContactProperty<String>>> {
    match property {
        JSContactProperty::Pointer(pointer) => Some(pointer),
        _ => None,
    }
}

#[test]
fn metadata_properties_are_opaque() {
    for (name, opaque) in [
        ("metadata", true),
        ("privateMetadata", true),
        ("metadata/x.example/0", true),
        ("privateMetadata/title", true),
        ("title", false),
        ("uid", false),
        ("alerts/metadata", false),
    ] {
        let event = JSCalendarProperty::<String>::try_parse(None, name).expect("event property");
        assert_eq!(event.is_opaque(), opaque, "{name}");
        let card =
            JSContactProperty::<String>::try_parse(None, name.replace("title", "name").as_str())
                .or_else(|| JSContactProperty::<String>::try_parse(None, name))
                .expect("card property");
        assert_eq!(card.is_opaque(), opaque, "{name}");
    }
}

#[test]
fn metadata_values_round_trip_verbatim() {
    let patch = verbatim_patch();
    for (json, members) in [(verbatim_object("Event"), 2), (patch.clone(), 6)] {
        assert_verbatim::<JSCalendarProperty<String>, JSCalendarValue<String, String>>(
            &json,
            members,
            event_pointer,
        );
    }
    for (json, members) in [(verbatim_object("Card"), 2), (patch, 6)] {
        assert_verbatim::<JSContactProperty<String>, JSContactValue<String, String>>(
            &json,
            members,
            card_pointer,
        );
    }

    for json in [
        format!(r#"{{"@type":"Event","start":"{VERBATIM_DATE}"}}"#),
        format!(r#"{{"@type":"Event","x.example":{VERBATIM_NAMESPACE}}}"#),
    ] {
        let value = EventValue::parse_json(&json).expect("valid event");
        assert_ne!(serde_json::to_string(&value).expect("serializes"), json);
    }
    for json in [
        format!(r#"{{"@type":"Card","created":"{VERBATIM_DATE}"}}"#),
        format!(r#"{{"@type":"Card","x.example":{VERBATIM_NAMESPACE}}}"#),
    ] {
        let value = CardValue::parse_json(&json).expect("valid card");
        assert_ne!(serde_json::to_string(&value).expect("serializes"), json);
    }
}

#[test]
fn metadata_patches_apply_verbatim() {
    assert_patches_verbatim::<JSCalendarProperty<String>, JSCalendarValue<String, String>>(
        r#"{"@type":"Event","title":"t","metadata":{"x.example":{"k":"v"}},"privateMetadata":{"z.example":{}}}"#,
        event_pointer,
    );
    assert_patches_verbatim::<JSContactProperty<String>, JSContactValue<String, String>>(
        r#"{"@type":"Card","uid":"u","metadata":{"x.example":{"k":"v"}},"privateMetadata":{"z.example":{}}}"#,
        card_pointer,
    );
}

#[test]
fn metadata_inside_nested_patch_pointer_keeps_its_next_segment_untyped() {
    let json = format!(
        r#"{{"recurrenceOverrides/2025-01-02T09:00:00/metadata/start":"{VERBATIM_DATE}"}}"#
    );
    let value = EventValue::parse_json(&json).expect("valid patch");
    assert_eq!(serde_json::to_string(&value).expect("serializes"), json);
    assert!(
        value
            .as_object()
            .expect("object")
            .iter()
            .all(|(_, item)| matches!(item, Value::Str(_))),
        "{value:?}"
    );

    let json = format!(r#"{{"localizations/de/metadata/created":"{VERBATIM_DATE}"}}"#);
    let value = CardValue::parse_json(&json).expect("valid patch");
    assert_eq!(serde_json::to_string(&value).expect("serializes"), json);
    assert!(
        value
            .as_object()
            .expect("object")
            .iter()
            .all(|(_, item)| matches!(item, Value::Str(_))),
        "{value:?}"
    );
}

const ORDINARY_NAME: &str = "ordinaryName";

const EVENT_NESTED_METADATA: &str = concat!(
    r#"{"@type":"Event","uid":"a","{root}":{"x":{"start":"2025-01-01T09:00:00+00:00"}},"#,
    r#""participants":{"p1":{"@type":"Participant","name":"A","{nested}":{"start":"2025-01-01T09:00:00+00:00","@type":"Location"}}},"#,
    r#""alerts":{"a1":{"@type":"Alert","trigger":{"@type":"OffsetTrigger","offset":"-PT15M","{nested}":{"offset":"-PT5M"}}}},"#,
    r#""participants/p2":{"@type":"Participant","{nested}":{"start":"2025-01-01T09:00:00+00:00"}},"#,
    r#""recurrenceOverrides":{"2025-01-02T09:00:00":{"title":"Moved","{root}":{"start":"2025-01-01T09:00:00+00:00"}}},"#,
    r#""recurrenceOverrides/2025-01-03T09:00:00":{"{root}":{"start":"2025-01-01T09:00:00+00:00"}}}"#
);

const GROUP_NESTED_METADATA: &str = concat!(
    r#"{"@type":"Group","entries":[{"@type":"Task","uid":"a","{root}":{"start":"2025-01-01T09:00:00+00:00"},"#,
    r#""links":{"l1":{"href":"h","{nested}":{"start":"2025-01-01T09:00:00+00:00"}}}}]}"#
);

const CARD_NESTED_METADATA: &str = concat!(
    r#"{"@type":"Card","uid":"u","{root}":{"x":{"created":"2025-01-01T09:00:00+00:00"}},"#,
    r#""name":{"full":"J","{nested}":{"created":"2025-01-01T09:00:00+00:00"}},"#,
    r#""emails":{"e1":{"address":"a@b","{nested}":{"created":"2025-01-01T09:00:00+00:00"}}},"#,
    r#""emails/e2":{"address":"c@d","{nested}":{"updated":"2025-01-01T09:00:00+00:00"}},"#,
    r#""localizations":{"de":{"{root}":{"created":"2025-01-01T09:00:00+00:00"}}}}"#
);

fn metadata_names<P: Property, E: Element<Property = P>>(
    value: &Value<'_, P, E>,
    path: &str,
    names: &mut Vec<String>,
) {
    match value {
        Value::Object(object) => {
            for (key, item) in object.iter() {
                let path = format!("{path}/{key}", key = key.to_string());
                if METADATA_NAMES
                    .iter()
                    .any(|name| path.ends_with(&format!("/{name}")))
                {
                    let kind = if key
                        .as_property()
                        .is_some_and(|property| property.is_opaque())
                    {
                        "root"
                    } else {
                        "name"
                    };
                    names.push(format!("{path}={kind}"));
                }
                metadata_names(item, &path, names);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                metadata_names(item, &format!("{path}/{index}"), names);
            }
        }
        _ => {}
    }
}

fn assert_nested_metadata_is_ordinary<P: Property, E: Element<Property = P>>(
    template: &str,
    expected: &[&str],
) {
    for root in METADATA_NAMES {
        let json = template.replace("{root}", root);
        let nested = json.replace("{nested}", root);
        let renamed = json.replace("{nested}", ORDINARY_NAME);

        let direct = Value::<P, E>::parse_json(&nested).expect("parses");
        let via_serde: Value<'_, P, E> = serde_json::from_str(&nested).expect("parses");
        let ordinary = format!("{:?}", Value::<P, E>::parse_json(&renamed).expect("parses"))
            .replace(&format!("{ORDINARY_NAME:?}"), &format!("{root:?}"));
        assert_eq!(format!("{direct:?}"), ordinary, "{nested}");
        assert_eq!(format!("{via_serde:?}"), ordinary, "{nested}");

        let mut names = Vec::new();
        metadata_names(&direct, "", &mut names);
        let expected = expected
            .iter()
            .map(|path| path.replace("{root}", root))
            .collect::<Vec<_>>();
        assert_eq!(names, expected, "{nested}");
    }
}

#[test]
fn nested_metadata_keys_are_ordinary_names() {
    assert_nested_metadata_is_ordinary::<JSCalendarProperty<String>, JSCalendarValue<String, String>>(
        EVENT_NESTED_METADATA,
        &[
            "/{root}=root",
            "/participants/p1/{root}=name",
            "/alerts/a1/trigger/{root}=name",
            "/participants/p2/{root}=name",
            "/recurrenceOverrides/2025-01-02T09:00:00/{root}=root",
            "/recurrenceOverrides/2025-01-03T09:00:00/{root}=root",
        ],
    );
    assert_nested_metadata_is_ordinary::<JSCalendarProperty<String>, JSCalendarValue<String, String>>(
        GROUP_NESTED_METADATA,
        &["/entries/0/{root}=root", "/entries/0/links/l1/{root}=name"],
    );
    assert_nested_metadata_is_ordinary::<JSContactProperty<String>, JSContactValue<String, String>>(
        CARD_NESTED_METADATA,
        &[
            "/{root}=root",
            "/name/{root}=name",
            "/emails/e1/{root}=name",
            "/emails/e2/{root}=name",
            "/localizations/de/{root}=root",
        ],
    );

    for (name, root) in [("metadata/x", true), ("participants/p1/metadata", false)] {
        let property = JSCalendarProperty::<String>::try_parse(None, name).expect("pointer");
        assert_eq!(property.is_opaque(), root, "{name}");
    }
    assert_eq!(
        JSCalendarProperty::<String>::try_parse(
            Some(&Key::Property(JSCalendarProperty::Participants)),
            "metadata"
        ),
        None
    );
    assert_eq!(
        JSCalendarProperty::<String>::from_str("privateMetadata"),
        Ok(JSCalendarProperty::PrivateMetadata)
    );
    assert_eq!(
        JSContactProperty::<String>::from_str("metadata"),
        Ok(JSContactProperty::Metadata)
    );
}
