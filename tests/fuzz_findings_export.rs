/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    Entry, Parser,
    icalendar::{ICalendar, ICalendarProperty},
    jscalendar::JSCalendar,
    jscontact::JSContact,
    vcard::{VCard, VCardProperty, VCardVersion},
};

#[test]
fn jscalendar_properties_cannot_inject_component_boundaries() {
    let ical = JSCalendar::<String, String>::parse(concat!(
        r#"{"@type":"Event","uid":"a","start":"2024-01-01T10:00:00","iCalendar":{"name":"vevent","#,
        r#""properties":[["end",{},"text","VEVENT"],["begin",{},"text","VTODO"],"#,
        r#"["summary",{},"text","injected"]]}}"#
    ))
    .expect("the event parses")
    .into_icalendar()
    .expect("the event exports");
    let reparsed = ICalendar::parse(ical.to_string()).expect("the export parses");
    assert_eq!(reparsed.components.len(), ical.components.len());
    assert!(ical.components.iter().all(|component| {
        component.entries.iter().all(|entry| {
            !matches!(
                entry.name,
                ICalendarProperty::Begin | ICalendarProperty::End
            )
        })
    }));
}

#[test]
fn jscontact_properties_cannot_inject_card_boundaries() {
    let vcard = JSContact::<String, String>::parse(concat!(
        r#"{"@type":"Card","version":"1.0","name":{"full":"a"},"vCard":{"properties":["#,
        r#"["end",{},"text","VCARD"],["begin",{},"text","VCARD"],"#,
        r#"["note",{},"text","injected"]]}}"#
    ))
    .expect("the card parses")
    .into_vcard()
    .expect("the card exports");
    assert!(
        vcard
            .entries
            .iter()
            .all(|entry| !matches!(entry.name, VCardProperty::Begin | VCardProperty::End))
    );
    let mut written = String::new();
    vcard
        .write_to(&mut written, VCardVersion::V4_0)
        .expect("the card writes to a String");
    let mut parser = Parser::new(&written);
    assert!(matches!(parser.entry(), Entry::VCard(_)));
    assert!(matches!(parser.entry(), Entry::Eof));
    assert!(VCard::parse(written).is_ok());
}
