/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

#![no_main]

use calcard::{
    Entry,
    common::export::ExportError,
    icalendar::{ArchivedICalendar, ICalendar, ICalendarComponentType},
    jscalendar::{
        JSCalendar, export::ExportOptions as CalendarExportOptions,
        import::ImportOptions as CalendarImportOptions,
    },
    jscontact::{
        JSContact, export::ExportOptions as ContactExportOptions,
        import::ImportOptions as ContactImportOptions,
    },
    vcard::{ArchivedVCard, VCard, VCardVersion},
};
use calcard_fuzz::{EntryStream, Folded, Id};
use libfuzzer_sys::fuzz_target;
use rkyv::rancor::Error as RkyvError;

const WRITE_VERSIONS: [VCardVersion; 2] = [VCardVersion::V3_0, VCardVersion::V4_0];
const EXPANSION_LIMITS: [usize; 4] = [0, 1, 7, 3000];
const EMBEDDED_LIMITS: [usize; 4] = [0, 16, 4096, usize::MAX];

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let variant = Variant::of(data);
    for entry in EntryStream::new(&text, false) {
        match entry {
            Entry::ICalendar(ical) => CalendarConversion { ical, variant }.run(),
            Entry::VCard(vcard) => CardConversion { vcard, variant }.run(),
            _ => {}
        }
    }
    JsonConversion {
        json: &text,
        variant,
    }
    .run();
});

#[derive(Clone, Copy)]
struct Variant(u32);

impl Variant {
    fn of(data: &[u8]) -> Self {
        Variant(data.iter().fold(0x811c_9dc5u32, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
        }))
    }

    fn flag(self, bit: u32) -> bool {
        self.0 & (1 << bit) != 0
    }

    fn pick<T: Copy, const N: usize>(self, shift: u32, choices: [T; N]) -> T {
        choices
            .get((self.0 >> shift) as usize % N)
            .copied()
            .expect("the index is reduced modulo the number of choices")
    }

    fn calendar_import(self) -> CalendarImportOptions {
        CalendarImportOptions::new()
            .include_ical_components(!self.flag(0))
            .return_first(self.flag(1))
    }

    fn calendar_export(self) -> CalendarExportOptions {
        CalendarExportOptions::new()
            .max_expansions(self.pick(4, EXPANSION_LIMITS))
            .max_embedded_size(self.pick(6, EMBEDDED_LIMITS))
    }

    fn contact_import(self) -> ContactImportOptions {
        ContactImportOptions::new().include_vcard_parameters(!self.flag(2))
    }

    fn contact_export(self) -> ContactExportOptions {
        ContactExportOptions::new().max_embedded_size(self.pick(6, EMBEDDED_LIMITS))
    }

    fn declines(self, index: usize) -> bool {
        self.flag(3) && index % 2 == 1
    }
}

#[derive(Default)]
struct BlobStore(Vec<Vec<u8>>);

impl BlobStore {
    fn store(&mut self, data: &[u8], decline: bool) -> Option<Id> {
        let id = Id(u64::try_from(self.0.len()).ok()?);
        self.0.push(data.to_vec());
        (!decline).then_some(id)
    }

    fn resolve(&self, id: &Id) -> Option<Vec<u8>> {
        usize::try_from(id.0)
            .ok()
            .and_then(|index| self.0.get(index))
            .cloned()
    }
}

struct Written;

impl Written {
    fn calendar(ical: &ICalendar) -> String {
        let text = ical.to_string();
        Folded(&text).check("exported iCalendar");
        let bytes = rkyv::to_bytes::<RkyvError>(ical).expect("exported iCalendar archives");
        let archived = rkyv::access::<ArchivedICalendar, RkyvError>(&bytes)
            .expect("exported iCalendar archive validates");
        assert_eq!(
            archived.to_string(),
            text,
            "archived writer differs from the native writer on an exported calendar"
        );
        EntryStream::new(&text, false).for_each(drop);
        text
    }

    fn card(vcard: &VCard) {
        let bytes = rkyv::to_bytes::<RkyvError>(vcard).expect("exported vCard archives");
        let archived = rkyv::access::<ArchivedVCard, RkyvError>(&bytes)
            .expect("exported vCard archive validates");
        for version in WRITE_VERSIONS {
            let mut text = String::new();
            vcard
                .write_to(&mut text, version)
                .expect("exported vCard writes to a String");
            Folded(&text).check("exported vCard");
            let mut from_archive = String::new();
            archived
                .write_to(&mut from_archive, version)
                .expect("archived exported vCard writes to a String");
            assert_eq!(
                from_archive, text,
                "archived writer differs from the native writer on an exported card at {version}"
            );
            EntryStream::new(&text, false).for_each(drop);
        }
    }
}

struct CalendarConversion {
    ical: ICalendar,
    variant: Variant,
}

impl CalendarConversion {
    fn run(self) {
        let rooted = matches!(
            self.ical.components.first(),
            Some(root) if root.component_type == ICalendarComponentType::VCalendar
        );
        let imported = self.ical.clone().into_jscalendar::<Id, Id>();
        Self::export_own(imported.clone(), CalendarExportOptions::new(), None, rooted);
        let json = serde_json::to_string(&imported.0).expect("imported JSCalendar serializes");
        let reparsed = JSCalendar::<Id, Id>::parse(&json)
            .unwrap_or_else(|err| panic!("imported JSCalendar JSON does not parse: {err}: {json}"));
        let json_again =
            serde_json::to_string(&reparsed.0).expect("reparsed JSCalendar serializes");
        let reparsed_again = JSCalendar::<Id, Id>::parse(&json_again)
            .unwrap_or_else(|err| panic!("reserialized JSCalendar JSON does not parse: {err}"));
        assert_eq!(
            serde_json::to_string(&reparsed_again.0).expect("JSCalendar serializes"),
            json_again,
            "JSCalendar JSON is not a fixed point of parse and serialize"
        );
        Self::export_own(reparsed, CalendarExportOptions::new(), None, rooted);

        let variant = self.variant;
        let mut blobs = BlobStore::default();
        let imported = self
            .ical
            .into_jscalendar_with::<Id, Id, _>(variant.calendar_import().with_blob_ids(
                |data: &[u8]| {
                    let decline = variant.declines(blobs.0.len());
                    blobs.store(data, decline)
                },
            ))
            .expect("a blob id generator that never fails cannot fail the import");
        let json = serde_json::to_string(&imported.0).expect("imported JSCalendar serializes");
        JSCalendar::<Id, Id>::parse(&json)
            .unwrap_or_else(|err| panic!("imported JSCalendar JSON does not parse: {err}: {json}"));
        Self::export_own(imported, variant.calendar_export(), Some(&blobs), rooted);
    }

    fn export_own(
        jscal: JSCalendar<'_, Id, Id>,
        options: CalendarExportOptions,
        blobs: Option<&BlobStore>,
        rooted: bool,
    ) {
        let result = match blobs {
            Some(blobs) => jscal.into_icalendar_with_report(
                options.with_blob_resolver(|id: &Id| blobs.resolve(id)),
            ),
            None => jscal.into_icalendar_with_report(options),
        };
        match result {
            Ok((ical, _)) => {
                let text = Written::calendar(&ical);
                let reimported = ical.into_jscalendar::<Id, Id>();
                serde_json::to_string(&reimported.0)
                    .unwrap_or_else(|err| panic!("reimport of {text:?} does not serialize: {err}"));
            }
            Err(ExportError::NotGroup) if !rooted => {}
            Err(
                ExportError::NoComponents
                | ExportError::EmbeddedSizeExceeded { .. }
                | ExportError::InvalidPatch { .. },
            ) => {}
            Err(err) => panic!("export of an imported calendar failed: {err:?}"),
        }
    }
}

struct CardConversion {
    vcard: VCard,
    variant: Variant,
}

impl CardConversion {
    fn run(self) {
        let imported = self.vcard.clone().into_jscontact::<Id, Id>();
        Self::export_own(imported.clone(), ContactExportOptions::new(), None);
        let json = serde_json::to_string(&imported.0).expect("imported JSContact serializes");
        let reparsed = JSContact::<Id, Id>::parse(&json)
            .unwrap_or_else(|err| panic!("imported JSContact JSON does not parse: {err}: {json}"));
        let json_again = serde_json::to_string(&reparsed.0).expect("reparsed JSContact serializes");
        let reparsed_again = JSContact::<Id, Id>::parse(&json_again)
            .unwrap_or_else(|err| panic!("reserialized JSContact JSON does not parse: {err}"));
        assert_eq!(
            serde_json::to_string(&reparsed_again.0).expect("JSContact serializes"),
            json_again,
            "JSContact JSON is not a fixed point of parse and serialize"
        );
        Self::export_own(reparsed, ContactExportOptions::new(), None);

        let variant = self.variant;
        let mut blobs = BlobStore::default();
        let imported = self
            .vcard
            .into_jscontact_with::<Id, Id, _>(variant.contact_import().with_blob_ids(
                |data: &[u8]| {
                    let decline = variant.declines(blobs.0.len());
                    blobs.store(data, decline)
                },
            ))
            .expect("a blob id generator that never fails cannot fail the import");
        let json = serde_json::to_string(&imported.0).expect("imported JSContact serializes");
        JSContact::<Id, Id>::parse(&json)
            .unwrap_or_else(|err| panic!("imported JSContact JSON does not parse: {err}: {json}"));
        Self::export_own(imported, variant.contact_export(), Some(&blobs));
    }

    fn export_own(
        jscontact: JSContact<'_, Id, Id>,
        options: ContactExportOptions,
        blobs: Option<&BlobStore>,
    ) {
        let result = match blobs {
            Some(blobs) => {
                jscontact.into_vcard_with(options.with_blob_resolver(|id: &Id| blobs.resolve(id)))
            }
            None => jscontact.into_vcard_with(options),
        };
        match result {
            Ok(vcard) => {
                Written::card(&vcard);
                let reimported = vcard.into_jscontact::<Id, Id>();
                serde_json::to_string(&reimported.0).expect("reimported JSContact serializes");
            }
            Err(ExportError::EmbeddedSizeExceeded { .. }) => {}
            Err(err) => panic!("export of an imported card failed: {err:?}"),
        }
    }
}

struct JsonConversion<'x> {
    json: &'x str,
    variant: Variant,
}

impl JsonConversion<'_> {
    fn run(self) {
        if let Ok(jscal) = JSCalendar::<Id, Id>::parse(self.json) {
            let json = serde_json::to_string(&jscal.0).expect("parsed JSCalendar serializes");
            JSCalendar::<Id, Id>::parse(&json).unwrap_or_else(|err| {
                panic!("serialized JSCalendar does not parse: {err}: {json}")
            });
            for options in [CalendarExportOptions::new(), self.variant.calendar_export()] {
                if let Ok((ical, _)) = jscal.clone().into_icalendar_with_report(options) {
                    Written::calendar(&ical);
                    let reimported = ical.into_jscalendar::<Id, Id>();
                    serde_json::to_string(&reimported.0).expect("reimported JSCalendar serializes");
                }
            }
        }
        if let Ok(jscontact) = JSContact::<Id, Id>::parse(self.json) {
            let json = serde_json::to_string(&jscontact.0).expect("parsed JSContact serializes");
            JSContact::<Id, Id>::parse(&json)
                .unwrap_or_else(|err| panic!("serialized JSContact does not parse: {err}: {json}"));
            for options in [ContactExportOptions::new(), self.variant.contact_export()] {
                if let Ok(vcard) = jscontact.clone().into_vcard_with(options) {
                    Written::card(&vcard);
                    let reimported = vcard.into_jscontact::<Id, Id>();
                    serde_json::to_string(&reimported.0).expect("reimported JSContact serializes");
                }
            }
        }
    }
}
