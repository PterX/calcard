/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    common::{
        blob::{BlobIdGenerator, BlobIdOutcome, NoBlobIds},
        export::{ExportError, ImportError},
    },
    icalendar::{ICalendar, ICalendarParameterName, ICalendarProperty, ICalendarValue},
    jscalendar::{JSCalendar, export::ExportOptions, import::ImportOptions},
};
use std::collections::HashMap;

const BLOB_ICAL: &str = concat!(
    "BEGIN:VCALENDAR\r\n",
    "VERSION:2.0\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:blob-test\r\n",
    "DTSTART:20250101T100000Z\r\n",
    "ATTACH;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=application/pdf;FILENAME=report.pdf:JVBERv8Agf4=\r\n",
    "ATTACH;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=text/plain:aGVsbG8gd29ybGQ=\r\n",
    "IMAGE;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=image/jpeg;DISPLAY=BADGE:/9j/4A==\r\n",
    "BEGIN:PARTICIPANT\r\n",
    "UID:participant-1\r\n",
    "CALENDAR-ADDRESS:mailto:foo@example.com\r\n",
    "ATTACH;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=image/png:iVBORw0KGgo=\r\n",
    "END:PARTICIPANT\r\n",
    "BEGIN:VLOCATION\r\n",
    "UID:location-1\r\n",
    "NAME:Room 101\r\n",
    "ATTACH;ENCODING=BASE64;VALUE=BINARY;FMTTYPE=image/jpeg;JSID=floorplan:/9j/4A==\r\n",
    "END:VLOCATION\r\n",
    "END:VEVENT\r\n",
    "END:VCALENDAR\r\n"
);

type BinaryEntry = (ICalendarProperty, Vec<u8>, Option<String>);

fn binary_entries(ical: &ICalendar) -> Vec<BinaryEntry> {
    let mut entries = ical
        .components
        .iter()
        .flat_map(|component| component.entries.iter())
        .filter_map(|entry| match entry.values.first() {
            Some(ICalendarValue::Binary(data)) => Some((
                entry.name.clone(),
                data.clone(),
                entry
                    .parameter(&ICalendarParameterName::Fmttype)
                    .and_then(|value| value.as_text())
                    .map(str::to_string),
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    entries.sort_unstable();
    entries
}

#[derive(Default)]
struct Store {
    blobs: Vec<Vec<u8>>,
    content_types: Vec<Option<String>>,
}

impl BlobIdGenerator<String> for Store {
    fn blob_id(&mut self, data: Vec<u8>, content_type: Option<&str>) -> BlobIdOutcome<String> {
        self.blobs.push(data);
        self.content_types.push(content_type.map(str::to_string));
        BlobIdOutcome::Generated(format!("blob{}", self.blobs.len() - 1))
    }
}

impl Store {
    fn get(&self, blob_id: &str) -> Option<Vec<u8>> {
        blob_id
            .strip_prefix("blob")
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| self.blobs.get(index))
            .cloned()
    }
}

fn import_blobs(ical: &str) -> (JSCalendar<'static, String, String>, Store) {
    let mut store = Store::default();
    let jscal = ICalendar::parse(ical)
        .unwrap()
        .into_jscalendar_with::<String, String, _>(
            ImportOptions::new().with_blob_id_generator(&mut store),
        )
        .expect("converts");
    (jscal, store)
}

#[test]
fn blob_id_links_roundtrip() {
    let original = ICalendar::parse(BLOB_ICAL).unwrap();
    let (jscal, store) = import_blobs(BLOB_ICAL);

    let json = jscal.to_string_pretty();
    assert!(!json.contains("data:"), "{json}");
    assert!(!json.contains("\"href\""), "{json}");
    assert_eq!(
        store.blobs.len(),
        4,
        "identical bytes must share one blob id"
    );
    assert_eq!(jscal.blob_ids().count(), 5);
    assert!(
        store
            .content_types
            .contains(&Some("application/pdf".to_string()))
    );
    assert!(json.contains("\"size\": 8"), "{json}");
    assert!(
        json.contains("\"contentType\": \"application/pdf\""),
        "{json}"
    );

    let reparsed = JSCalendar::<String, String>::parse(&json).unwrap();
    for exported in [jscal, reparsed] {
        let mut resolved = Vec::new();
        let ical = exported
            .into_icalendar_with(ExportOptions::new().with_blob_resolver(|blob_id: &String| {
                resolved.push(blob_id.clone());
                store.get(blob_id)
            }))
            .unwrap();
        resolved.sort_unstable();
        let resolved_len = resolved.len();
        resolved.dedup();
        assert_eq!(
            resolved.len(),
            resolved_len,
            "blobs resolved more than once"
        );
        assert_eq!(binary_entries(&ical), binary_entries(&original), "{ical}");

        let reparsed = ICalendar::parse(ical.to_string()).unwrap();
        assert_eq!(binary_entries(&reparsed), binary_entries(&original));
        for (parameter, value) in [
            (ICalendarParameterName::Filename, "report.pdf"),
            (ICalendarParameterName::Jsid, "floorplan"),
        ] {
            assert!(
                reparsed
                    .components
                    .iter()
                    .flat_map(|component| component.entries.iter())
                    .any(|entry| entry
                        .parameter(&parameter)
                        .and_then(|param| param.as_text())
                        == Some(value)),
                "{ical}"
            );
        }
    }
}

#[test]
fn blob_id_closure_without_annotations() {
    let mut seen = Vec::new();
    let jscal = ICalendar::parse(BLOB_ICAL)
        .unwrap()
        .into_jscalendar_with::<String, String, _>(ImportOptions::new().with_blob_ids(|data| {
            seen.push(data.len());
            Some(format!("b{}", seen.len()))
        }))
        .expect("converts");
    assert_eq!(jscal.blob_ids().count(), 5);
    assert_eq!(seen.len(), 4);
}

#[test]
fn blob_id_generator_declines() {
    let jscal = ICalendar::parse(BLOB_ICAL)
        .unwrap()
        .into_jscalendar_with::<String, String, _>(
            ImportOptions::new()
                .with_blob_ids(|data| (data.len() > 4).then(|| format!("large-{}", data.len()))),
        )
        .expect("converts");
    let json = jscal.to_string_pretty();
    assert_eq!(jscal.blob_ids().count(), 3, "{json}");
    assert_eq!(
        json.matches("data:image/jpeg;base64,/9j/4A==").count(),
        2,
        "draft-ietf-calext-jscalendar-icalendar-28 Section 2.3.3: FMTTYPE sets the data URL mediatype\n{json}"
    );
}

#[test]
fn blob_id_empty_and_large_binaries() {
    let large = (0..2_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect::<Vec<_>>();
    let mut ical = ICalendar::parse(concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:sizes\r\nDTSTART:20250101T100000Z\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AA==\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n"
    ))
    .unwrap();
    for entry in ical
        .components
        .iter_mut()
        .flat_map(|component| component.entries.iter_mut())
    {
        if let Some(ICalendarValue::Binary(data)) = entry.values.first_mut()
            && data.len() == 1
        {
            *data = large.clone();
        }
    }

    let mut store = Store::default();
    let jscal = ical
        .into_jscalendar_with::<String, String, _>(
            ImportOptions::new().with_blob_id_generator(&mut store),
        )
        .expect("converts");
    assert_eq!(store.blobs.len(), 1);
    assert_eq!(store.blobs.first().map(Vec::len), Some(large.len()));
    assert_eq!(jscal.blob_ids().count(), 1);

    let exported = jscal
        .into_icalendar_with(
            ExportOptions::new().with_blob_resolver(|blob_id: &String| store.get(blob_id)),
        )
        .unwrap();
    let reparsed = ICalendar::parse(exported.to_string()).unwrap();
    let binaries = binary_entries(&reparsed);
    assert_eq!(binaries.len(), 1, "empty attachments must not be exported");
    assert_eq!(binaries.first().map(|(_, data, _)| data), Some(&large));

    let empty_blob = JSCalendar::<String, String>::parse(
        r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e",
            "start": "2025-01-01T10:00:00",
            "links": {"l": {"@type": "Link", "blobId": "empty", "rel": "enclosure"}}}]}"#,
    )
    .unwrap()
    .into_icalendar_with(ExportOptions::new().with_blob_resolver(|_: &String| Some(Vec::new())));
    assert_eq!(
        empty_blob,
        Err(ExportError::UnresolvedBlob {
            blob_id: "empty".to_string()
        })
    );
}

#[test]
fn blob_id_links_without_options() {
    let ical = ICalendar::parse(BLOB_ICAL).unwrap();
    let default = ical.clone().into_jscalendar::<String, String>();
    let json = default.to_string_pretty();
    assert!(
        json.contains("\"href\": \"data:application/pdf;base64,JVBERv8Agf4=\""),
        "draft-ietf-calext-jscalendar-icalendar-28 Section 2.3.3: FMTTYPE sets the data URL mediatype\n{json}"
    );
    assert!(
        json.contains("\"href\": \"data:text/plain;base64,aGVsbG8gd29ybGQ=\""),
        "draft-ietf-calext-jscalendar-icalendar-28 Section 2.3.3: FMTTYPE sets the data URL mediatype\n{json}"
    );
    assert!(!json.contains("blobId"), "{json}");
    assert_eq!(default.blob_ids().count(), 0);
    assert_eq!(
        binary_entries(&default.into_icalendar().unwrap()),
        binary_entries(&ical)
    );
}

#[test]
fn blob_id_link_without_resolver_is_preserved_as_a_jsprop() {
    let json = r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e", "start": "2025-01-01T10:00:00", "timeZone": "Etc/UTC",
        "links": {"doc": {"@type": "Link", "blobId": "blob7", "rel": "enclosure", "contentType": "application/pdf", "size": 8}}}]}"#;
    let exported = JSCalendar::<String, String>::parse(json)
        .unwrap()
        .into_icalendar()
        .expect("RFC 9610 Section 3: a blobId is the normal shape of a JMAP object")
        .to_string();
    assert!(exported.contains("JSPROP;JSPTR=\"links\":"), "{exported}");
    assert!(exported.contains("blob7"), "{exported}");

    let roundtrip = ICalendar::parse(&exported)
        .unwrap()
        .into_jscalendar::<String, String>()
        .to_string_pretty();
    assert!(roundtrip.contains(r#""blobId": "blob7""#), "{roundtrip}");
}

#[test]
fn blob_id_link_with_a_failing_resolver_is_an_error() {
    let json = r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e", "start": "2025-01-01T10:00:00", "timeZone": "Etc/UTC",
        "links": {"doc": {"@type": "Link", "blobId": "blob7", "rel": "enclosure", "contentType": "application/pdf", "size": 8}}}]}"#;
    assert_eq!(
        JSCalendar::<String, String>::parse(json)
            .unwrap()
            .into_icalendar_with(ExportOptions::new().with_blob_resolver(|_: &String| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "blob7".to_string()
        })
    );
}

#[test]
fn identical_link_bytes_keep_distinct_ids() {
    let ical = concat!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:dup\r\nDTSTART:20250101T100000Z\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY;FILENAME=a.bin:/9j/4A==\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY;FILENAME=b.bin:/9j/4A==\r\n",
        "IMAGE;ENCODING=BASE64;VALUE=BINARY;DISPLAY=BADGE:/9j/4A==\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    for jscal in [
        ICalendar::parse(ical)
            .unwrap()
            .into_jscalendar::<String, String>(),
        import_blobs(ical).0,
    ] {
        let json: serde_json::Value = serde_json::from_str(&jscal.to_string_pretty()).unwrap();
        assert_eq!(
            json["entries"][0]["links"]
                .as_object()
                .map(|links| links.len()),
            Some(3),
            "{json}"
        );
    }

    let (jscal, store) = import_blobs(ical);
    assert_eq!(store.blobs.len(), 1);
    let exported = jscal
        .into_icalendar_with(
            ExportOptions::new().with_blob_resolver(|blob_id: &String| store.get(blob_id)),
        )
        .unwrap();
    assert_eq!(binary_entries(&exported).len(), 3, "{exported}");
}

const OVERRIDE_JSON: &str = r#"{"@type": "Group", "entries": [{
    "@type": "Event", "uid": "blob-overrides", "title": "Review",
    "start": "2025-01-06T09:00:00", "timeZone": "Europe/Berlin", "duration": "PT1H",
    "recurrenceRule": {"frequency": "daily"},
    "links": {"agenda": {"@type": "Link", "blobId": "blob0", "rel": "enclosure", "contentType": "text/plain", "size": 11}},
    "recurrenceOverrides": {
        "2025-01-07T09:00:00": {"links/minutes": {"@type": "Link", "blobId": "blob1", "rel": "enclosure", "contentType": "text/plain", "size": 5}},
        "2025-01-09T09:00:00": {"title": "Review (moved)"}
    }
}]}"#;

#[test]
fn blob_ids_include_localizations() {
    let jscal = JSCalendar::<String, String>::parse(
        r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e", "start": "2025-01-01T10:00:00",
            "links": {"l1": {"@type": "Link", "blobId": "base", "rel": "enclosure"}},
            "localizations": {"fr": {"links/l1/blobId": "localized",
                "links/l2": {"@type": "Link", "blobId": "nested", "rel": "enclosure"}}}}]}"#,
    )
    .unwrap();
    let mut blob_ids = jscal.blob_ids().map(String::as_str).collect::<Vec<_>>();
    blob_ids.sort_unstable();
    assert_eq!(
        blob_ids,
        ["base", "localized", "nested"],
        "RFC 9610 Section 3: a localized blobId is still a blob reference"
    );
}

#[test]
fn blob_ids_inside_override_patches() {
    let blobs = HashMap::from([
        ("blob0".to_string(), b"hello world".to_vec()),
        ("blob1".to_string(), b"notes".to_vec()),
    ]);
    let jscal = JSCalendar::<String, String>::parse(OVERRIDE_JSON).unwrap();
    assert_eq!(jscal.blob_ids().count(), 2);

    let mut calls = HashMap::<String, usize>::new();
    let ical = jscal
        .into_icalendar_with(ExportOptions::new().with_blob_resolver(|blob_id: &String| {
            *calls.entry(blob_id.clone()).or_default() += 1;
            blobs.get(blob_id).cloned()
        }))
        .unwrap();
    assert!(calls.values().all(|calls| *calls == 1), "{calls:?}");
    assert_eq!(calls.len(), 2);

    let binaries = binary_entries(&ical);
    assert_eq!(
        binaries
            .iter()
            .filter(|(_, data, _)| data == b"hello world")
            .count(),
        3,
        "{ical}"
    );
    assert_eq!(
        binaries
            .iter()
            .filter(|(_, data, _)| data == b"notes")
            .count(),
        1,
        "{ical}"
    );

    let mut store = Store::default();
    let reimported = ICalendar::parse(ical.to_string())
        .unwrap()
        .into_jscalendar_with::<String, String, _>(
            ImportOptions::new().with_blob_id_generator(&mut store),
        )
        .expect("converts");
    assert_eq!(
        store.blobs.len(),
        2,
        "one generator call per distinct content"
    );
    let json: serde_json::Value = serde_json::from_str(&reimported.to_string_pretty()).unwrap();
    let overrides = &json["entries"][0]["recurrenceOverrides"];
    assert!(
        overrides["2025-01-07T09:00:00"]
            .as_object()
            .is_some_and(|patch| patch.keys().any(|key| key.starts_with("links/"))),
        "{json}"
    );
}

#[test]
fn unresolved_blob_id_is_an_export_error() {
    let json = r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e",
        "start": "2025-01-06T09:00:00", "timeZone": "Europe/Berlin", "duration": "PT1H",
        "recurrenceRule": {"frequency": "daily"},
        "links": {"agenda": {"@type": "Link", "blobId": "blob0", "rel": "enclosure"}},
        "recurrenceOverrides": {
            "2025-01-07T09:00:00": {"links/missing": {"@type": "Link", "blobId": "unknown", "rel": "enclosure"}}
        }
    }]}"#;

    assert_eq!(
        JSCalendar::<String, String>::parse(json)
            .unwrap()
            .into_icalendar_with(ExportOptions::new().with_blob_resolver(|blob_id: &String| {
                (blob_id == "blob0").then(|| b"hello world".to_vec())
            })),
        Err(ExportError::UnresolvedBlob {
            blob_id: "unknown".to_string()
        })
    );
}

#[test]
fn an_override_patch_without_a_parent_is_rejected_on_its_own() {
    let json = r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e",
        "start": "2025-01-06T09:00:00", "timeZone": "Europe/Berlin", "duration": "PT1H",
        "recurrenceRule": {"frequency": "daily"},
        "recurrenceOverrides": {
            "2025-01-07T09:00:00": {"links/missing/title": "Minutes"},
            "2025-01-07T20:00:00": {"links/missing/title": "Minutes"}
        }
    },
    {"@type": "Event", "uid": "other", "start": "2025-02-01T09:00:00", "timeZone": "Europe/Berlin"}]}"#;

    let exported = JSCalendar::<String, String>::parse(json)
        .unwrap()
        .into_icalendar()
        .expect(
            "draft-ietf-calext-jscalendarbis-20 Section 1.5.9: reject the PatchObject, not the document",
        )
        .to_string();
    assert!(exported.contains("UID:other"), "{exported}");
    assert!(!exported.contains("RECURRENCE-ID"), "{exported}");
    assert!(!exported.contains("Minutes"), "{exported}");
    assert!(
        exported.contains("RDATE;TZID=Europe/Berlin:20250107T200000"),
        "draft-ietf-calext-jscalendar-icalendar-28: the occurrence degrades to an RDATE\n{exported}"
    );
}

#[test]
fn embedded_size_budget_is_an_export_error() {
    let json = r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e",
        "start": "2025-01-06T09:00:00", "timeZone": "Europe/Berlin",
        "links": {"a": {"@type": "Link", "blobId": "blob0", "rel": "enclosure"}}
    }]}"#;

    assert_eq!(
        JSCalendar::<String, String>::parse(json)
            .unwrap()
            .into_icalendar_with(
                ExportOptions::new()
                    .max_embedded_size(4)
                    .with_blob_resolver(|_: &String| Some(b"hello world".to_vec()))
            ),
        Err(ExportError::EmbeddedSizeExceeded { max: 4 })
    );
}

#[test]
fn a_top_level_event_or_task_is_exported() {
    for json in [
        r#"{"@type": "Event", "uid": "e", "start": "2025-01-01T10:00:00"}"#,
        r#"{"@type": "Task", "uid": "e", "start": "2025-01-01T10:00:00"}"#,
    ] {
        let exported = JSCalendar::<String, String>::parse(json)
            .unwrap()
            .into_icalendar()
            .expect("draft-ietf-jmap-calendars-29: a CalendarEvent is an Event, not a Group")
            .to_string();
        assert!(exported.contains("UID:e"), "{exported}");
        assert!(exported.contains("DTSTART:20250101T100000"), "{exported}");
    }
}

#[test]
fn an_object_that_is_not_a_group_is_an_export_error() {
    assert_eq!(
        JSCalendar::<String, String>::parse(r#"{"@type": "Link", "href": "https://example.com"}"#)
            .unwrap()
            .into_icalendar(),
        Err(ExportError::NotGroup)
    );
    assert_eq!(
        JSCalendar::<String, String>::parse(r#"{"@type": "Group", "entries": []}"#)
            .unwrap()
            .into_icalendar(),
        Err(ExportError::NoComponents)
    );
}

#[test]
fn an_unparseable_blob_id_is_an_export_error() {
    let json = r#"{"@type": "Group", "entries": [{"@type": "Event", "uid": "e",
        "start": "2025-01-01T10:00:00", "timeZone": "Etc/UTC",
        "links": {"doc": {"@type": "Link", "blobId": "not-a-blob", "rel": "enclosure"}}}]}"#;

    assert_eq!(
        JSCalendar::<String, u32>::parse(json)
            .unwrap()
            .into_icalendar_with(ExportOptions::new().with_blob_resolver(|_: &u32| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "not-a-blob".to_string()
        })
    );
}

#[derive(Default)]
struct FailingStore {
    calls: usize,
}

impl BlobIdGenerator<String> for FailingStore {
    fn blob_id(&mut self, _: Vec<u8>, _: Option<&str>) -> BlobIdOutcome<String> {
        self.calls += 1;
        BlobIdOutcome::Failed
    }
}

#[test]
fn a_failing_blob_id_generator_is_an_import_error() {
    let ical = ICalendar::parse(BLOB_ICAL).unwrap();
    let mut store = FailingStore::default();
    let imported = ical.into_jscalendar_with::<String, String, _>(
        ImportOptions::new().with_blob_id_generator(&mut store),
    );

    assert!(store.calls > 0);
    assert_eq!(imported, Err(ImportError::BlobIdFailed));

    let mut declining = NoBlobIds;
    assert!(
        ICalendar::parse(BLOB_ICAL)
            .unwrap()
            .into_jscalendar_with::<String, String, _>(
                ImportOptions::new().with_blob_id_generator(&mut declining),
            )
            .is_ok(),
        "a generator that declines is not a failure"
    );
}

#[test]
fn a_declining_blob_id_generator_still_inlines_the_binary() {
    let imported = ICalendar::parse(BLOB_ICAL)
        .unwrap()
        .into_jscalendar_with::<String, String, _>(
            ImportOptions::new().with_blob_ids(|_: &[u8]| None::<String>),
        )
        .expect("declining is not a failure");

    assert_eq!(imported.blob_ids().count(), 0);
    assert!(
        imported.to_string_pretty().contains("data:"),
        "{}",
        imported.to_string_pretty()
    );
}
