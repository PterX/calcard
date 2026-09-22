/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    icalendar::{
        ICalendar, ICalendarDisplayType, ICalendarFeatureType, ICalendarParameterName,
        ICalendarParameterValue, ICalendarProperty, ICalendarRelationshipType, ICalendarUserTypes,
    },
    jscalendar::{
        JSCalendar, JSCalendarLinkDisplay, JSCalendarParticipantKind,
        JSCalendarParticipationStatus, JSCalendarProperty, JSCalendarRelation, JSCalendarValue,
        JSCalendarVirtualLocationFeature,
    },
};
use jmap_tools::{JsonPointer, JsonPointerItem, Key, Value};
use serde_json::Value as JsonValue;

type JSValue = Value<'static, JSCalendarProperty<String>, JSCalendarValue<String, String>>;
type JSKey = Key<'static, JSCalendarProperty<String>>;
type TypedKey = Result<JSCalendarProperty<String>, String>;

const JSPROP_REFERENCE: &str = "draft-ietf-calext-jscalendar-icalendar-28 Section 4.1.2 (JSPROP values form a PatchObject) and Section 4.2.2 (the JSPTR value MUST be quoted); draft-ietf-calext-jscalendarbis-20 Section 1.5.9 (patch values MUST be valid for the property being set)";

fn ical_event(lines: &[&str]) -> String {
    let mut ical = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\n");
    for line in lines {
        ical.push_str(line);
        ical.push_str("\r\n");
    }
    ical.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    ical
}

fn import(ical: &str) -> JSCalendar<'static, String, String> {
    ICalendar::parse(ical)
        .expect("valid iCalendar")
        .into_jscalendar::<String, String>()
}

fn normalize(jscal: &JSCalendar<'_, String, String>) -> JsonValue {
    serde_json::from_str(&jscal.to_string_pretty()).unwrap_or_default()
}

fn roundtrip(
    ical: &str,
) -> (
    JSCalendar<'static, String, String>,
    ICalendar,
    JSCalendar<'static, String, String>,
) {
    let imported = import(ical);
    let exported = imported
        .clone()
        .into_icalendar()
        .expect("JSCalendar exports to iCalendar");
    let reimported = import(&exported.to_string());
    assert_eq!(
        normalize(&reimported),
        normalize(&imported),
        "{JSPROP_REFERENCE}\n{exported}"
    );
    (imported, exported, reimported)
}

fn event<'a>(jscal: &'a JSCalendar<'static, String, String>) -> &'a JSValue {
    jscal
        .0
        .as_object_and_get(&Key::Property(JSCalendarProperty::Entries))
        .and_then(Value::as_array)
        .and_then(<[JSValue]>::first)
        .expect("one entry")
}

fn member<'a>(value: &'a JSValue, path: &[JSKey]) -> &'a JSValue {
    path.iter().fold(value, |value, key| {
        value
            .as_object_and_get(key)
            .unwrap_or_else(|| panic!("{JSPROP_REFERENCE}: missing {key:?} in {value:?}"))
    })
}

fn typed_keys(value: &JSValue) -> Vec<TypedKey> {
    value
        .as_object()
        .expect("object value")
        .keys()
        .map(|key| match key {
            Key::Property(property) => Ok(property.clone()),
            key => Err(key.to_string().into_owned()),
        })
        .collect()
}

fn exported_params(
    ical: &ICalendar,
    property: ICalendarProperty,
    parameter: ICalendarParameterName,
) -> Vec<ICalendarParameterValue> {
    ical.components
        .iter()
        .flat_map(|component| component.entries.iter())
        .filter(|entry| entry.name == property)
        .flat_map(|entry| entry.params.iter())
        .filter(|param| param.name == parameter)
        .map(|param| param.value.clone())
        .collect()
}

fn assert_typed_map(
    lines: &[&str],
    path: &[JSKey],
    expected_keys: &[TypedKey],
    exported_param: (ICalendarProperty, ICalendarParameterName),
    expected_params: &[ICalendarParameterValue],
    reference: &str,
) {
    let (imported, exported, reimported) = roundtrip(&ical_event(lines));

    assert_eq!(
        typed_keys(member(event(&imported), path)),
        expected_keys,
        "{JSPROP_REFERENCE}; {reference}"
    );
    assert_eq!(
        typed_keys(member(event(&reimported), path)),
        expected_keys,
        "{reference}\n{exported}"
    );

    let (property, parameter) = exported_param;
    assert_eq!(
        exported_params(&exported, property, parameter),
        expected_params,
        "{reference}\n{exported}"
    );
}

#[test]
fn jsprop_features_restore_typed_keys() {
    assert_typed_map(
        &[
            "UID:jsprop-features",
            "CONFERENCE;VALUE=URI;JSID=v1:https://video.example.com/1",
            r#"JSPROP;JSPTR="virtualLocations/v1/features":{"audio":true\,"video":true\,"example.com:whiteboard":true}"#,
        ],
        &[
            Key::Property(JSCalendarProperty::VirtualLocations),
            Key::Borrowed("v1"),
            Key::Property(JSCalendarProperty::Features),
        ],
        &[
            Ok(JSCalendarProperty::VirtualLocationFeature(
                JSCalendarVirtualLocationFeature::Audio,
            )),
            Ok(JSCalendarProperty::VirtualLocationFeature(
                JSCalendarVirtualLocationFeature::Video,
            )),
            Err("example.com:whiteboard".into()),
        ],
        (
            ICalendarProperty::Conference,
            ICalendarParameterName::Feature,
        ),
        &[
            ICalendarParameterValue::Feature(ICalendarFeatureType::Audio),
            ICalendarParameterValue::Feature(ICalendarFeatureType::Video),
        ],
        "draft-ietf-calext-jscalendarbis-20 Sections 1.8.2 and 3.2.7; draft-ietf-calext-jscalendar-icalendar-28 Sections 2.3.10 and 3.7; RFC 7986 Section 6.3 (FEATURE values are iana-token or x-name, so vendor features convert to JSPROP)",
    );
}

#[test]
fn jsprop_relation_restores_typed_keys() {
    assert_typed_map(
        &[
            "UID:jsprop-relation",
            "RELATED-TO:parent-uid",
            r#"JSPROP;JSPTR="relatedTo/parent-uid/relation":{"parent":true\,"snooze":true\,"example.com:blocks":true}"#,
        ],
        &[
            Key::Property(JSCalendarProperty::RelatedTo),
            Key::Borrowed("parent-uid"),
            Key::Property(JSCalendarProperty::Relation),
        ],
        &[
            Ok(JSCalendarProperty::RelationValue(
                JSCalendarRelation::Parent,
            )),
            Ok(JSCalendarProperty::RelationValue(
                JSCalendarRelation::Snooze,
            )),
            Err("example.com:blocks".into()),
        ],
        (
            ICalendarProperty::RelatedTo,
            ICalendarParameterName::Reltype,
        ),
        &[
            ICalendarParameterValue::Reltype(ICalendarRelationshipType::Parent),
            ICalendarParameterValue::Reltype(ICalendarRelationshipType::Snooze),
        ],
        "draft-ietf-calext-jscalendarbis-20 Sections 1.5.10, 1.8.2 and 7.6.13; draft-ietf-calext-jscalendar-icalendar-28 Section 2.3.35; RFC 5545 Section 3.2.15 (RELTYPE values are iana-token or x-name, so vendor relations convert to JSPROP)",
    );
}

#[test]
fn jsprop_link_display_restores_typed_keys() {
    assert_typed_map(
        &[
            "UID:jsprop-display",
            "IMAGE;VALUE=URI;JSID=l1:https://example.com/i.png",
            r#"JSPROP;JSPTR="links/l1/display":{"badge":true\,"example.com:banner":true}"#,
        ],
        &[
            Key::Property(JSCalendarProperty::Links),
            Key::Borrowed("l1"),
            Key::Property(JSCalendarProperty::Display),
        ],
        &[
            Ok(JSCalendarProperty::LinkDisplay(
                JSCalendarLinkDisplay::Badge,
            )),
            Err("example.com:banner".into()),
        ],
        (ICalendarProperty::Image, ICalendarParameterName::Display),
        &[ICalendarParameterValue::Display(
            ICalendarDisplayType::Badge,
        )],
        "draft-ietf-calext-jscalendarbis-20 Sections 1.5.11, 1.8.2 and 7.6.6; RFC 7986 Section 6.1 (DISPLAY values are iana-token or x-name, so vendor display values convert to JSPROP)",
    );
}

#[test]
fn jsprop_scalar_restores_typed_value() {
    let (imported, exported, reimported) = roundtrip(&ical_event(&[
        "UID:jsprop-kind",
        "ATTENDEE;JSID=p1:mailto:p1@example.com",
        r#"JSPROP;JSPTR="participants/p1/kind":"resource""#,
    ]));
    let path = [
        Key::Property(JSCalendarProperty::Participants),
        Key::Borrowed("p1"),
        Key::Property(JSCalendarProperty::Kind),
    ];

    for jscal in [&imported, &reimported] {
        assert!(
            matches!(
                member(event(jscal), &path),
                Value::Element(JSCalendarValue::ParticipantKind(
                    JSCalendarParticipantKind::Resource
                ))
            ),
            "{JSPROP_REFERENCE}; draft-ietf-calext-jscalendarbis-20 Section 3.4.6\n{exported}"
        );
    }
    assert_eq!(
        exported_params(
            &exported,
            ICalendarProperty::Attendee,
            ICalendarParameterName::Cutype
        ),
        [ICalendarParameterValue::Cutype(
            ICalendarUserTypes::Resource
        )],
        "{exported}"
    );
}

fn pointer_matches(pointer: &JsonPointer<JSCalendarProperty<String>>, expected: &[&str]) -> bool {
    pointer.as_slice().len() == expected.len()
        && pointer
            .as_slice()
            .iter()
            .zip(expected)
            .all(|(item, expected)| matches!(item, JsonPointerItem::Key(key) if key == expected))
}

fn assert_typed_overrides(
    jscal: &JSCalendar<'static, String, String>,
    expected_ids: &[&str],
    exported: &ICalendar,
) {
    let overrides = member(
        event(jscal),
        &[Key::Property(JSCalendarProperty::RecurrenceOverrides)],
    );
    let mut ids = typed_keys(overrides)
        .into_iter()
        .map(|key| match key {
            Ok(JSCalendarProperty::DateTime(date_time)) => date_time.to_rfc3339(),
            key => panic!(
                "draft-ietf-calext-jscalendarbis-20 Section 3.3.4: untyped recurrence id {key:?}\n{exported}"
            ),
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, expected_ids, "{exported}");

    let patch = overrides
        .as_object()
        .and_then(|overrides| {
            overrides
                .iter()
                .find_map(|(key, patch)| (key == &"2025-01-15T09:00:00").then_some(patch))
        })
        .and_then(Value::as_object)
        .expect("patched occurrence");
    let status = patch
        .iter()
        .find_map(|(key, value)| match key {
            Key::Property(JSCalendarProperty::Pointer(pointer))
                if pointer_matches(pointer, &["participants", "p1", "participationStatus"]) =>
            {
                Some(value)
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "draft-ietf-calext-jscalendarbis-20 Section 1.5.9: untyped patch key in {patch:?}\n{exported}"
            )
        });
    assert!(
        matches!(
            status,
            Value::Element(JSCalendarValue::ParticipationStatus(
                JSCalendarParticipationStatus::Declined
            ))
        ),
        "{JSPROP_REFERENCE}\n{exported}"
    );
}

#[test]
fn jsprop_recurrence_overrides_restore_typed_keys() {
    let (imported, exported, reimported) = roundtrip(&ical_event(&[
        "UID:jsprop-overrides",
        "DTSTART;TZID=Europe/Berlin:20250108T090000",
        "RRULE:FREQ=WEEKLY",
        "SUMMARY:Sync",
        "ATTENDEE;JSID=p1:mailto:p1@example.com",
        r#"JSPROP;JSPTR="recurrenceOverrides":{"2025-01-15T09:00:00":{"title":"Moved"\,"participants/p1/participationStatus":"declined"}\,"2025-01-22T09:00:00":{"excluded":true}\,"2025-01-30T10:00:00":{}}"#,
    ]));
    let expected = [
        "2025-01-15T09:00:00",
        "2025-01-22T09:00:00",
        "2025-01-30T10:00:00",
    ];

    assert_typed_overrides(&imported, &expected, &exported);
    assert_typed_overrides(&reimported, &expected, &exported);

    let rendered = exported.to_string();
    for line in [
        "RECURRENCE-ID;TZID=Europe/Berlin:20250115T090000",
        "EXDATE;TZID=Europe/Berlin:20250122T090000",
        "RDATE;TZID=Europe/Berlin:20250130T100000",
        "ATTENDEE;PARTSTAT=DECLINED;JSID=p1:mailto:p1@example.com",
    ] {
        assert!(
            rendered.contains(line),
            "draft-ietf-calext-jscalendarbis-20 Section 3.3.4: missing {line}\n{rendered}"
        );
    }
    assert!(!rendered.contains("JSPROP"), "{rendered}");
}

#[test]
fn jsprop_recurrence_override_member_restores_typed_patch_keys() {
    let (imported, exported, reimported) = roundtrip(&ical_event(&[
        "UID:jsprop-override-member",
        "DTSTART;TZID=Europe/Berlin:20250108T090000",
        "RRULE:FREQ=WEEKLY",
        "SUMMARY:Sync",
        "ATTENDEE;JSID=p1:mailto:p1@example.com",
        "EXDATE;TZID=Europe/Berlin:20250122T090000",
        r#"JSPROP;JSPTR="recurrenceOverrides/2025-01-15T09:00:00":{"title":"Moved"\,"participants/p1/participationStatus":"declined"}"#,
    ]));
    let expected = ["2025-01-15T09:00:00", "2025-01-22T09:00:00"];

    assert_typed_overrides(&imported, &expected, &exported);
    assert_typed_overrides(&reimported, &expected, &exported);
}

#[test]
fn jsprop_value_escaping_its_pointer_is_not_applied() {
    let imported = import(&ical_event(&[
        "UID:jsprop-escape",
        "CONFERENCE;VALUE=URI;JSID=v1:https://video.example.com/1",
        r#"JSPROP;JSPTR="virtualLocations/v1/features":{"audio":true}}\,"title":{"x":1"#,
    ]));
    let entry = event(&imported);

    assert!(
        member(
            entry,
            &[
                Key::Property(JSCalendarProperty::VirtualLocations),
                Key::Borrowed("v1")
            ]
        )
        .as_object_and_get(&Key::Property(JSCalendarProperty::Features))
        .is_none(),
        "{JSPROP_REFERENCE}"
    );
    assert!(
        entry
            .as_object_and_get(&Key::Property(JSCalendarProperty::Title))
            .is_none(),
        "{JSPROP_REFERENCE}"
    );
}

const EXISTING_REFERENCE: &str = "draft-ietf-calext-jscalendar-icalendar-28 Section 4.1.2: the PatchObject is applied after all other iCalendar elements are converted, and a pointer to an existing property MUST be ignored";

fn imported_event(lines: &[&str]) -> JsonValue {
    normalize(&import(&ical_event(lines)))["entries"][0].clone()
}

#[test]
fn jsprop_targeting_existing_property_is_ignored() {
    let event = imported_event(&[
        "UID:jsprop-existing",
        "SUMMARY:Sync",
        "ATTENDEE;ROLE=CHAIR;JSID=p1:mailto:p1@example.com",
        r#"JSPROP;JSPTR=title:"Replaced""#,
        r#"JSPROP;JSPTR="participants/p1/roles":{"attendee":true}"#,
        r#"JSPROP;JSPTR=uid:"other-uid""#,
        r#"JSPROP;JSPTR="@type":"Task""#,
    ]);

    assert_eq!(event["title"], "Sync", "{EXISTING_REFERENCE}");
    assert_eq!(event["uid"], "jsprop-existing", "{EXISTING_REFERENCE}");
    assert_eq!(event["@type"], "Event", "{EXISTING_REFERENCE}");
    assert_eq!(
        event["participants"]["p1"]["roles"],
        serde_json::json!({"chair": true}),
        "{EXISTING_REFERENCE}"
    );
}

#[test]
fn jsprop_targeting_existing_nested_member_is_ignored() {
    let event = imported_event(&[
        "UID:jsprop-existing-member",
        "ATTENDEE;ROLE=CHAIR;JSID=p1:mailto:p1@example.com",
        "ATTACH;JSID=l1:https://example.com/a.pdf",
        r#"JSPROP;JSPTR="participants/p1/calendarAddress":"mailto:other@example.com""#,
        r#"JSPROP;JSPTR="participants/p1/roles/chair":true"#,
        r#"JSPROP;JSPTR="links/l1/rel":"icon""#,
    ]);

    assert_eq!(
        event["participants"]["p1"],
        serde_json::json!({"@type": "Participant", "calendarAddress": "mailto:p1@example.com", "roles": {"chair": true}}),
        "{EXISTING_REFERENCE}"
    );
    assert_eq!(
        event["links"]["l1"]["rel"], "enclosure",
        "{EXISTING_REFERENCE}"
    );
}

#[test]
fn jsprop_adding_new_member_to_existing_object_is_applied() {
    let event = imported_event(&[
        "UID:jsprop-new-member",
        "ATTENDEE;ROLE=CHAIR;JSID=p1:mailto:p1@example.com",
        "CONFERENCE;VALUE=URI;FEATURE=AUDIO;JSID=v1:https://video.example.com/1",
        r#"JSPROP;JSPTR="participants/p1/roles/attendee":true"#,
        r#"JSPROP;JSPTR="participants/p1/example.com:badge":"gold""#,
        r#"JSPROP;JSPTR="virtualLocations/v1/features/screen":true"#,
    ]);

    assert_eq!(
        event["participants"]["p1"],
        serde_json::json!({
            "@type": "Participant", "calendarAddress": "mailto:p1@example.com",
            "roles": {"chair": true, "attendee": true}, "example.com:badge": "gold"
        }),
        "{EXISTING_REFERENCE}"
    );
    assert_eq!(
        event["virtualLocations"]["v1"]["features"],
        serde_json::json!({"audio": true, "screen": true}),
        "{EXISTING_REFERENCE}"
    );
}

#[test]
fn jsprop_recurrence_override_is_applied_after_override_components() {
    let ical = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VEVENT\r\nUID:jsprop-late\r\nDTSTART;TZID=Europe/Berlin:20250108T090000\r\n",
        "RRULE:FREQ=WEEKLY\r\nSUMMARY:Sync\r\n",
        "JSPROP;JSPTR=\"recurrenceOverrides/2025-01-15T09:00:00\":{\"title\":\"From JSPROP\"}\r\n",
        "JSPROP;JSPTR=\"recurrenceOverrides/2025-01-22T09:00:00\":{\"title\":\"Added\"}\r\n",
        "END:VEVENT\r\n",
        "BEGIN:VEVENT\r\nUID:jsprop-late\r\nRECURRENCE-ID;TZID=Europe/Berlin:20250115T090000\r\n",
        "DTSTART;TZID=Europe/Berlin:20250115T090000\r\nSUMMARY:From component\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    );
    let event = normalize(&import(ical))["entries"][0].clone();

    assert_eq!(
        event["recurrenceOverrides"],
        serde_json::json!({
            "2025-01-15T09:00:00": {"title": "From component"},
            "2025-01-22T09:00:00": {"title": "Added"}
        }),
        "{EXISTING_REFERENCE}"
    );

    let event = normalize(&import(&ical.replace(
        "JSPROP;JSPTR=\"recurrenceOverrides/2025-01-15T09:00:00\":{\"title\":\"From JSPROP\"}\r\nJSPROP;JSPTR=\"recurrenceOverrides/2025-01-22T09:00:00\":{\"title\":\"Added\"}",
        "JSPROP;JSPTR=\"recurrenceOverrides\":{\"2025-01-22T09:00:00\":{\"title\":\"Added\"}}",
    )))["entries"][0]
        .clone();

    assert_eq!(
        event["recurrenceOverrides"],
        serde_json::json!({"2025-01-15T09:00:00": {"title": "From component"}}),
        "{EXISTING_REFERENCE}"
    );
}

fn export_roundtrip(json: &str) -> (Vec<String>, JsonValue, JsonValue) {
    let source = JSCalendar::<String, String>::parse(json).expect("valid JSCalendar");
    let exported = source
        .clone()
        .into_icalendar()
        .expect("JSCalendar exports to iCalendar")
        .to_string();
    let jsprops = exported
        .replace("\r\n ", "")
        .split("\r\n")
        .filter(|line| line.starts_with("JSPROP"))
        .map(str::to_string)
        .collect();
    let reimported = import(&exported);
    let strip = |jscal: &JSCalendar<'_, String, String>| {
        let mut event = normalize(jscal)["entries"][0].clone();
        if let Some(event) = event.as_object_mut() {
            event.remove("iCalendar");
        }
        event
    };
    (jsprops, strip(&source), strip(&reimported))
}

fn roles_event(organizer: Option<&str>, participants: &str) -> String {
    let organizer = organizer
        .map(|address| format!(r#""organizerCalendarAddress": "{address}","#))
        .unwrap_or_default();
    format!(
        r#"{{"@type": "Group", "entries": [{{
            "@type": "Event", "uid": "jsprop-roles", "title": "Review",
            "start": "2026-05-04T09:00:00", "timeZone": "Europe/Madrid", "duration": "PT1H",
            {organizer} "participants": {participants}
        }}]}}"#
    )
}

#[test]
fn exported_roles_jsprops_round_trip() {
    for (participants, expected_jsprops) in [
        (
            r#"{"p1": {"@type": "Participant", "calendarAddress": "mailto:p1@example.com", "roles": {"attendee": true, "example.com:scribe": true}}}"#,
            vec![
                r#"JSPROP;JSPTR="participants/p1/roles":{"attendee":true\,"example.com:scribe":true}"#,
            ],
        ),
        (
            r#"{"p1": {"@type": "Participant", "calendarAddress": "mailto:p1@example.com", "roles": {"chair": true, "attendee": true, "example.com:scribe": true}}}"#,
            vec![
                r#"JSPROP;JSPTR="participants/p1/roles/attendee":true"#,
                r#"JSPROP;JSPTR="participants/p1/roles/example.com:scribe":true"#,
            ],
        ),
    ] {
        let (jsprops, source, reimported) = export_roundtrip(&roles_event(None, participants));
        assert_eq!(jsprops, expected_jsprops, "{EXISTING_REFERENCE}");
        assert_eq!(reimported, source, "{EXISTING_REFERENCE}");
    }
}

#[test]
fn exported_organizer_roles_use_member_pointers() {
    for (roles, expected_jsprops, expected_roles) in [
        (
            r#"{"owner": true, "attendee": true}"#,
            vec![r#"JSPROP;JSPTR="participants/p1/roles/attendee":true"#],
            serde_json::json!({"owner": true, "attendee": true}),
        ),
        (
            r#"{"attendee": true}"#,
            vec![r#"JSPROP;JSPTR="participants/p1/roles/attendee":true"#],
            serde_json::json!({"owner": true, "attendee": true}),
        ),
    ] {
        let participants = format!(
            r#"{{"p1": {{"@type": "Participant", "calendarAddress": "mailto:p1@example.com", "roles": {roles}}}}}"#
        );
        let (jsprops, mut source, reimported) =
            export_roundtrip(&roles_event(Some("mailto:p1@example.com"), &participants));
        assert_eq!(jsprops, expected_jsprops, "{EXISTING_REFERENCE}");
        source["participants"]["p1"]["roles"] = expected_roles;
        assert_eq!(
            reimported, source,
            "{EXISTING_REFERENCE}; the ORGANIZER participant always converts with the owner role"
        );
    }
}

#[test]
fn exported_links_without_icalendar_representation_round_trip() {
    for (links, expected_jsprops) in [
        (
            r#"{"l1": {"@type": "Link", "rel": "enclosure"}}"#,
            vec![r#"JSPROP;JSPTR="links":{"l1":{"@type":"Link"\,"rel":"enclosure"}}"#],
        ),
        (
            r#"{"l1": {"@type": "Link", "rel": "enclosure"}, "l2": {"@type": "Link", "href": "https://example.com/a.pdf", "rel": "enclosure", "example.com:pages": 3}}"#,
            vec![
                r#"JSPROP;JSPTR="links/l2/example.com:pages":3"#,
                r#"JSPROP;JSPTR="links/l1":{"@type":"Link"\,"rel":"enclosure"}"#,
            ],
        ),
    ] {
        let json = format!(
            r#"{{"@type": "Group", "entries": [{{"@type": "Event", "uid": "jsprop-links", "title": "Docs", "links": {links}}}]}}"#
        );
        let (mut jsprops, source, reimported) = export_roundtrip(&json);
        jsprops.sort_unstable();
        let mut expected_jsprops = expected_jsprops;
        expected_jsprops.sort_unstable();
        assert_eq!(jsprops, expected_jsprops, "{EXISTING_REFERENCE}");
        assert_eq!(reimported, source, "{EXISTING_REFERENCE}");
    }
}
