/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    icalendar::{ICalendar, ICalendarComponent, ICalendarComponentType},
    jscalendar::JSCalendar,
};

fn export(json: &str) -> Option<ICalendar> {
    JSCalendar::<String, String>::parse(json)
        .expect("valid JSCalendar")
        .into_icalendar()
        .ok()
}

fn component_types(ical: &ICalendar) -> Vec<ICalendarComponentType> {
    ical.components
        .iter()
        .map(|component| component.component_type.clone())
        .collect()
}

#[test]
fn rfc5545_3_6_group_without_entries_is_not_exported() {
    for json in [
        r#"{"@type": "Group"}"#,
        r#"{"@type": "Group", "entries": []}"#,
        r#"{"@type": "Group", "title": "Holidays", "source": "https://example.com/h.ics"}"#,
        r#"{}"#,
        r#"{"title": "Holidays"}"#,
    ] {
        assert_eq!(export(json), None, "{json}");
    }
}

#[test]
fn jscalendarbis_4_3_group_entries_other_than_event_or_task_are_ignored() {
    for json in [
        r#"{"@type": "Group", "entries": [{"@type": "Alert"}]}"#,
        r#"{"@type": "Group", "entries": [{"@type": "Group", "entries": [{"@type": "Event"}]}]}"#,
        r#"{"@type": "Group", "entries": [{"@type": "Location", "name": "Room"}]}"#,
        r#"{"@type": "Group", "entries": [{"@type": "Participant"}]}"#,
        r#"{"@type": "Group", "entries": [{"@type": "Unknown"}]}"#,
        r#"{"@type": "Group", "entries": ["Event", 1, null]}"#,
    ] {
        assert_eq!(export(json), None, "{json}");
    }

    let ical = export(
        r#"{"@type": "Group", "entries": [{"@type": "Alert"}, {"@type": "Task", "title": "t"}]}"#,
    )
    .expect("group with a task");
    assert_eq!(
        component_types(&ical),
        [
            ICalendarComponentType::VCalendar,
            ICalendarComponentType::VTodo
        ]
    );
}

#[test]
fn rfc5545_3_4_only_groups_events_and_tasks_export_to_a_vcalendar() {
    for json in [r#"{"@type": "Alert"}"#, r#"{"@type": "Location"}"#] {
        assert_eq!(export(json), None, "{json}");
    }

    for (json, component_type) in [
        (
            r#"{"@type": "Event", "title": "Lunch"}"#,
            ICalendarComponentType::VEvent,
        ),
        (
            r#"{"@type": "Task", "title": "Chores"}"#,
            ICalendarComponentType::VTodo,
        ),
    ] {
        let ical = export(json).expect("draft-ietf-jmap-calendars-29: a CalendarEvent is an Event");
        assert_eq!(
            component_types(&ical),
            [ICalendarComponentType::VCalendar, component_type],
            "{json}"
        );
    }

    let ical = export(r#"{"@type": "Group", "entries": [{"@type": "Event", "title": "Lunch"}]}"#)
        .expect("group with an event");
    assert_eq!(
        component_types(&ical),
        [
            ICalendarComponentType::VCalendar,
            ICalendarComponentType::VEvent
        ]
    );
    assert_eq!(
        ical.components
            .first()
            .map(|root| root.component_ids.as_slice()),
        Some(&[1][..])
    );
}

#[test]
fn rfc5545_3_6_import_of_invalid_component_trees_does_not_panic() {
    assert!(
        ICalendar::default()
            .into_jscalendar::<String, String>()
            .into_icalendar()
            .is_err()
    );

    let dangling = ICalendar {
        components: vec![
            ICalendarComponent {
                component_type: ICalendarComponentType::VCalendar,
                entries: vec![],
                component_ids: vec![0, 1, 7],
            },
            ICalendarComponent {
                component_type: ICalendarComponentType::VEvent,
                entries: vec![],
                component_ids: vec![1, 2, 9],
            },
            ICalendarComponent {
                component_type: ICalendarComponentType::VAlarm,
                entries: vec![],
                component_ids: vec![0, 42],
            },
        ],
    };
    let exported = dangling
        .into_jscalendar::<String, String>()
        .into_icalendar()
        .expect("the event survives");
    let types = component_types(&exported);
    assert_eq!(types.first(), Some(&ICalendarComponentType::VCalendar));
    assert!(
        types.contains(&ICalendarComponentType::VEvent),
        "{exported}"
    );
}
