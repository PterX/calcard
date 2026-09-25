/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

#![no_main]

use calcard::{
    Entry, Parser,
    common::CalendarScale,
    icalendar::{
        ArchivedICalendar, ICalendar, ICalendarParameterValue, ICalendarProperty, ICalendarValue,
    },
    vcard::{
        ArchivedVCard, VCard, VCardEntry, VCardParameterName, VCardParameterValue, VCardProperty,
        VCardValue, VCardVersion,
    },
};
use calcard_fuzz::{EntryStream, Folded, Token};
use libfuzzer_sys::fuzz_target;
use rkyv::rancor::Error as RkyvError;

const VERSIONS: [VCardVersion; 4] = [
    VCardVersion::V2_0,
    VCardVersion::V2_1,
    VCardVersion::V3_0,
    VCardVersion::V4_0,
];

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    for entry in EntryStream::new(&text, false) {
        match entry {
            Entry::ICalendar(ical) => CalendarRoundTrip(ical).check(),
            Entry::VCard(vcard) => CardRoundTrip(vcard).check(),
            _ => {}
        }
    }
});

struct CalendarRoundTrip(ICalendar);

impl CalendarRoundTrip {
    fn check(self) {
        let first = Self::written(&self.0);
        let Some(normalized) = Self::reparse(&first, false) else {
            return;
        };
        let second = Self::written(&normalized);
        if !Self::is_plain(&normalized) {
            return;
        }
        let reparsed = Self::reparse(&second, true).expect("strict reparse returns a calendar");
        assert_eq!(
            reparsed.to_string(),
            second,
            "normalized writer output is not a fixed point of parse and write"
        );
        let expected = Self::canonical(normalized);
        assert_eq!(
            reparsed.components.len(),
            expected.components.len(),
            "component count changed in the round trip of normalized output"
        );
        for (index, (before, after)) in expected
            .components
            .iter()
            .zip(reparsed.components.iter())
            .enumerate()
        {
            assert_eq!(
                before, after,
                "component {index} changed in the round trip of normalized output"
            );
        }
    }

    fn written(ical: &ICalendar) -> String {
        let text = ical.to_string();
        Folded(&text).check("iCalendar writer");

        let bytes = rkyv::to_bytes::<RkyvError>(ical).expect("iCalendar archives");
        let archived =
            rkyv::access::<ArchivedICalendar, RkyvError>(&bytes).expect("archive validates");
        assert_eq!(
            archived.to_string(),
            text,
            "archived iCalendar writer differs from the native writer"
        );
        let restored =
            rkyv::deserialize::<ICalendar, RkyvError>(archived).expect("archive deserializes");
        assert_eq!(&restored, ical, "rkyv round trip changed the calendar");

        for (component, archived_component) in
            ical.components.iter().zip(archived.components.iter())
        {
            for (entry, archived_entry) in component
                .entries
                .iter()
                .zip(archived_component.entries.iter())
            {
                for with_value in [true, false] {
                    let mut native = String::new();
                    let mut from_archive = String::new();
                    entry
                        .write_with_value(&mut native, with_value)
                        .expect("entry writer succeeds on a String");
                    archived_entry
                        .write_to(&mut from_archive, with_value)
                        .expect("archived entry writer succeeds on a String");
                    assert_eq!(
                        native, from_archive,
                        "archived entry writer differs (with_value {with_value})"
                    );
                    Folded(&native).check("iCalendar entry writer");
                }
            }
        }
        text
    }

    fn is_plain(ical: &ICalendar) -> bool {
        ical.components.iter().all(|component| {
            Token(component.component_type.as_str()).is_plain()
                && component.entries.iter().all(|entry| {
                    Token(entry.name.as_str()).is_plain()
                        && entry.params.iter().all(|param| {
                            Token(param.name.as_str()).is_plain()
                                && match &param.value {
                                    ICalendarParameterValue::Text(text) => {
                                        Token(text).is_stable_text()
                                    }
                                    _ => true,
                                }
                        })
                        && entry.values.iter().all(|value| match value {
                            ICalendarValue::Text(text) => Token(text).is_stable_text(),
                            _ => true,
                        })
                        && !(matches!(
                            entry.name,
                            ICalendarProperty::Rrule | ICalendarProperty::Exrule
                        ) && entry
                            .values
                            .iter()
                            .any(|value| matches!(value, ICalendarValue::Text(_))))
                })
        })
    }

    fn canonical(mut ical: ICalendar) -> ICalendar {
        for value in ical
            .components
            .iter_mut()
            .flat_map(|component| component.entries.iter_mut())
            .flat_map(|entry| entry.values.iter_mut())
        {
            if let ICalendarValue::RecurrenceRule(rule) = value
                && rule.skip.is_some()
                && rule.rscale.is_none()
            {
                rule.rscale = Some(CalendarScale::Gregorian);
            }
        }
        ical
    }

    fn reparse(text: &str, alone: bool) -> Option<ICalendar> {
        let mut parser = Parser::new(text);
        let reparsed = match parser.entry() {
            Entry::ICalendar(reparsed) => reparsed,
            other if alone => panic!("normalized writer output parses as {other:?}"),
            _ => return None,
        };
        match parser.entry() {
            Entry::Eof => Some(reparsed),
            other if alone => panic!("normalized writer output has a trailing entry {other:?}"),
            _ => None,
        }
    }
}

struct CardRoundTrip(VCard);

impl CardRoundTrip {
    fn check(self) {
        let vcard = self.0;
        let bytes = rkyv::to_bytes::<RkyvError>(&vcard).expect("vCard archives");
        let archived = rkyv::access::<ArchivedVCard, RkyvError>(&bytes).expect("archive validates");
        let restored =
            rkyv::deserialize::<VCard, RkyvError>(archived).expect("archive deserializes");
        assert_eq!(restored, vcard, "rkyv round trip changed the card");

        for version in VERSIONS {
            let text = Self::write(&vcard, version);
            Folded(&text).check("vCard writer");
            let mut from_archive = String::new();
            archived
                .write_to(&mut from_archive, version)
                .expect("archived vCard writer succeeds on a String");
            assert_eq!(
                from_archive, text,
                "archived vCard writer differs from the native writer at {version}"
            );

            for (entry, archived_entry) in vcard.entries.iter().zip(archived.entries.iter()) {
                let mut native = String::new();
                let mut from_archive = String::new();
                entry
                    .write_with_version(&mut native, version)
                    .expect("entry writer succeeds on a String");
                archived_entry
                    .write_with_version(&mut from_archive, true, version)
                    .expect("archived entry writer succeeds on a String");
                assert_eq!(
                    native, from_archive,
                    "archived vCard entry writer differs at {version}"
                );
            }
        }

        let version = VCardVersion::V4_0;
        if let Some(normalized) =
            Self::reparse(&Self::write(&vcard, version), false).filter(Self::is_plain)
        {
            let second = Self::write(&normalized, version);
            let reparsed = Self::reparse(&second, true).expect("strict reparse returns a card");
            assert_eq!(
                Self::write(&reparsed, version),
                second,
                "normalized vCard output at {version} is not a fixed point of parse and write"
            );
            let before = Self::comparable(&normalized);
            let after = Self::comparable(&reparsed);
            assert_eq!(
                before.len(),
                after.len(),
                "entry count changed in the round trip of normalized output at {version}"
            );
            for (index, (before, after)) in before.iter().zip(after.iter()).enumerate() {
                let binary_as_text = matches!(
                    (before.values.first(), after.values.first()),
                    (Some(VCardValue::Binary(_)), Some(VCardValue::Text(_)))
                );
                assert!(
                    before == after || binary_as_text,
                    "entry {index} changed in the round trip of normalized output at {version}: \
                     {before:?} became {after:?}"
                );
            }
        }
    }

    fn is_plain(vcard: &VCard) -> bool {
        vcard.entries.iter().all(|entry| {
            Token(entry.name.as_str()).is_plain()
                && entry
                    .group
                    .as_deref()
                    .is_none_or(|group| Token(group).is_plain())
                && entry.params.iter().all(|param| {
                    Token(param.name.as_str()).is_plain()
                        && param.name != VCardParameterName::Jscomps
                        && match &param.value {
                            VCardParameterValue::Text(text) => Token(text).is_stable_text(),
                            _ => true,
                        }
                })
                && entry.values.iter().all(|value| match value {
                    VCardValue::PartialDateTime(date) => {
                        date.year.is_some() == date.month.is_some()
                            && date.month.is_some() == date.day.is_some()
                    }
                    VCardValue::Text(text) => Token(text).is_stable_text(),
                    VCardValue::Binary(data) => data
                        .content_type
                        .as_deref()
                        .is_none_or(|content_type| !content_type.contains('\\')),
                    _ => true,
                })
        })
    }

    fn write(vcard: &VCard, version: VCardVersion) -> String {
        let mut text = String::new();
        vcard
            .write_to(&mut text, version)
            .expect("vCard writer succeeds on a String");
        text
    }

    fn comparable(vcard: &VCard) -> Vec<&VCardEntry> {
        vcard
            .entries
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.name,
                    VCardProperty::Version | VCardProperty::Begin | VCardProperty::End
                )
            })
            .collect()
    }

    fn reparse(text: &str, alone: bool) -> Option<VCard> {
        let mut parser = Parser::new(text);
        let reparsed = match parser.entry() {
            Entry::VCard(reparsed) => reparsed,
            other if alone => panic!("normalized vCard output parses as {other:?}"),
            _ => return None,
        };
        match parser.entry() {
            Entry::Eof => Some(reparsed),
            other if alone => panic!("normalized vCard output has a trailing entry {other:?}"),
            _ => None,
        }
    }
}
