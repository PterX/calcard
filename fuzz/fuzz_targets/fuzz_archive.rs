/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

#![no_main]

use calcard::{
    icalendar::{ArchivedICalendar, ICalendar, ICalendarProperty},
    vcard::{ArchivedVCard, VCard, VCardVersion},
};
use libfuzzer_sys::fuzz_target;
use rkyv::{rancor::Error as RkyvError, util::AlignedVec};

const VERSIONS: [VCardVersion; 4] = [
    VCardVersion::V2_0,
    VCardVersion::V2_1,
    VCardVersion::V3_0,
    VCardVersion::V4_0,
];

fuzz_target!(|data: &[u8]| {
    let Some((kind, archive)) = data.split_first() else {
        return;
    };
    let mut bytes = AlignedVec::<16>::with_capacity(archive.len());
    bytes.extend_from_slice(archive);
    if kind % 2 == 0 {
        CalendarArchive(&bytes).check();
    } else {
        CardArchive(&bytes).check();
    }
});

struct CalendarArchive<'x>(&'x [u8]);

impl CalendarArchive<'_> {
    fn check(&self) {
        let Ok(archived) = rkyv::access::<ArchivedICalendar, RkyvError>(self.0) else {
            return;
        };
        let text = archived.to_string();
        for entry in archived
            .components
            .iter()
            .flat_map(|component| component.entries.iter())
        {
            for with_value in [true, false] {
                entry
                    .write_to(&mut String::new(), with_value)
                    .expect("archived entry writer succeeds on a String");
            }
        }
        let Ok(mut native) = rkyv::deserialize::<ICalendar, RkyvError>(archived) else {
            return;
        };
        assert_eq!(
            native.size(),
            archived.size(),
            "native and archived sizes differ"
        );
        assert!(
            native.uids().eq(archived.uids()),
            "native and archived uids differ"
        );
        for component in &mut native.components {
            component.entries.retain(|entry| {
                !matches!(
                    entry.name,
                    ICalendarProperty::Begin | ICalendarProperty::End
                )
            });
        }
        assert_eq!(
            native.to_string(),
            text,
            "archived writer differs from the native writer on a deserialized archive"
        );
        let again = rkyv::to_bytes::<RkyvError>(&native).expect("deserialized calendar archives");
        let reread = rkyv::access::<ArchivedICalendar, RkyvError>(&again)
            .expect("a fresh archive validates");
        assert_eq!(
            reread.to_string(),
            text,
            "archive round trip changed the text"
        );
    }
}

struct CardArchive<'x>(&'x [u8]);

impl CardArchive<'_> {
    fn check(&self) {
        let Ok(archived) = rkyv::access::<ArchivedVCard, RkyvError>(self.0) else {
            return;
        };
        let texts = VERSIONS.map(|version| {
            let mut text = String::new();
            let result = archived.write_to(&mut text, version);
            (result.is_ok(), text)
        });
        for entry in archived.entries.iter() {
            for version in VERSIONS {
                for with_value in [true, false] {
                    entry
                        .write_with_version(&mut String::new(), with_value, version)
                        .expect("archived entry writer succeeds on a String");
                }
            }
        }
        let Ok(native) = rkyv::deserialize::<VCard, RkyvError>(archived) else {
            return;
        };
        for (version, (archived_ok, archived_text)) in VERSIONS.iter().zip(texts.iter()) {
            let mut text = String::new();
            let native_ok = native.write_to(&mut text, *version).is_ok();
            assert_eq!(
                (native_ok, &text),
                (*archived_ok, archived_text),
                "archived vCard writer differs from the native writer at {version}"
            );
        }
        assert_eq!(
            native.uid(),
            archived.uid(),
            "native and archived uids differ"
        );
    }
}
