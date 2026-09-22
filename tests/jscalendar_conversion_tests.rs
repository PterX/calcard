/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    common::{blob::NoBlobIds, export::ExportError},
    icalendar::{
        ICalendar, ICalendarParameterName, ICalendarParameterValue, ICalendarProperty,
        ICalendarValue, Uri,
    },
    jscalendar::{
        JSCalendar, JSCalendarProperty, RecurrenceOverrides, export::ExportOptions,
        import::ImportOptions, uuid5,
    },
};
use jmap_tools::{JsonPointer, JsonPointerHandler, Key, Value};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

const DRAFT: &str = "draft-ietf-calext-jscalendar-icalendar-28";

fn event_json(members: &str) -> String {
    format!(
        r#"{{"@type": "Group", "entries": [{{"@type": "Event", "uid": "conversion",
            "start": "2025-01-06T09:00:00", "timeZone": "Europe/Berlin", "duration": "PT1H",
            {members}}}]}}"#
    )
}

fn export(json: &str) -> ICalendar {
    JSCalendar::<String, String>::parse(json)
        .expect("valid JSCalendar")
        .into_icalendar()
        .expect("JSCalendar exports to iCalendar")
}

fn import(ical: &str) -> JSCalendar<'static, String, String> {
    ICalendar::parse(ical)
        .expect("valid iCalendar")
        .into_jscalendar::<String, String>()
}

fn import_blobs(ical: &str) -> JSCalendar<'static, String, String> {
    ICalendar::parse(ical)
        .expect("valid iCalendar")
        .into_jscalendar_with::<String, String, _>(
            ImportOptions::new().with_blob_ids(|data| Some(format!("blob-{}", data.len()))),
        )
        .expect("converts")
}

fn json(jscal: &JSCalendar<'_, String, String>) -> JsonValue {
    serde_json::from_str(&jscal.to_string_pretty()).expect("valid JSON")
}

fn entry(jscal: &JSCalendar<'_, String, String>) -> JsonValue {
    json(jscal)["entries"][0].clone()
}

fn unfolded(ical: &ICalendar) -> String {
    ical.to_string().replace("\r\n ", "")
}

fn lines(ical: &ICalendar) -> Vec<String> {
    unfolded(ical).split("\r\n").map(str::to_string).collect()
}

fn ical_event(lines: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:conversion\r\nDTSTART:20250101T100000Z\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn entries_named<'x>(
    ical: &'x ICalendar,
    name: &'x ICalendarProperty,
) -> impl Iterator<Item = &'x calcard::icalendar::ICalendarEntry> + 'x {
    ical.components
        .iter()
        .flat_map(|component| component.entries.iter())
        .filter(move |entry| &entry.name == name)
}

const BINARY_ATTACH: &str =
    "ATTACH;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=application/pdf:JVBERv8Agf4=\r\n";

#[test]
fn r11_1_identical_links_get_unique_keys_in_linear_time() {
    let count = 2_000;
    let ical = ical_event(&"ATTACH:https://example.com/a\r\n".repeat(count));
    let started = std::time::Instant::now();
    let imported = import(&ical);
    let elapsed = started.elapsed();
    let links = entry(&imported)["links"]
        .as_object()
        .map(|links| links.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let base = uuid5("https://example.com/a");

    assert_eq!(links.len(), count, "{DRAFT} Section 2.1.3: keys are unique");
    for key in [base.clone(), format!("{base}-2"), format!("{base}-{count}")] {
        assert!(links.contains(&key), "missing {key}");
    }
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "importing {count} identical links took {elapsed:?}"
    );

    let mixed = entry(&import(&ical_event(&format!(
        concat!(
            "ATTACH:https://example.com/a\r\n",
            "ATTACH;JSID={base}-2:https://example.com/b\r\n",
            "ATTACH:https://example.com/a\r\n",
            "ATTACH;JSID=k:https://example.com/c\r\n",
            "ATTACH;JSID=k;FILENAME=d.pdf:https://example.com/d\r\n",
        ),
        base = base
    ))));
    let links = &mixed["links"];
    assert_eq!(
        links.as_object().map(|links| links.len()),
        Some(4),
        "{DRAFT} Section 2.1.3: a JSID parameter MUST be used as key\n{mixed:#}"
    );
    assert_eq!(links[&base]["href"], "https://example.com/a");
    assert_eq!(links[format!("{base}-2")]["href"], "https://example.com/b");
    assert_eq!(links[format!("{base}-3")]["href"], "https://example.com/a");
    assert_eq!(links["k"]["href"], "https://example.com/d");
    assert_eq!(links["k"]["title"], "d.pdf");
}

#[test]
fn r11_3_binary_link_keys_are_stable_across_import_modes() {
    for value in [
        BINARY_ATTACH,
        "ATTACH;ENCODING=BASE64;FMTTYPE=application/pdf:JVBERv8Agf4=\r\n",
        "ATTACH;FMTTYPE=application/pdf:data:application/pdf;base64,JVBERv8Agf4=\r\n",
        "IMAGE;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=image/png:JVBERv8Agf4=\r\n",
    ] {
        let ical = ical_event(value);
        let plain = entry(&import(&ical));
        let blobs = entry(&import_blobs(&ical));
        let plain_keys = plain["links"]
            .as_object()
            .map(|links| links.keys().cloned().collect::<Vec<_>>());
        let blob_keys = blobs["links"]
            .as_object()
            .map(|links| links.keys().cloned().collect::<Vec<_>>());

        assert_eq!(
            plain_keys, blob_keys,
            "{DRAFT} Section 2.1.3: keys SHOULD be stable\n{plain:#}\n{blobs:#}"
        );
        assert_eq!(
            plain_keys,
            Some(vec![uuid5(b"%PDF\xff\x00\x81\xfe")]),
            "{value}"
        );

        let key = plain_keys.into_iter().flatten().next().unwrap_or_default();
        let mut update = import(&ical);
        let pointer =
            JsonPointer::<JSCalendarProperty<String>>::parse(&format!("links/{key}/title"));
        let applied = update
            .0
            .as_object_mut()
            .and_then(|group| group.get_mut(&Key::Property(JSCalendarProperty::Entries)))
            .and_then(Value::as_array_mut)
            .and_then(|entries| entries.first_mut())
            .is_some_and(|event| event.patch_jptr(pointer.iter(), Value::Str("Report".into())));
        assert!(
            applied,
            "a patch on the key returned with blob ids applies to a plain import"
        );

        let exported = unfolded(&update.into_icalendar().expect("exports"));
        assert!(
            !exported.contains("JSID="),
            "{DRAFT} Section 2.3.3: the JSID parameter may be omitted for UUIDv5 keys\n{exported}"
        );
    }
}

#[test]
fn r11_8_binary_links_keep_their_relation() {
    let source = event_json(
        r#""links": {"l1": {"@type": "Link", "blobId": "b1", "rel": "describedby", "contentType": "text/html"},
                     "l2": {"@type": "Link", "blobId": "b1", "rel": "enclosure"},
                     "l3": {"@type": "Link", "blobId": "b1", "rel": "example.com:reference"}}"#,
    );
    let ical = JSCalendar::<String, String>::parse(&source)
        .expect("valid JSCalendar")
        .into_icalendar_with(
            ExportOptions::new().with_blob_resolver(|_: &String| Some(b"<p>hi</p>".to_vec())),
        )
        .expect("exports");
    let rendered = unfolded(&ical);
    assert!(
        rendered.contains(
            "ATTACH;ENCODING=BASE64;LINKREL=describedby;FMTTYPE=text/html;JSID=l1;VALUE=BINARY:"
        ),
        "{DRAFT} Section 3.4: LINKREL MAY be set on ATTACH with a BINARY value\n{rendered}"
    );
    assert!(
        rendered.contains("ATTACH;ENCODING=BASE64;JSID=l2;VALUE=BINARY:"),
        "{rendered}"
    );
    assert!(
        rendered.contains(
            "ATTACH;ENCODING=BASE64;LINKREL=\"example.com:reference\";JSID=l3;VALUE=BINARY:"
        ),
        "RFC 9253 Section 6.1: an extension relation type is a quoted URI\n{rendered}"
    );

    for imported in [import(&rendered), import_blobs(&rendered)] {
        let links = entry(&imported)["links"].clone();
        assert_eq!(links["l1"]["rel"], "describedby", "{links:#}");
        assert_eq!(links["l2"]["rel"], "enclosure", "{links:#}");
        assert_eq!(links["l3"]["rel"], "example.com:reference", "{links:#}");
    }

    let uri_attach = import(&ical_event(
        "ATTACH;LINKREL=describedby:https://example.com/a\r\n",
    ));
    assert_eq!(
        entry(&uri_attach)["links"][uuid5("https://example.com/a")]["rel"],
        "enclosure",
        "{DRAFT} Section 2.3.3: an ATTACH with a URI value converts to rel enclosure"
    );
}

#[test]
fn r11_9_fmttype_sets_the_data_url_media_type() {
    let plain = entry(&import(&ical_event(BINARY_ATTACH)));
    let link = plain["links"]
        .as_object()
        .and_then(|links| links.values().next())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        link["href"], "data:application/pdf;base64,JVBERv8Agf4=",
        "{DRAFT} Section 2.3.3: FMTTYPE MUST be set in the mediatype part of the data URL"
    );
    assert_eq!(link["contentType"], "application/pdf");

    let parameters = ical_event("ATTACH:data:text/plain;charset=iso-8859-1;base64,aGVsbG8=\r\n");
    let plain = entry(&import(&parameters));
    let blobs = entry(&import_blobs(&parameters));
    let first = |value: &JsonValue| {
        value["links"]
            .as_object()
            .and_then(|links| links.values().next())
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(
        first(&plain)["href"],
        "data:text/plain;charset=iso-8859-1;base64,aGVsbG8=",
        "RFC 2397 Section 3: mediatype parameters are part of the data URL"
    );
    assert_eq!(
        first(&blobs)["contentType"],
        "text/plain;charset=iso-8859-1"
    );
}

#[test]
fn r11_10_base64_attach_without_value_binary_exports_as_binary() {
    let ical = ical_event("ATTACH;ENCODING=BASE64;FMTTYPE=text/plain:aGVsbG8gd29ybGQ=\r\n");
    let imported = import(&ical);
    let link = entry(&imported)["links"]
        .as_object()
        .and_then(|links| links.values().next())
        .cloned()
        .unwrap_or_default();
    assert_eq!(link["href"], "data:text/plain;base64,aGVsbG8gd29ybGQ=");

    let exported = imported.into_icalendar().expect("exports");
    assert_eq!(
        entries_named(&exported, &ICalendarProperty::Attach)
            .filter_map(|entry| entry.values.first())
            .collect::<Vec<_>>(),
        [&ICalendarValue::Binary(b"hello world".to_vec())],
        "RFC 5545 Section 3.8.1.1\n{exported}"
    );
}

#[test]
fn r11_11_unusable_data_hrefs_are_preserved_as_jsprop() {
    let ical = export(&event_json(
        r#""links": {"bad": {"@type": "Link", "href": "data:text/plain;base64,@@@", "rel": "enclosure"},
                     "good": {"@type": "Link", "href": "data:text/plain;base64,aGVsbG8=", "rel": "enclosure"}}"#,
    ));
    let rendered = unfolded(&ical);
    assert!(
        rendered.contains(r#"JSPROP;JSPTR="links/bad":{"@type":"Link"\,"href":"data:text/plain\;base64\,@@@"\,"rel":"enclosure"}"#),
        "{rendered}"
    );
    assert!(
        rendered.contains("ATTACH;JSID=good:data:text/plain;base64,aGVsbG8="),
        "{rendered}"
    );
    let reimported = entry(&import(&rendered));
    assert_eq!(
        reimported["links"]["bad"]["href"],
        "data:text/plain;base64,@@@"
    );
    assert_eq!(
        reimported["links"]["good"]["href"],
        "data:text/plain;base64,aGVsbG8="
    );
}

#[test]
fn r11_6_link_property_and_value_type_parameters() {
    let source = event_json(
        r#""links": {
            "a": {"@type": "Link", "href": "https://example.com/a.pdf"},
            "b": {"@type": "Link", "href": "https://example.com/b", "rel": "preview", "title": "Preview"},
            "c": {"@type": "Link", "href": "https://example.com/c.png", "rel": "icon"},
            "d": {"@type": "Link", "href": "https://example.com/d", "rel": "https://example.com/rel/d"},
            "e": {"@type": "Link", "href": "https://example.com/e", "rel": "not a relation"},
            "f": {"@type": "Link", "href": "https://example.com/f.png", "rel": "not a relation", "display": {"badge": true}}
        },
        "iCalendar": {"convertedProperties": {"links/f/href": {"name": "link"}}},
        "virtualLocations": {"v": {"@type": "VirtualLocation", "uri": "https://video.example.com/v"}},
        "locations": {"l": {"@type": "Location", "name": "Room", "coordinates": "geo:48.198634,16.371648;u=40"}}"#,
    );
    let ical = export(&source);
    let rendered = lines(&ical);
    for expected in [
        "ATTACH;JSID=a:https://example.com/a.pdf",
        "LINK;LINKREL=preview;LABEL=Preview;JSID=b;VALUE=URI:https://example.com/b",
        "IMAGE;JSID=c;VALUE=URI:https://example.com/c.png",
        "LINK;LINKREL=\"https://example.com/rel/d\";JSID=d;VALUE=URI:https://example.com/d",
        "JSPROP;JSPTR=\"links/e\":{\"@type\":\"Link\"\\,\"href\":\"https://example.com/e\"\\,\"rel\":\"not a relation\"}",
        "CONFERENCE;JSID=v;VALUE=URI:https://video.example.com/v",
        "COORDINATES;VALUE=URI:geo:48.198634,16.371648;u=40",
    ] {
        assert!(
            rendered.iter().any(|line| line == expected),
            "jscalendarbis-20 Section 1.5.11 (rel default enclosure); RFC 9253 Sections 6.1 and 8.2; RFC 7986 Sections 5.10 and 5.11; icalendar-jscalendar-extensions-07 Section 4.1: missing {expected}\n{rendered:#?}"
        );
    }
    let link_lines = rendered
        .iter()
        .filter(|line| line.starts_with("LINK;") || line.starts_with("LINK:"));
    assert_eq!(link_lines.clone().count(), 3, "{rendered:#?}");
    for line in link_lines {
        assert!(
            line.contains("LINKREL="),
            "RFC 9253 Section 6.1: LINKREL MUST be specified on all LINK properties: {line}"
        );
    }

    let reimported = entry(&import(&ical.to_string()));
    let source: JsonValue = serde_json::from_str(&source).expect("json");
    assert_eq!(reimported["links"]["a"]["rel"], "enclosure");
    assert_eq!(reimported["links"]["b"]["title"], "Preview");
    assert_eq!(reimported["links"]["d"]["rel"], "https://example.com/rel/d");
    assert_eq!(
        reimported["links"]["e"], source["entries"][0]["links"]["e"],
        "{DRAFT} Section 4.1.2\n{reimported:#}"
    );
    assert_eq!(
        reimported["links"]["f"]["href"],
        "https://example.com/f.png"
    );
}

#[test]
fn r11_6_vendor_values_that_are_not_parameter_tokens_convert_to_jsprop() {
    let source = event_json(
        r#""virtualLocations": {"v": {"@type": "VirtualLocation", "uri": "https://video.example.com/v", "features": {"example.com:whiteboard": true}}},
           "links": {"l": {"@type": "Link", "href": "https://example.com/i.png", "rel": "icon", "display": {"example.com:banner": true, "x-poster": true}}},
           "relatedTo": {"parent": {"relation": {"example.com:blocks": true}}},
           "participants": {"p": {"@type": "Participant", "calendarAddress": "mailto:p@example.com", "kind": "example.com:robot"}}"#,
    );
    let ical = export(&source);
    let rendered = lines(&ical);
    for expected in [
        "CONFERENCE;JSID=v;VALUE=URI:https://video.example.com/v",
        "JSPROP;JSPTR=\"virtualLocations/v/features\":{\"example.com:whiteboard\":true}",
        "IMAGE;DISPLAY=x-poster;JSID=l;VALUE=URI:https://example.com/i.png",
        "JSPROP;JSPTR=\"links/l/display/example.com:banner\":true",
        "RELATED-TO:parent",
        "JSPROP;JSPTR=\"relatedTo/parent/relation\":{\"example.com:blocks\":true}",
        "ATTENDEE;JSID=p:mailto:p@example.com",
        "JSPROP;JSPTR=\"participants/p/kind\":\"example.com:robot\"",
    ] {
        assert!(
            rendered.iter().any(|line| line == expected),
            "RFC 7986 Sections 6.1 and 6.3; RFC 5545 Sections 3.2.3 and 3.2.15: parameter values are iana-token or x-name; missing {expected}\n{rendered:#?}"
        );
    }

    let reimported = entry(&import(&ical.to_string()));
    let mut source: JsonValue = serde_json::from_str(&source).expect("json");
    let source = source["entries"][0].take();
    for pointer in [
        "/virtualLocations/v/features",
        "/links/l/display",
        "/relatedTo/parent/relation",
        "/participants/p/kind",
    ] {
        assert_eq!(
            reimported.pointer(pointer),
            source.pointer(pointer),
            "{pointer}\n{reimported:#}"
        );
    }
}

#[test]
fn r11_14_jsid_is_written_when_a_jsptr_references_the_key() {
    let link = uuid5("https://example.com/a.pdf");
    let conference = uuid5("https://video.example.com/1");
    let location = uuid5("Room 1");
    let participant = uuid5("mailto:p@example.com");
    let source = event_json(&format!(
        r#""links": {{"{link}": {{"@type": "Link", "href": "https://example.com/a.pdf", "rel": "enclosure", "example.com:pages": 3}}}},
           "virtualLocations": {{"{conference}": {{"@type": "VirtualLocation", "uri": "https://video.example.com/1", "example.com:pin": "1234"}}}},
           "locations": {{"{location}": {{"@type": "Location", "name": "Room 1", "example.com:floor": 2}}}},
           "participants": {{"{participant}": {{"@type": "Participant", "calendarAddress": "mailto:p@example.com", "example.com:badge": "gold"}}}}"#
    ));
    let ical = export(&source);
    let rendered = unfolded(&ical);

    for key in [&link, &conference, &location, &participant] {
        assert!(
            rendered.contains(&format!("JSID={key}")),
            "{DRAFT} Sections 2.3.3, 2.3.10, 2.3.25 and 2.3.29: JSID may only be omitted if the key is not part of a JSON pointer\n{rendered}"
        );
    }

    let reimported = entry(&import(&ical.to_string()));
    let source: JsonValue = serde_json::from_str(&source).expect("json");
    for pointer in [
        format!("/links/{link}/example.com:pages"),
        format!("/virtualLocations/{conference}/example.com:pin"),
        format!("/locations/{location}/example.com:floor"),
        format!("/participants/{participant}/example.com:badge"),
    ] {
        assert_eq!(
            reimported.pointer(&pointer),
            source["entries"][0].pointer(&pointer),
            "{pointer}\n{rendered}"
        );
    }

    let plain = export(&event_json(&format!(
        r#""links": {{"{link}": {{"@type": "Link", "href": "https://example.com/a.pdf", "rel": "enclosure"}}}}"#
    )));
    assert!(!unfolded(&plain).contains("JSID="), "{plain}");
}

#[test]
fn r11_15_no_blob_ids_resolver_behaves_like_no_resolver() {
    let source =
        event_json(r#""links": {"doc": {"@type": "Link", "blobId": "blob7", "rel": "enclosure"}}"#);
    let default = JSCalendar::<String, String>::parse(&source)
        .expect("valid JSCalendar")
        .into_icalendar()
        .expect("a blobId without a resolver is preserved");
    let explicit = JSCalendar::<String, String>::parse(&source)
        .expect("valid JSCalendar")
        .into_icalendar_with(ExportOptions::new().with_resolver(NoBlobIds))
        .expect("a blobId without a resolver is preserved");

    assert_eq!(default, explicit);
    let rendered = unfolded(&default);
    assert!(
        rendered.contains(r#""blobId":"blob7""#),
        "RFC 9610 Section 3: the blobId survives as a JSPROP\n{rendered}"
    );
}

#[test]
fn r11_15_a_resolver_that_fails_is_an_export_error() {
    let source =
        event_json(r#""links": {"doc": {"@type": "Link", "blobId": "blob7", "rel": "enclosure"}}"#);
    assert_eq!(
        JSCalendar::<String, String>::parse(&source)
            .expect("valid JSCalendar")
            .into_icalendar_with(ExportOptions::new().with_blob_resolver(|_: &String| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "blob7".to_string(),
        })
    );
}

#[test]
fn r12_9_binaries_are_deduplicated_by_content() {
    let ical = ical_event(concat!(
        "ATTACH;ENCODING=BASE64;VALUE=BINARY;JSID=a:aGVsbG8=\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY;JSID=b:aGVsbG8=\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY;JSID=c:d29ybGQ=\r\n",
        "IMAGE;ENCODING=BASE64;VALUE=BINARY;JSID=d:d29ybGQh\r\n",
        "ATTACH;VALUE=URI;JSID=e:data:text/plain;base64,aGVsbG8=\r\n",
    ));
    let mut calls = Vec::new();
    let imported = ICalendar::parse(&ical)
        .expect("valid iCalendar")
        .into_jscalendar_with::<String, String, _>(ImportOptions::new().with_blob_ids(
            |data: &[u8]| {
                calls.push(data.to_vec());
                Some(format!("blob{}", calls.len()))
            },
        ))
        .expect("converts");
    let links = entry(&imported)["links"].clone();

    assert_eq!(calls.len(), 3, "one generator call per distinct content");
    assert_eq!(links["a"]["blobId"], links["b"]["blobId"]);
    assert_eq!(links["a"]["blobId"], links["e"]["blobId"]);
    assert_ne!(
        links["a"]["blobId"], links["c"]["blobId"],
        "binaries of equal length and different content keep distinct blob ids"
    );
    assert_ne!(links["a"]["blobId"], links["d"]["blobId"]);
}

#[test]
fn r8_1_max_embedded_size_aborts_export() {
    let overrides = (7..20)
        .map(|day| format!(r#""2025-01-{day:02}T09:00:00": {{"title": "Moved {day}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let source = event_json(&format!(
        r#""title": "Daily", "recurrenceRule": {{"frequency": "daily"}},
           "links": {{"doc": {{"@type": "Link", "blobId": "blob", "rel": "enclosure"}}}},
           "recurrenceOverrides": {{{overrides}}}"#
    ));
    let blob = vec![7u8; 1_000];
    let export_with = |max_embedded_size: usize| {
        let mut calls = 0;
        let ical = JSCalendar::<String, String>::parse(&source)
            .expect("valid JSCalendar")
            .into_icalendar_with(
                ExportOptions::new()
                    .max_embedded_size(max_embedded_size)
                    .with_blob_resolver(|_: &String| {
                        calls += 1;
                        Some(blob.clone())
                    }),
            );
        (ical, calls)
    };

    let (ical, calls) = export_with(1_000);
    let ical = ical.expect("one distinct binary is charged once, however often it is emitted");
    assert_eq!(calls, 1);
    assert_eq!(
        entries_named(&ical, &ICalendarProperty::Attach).count(),
        14,
        "{ical}"
    );
    assert!(
        export_with(999).0.is_err(),
        "the export stops once the embedded binaries exceed the limit"
    );

    let data_source = source.replace(
        r#""blobId": "blob""#,
        r#""href": "data:application/octet-stream;base64,AAAA""#,
    );
    let export_data = |max_embedded_size: usize| {
        JSCalendar::<String, String>::parse(&data_source)
            .expect("valid JSCalendar")
            .into_icalendar_with(ExportOptions::new().max_embedded_size(max_embedded_size))
    };
    assert!(
        export_data(2).is_err(),
        "decoded data: hrefs count toward the limit"
    );
    assert!(
        export_data(3).is_ok(),
        "the three decoded bytes are charged once"
    );
}

#[test]
fn resolver_is_called_once_per_blob_id_and_may_move_the_data() {
    let source = event_json(
        r#""title": "Daily", "recurrenceRule": {"frequency": "daily"},
           "links": {"doc": {"@type": "Link", "blobId": "doc", "rel": "enclosure"}},
           "recurrenceOverrides": {
               "2025-01-07T09:00:00": {"title": "Moved", "links/extra": {"@type": "Link", "blobId": "doc", "rel": "enclosure"}},
               "2025-01-08T09:00:00": {"title": "Moved again"}
           }"#,
    );
    let mut blobs = HashMap::from([("doc".to_string(), b"content".to_vec())]);
    let ical = JSCalendar::<String, String>::parse(&source)
        .expect("valid JSCalendar")
        .into_icalendar_with(
            ExportOptions::new().with_blob_resolver(|blob_id: &String| blobs.remove(blob_id)),
        )
        .expect("exports");

    assert_eq!(
        entries_named(&ical, &ICalendarProperty::Attach)
            .filter(
                |entry| entry.values.first() == Some(&ICalendarValue::Binary(b"content".to_vec()))
            )
            .count(),
        4,
        "{ical}"
    );
}

#[test]
fn r11_4_organizer_roles_survive_with_a_co_owner() {
    let organizer = uuid5("mailto:org@example.com");
    for (participants, pointer, expected) in [
        (
            format!(
                r#""{organizer}": {{"@type": "Participant", "calendarAddress": "mailto:org@example.com", "roles": {{"owner": true, "attendee": true}}}},
                   "p2": {{"@type": "Participant", "calendarAddress": "mailto:p2@example.com", "roles": {{"owner": true, "attendee": true}}}}"#
            ),
            format!("/participants/{organizer}/roles"),
            serde_json::json!({"owner": true, "attendee": true}),
        ),
        (
            r#""org1": {"@type": "Participant", "calendarAddress": "mailto:org@example.com", "roles": {"owner": true, "attendee": true}, "description": "Organizer"},
               "p2": {"@type": "Participant", "calendarAddress": "mailto:p2@example.com", "roles": {"owner": true}}"#
                .to_string(),
            "/participants/org1/roles".to_string(),
            serde_json::json!({"owner": true, "attendee": true}),
        ),
        (
            format!(
                r#""{organizer}": {{"@type": "Participant", "calendarAddress": "mailto:org@example.com", "roles": {{"attendee": true}}}},
                   "p2": {{"@type": "Participant", "calendarAddress": "mailto:p2@example.com", "roles": {{"owner": true}}}}"#
            ),
            format!("/participants/{organizer}/roles"),
            serde_json::json!({"attendee": true}),
        ),
        (
            format!(
                r#""{organizer}": {{"@type": "Participant", "calendarAddress": "mailto:org@example.com", "roles": {{"owner": true}}}},
                   "p2": {{"@type": "Participant", "calendarAddress": "mailto:p2@example.com", "roles": {{"owner": true}}}}"#
            ),
            format!("/participants/{organizer}/roles"),
            serde_json::json!({"owner": true}),
        ),
        (
            r#""org1": {"@type": "Participant", "calendarAddress": "mailto:org@example.com", "roles": {"owner": true}, "description": "Organizer"},
               "p2": {"@type": "Participant", "calendarAddress": "mailto:p2@example.com", "roles": {"owner": true}}"#
                .to_string(),
            "/participants/org1/roles".to_string(),
            serde_json::json!({"owner": true}),
        ),
    ] {
        let source = event_json(&format!(
            r#""organizerCalendarAddress": "mailto:org@example.com", "participants": {{{participants}}}"#
        ));
        let ical = export(&source);
        let reimported = entry(&import(&ical.to_string()));

        assert_eq!(
            reimported.pointer(&pointer),
            Some(&expected),
            "{DRAFT} Sections 2.3.29 and 3.6\n{ical}\n{reimported:#}"
        );
        assert_eq!(
            reimported["participants"]["p2"]["roles"]["owner"], true,
            "{ical}"
        );
    }
}

#[test]
fn r11_7_participants_without_calendar_address_round_trip() {
    let source = event_json(
        r#""participants": {
            "p1": {"@type": "Participant", "name": "No address"},
            "p2": {"@type": "Participant", "name": "Resource", "kind": "resource", "roles": {"chair": true}, "participationStatus": "accepted", "expectReply": true, "example.com:note": "x"}
        }"#,
    );
    let ical = export(&source);
    let rendered = unfolded(&ical);
    assert!(!rendered.contains("ATTENDEE"), "{rendered}");
    assert_eq!(
        rendered.matches("BEGIN:PARTICIPANT").count(),
        2,
        "{rendered}"
    );

    let reimported = entry(&import(&ical.to_string()));
    let source: JsonValue = serde_json::from_str(&source).expect("json");
    for (participant, member) in [
        ("p1", "name"),
        ("p2", "name"),
        ("p2", "kind"),
        ("p2", "roles"),
        ("p2", "participationStatus"),
        ("p2", "expectReply"),
        ("p2", "example.com:note"),
    ] {
        assert_eq!(
            reimported["participants"][participant][member],
            source["entries"][0]["participants"][participant][member],
            "{DRAFT} Section 3.6: conversion without calendarAddress is implementation-specific, data is kept in the PARTICIPANT component; {participant}/{member}\n{rendered}"
        );
    }
}

#[test]
fn participant_component_jsprop_pointers_are_relative() {
    let ical = ical_event(concat!(
        "BEGIN:PARTICIPANT\r\nUID:p\r\nCALENDAR-ADDRESS:mailto:p@example.com\r\nDESCRIPTION:Speaker\r\n",
        "JSPROP;JSPTR=\"example.com:badge\":\"gold\"\r\nEND:PARTICIPANT\r\n",
        "ATTENDEE:mailto:p@example.com\r\n",
    ));
    let imported = import(&ical);
    let key = uuid5("mailto:p@example.com");
    assert_eq!(
        entry(&imported)["participants"][&key]["example.com:badge"],
        "gold"
    );

    let exported = imported.into_icalendar().expect("exports");
    let reimported = entry(&import(&exported.to_string()));
    assert_eq!(
        reimported["participants"][&key]["example.com:badge"], "gold",
        "{DRAFT} Section 4.1.2: a JSPROP in a PARTICIPANT component is relative to the Participant\n{exported}"
    );
}

#[test]
fn r2_organizer_exports_a_uri_value() {
    let ical = ical_event("ORGANIZER;CN=Org:mailto:org@example.com\r\n");
    let parsed = ICalendar::parse(&ical).expect("valid iCalendar");
    let exported = import(&ical).into_icalendar().expect("exports");
    let organizer = |ical: &ICalendar| {
        entries_named(ical, &ICalendarProperty::Organizer)
            .flat_map(|entry| entry.values.iter())
            .cloned()
            .collect::<Vec<_>>()
    };

    assert_eq!(
        organizer(&exported),
        [ICalendarValue::Uri(Uri::Location(
            "mailto:org@example.com".into()
        ))],
        "RFC 5545 Section 3.8.4.3: the ORGANIZER value type is CAL-ADDRESS"
    );
    assert_eq!(organizer(&exported), organizer(&parsed));
}

#[test]
fn r5_attendee_parameter_order_is_preserved() {
    for attendee in [
        "ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Jane;X-NUM-GUESTS=0:mailto:jane@example.com",
        "ATTENDEE;CN=Jane;CUTYPE=INDIVIDUAL;EMAIL=jane@example.com;PARTSTAT=ACCEPTED;ROLE=CHAIR;SCHEDULE-STATUS=2.0:mailto:jane@example.com",
        "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Jane:mailto:jane@example.com",
        "ATTENDEE;RSVP=FALSE;X-B=2;X-B=3;X-A=1;SCHEDULE-AGENT=CLIENT:mailto:jane@example.com",
    ] {
        let ical = ical_event(&format!("{attendee}\r\n"));
        let parsed = ICalendar::parse(&ical).expect("valid iCalendar");
        for _ in 0..3 {
            let exported = import(&ical).into_icalendar().expect("exports");
            let params = |ical: &ICalendar| {
                entries_named(ical, &ICalendarProperty::Attendee)
                    .flat_map(|entry| entry.params.iter())
                    .cloned()
                    .collect::<Vec<_>>()
            };
            assert_eq!(params(&exported), params(&parsed), "{exported}");
        }
    }
}

#[test]
fn r1_alarms_export_action_and_description() {
    let source = event_json(
        r#""title": "Planning",
           "alerts": {
               "a1": {"@type": "Alert", "trigger": {"@type": "OffsetTrigger", "offset": "-PT15M"}},
               "a2": {"@type": "Alert", "action": "email", "trigger": {"@type": "OffsetTrigger", "offset": "-PT1H"}}
           }"#,
    );
    let ical = export(&source);
    let alarms = ical
        .components
        .iter()
        .filter(|component| component.component_type.is_alarm())
        .map(|alarm| {
            let mut names = alarm
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>();
            names.sort_unstable();
            names
        })
        .collect::<Vec<_>>();
    assert_eq!(
        alarms,
        [
            vec!["ACTION", "DESCRIPTION", "JSID", "TRIGGER"],
            vec!["ACTION", "DESCRIPTION", "JSID", "SUMMARY", "TRIGGER"]
        ],
        "RFC 5545 Section 3.6.6: ACTION is required, DISPLAY needs DESCRIPTION, EMAIL needs DESCRIPTION and SUMMARY; jscalendarbis-20 Section 3.5.1: action defaults to display\n{ical}"
    );
    let rendered = lines(&ical);
    assert!(
        rendered
            .iter()
            .any(|line| line == "DESCRIPTION;DERIVED=TRUE:Planning"),
        "RFC 9073 Section 5.3\n{rendered:#?}"
    );

    let reimported = entry(&import(&ical.to_string()));
    let mut expected: JsonValue = serde_json::from_str(&source).expect("json");
    expected["entries"][0]["alerts"]["a1"]["action"] = "display".into();
    assert_eq!(
        reimported["alerts"], expected["entries"][0]["alerts"],
        "derived alarm texts are not preserved on import\n{ical}"
    );
}

#[test]
fn r1_alerts_referenced_by_related_to_get_a_uid() {
    let source = event_json(
        r#""alerts": {
               "base": {"@type": "Alert", "action": "display", "trigger": {"@type": "OffsetTrigger", "offset": "-PT15M"}},
               "snooze": {"@type": "Alert", "action": "display", "trigger": {"@type": "AbsoluteTrigger", "when": "2025-01-06T08:50:00Z"},
                          "relatedTo": {"base": {"relation": {"snooze": true}}}}
           }"#,
    );
    let ical = export(&source);
    let alarms = ical
        .components
        .iter()
        .filter(|component| component.component_type.is_alarm())
        .collect::<Vec<_>>();
    let base = alarms
        .iter()
        .find(|alarm| alarm.jsid() == Some("base"))
        .expect("base alarm");
    let snooze = alarms
        .iter()
        .find(|alarm| alarm.jsid() == Some("snooze"))
        .expect("snooze alarm");
    let uid = base.uid().expect(
        "draft-ietf-calext-jscalendar-icalendar-28 Section 3.1: the UID property for such a VALARM MUST be set",
    );
    assert_eq!(
        snooze
            .property(&ICalendarProperty::RelatedTo)
            .and_then(|entry| entry.values.first())
            .and_then(ICalendarValue::as_text),
        Some(uid),
        "{DRAFT} Section 3.1: the RELATED-TO value is the UID of the related VALARM\n{ical}"
    );

    let reimported = entry(&import(&ical.to_string()));
    assert_eq!(
        reimported["alerts"]["snooze"]["relatedTo"],
        serde_json::json!({"base": {"relation": {"snooze": true}}}),
        "{ical}"
    );
}

#[test]
fn r11_6_show_without_time_is_only_written_as_true() {
    for (members, expected) in [
        (r#""showWithoutTime": false"#, None),
        (
            r#""showWithoutTime": true"#,
            Some("SHOW-WITHOUT-TIME;VALUE=BOOLEAN:TRUE"),
        ),
    ] {
        let rendered = lines(&export(&event_json(members)));
        assert_eq!(
            rendered
                .iter()
                .find(|line| line.contains("SHOW-WITHOUT-TIME"))
                .map(String::as_str),
            expected,
            "icalendar-jscalendar-extensions-07 Section 4.2: the value MUST be TRUE and VALUE=BOOLEAN is required\n{rendered:#?}"
        );
    }

    let all_day = export(
        r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "all-day", "start": "2025-01-06T00:00:00", "duration": "P1D", "showWithoutTime": true}]}"#,
    );
    assert!(
        !unfolded(&all_day).contains("SHOW-WITHOUT-TIME"),
        "{DRAFT} Section 3.2: only DATE-TIME values carry SHOW-WITHOUT-TIME\n{all_day}"
    );

    let task = r#"{"@type": "Group", "entries": [{"@type": "Task", "uid": "task", "title": "Chores", "showWithoutTime": true}]}"#;
    let exported = export(task);
    assert_eq!(
        entry(&import(&exported.to_string()))["showWithoutTime"],
        true,
        "a task without start or due keeps showWithoutTime\n{exported}"
    );
}

#[test]
fn r11_12_r11_13_jsprop_edge_cases() {
    let quoted = unfolded(&export(&event_json(
        r#""participants": {"p1": {"@type": "Participant", "calendarAddress": "mailto:p1@example.com", "roles": {"attendee": true}}}"#,
    )));
    assert!(
        quoted.contains("JSPROP;JSPTR=\"participants/p1/roles\":"),
        "{DRAFT} Section 4.2.2: the parameter value MUST be quoted\n{quoted}"
    );

    let leading = entry(&import(&ical_event(
        "JSPROP;X-FOO=bar;JSPTR=\"example.com:foo\":\"value\"\r\n",
    )));
    assert_eq!(
        leading["example.com:foo"], "value",
        "{DRAFT} Section 4.1.2: other parameters can be specified on JSPROP"
    );

    let imported = import(&ical_event(concat!(
        "JSPROP;JSPTR=\"\":{\"title\":\"root\"}\r\n",
        "JSPROP;JSPTR=\"links/*/title\":\"wildcard\"\r\n",
    )));
    let properties = entry(&imported)["iCalendar"]["properties"].clone();
    assert_eq!(
        properties.as_array().map(|properties| properties.len()),
        Some(2),
        "JSPROPs with a root or wildcard pointer are kept as unconverted properties\n{properties:#}"
    );
    let exported = unfolded(&imported.into_icalendar().expect("exports"));
    assert!(exported.contains("JSPROP;JSPTR=\"\":"), "{exported}");
    assert!(
        exported.contains("JSPROP;JSPTR=\"links/*/title\":"),
        "{exported}"
    );

    let nulls = unfolded(&export(&event_json(
        r#""example.com:foo": null,
           "links": {"l1": {"@type": "Link", "href": "https://example.com/a.pdf", "rel": "enclosure", "example.com:bar": null}}"#,
    )));
    assert!(
        !nulls.contains(":null\r\n"),
        "{DRAFT} Section 4.1.2: the JSON value MUST NOT be the null value\n{nulls}"
    );
}

#[test]
fn r13_out_of_range_numbers_are_not_cast() {
    let source = r#"{"@type": "Group", "entries": [{"@type": "Task", "uid": "numbers",
        "percentComplete": 150, "priority": -1, "sequence": 18446744073709551615,
        "participants": {"p": {"@type": "Participant", "name": "P", "percentComplete": 2.5}},
        "links": {"l": {"@type": "Link", "href": "https://example.com/a", "rel": "enclosure", "size": -5}}}]}"#;
    let ical = export(source);
    let rendered = unfolded(&ical);
    for property in ["PERCENT-COMPLETE", "PRIORITY", "SEQUENCE", "SIZE="] {
        assert!(
            !rendered.contains(property),
            "jscalendarbis-20 Sections 3.1.7, 3.4.3 and 4.2.4: invalid values are not converted by sign flips or truncation\n{rendered}"
        );
    }

    let reimported = entry(&import(&ical.to_string()));
    let source: JsonValue = serde_json::from_str(source).expect("json");
    for pointer in [
        "/percentComplete",
        "/priority",
        "/sequence",
        "/participants/p/percentComplete",
        "/links/l/size",
    ] {
        assert_eq!(
            reimported.pointer(pointer),
            source["entries"][0].pointer(pointer),
            "{pointer}\n{rendered}"
        );
    }

    let valid = unfolded(&export(
        r#"{"@type": "Group", "entries": [{"@type": "Task", "uid": "valid", "percentComplete": 100, "priority": 9, "sequence": 3}]}"#,
    ));
    for line in ["PERCENT-COMPLETE:100", "PRIORITY:9", "SEQUENCE:3"] {
        assert!(valid.contains(line), "{valid}");
    }
}

#[test]
fn r13_non_finite_floats_are_not_imported_as_numbers() {
    let imported = import(&ical_event(concat!(
        "X-SCORE;VALUE=FLOAT:NaN\r\n",
        "X-WEIGHT;VALUE=FLOAT:inf\r\n",
    )));
    assert!(imported.0 == imported.0.clone());
    assert_eq!(
        entry(&imported)["iCalendar"]["properties"],
        serde_json::json!([
            ["x-score", {}, "FLOAT", "NaN"],
            ["x-weight", {}, "FLOAT", "inf"]
        ])
    );
}

#[test]
fn r13_unconverted_property_numbers_are_checked_on_export() {
    let ical = export(&event_json(
        r#""iCalendar": {"properties": [
            ["x-big", {}, "integer", 18446744073709551615],
            ["x-negative", {}, "integer", -5],
            ["x-ratio", {}, "float", 2.5],
            ["x-score", {}, "float", "NaN"],
            ["x-weight", {}, "float", "inf"]
        ]}"#,
    ));
    assert!(
        ical == ical.clone(),
        "RFC 5545 Section 3.3.7: FLOAT values are finite"
    );
    let rendered = lines(&ical);
    for expected in [
        "X-BIG;VALUE=INTEGER:18446744073709551615",
        "X-NEGATIVE;VALUE=INTEGER:-5",
        "X-RATIO;VALUE=FLOAT:2.5",
        "X-SCORE;VALUE=FLOAT:NaN",
        "X-WEIGHT;VALUE=FLOAT:inf",
    ] {
        assert!(
            rendered.iter().any(|line| line == expected),
            "{DRAFT} Section 5.1.1: numbers convert without wrapping; missing {expected}\n{rendered:#?}"
        );
    }
}

#[test]
fn rfc5545_3_6_6_derived_alarms_carry_a_trigger() {
    let rendered = unfolded(&export(&event_json(
        r#""alerts": {"k1": {"@type": "Alert", "action": "display"}}"#,
    )));
    assert!(
        rendered.contains("TRIGGER:PT0S"),
        "RFC 5545 Section 3.6.6: the action and trigger are both REQUIRED\n{rendered}"
    );
    assert!(
        rendered.contains("DESCRIPTION;DERIVED=TRUE:conversion"),
        "the UID stands in for a missing SUMMARY rather than invented text\n{rendered}"
    );
    assert!(!rendered.contains("Reminder"), "{rendered}");

    let rendered = unfolded(&export(&event_json(
        r#""title": "Standup", "alerts": {"k1": {"@type": "Alert", "action": "display",
            "trigger": {"@type": "OffsetTrigger", "offset": "-PT5M"}}}"#,
    )));
    assert!(rendered.contains("TRIGGER:-PT5M"), "{rendered}");
    assert!(!rendered.contains("TRIGGER:PT0S"), "{rendered}");
    assert!(
        rendered.contains("DESCRIPTION;DERIVED=TRUE:Standup"),
        "{rendered}"
    );
}

#[test]
fn out_of_range_numbers_export_as_jsprop() {
    for (members, pointer, kept) in [
        (
            r#""recurrenceRule": {"@type": "RecurrenceRule", "frequency": "daily", "interval": 0}"#,
            "recurrenceRule/interval",
            "0",
        ),
        (
            r#""recurrenceRule": {"@type": "RecurrenceRule", "frequency": "daily", "count": 0}"#,
            "recurrenceRule/count",
            "0",
        ),
        (
            r#""sequence": 99999999999999999"#,
            "sequence",
            "99999999999999999",
        ),
        (r#""sequence": -1"#, "sequence", "-1"),
    ] {
        let rendered = unfolded(&export(&event_json(members)));
        assert!(
            rendered.contains(&format!("JSPROP;JSPTR=\"{pointer}\":{kept}")),
            "jscalendarbis-20 Section 3.3.3, RFC 5545 Sections 3.3.8 and 3.3.10: out-of-range values are preserved\n{rendered}"
        );
        for part in ["INTERVAL=0", "COUNT=0", "SEQUENCE:9", "SEQUENCE:-"] {
            assert!(!rendered.contains(part), "{part}\n{rendered}");
        }
    }

    let rendered = unfolded(&export(&event_json(
        r#""sequence": 3, "recurrenceRule": {"@type": "RecurrenceRule", "frequency": "daily", "interval": 2, "count": 3}"#,
    )));
    assert!(rendered.contains("SEQUENCE:3"), "{rendered}");
    assert!(
        rendered.contains("RRULE:FREQ=DAILY;COUNT=3;INTERVAL=2"),
        "{rendered}"
    );
    assert!(!rendered.contains("JSPROP"), "{rendered}");
}

#[test]
fn r13_patch_mode_rejects_patches_that_do_not_apply() {
    let source = event_json(
        r#""title": "Daily", "recurrenceRule": {"frequency": "daily"},
           "recurrenceOverrides": {"2025-01-07T09:00:00": {"participants/missing/participationStatus": "declined"}}"#,
    );
    for recurrence_overrides in [RecurrenceOverrides::Full, RecurrenceOverrides::Patch] {
        let rendered = unfolded(
            &JSCalendar::<String, String>::parse(&source)
                .expect("valid JSCalendar")
                .into_icalendar_with(
                    ExportOptions::new().recurrence_overrides(recurrence_overrides),
                )
                .expect("the rest of the calendar survives"),
        );
        assert!(
            !rendered.contains("RECURRENCE-ID") && !rendered.contains("ATTENDEE"),
            "jscalendarbis-20 Section 1.5.9: implementations MUST reject a PatchObject if any of its patches are invalid ({recurrence_overrides:?})\n{rendered}"
        );
        assert!(
            rendered.contains("RRULE:FREQ=DAILY") && rendered.contains("SUMMARY:Daily"),
            "{rendered}"
        );
    }
}

#[test]
fn value_parameters_are_not_converted_to_members() {
    let ical = ICalendar::parse(ical_event(
        "CONFERENCE;VALUE=URI;LABEL=Call:https://video.example.com/1\r\n",
    ))
    .expect("valid iCalendar");
    let conference = entries_named(&ical, &ICalendarProperty::Conference)
        .flat_map(|entry| entry.params.iter())
        .map(|param| (param.name.clone(), param.value.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        conference,
        [
            (
                ICalendarParameterName::Label,
                ICalendarParameterValue::Text("Call".into())
            ),
            (
                ICalendarParameterName::Value,
                ICalendarParameterValue::Value(calcard::icalendar::ICalendarValueType::Uri)
            )
        ]
    );
    let exported = import(&ical.to_string()).into_icalendar().expect("exports");
    assert_eq!(
        entries_named(&exported, &ICalendarProperty::Conference)
            .flat_map(|entry| entry.params.iter())
            .map(|param| (param.name.clone(), param.value.clone()))
            .collect::<Vec<_>>(),
        conference,
        "RFC 7986 Section 5.11\n{exported}"
    );
}

#[test]
fn participant_id_matches_the_imported_participant_key() {
    let ical = ICalendar::parse(ical_event(concat!(
        "ORGANIZER;CN=Org:mailto:org@example.com\r\n",
        "ATTENDEE;CN=Jane:mailto:jane@example.com\r\n",
        "ATTENDEE;JSID=tom;CN=Tom:mailto:tom@example.com\r\n",
    )))
    .expect("valid iCalendar");
    let mut ids = ical
        .components
        .iter()
        .flat_map(|component| component.entries.iter())
        .filter(|entry| {
            matches!(
                entry.name,
                ICalendarProperty::Attendee | ICalendarProperty::Organizer
            )
        })
        .filter_map(|entry| entry.participant_id().map(|id| id.into_owned()))
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut keys = entry(&ical.into_jscalendar::<String, String>())["participants"]
        .as_object()
        .map(|participants| participants.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort_unstable();

    assert_eq!(
        ids, keys,
        "{DRAFT} Sections 2.3.4 and 2.3.29: the key is the JSID parameter or the UUIDv5 of the property value"
    );
    assert!(keys.contains(&uuid5("mailto:jane@example.com")));
    assert!(keys.contains(&"tom".to_string()));
}

#[test]
fn jscalendarbis_3_4_6_delegation_is_a_set() {
    let ical = ical_event(concat!(
        "ATTENDEE;PARTSTAT=DELEGATED;DELEGATED-TO=\"mailto:bill@example.com\",\"mailto:zed@example.com\":mailto:jane@example.com\r\n",
        "ATTENDEE;DELEGATED-FROM=\"mailto:jane@example.com\":mailto:bill@example.com\r\n",
    ));
    let imported = import(&ical);
    let participants = entry(&imported)["participants"].clone();
    let participant = |address: &str| {
        participants
            .as_object()
            .and_then(|participants| {
                participants
                    .values()
                    .find(|participant| participant["calendarAddress"] == address)
            })
            .cloned()
            .unwrap_or_else(|| panic!("{address} missing: {participants}"))
    };

    assert_eq!(
        participant("mailto:jane@example.com")["delegatedTo"],
        serde_json::json!({"mailto:bill@example.com": true, "mailto:zed@example.com": true}),
        "draft-ietf-calext-jscalendarbis-20 Section 3.4.6: delegatedTo is a String[Boolean]\n{participants}"
    );
    assert_eq!(
        participant("mailto:bill@example.com")["delegatedFrom"],
        serde_json::json!({"mailto:jane@example.com": true}),
        "draft-ietf-calext-jscalendarbis-20 Section 3.4.6: delegatedFrom is a String[Boolean]\n{participants}"
    );

    let exported = lines(&imported.into_icalendar().expect("exports"));
    for expected in [
        "ATTENDEE;PARTSTAT=DELEGATED;DELEGATED-TO=\"mailto:bill@example.com\",\"mailto:zed@example.com\":mailto:jane@example.com",
        "ATTENDEE;DELEGATED-FROM=\"mailto:jane@example.com\":mailto:bill@example.com",
    ] {
        assert!(
            exported.iter().any(|line| line == expected),
            "{exported:#?}"
        );
    }
}

#[test]
fn jscalendarbis_3_4_6_delegation_accepts_a_bare_string() {
    let json = event_json(
        r#""participants": {"jane": {"@type": "Participant", "calendarAddress": "mailto:jane@example.com",
            "roles": {"attendee": true}, "delegatedTo": "mailto:bill@example.com",
            "delegatedFrom": {"mailto:zed@example.com": true, "mailto:amy@example.com": false}}}"#,
    );
    let exported = lines(&export(&json));

    assert!(
        exported
            .iter()
            .any(|line| line.contains("DELEGATED-TO=\"mailto:bill@example.com\"")),
        "{exported:#?}"
    );
    assert!(
        exported
            .iter()
            .any(|line| line.contains("DELEGATED-FROM=\"mailto:zed@example.com\"")),
        "{exported:#?}"
    );
    assert!(
        !exported.iter().any(|line| line.contains("amy@example.com")),
        "draft-ietf-calext-jscalendarbis-20 Section 3.4.6: only true members are delegates\n{exported:#?}"
    );
}

#[test]
fn the_embedded_size_budget_is_caller_supplied() {
    let oversized = "A".repeat(4 * 1024);
    let json = event_json(&format!(
        r#""links": {{"a": {{"@type": "Link", "rel": "enclosure", "href": "data:application/octet-stream;base64,{oversized}"}}}}"#
    ));

    assert!(
        JSCalendar::<String, String>::parse(&json)
            .expect("valid JSCalendar")
            .into_icalendar()
            .is_ok(),
        "the library picks no policy number on the caller's behalf"
    );
    assert_eq!(
        JSCalendar::<String, String>::parse(&json)
            .expect("valid JSCalendar")
            .into_icalendar_with(ExportOptions::new().max_embedded_size(1024)),
        Err(ExportError::EmbeddedSizeExceeded { max: 1024 })
    );
}

#[test]
fn the_embedded_size_budget_is_charged_per_distinct_binary() {
    let json = event_json(
        r#""recurrenceRule": {"@type": "RecurrenceRule", "frequency": "daily", "count": 4},
           "recurrenceOverrides": {"2025-01-07T09:00:00": {"title": "Second"},
                                   "2025-01-08T09:00:00": {"title": "Third"}},
           "links": {"a": {"@type": "Link", "rel": "enclosure", "href": "data:text/plain;base64,aGVsbG8gd29ybGQ="},
                     "b": {"@type": "Link", "rel": "enclosure", "href": "data:text/plain;base64,aGVsbG8gd29ybGQ="}}"#,
    );
    let ical = JSCalendar::<String, String>::parse(&json)
        .expect("valid JSCalendar")
        .into_icalendar_with(ExportOptions::new().max_embedded_size(11))
        .expect("an identical binary is charged once, however often it is emitted");
    assert!(
        unfolded(&ical).matches("aGVsbG8gd29ybGQ=").count() > 2,
        "{ical}"
    );

    assert_eq!(
        JSCalendar::<String, String>::parse(&json)
            .expect("valid JSCalendar")
            .into_icalendar_with(ExportOptions::new().max_embedded_size(10)),
        Err(ExportError::EmbeddedSizeExceeded { max: 10 })
    );
}
