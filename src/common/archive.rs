/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use rkyv::{
    Archive, Archived, Deserialize, Place, Serialize,
    rancor::Fallible,
    ser::{Allocator, Writer},
    vec::{ArchivedVec, VecResolver},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use smallvec::{Array, SmallVec};

pub struct Unboxed;

impl<T: Archive> ArchiveWith<Box<T>> for Unboxed {
    type Archived = T::Archived;
    type Resolver = T::Resolver;

    fn resolve_with(field: &Box<T>, resolver: Self::Resolver, out: Place<Self::Archived>) {
        T::resolve(field, resolver, out);
    }
}

impl<T: Serialize<S>, S: Fallible + ?Sized> SerializeWith<Box<T>, S> for Unboxed {
    fn serialize_with(field: &Box<T>, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        T::serialize(field, serializer)
    }
}

impl<T, D> DeserializeWith<T::Archived, Box<T>, D> for Unboxed
where
    T: Archive,
    T::Archived: Deserialize<T, D>,
    D: Fallible + ?Sized,
{
    fn deserialize_with(field: &T::Archived, deserializer: &mut D) -> Result<Box<T>, D::Error> {
        field.deserialize(deserializer).map(Box::new)
    }
}

pub struct InlineVec;

impl<A> ArchiveWith<SmallVec<A>> for InlineVec
where
    A: Array,
    A::Item: Archive,
{
    type Archived = ArchivedVec<Archived<A::Item>>;
    type Resolver = VecResolver;

    fn resolve_with(field: &SmallVec<A>, resolver: Self::Resolver, out: Place<Self::Archived>) {
        ArchivedVec::resolve_from_slice(field.as_slice(), resolver, out);
    }
}

impl<A, S> SerializeWith<SmallVec<A>, S> for InlineVec
where
    A: Array,
    A::Item: Serialize<S>,
    S: Fallible + Allocator + Writer + ?Sized,
{
    fn serialize_with(field: &SmallVec<A>, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        ArchivedVec::serialize_from_slice(field.as_slice(), serializer)
    }
}

impl<A, D> DeserializeWith<ArchivedVec<Archived<A::Item>>, SmallVec<A>, D> for InlineVec
where
    A: Array,
    A::Item: Archive,
    Archived<A::Item>: Deserialize<A::Item, D>,
    D: Fallible + ?Sized,
{
    fn deserialize_with(
        field: &ArchivedVec<Archived<A::Item>>,
        deserializer: &mut D,
    ) -> Result<SmallVec<A>, D::Error> {
        let mut items = SmallVec::with_capacity(field.len());
        for item in field.iter() {
            items.push(item.deserialize(deserializer)?);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::{InlineVec, Unboxed};
    use crate::{
        common::Data,
        icalendar::{ArchivedICalendar, ICalendar, ICalendarValue, Uri},
        vcard::{ArchivedVCard, VCard, VCardValue},
    };
    use rkyv::rancor::Error;
    use smallvec::SmallVec;

    #[derive(Debug, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
    struct Wrapped {
        #[rkyv(with = InlineVec)]
        values: SmallVec<[String; 1]>,
        #[rkyv(with = Unboxed)]
        data: Box<Data>,
    }

    #[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
    struct Plain {
        values: Vec<String>,
        data: Data,
    }

    #[test]
    fn wrappers_archive_like_vec_and_inline_data() {
        for count in 0..=4 {
            for len in [0, 1, 39] {
                for content_type in [None, Some(""), Some("text/plain")] {
                    for data_len in [0u8, 1, 19] {
                        let values = (0..count)
                            .map(|extra| "x".repeat(len + extra))
                            .collect::<Vec<_>>();
                        let data = Data {
                            content_type: content_type.map(str::to_string),
                            data: (0..data_len).collect(),
                        };
                        let wrapped = Wrapped {
                            values: values.iter().cloned().collect(),
                            data: Box::new(data.clone()),
                        };
                        let plain = Plain { values, data };
                        let wrapped_bytes = rkyv::to_bytes::<Error>(&wrapped).expect("archive");
                        let plain_bytes = rkyv::to_bytes::<Error>(&plain).expect("archive");
                        assert_eq!(wrapped_bytes.as_slice(), plain_bytes.as_slice());
                        let archived =
                            rkyv::access::<ArchivedWrapped, Error>(&wrapped_bytes).expect("access");
                        let restored =
                            rkyv::deserialize::<Wrapped, Error>(archived).expect("deserialize");
                        assert_eq!(restored.values.capacity(), count.max(1));
                        assert_eq!(restored.values.spilled(), count > 1);
                        assert_eq!(restored, wrapped);
                    }
                }
            }
        }
    }

    #[test]
    fn calendar_and_card_values_round_trip_through_the_archive() {
        let ical = ICalendar::parse(concat!(
            "BEGIN:VCALENDAR\r\n",
            "BEGIN:VEVENT\r\n",
            "DTSTART;VALUE=DATE:20250101\r\n",
            "DTEND;TZID=Europe/Paris:20250102T090000\r\n",
            "DTSTAMP:20250101T000000Z\r\n",
            "RDATE;VALUE=PERIOD:20231223T150000Z/PT2H,20231224T150000Z/20231224T170000Z\r\n",
            "EXDATE:20250301T090000Z,20250302T090000Z,20250303T090000Z\r\n",
            "ATTACH:data:text/plain;base64,SGVsbG8=\r\n",
            "SUMMARY:x\r\n",
            "END:VEVENT\r\n",
            "END:VCALENDAR\r\n"
        ))
        .expect("valid calendar");
        let values = ical
            .components
            .iter()
            .flat_map(|component| &component.entries)
            .flat_map(|entry| &entry.values);
        assert!(
            values
                .clone()
                .any(|value| matches!(value, ICalendarValue::PartialDateTime(_)))
        );
        assert!(
            values
                .clone()
                .any(|value| matches!(value, ICalendarValue::Period(_)))
        );
        assert!(
            values
                .clone()
                .any(|value| matches!(value, ICalendarValue::Uri(Uri::Data(_))))
        );
        let bytes = rkyv::to_bytes::<Error>(&ical).expect("archive");
        let archived = rkyv::access::<ArchivedICalendar, Error>(&bytes).expect("access");
        assert_eq!(archived.to_string(), ical.to_string());
        let restored = rkyv::deserialize::<ICalendar, Error>(archived).expect("deserialize");
        assert_eq!(restored, ical);
        for (entry, restored) in ical
            .components
            .iter()
            .flat_map(|component| &component.entries)
            .zip(
                restored
                    .components
                    .iter()
                    .flat_map(|component| &component.entries),
            )
        {
            assert_eq!(
                restored.values.capacity(),
                entry.values.len().max(1),
                "{entry:?}"
            );
        }

        for card in [
            concat!(
                "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
                "BDAY:--0412\r\n",
                "ANNIVERSARY:20100101T120000Z\r\n",
                "NICKNAME:a,b,c\r\n",
                "PHOTO:data:image/png;base64,iVBORw0KGgo=\r\n",
                "END:VCARD\r\n"
            ),
            concat!(
                "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jane\r\n",
                "BDAY:1980-04-12\r\n",
                "PHOTO;ENCODING=b;TYPE=PNG:iVBORw0KGgo=\r\n",
                "END:VCARD\r\n"
            ),
        ] {
            let vcard = VCard::parse(card).expect("valid vCard");
            let values = vcard.entries.iter().flat_map(|entry| &entry.values);
            assert!(
                values
                    .clone()
                    .any(|value| matches!(value, VCardValue::PartialDateTime(_))),
                "{card:?}"
            );
            assert!(
                values
                    .clone()
                    .any(|value| matches!(value, VCardValue::Binary(_))),
                "{card:?}"
            );
            let bytes = rkyv::to_bytes::<Error>(&vcard).expect("archive");
            let archived = rkyv::access::<ArchivedVCard, Error>(&bytes).expect("access");
            assert_eq!(archived.to_string(), vcard.to_string());
            let restored = rkyv::deserialize::<VCard, Error>(archived).expect("deserialize");
            assert_eq!(restored, vcard);
            for (entry, restored) in vcard.entries.iter().zip(&restored.entries) {
                assert_eq!(
                    restored.values.capacity(),
                    entry.values.len().max(1),
                    "{entry:?}"
                );
            }
        }
    }
}
