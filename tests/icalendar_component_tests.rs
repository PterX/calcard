/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    Entry, Parser,
    icalendar::{
        ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarProperty,
        ICalendarValue,
    },
};

const FROM: i64 = 1_704_067_200;
const TO: i64 = 1_735_689_600;

fn component(
    component_type: ICalendarComponentType,
    component_ids: Vec<u32>,
) -> ICalendarComponent {
    ICalendarComponent {
        component_type,
        entries: vec![],
        component_ids,
    }
}

fn calendar_with_event() -> ICalendar {
    ICalendar::parse(concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n",
        "BEGIN:VEVENT\r\nUID:a\r\nDTSTART;TZID=Europe/Berlin:20240301T100000\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    ))
    .expect("valid iCalendar")
}

#[test]
fn rfc5545_3_4_add_timezone_requires_a_vcalendar_root() {
    let mut empty = ICalendar::default();
    assert_eq!(empty.add_timezone("Europe/Berlin", FROM, TO), None);
    assert_eq!(empty.add_missing_timezones(), 0);
    assert!(empty.components.is_empty());

    let mut event_root = ICalendar {
        components: vec![component(ICalendarComponentType::VEvent, vec![])],
    };
    assert_eq!(event_root.add_timezone("Europe/Berlin", FROM, TO), None);
    assert_eq!(event_root.components.len(), 1);
}

#[test]
fn rfc5545_3_6_5_add_timezone_builds_observances() {
    let mut ical = calendar_with_event();
    assert_eq!(ical.add_missing_timezones(), 1);

    let root = ical.components.first().expect("root");
    let timezone_id = *root.component_ids.first().expect("timezone first");
    let timezone = ical.component_by_id(timezone_id).expect("timezone");
    assert_eq!(timezone.component_type, ICalendarComponentType::VTimezone);
    assert!(!timezone.component_ids.is_empty());
    assert!(timezone.component_ids.iter().all(|id| {
        ical.component_by_id(*id).is_some_and(|observance| {
            matches!(
                observance.component_type,
                ICalendarComponentType::Standard | ICalendarComponentType::Daylight
            )
        })
    }));
}

#[test]
fn rfc5545_3_6_5_copy_timezones_skips_invalid_definitions() {
    let source = ICalendar {
        components: vec![
            component(ICalendarComponentType::VCalendar, vec![1, 2]),
            ICalendarComponent {
                component_type: ICalendarComponentType::VTimezone,
                entries: vec![ICalendarEntry::new(ICalendarProperty::Tzid).with_value("Empty")],
                component_ids: vec![],
            },
            ICalendarComponent {
                component_type: ICalendarComponentType::VTimezone,
                entries: vec![ICalendarEntry::new(ICalendarProperty::Tzid).with_value("Dangling")],
                component_ids: vec![3, 99],
            },
            component(ICalendarComponentType::Standard, vec![]),
        ],
    };

    let mut empty = ICalendar::default();
    empty.copy_timezones(&source);
    assert!(empty.components.is_empty());

    let mut ical = ICalendar {
        components: vec![component(ICalendarComponentType::VCalendar, vec![])],
    };
    ical.copy_timezones(&source);
    assert_eq!(
        ical.components
            .iter()
            .map(|component| (
                component.component_type.clone(),
                component.component_ids.clone()
            ))
            .collect::<Vec<_>>(),
        [
            (ICalendarComponentType::VCalendar, vec![1]),
            (ICalendarComponentType::VTimezone, vec![2]),
            (ICalendarComponentType::Standard, vec![]),
        ]
    );
    assert_eq!(
        ical.to_string().replace("\r\n", "\n"),
        "BEGIN:VCALENDAR\nBEGIN:VTIMEZONE\nTZID:Dangling\nBEGIN:STANDARD\nEND:STANDARD\nEND:VTIMEZONE\nEND:VCALENDAR\n"
    );
}

#[test]
fn rfc5545_3_6_remove_component_ids_keeps_a_calendar_component() {
    let original = calendar_with_event();

    let mut ical = original.clone();
    assert!(!ical.remove_component_ids(&[0]));
    assert_eq!(ical, original);

    let mut ical = original.clone();
    assert!(!ical.remove_component_ids(&[1]));
    assert_eq!(ical, original);

    let mut ical = original.clone();
    assert!(ical.remove_component_ids(&[2, 42]));
    assert_eq!(
        ical.components
            .iter()
            .map(|component| (
                component.component_type.clone(),
                component.component_ids.clone()
            ))
            .collect::<Vec<_>>(),
        [
            (ICalendarComponentType::VCalendar, vec![1]),
            (ICalendarComponentType::VEvent, vec![]),
        ]
    );

    let mut ical = original.clone();
    assert_eq!(ical.add_missing_timezones(), 1);
    let timezone_ids = ical
        .components
        .first()
        .map(|root| root.component_ids.clone())
        .unwrap_or_default();
    assert_eq!(timezone_ids.len(), 2);
    assert!(!ical.remove_component_ids(&timezone_ids));
    assert!(ical.remove_component_ids(&[1]));
    assert_eq!(
        ical.components.first().map(|root| root.component_ids.len()),
        Some(1)
    );
    assert!(
        ical.components
            .iter()
            .all(|component| component.component_type != ICalendarComponentType::VEvent)
    );
}

fn empty_calendar_rejection() -> Entry {
    Entry::InvalidLine("BEGIN:VCALENDAR".to_string())
}

const EMPTY_CALENDARS: [&str; 4] = [
    "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
    "\u{feff}BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nEND:VCALENDAR\r\n",
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
];

#[test]
fn rfc5545_3_6_lenient_parsing_keeps_vcalendar_without_components() {
    // RFC 5545, Section 3.6 requires `component = 1*(...)`, but default parsing
    // is lenient and must keep what it was given.
    for input in EMPTY_CALENDARS {
        assert!(
            matches!(ICalendar::parse(input), Ok(ical) if ical.components.len() == 1),
            "{input}"
        );
        assert!(
            matches!(Parser::new(input).entry(), Entry::ICalendar(_)),
            "{input}"
        );
    }
}

#[test]
fn rfc5545_3_6_strict_parsing_rejects_vcalendar_without_components() {
    for input in EMPTY_CALENDARS {
        assert_eq!(
            Parser::new(input).strict().entry(),
            empty_calendar_rejection(),
            "{input}"
        );
    }

    // A top-level component that is not a VCALENDAR is not subject to the check.
    for input in [
        "BEGIN:VEVENT\r\nUID:a\r\nEND:VEVENT\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nEND:VALARM\r\n",
    ] {
        assert!(
            matches!(Parser::new(input).strict().entry(), Entry::ICalendar(_)),
            "{input}"
        );
    }

    // Lenient parsing keeps a VCALENDAR that does have components.
    for input in [
        "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:X\r\nEND:VTIMEZONE\r\nEND:VCALENDAR\r\n",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n",
    ] {
        assert!(
            matches!(Parser::new(input).entry(), Entry::ICalendar(_)),
            "{input}"
        );
    }
}

#[test]
fn rfc5545_3_6_strict_parsing_accepts_nested_components() {
    let input = concat!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n",
        "BEGIN:VEVENT\r\nUID:a\r\nDTSTART:20250101T100000Z\r\n",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\n",
        "END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let mut parser = Parser::new(input).strict();
    let Entry::ICalendar(ical) = parser.entry() else {
        panic!("a VCALENDAR with nested components parses in strict mode: {input}");
    };
    assert_eq!(ical.components.len(), 3);
    assert_eq!(parser.entry(), Entry::Eof);

    assert_eq!(
        Parser::new("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VTODO\r\nEND:VCALENDAR\r\n")
            .strict()
            .entry(),
        Entry::UnexpectedComponentEnd {
            expected: ICalendarComponentType::VEvent,
            found: ICalendarComponentType::VTodo,
        }
    );
}

#[test]
fn rfc5545_3_4_stream_keeps_every_calendar_object() {
    let mut parser = Parser::new(concat!(
        "BEGIN:VCALENDAR\r\nPRODID:-//empty//EN\r\nEND:VCALENDAR\r\n",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:first\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        "GARBAGE:line\r\n",
        "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:second\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
    ));

    let empty =
        |entry: Entry| matches!(entry, Entry::ICalendar(ical) if ical.uids().next().is_none());

    assert!(empty(parser.entry()));
    let Entry::ICalendar(first) = parser.entry() else {
        panic!("expected the first calendar");
    };
    assert_eq!(first.uids().collect::<Vec<_>>(), ["first"]);
    assert_eq!(
        parser.entry(),
        Entry::InvalidLine("GARBAGE:line".to_string())
    );
    assert!(empty(parser.entry()));
    let Entry::ICalendar(second) = parser.entry() else {
        panic!("expected the second calendar");
    };
    assert_eq!(second.uids().collect::<Vec<_>>(), ["second"]);
    assert_eq!(parser.entry(), Entry::Eof);
}

fn nested_calendar(depth: usize) -> String {
    let nested = depth.saturating_sub(2);
    let mut ical = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:nested\r\nDTSTART:20250101T100000Z\r\n",
    );
    ical.extend(std::iter::repeat_n(
        "BEGIN:X-NESTED\r\nX-DEPTH:1\r\n",
        nested,
    ));
    ical.extend(std::iter::repeat_n("END:X-NESTED\r\n", nested));
    ical.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    ical
}

#[test]
fn rfc5545_3_6_component_nesting_is_not_capped() {
    for depth in [34, 1_000, 5_000] {
        let input = nested_calendar(depth);
        let ical = ICalendar::parse(&input).unwrap_or_else(|entry| {
            panic!("RFC 5545 Section 3.6 defines no nesting bound, got {entry:?} at depth {depth}")
        });
        assert_eq!(ical.components.len(), depth, "depth {depth}");
        assert_eq!(
            ICalendar::parse(ical.to_string()),
            Ok(ical),
            "depth {depth}"
        );
    }

    let input = nested_calendar(1_000);
    let mut parser = Parser::new(&input);
    assert!(matches!(parser.entry(), Entry::ICalendar(_)));
    assert_eq!(parser.entry(), Entry::Eof);
}

/// The stack a conversion is allowed to use, whatever the nesting depth.
#[cfg(feature = "jmap")]
const CONVERSION_STACK: usize = 1024 * 1024;

#[cfg(feature = "jmap")]
fn convert_in_a_small_stack(depth: usize) -> (usize, usize, usize, bool) {
    let input = nested_calendar(depth);
    std::thread::Builder::new()
        .stack_size(CONVERSION_STACK)
        .spawn(move || {
            let ical = ICalendar::parse(&input).expect("deep nesting is accepted");
            let depth = ical.components.len();
            let written = ical.to_string();
            let jscal = ical.into_jscalendar::<String, String>();
            let json = jscal.to_string_pretty();
            let exported = jscal.into_icalendar().map(|ical| ical.to_string());
            (depth, written.len(), json.len(), exported.is_ok())
        })
        .expect("spawn")
        .join()
        .expect("conversion of a deeply nested calendar fits the stack budget")
}

#[cfg(feature = "jmap")]
#[test]
fn deeply_nested_components_convert_in_a_small_stack() {
    let converted = convert_in_a_small_stack(5_000);
    assert_eq!(converted.0, 5_000);
    assert!(converted.1 > 0 && converted.2 > 0 && converted.3);
}

#[cfg(feature = "jmap")]
#[test]
fn conversion_stack_use_does_not_grow_with_nesting() {
    // Ten times the nesting has to fit the same budget, which it only can if
    // nesting costs no stack of its own.
    let shallow = convert_in_a_small_stack(500);
    let deep = convert_in_a_small_stack(5_000);
    assert_eq!(shallow.0, 500);
    assert_eq!(deep.0, 5_000);
    assert!(shallow.3 && deep.3);
}

#[test]
fn rfc5545_3_6_cyclic_component_ids_write_each_component_once() {
    let mut components = vec![component(
        ICalendarComponentType::VCalendar,
        vec![0, 0, 1, 1],
    )];
    components.extend((0..19).map(|_| component(ICalendarComponentType::VEvent, vec![])));
    let written = ICalendar { components }.to_string();

    assert_eq!(
        written, "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        "each component id is written at most once"
    );
}

#[test]
fn rfc9074_uids_skip_subcomponents() {
    let ical = ICalendar::parse(concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:event\r\n",
        "DTSTART:20250101T090000Z\r\n",
        "BEGIN:VALARM\r\n",
        "UID:alarm\r\n",
        "ACTION:DISPLAY\r\n",
        "DESCRIPTION:Reminder\r\n",
        "TRIGGER:-PT15M\r\n",
        "END:VALARM\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    ))
    .expect("valid iCalendar");

    assert_eq!(ical.uids().collect::<Vec<_>>(), ["event"]);

    let mut reordered = ICalendar::default();
    reordered.components.push(ical.components[0].clone());
    reordered.components.push(ical.components[2].clone());
    reordered.components.push(ical.components[1].clone());
    if let Some(root) = reordered.components.first_mut() {
        root.component_ids = vec![2];
    }
    if let Some(event) = reordered.components.get_mut(2) {
        event.component_ids = vec![1];
    }

    assert_eq!(
        reordered.uids().collect::<Vec<_>>(),
        ["event"],
        "RFC 9074 Section 4: an alarm UID is not the UID of the calendar object"
    );

    let ical = ICalendar::parse(concat!(
        "BEGIN:VCALENDAR\r\n",
        "UID:calendar\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:event\r\n",
        "DTSTART:20250101T090000Z\r\n",
        "BEGIN:PARTICIPANT\r\n",
        "UID:participant\r\n",
        "CALENDAR-ADDRESS:mailto:a@example.com\r\n",
        "END:PARTICIPANT\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    ))
    .expect("valid iCalendar");
    assert_eq!(
        ical.uids().collect::<Vec<_>>(),
        ["calendar", "event"],
        "RFC 7986 Section 5.3: UID may also be defined on the iCalendar object"
    );
}

#[test]
fn rfc5545_3_6_removing_a_component_removes_children_stored_before_it() {
    let binary = |value: &str| {
        let mut entry = ICalendarEntry::new(ICalendarProperty::Attach);
        entry.values = [ICalendarValue::Binary(value.as_bytes().to_vec())].into();
        entry
    };
    let with_binaries = |component_type: ICalendarComponentType,
                         component_ids: Vec<u32>,
                         entries: Vec<ICalendarEntry>| {
        ICalendarComponent {
            component_type,
            entries,
            component_ids,
        }
    };

    let mut ical = ICalendar {
        components: vec![
            with_binaries(ICalendarComponentType::VCalendar, vec![3, 4], vec![]),
            with_binaries(
                ICalendarComponentType::VLocation,
                vec![],
                vec![binary("orphan")],
            ),
            with_binaries(ICalendarComponentType::Participant, vec![1], vec![]),
            with_binaries(ICalendarComponentType::VEvent, vec![2], vec![]),
            with_binaries(ICalendarComponentType::VEvent, vec![], vec![binary("kept")]),
        ],
    };

    assert!(ical.remove_component_ids(&[3]));
    assert_eq!(
        ical.blob_binaries().collect::<Vec<_>>(),
        [b"kept".as_slice()],
        "RFC 5545 Section 3.6: a removed component takes its subcomponents with it"
    );
    assert_eq!(ical.embedded_size(), 4);
    assert_eq!(ical.components.len(), 2);
}
