/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use ahash::AHashSet;

use crate::{
    icalendar::{ICalendar, ICalendarProperty, ICalendarValue, Uri},
    vcard::{VCard, VCardProperty, VCardValue},
};

pub(crate) fn distinct_size<'x>(binaries: impl Iterator<Item = &'x [u8]>) -> usize {
    let mut distinct: AHashSet<&[u8]> = AHashSet::new();
    let mut size = 0usize;

    for data in binaries {
        if distinct.insert(data) {
            size = size.saturating_add(data.len());
        }
    }

    size
}

impl ICalendar {
    pub fn blob_binaries(&self) -> impl Iterator<Item = &[u8]> {
        self.components
            .iter()
            .filter(|component| component.component_type.converts_links())
            .flat_map(|component| component.entries.iter())
            .filter(|entry| {
                matches!(
                    entry.name,
                    ICalendarProperty::Attach | ICalendarProperty::Image
                )
            })
            .filter_map(|entry| entry.values.first().and_then(ICalendarValue::binary_bytes))
    }

    pub fn embedded_binaries(&self) -> impl Iterator<Item = &[u8]> {
        self.components
            .iter()
            .flat_map(|component| component.entries.iter())
            .flat_map(|entry| entry.values.iter())
            .filter_map(|value| match value {
                ICalendarValue::Binary(data) => Some(data.as_slice()),
                ICalendarValue::Uri(Uri::Data(data)) => Some(data.data.as_slice()),
                _ => None,
            })
    }

    pub fn embedded_size(&self) -> usize {
        distinct_size(self.embedded_binaries())
    }
}

impl VCard {
    pub fn blob_binaries(&self) -> impl Iterator<Item = &[u8]> {
        self.entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.name,
                    VCardProperty::Photo | VCardProperty::Logo | VCardProperty::Sound
                )
            })
            .filter_map(|entry| match entry.values.first() {
                Some(VCardValue::Binary(data)) if !data.data.is_empty() => {
                    Some(data.data.as_slice())
                }
                _ => None,
            })
    }

    pub fn embedded_binaries(&self) -> impl Iterator<Item = &[u8]> {
        self.entries
            .iter()
            .flat_map(|entry| entry.values.iter())
            .filter_map(|value| match value {
                VCardValue::Binary(data) => Some(data.data.as_slice()),
                _ => None,
            })
    }

    pub fn embedded_size(&self) -> usize {
        distinct_size(self.embedded_binaries())
    }
}

#[cfg(feature = "rkyv")]
mod archived {
    use super::distinct_size;
    use crate::{
        icalendar::{
            ArchivedICalendar, ArchivedICalendarProperty, ArchivedICalendarValue, ArchivedUri,
        },
        vcard::{ArchivedVCard, ArchivedVCardProperty, ArchivedVCardValue},
    };

    impl ArchivedICalendar {
        pub fn blob_binaries(&self) -> impl Iterator<Item = &[u8]> {
            self.components
                .iter()
                .filter(|component| component.component_type.converts_links())
                .flat_map(|component| component.entries.iter())
                .filter(|entry| {
                    matches!(
                        entry.name,
                        ArchivedICalendarProperty::Attach | ArchivedICalendarProperty::Image
                    )
                })
                .filter_map(|entry| {
                    entry
                        .values
                        .first()
                        .and_then(ArchivedICalendarValue::binary_bytes)
                })
        }

        pub fn embedded_binaries(&self) -> impl Iterator<Item = &[u8]> {
            self.components
                .iter()
                .flat_map(|component| component.entries.iter())
                .flat_map(|entry| entry.values.iter())
                .filter_map(|value| match value {
                    ArchivedICalendarValue::Binary(data) => Some(data.as_ref()),
                    ArchivedICalendarValue::Uri(ArchivedUri::Data(data)) => {
                        Some(data.data.as_ref())
                    }
                    _ => None,
                })
        }

        pub fn embedded_size(&self) -> usize {
            distinct_size(self.embedded_binaries())
        }
    }

    impl ArchivedICalendarValue {
        pub(crate) fn binary_bytes(&self) -> Option<&[u8]> {
            match self {
                ArchivedICalendarValue::Binary(data) => Some(data.as_ref()),
                ArchivedICalendarValue::Uri(ArchivedUri::Data(data)) => Some(data.data.as_ref()),
                _ => None,
            }
            .filter(|data| !data.is_empty())
        }
    }

    impl ArchivedVCard {
        pub fn blob_binaries(&self) -> impl Iterator<Item = &[u8]> {
            self.entries
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.name,
                        ArchivedVCardProperty::Photo
                            | ArchivedVCardProperty::Logo
                            | ArchivedVCardProperty::Sound
                    )
                })
                .filter_map(|entry| match entry.values.first() {
                    Some(ArchivedVCardValue::Binary(data)) if !data.data.is_empty() => {
                        Some(data.data.as_ref())
                    }
                    _ => None,
                })
        }

        pub fn embedded_binaries(&self) -> impl Iterator<Item = &[u8]> {
            self.entries
                .iter()
                .flat_map(|entry| entry.values.iter())
                .filter_map(|value| match value {
                    ArchivedVCardValue::Binary(data) => Some(data.data.as_ref()),
                    _ => None,
                })
        }

        pub fn embedded_size(&self) -> usize {
            distinct_size(self.embedded_binaries())
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{icalendar::ICalendar, vcard::VCard};

    const BINARIES: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:size\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQ=\r\n",
        "ATTACH:data:text/plain;base64,AAEC\r\n",
        "ATTACH:https://example.com/remote.pdf\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    );

    const COPIES: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:copies\r\n",
        "DTSTART:20300101T090000Z\r\n",
        "RRULE:FREQ=DAILY;COUNT=5\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQ=\r\n",
        "ATTACH:data:text/plain;base64,AAECAwQ=\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwU=\r\n",
        "END:VEVENT\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:copies\r\n",
        "RECURRENCE-ID:20300102T090000Z\r\n",
        "DTSTART:20300102T100000Z\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQ=\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAEC\r\n",
        "END:VEVENT\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:copies\r\n",
        "RECURRENCE-ID:20300103T090000Z\r\n",
        "DTSTART:20300103T100000Z\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQ=\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwU=\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    );

    const ALARMS_AND_KEYS: &str = concat!(
        "BEGIN:VCALENDAR\r\n",
        "BEGIN:VEVENT\r\n",
        "UID:alarms\r\n",
        "DTSTART:20300101T090000Z\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQ=\r\n",
        "IMAGE;ENCODING=BASE64;VALUE=BINARY:AAECAwU=\r\n",
        "BEGIN:VALARM\r\n",
        "ACTION:AUDIO\r\n",
        "TRIGGER:-PT15M\r\n",
        "ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwY=\r\n",
        "END:VALARM\r\n",
        "END:VEVENT\r\n",
        "END:VCALENDAR\r\n"
    );

    fn ical(text: &str) -> ICalendar {
        ICalendar::parse(text).expect("valid iCalendar")
    }

    fn vcard(text: &str) -> VCard {
        VCard::parse(text).expect("valid vCard")
    }

    #[test]
    fn embedded_size_counts_inline_binaries() {
        assert_eq!(ical(BINARIES).embedded_size(), 8);

        let vcard = vcard(concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:4.0\r\n",
            "FN:Size\r\n",
            "PHOTO:data:image/png;base64,AAECAw==\r\n",
            "LOGO:https://example.com/logo.png\r\n",
            "END:VCARD\r\n"
        ));
        assert_eq!(vcard.embedded_size(), 4);
    }

    #[test]
    fn embedded_size_counts_each_binary_once() {
        assert_eq!(ical(COPIES).embedded_size(), 13);

        let vcard = vcard(concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:4.0\r\n",
            "FN:Copies\r\n",
            "PHOTO:data:image/png;base64,AAECAw==\r\n",
            "LOGO:data:image/png;base64,AAECAw==\r\n",
            "SOUND:data:audio/ogg;base64,AAEC\r\n",
            "END:VCARD\r\n"
        ));
        assert_eq!(vcard.embedded_size(), 7);
    }

    #[test]
    fn embedded_size_counts_alarms_and_keys() {
        assert_eq!(ical(ALARMS_AND_KEYS).embedded_size(), 15);

        let vcard = vcard(concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:4.0\r\n",
            "FN:Keys\r\n",
            "PHOTO:data:image/png;base64,AAECAw==\r\n",
            "KEY;ENCODING=BASE64;TYPE=PGP:AAECAwQ=\r\n",
            "END:VCARD\r\n"
        ));
        assert_eq!(vcard.embedded_size(), 9);
    }

    #[test]
    fn blob_binaries_match_the_imported_links() {
        assert_eq!(
            ical(BINARIES).blob_binaries().collect::<Vec<_>>(),
            [&[0u8, 1, 2, 3, 4][..], &[0, 1, 2][..]]
        );
        assert_eq!(
            ical(ALARMS_AND_KEYS).blob_binaries().collect::<Vec<_>>(),
            [&[0u8, 1, 2, 3, 4][..], &[0, 1, 2, 3, 5][..]]
        );

        let vcard = vcard(concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:4.0\r\n",
            "FN:Blobs\r\n",
            "PHOTO:data:image/png;base64,AAECAw==\r\n",
            "LOGO:https://example.com/logo.png\r\n",
            "KEY;ENCODING=BASE64;TYPE=PGP:AAECAwQ=\r\n",
            "END:VCARD\r\n"
        ));
        assert_eq!(vcard.blob_binaries().collect::<Vec<_>>(), [&[0u8, 1, 2, 3]]);
    }

    #[test]
    fn embedded_size_of_many_duplicates_is_linear() {
        let count = 20_000;
        let mut text = String::from("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:flood\r\n");
        for _ in 0..count {
            text.push_str("ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQFBgcICQoLDA0ODw==\r\n");
        }
        text.push_str("ATTACH;ENCODING=BASE64;VALUE=BINARY:AAECAwQFBgcICQoLDA0OEA==\r\n");
        text.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");

        let ical = ical(&text);
        assert_eq!(ical.embedded_binaries().count(), count + 1);

        let started = std::time::Instant::now();
        let size = ical.embedded_size();
        let elapsed = started.elapsed();

        assert_eq!(size, 32);
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "sizing {count} identical binaries took {elapsed:?}"
        );
    }

    #[cfg(feature = "rkyv")]
    #[test]
    fn archived_binaries_match_the_native_ones() {
        for text in [BINARIES, COPIES, ALARMS_AND_KEYS] {
            let ical = ical(text);
            let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&ical).expect("serializes");
            let archived =
                rkyv::access::<crate::icalendar::ArchivedICalendar, rkyv::rancor::Error>(&bytes)
                    .expect("accesses");

            assert_eq!(
                archived.blob_binaries().collect::<Vec<_>>(),
                ical.blob_binaries().collect::<Vec<_>>()
            );
            assert_eq!(
                archived.embedded_binaries().collect::<Vec<_>>(),
                ical.embedded_binaries().collect::<Vec<_>>()
            );
            assert_eq!(archived.embedded_size(), ical.embedded_size());
        }

        let vcard = vcard(concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:4.0\r\n",
            "FN:Archived\r\n",
            "PHOTO:data:image/png;base64,AAECAw==\r\n",
            "LOGO:data:image/png;base64,AAECAw==\r\n",
            "KEY;ENCODING=BASE64;TYPE=PGP:AAECAwQ=\r\n",
            "END:VCARD\r\n"
        ));
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&vcard).expect("serializes");
        let archived = rkyv::access::<crate::vcard::ArchivedVCard, rkyv::rancor::Error>(&bytes)
            .expect("accesses");

        assert_eq!(
            archived.blob_binaries().collect::<Vec<_>>(),
            vcard.blob_binaries().collect::<Vec<_>>()
        );
        assert_eq!(
            archived.embedded_binaries().collect::<Vec<_>>(),
            vcard.embedded_binaries().collect::<Vec<_>>()
        );
        assert_eq!(archived.embedded_size(), vcard.embedded_size());
    }
}
