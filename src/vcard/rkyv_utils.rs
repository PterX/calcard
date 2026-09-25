/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use super::*;
use crate::common::{ArchivedCalendarScale, ArchivedData, ArchivedPartialDateTime};

impl ArchivedVCard {
    pub fn uid(&self) -> Option<&str> {
        self.property(&VCardProperty::Uid)
            .and_then(|e| e.values.first())
            .and_then(|v| v.as_text())
    }

    pub fn property(&self, prop: &VCardProperty) -> Option<&ArchivedVCardEntry> {
        self.entries.iter().find(|entry| &entry.name == prop)
    }

    pub fn properties<'x, 'y: 'x>(
        &'x self,
        prop: &'y VCardProperty,
    ) -> impl Iterator<Item = &'x ArchivedVCardEntry> + 'x {
        self.entries.iter().filter(move |entry| &entry.name == prop)
    }

    pub fn version(&self) -> Option<VCardVersion> {
        self.entries
            .iter()
            .find(|e| e.name == VCardProperty::Version)
            .and_then(|e| {
                e.values
                    .first()
                    .and_then(|v| v.as_text())
                    .and_then(VCardVersion::try_parse)
            })
    }
}

impl ArchivedVCardEntry {
    #[inline]
    pub fn parameters(
        &self,
        prop: &VCardParameterName,
    ) -> impl Iterator<Item = &ArchivedVCardParameterValue> {
        self.params.iter().filter_map(move |param| {
            if &param.name == prop {
                Some(&param.value)
            } else {
                None
            }
        })
    }
}

impl ArchivedVCardValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ArchivedVCardValue::Text(v) => Some(v.as_str()),
            ArchivedVCardValue::Sex(v) => Some(v.as_str()),
            ArchivedVCardValue::GramGender(v) => Some(v.as_str()),
            ArchivedVCardValue::Kind(v) => Some(v.as_str()),
            ArchivedVCardValue::Component(v) => v.first().map(|s| s.as_str()),
            ArchivedVCardValue::Integer(_)
            | ArchivedVCardValue::Float(_)
            | ArchivedVCardValue::Boolean(_)
            | ArchivedVCardValue::PartialDateTime(_)
            | ArchivedVCardValue::Binary(_) => None,
        }
    }

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            ArchivedVCardValue::Integer(i) => Some(i.to_native()),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            ArchivedVCardValue::Float(f) => Some(f.to_native()),
            _ => None,
        }
    }

    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            ArchivedVCardValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_partial_date_time(&self) -> Option<&ArchivedPartialDateTime> {
        match self {
            ArchivedVCardValue::PartialDateTime(dt) => Some(dt),
            _ => None,
        }
    }

    pub fn as_binary(&self) -> Option<&ArchivedData> {
        match self {
            ArchivedVCardValue::Binary(d) => Some(d),
            _ => None,
        }
    }
}

impl ArchivedVCardParameterName {
    pub fn as_str(&self) -> &str {
        match self {
            ArchivedVCardParameterName::Language => "LANGUAGE",
            ArchivedVCardParameterName::Value => "VALUE",
            ArchivedVCardParameterName::Pref => "PREF",
            ArchivedVCardParameterName::Altid => "ALTID",
            ArchivedVCardParameterName::Pid => "PID",
            ArchivedVCardParameterName::Type => "TYPE",
            ArchivedVCardParameterName::Mediatype => "MEDIATYPE",
            ArchivedVCardParameterName::Calscale => "CALSCALE",
            ArchivedVCardParameterName::SortAs => "SORT-AS",
            ArchivedVCardParameterName::Geo => "GEO",
            ArchivedVCardParameterName::Tz => "TZ",
            ArchivedVCardParameterName::Index => "INDEX",
            ArchivedVCardParameterName::Level => "LEVEL",
            ArchivedVCardParameterName::Group => "GROUP",
            ArchivedVCardParameterName::Cc => "CC",
            ArchivedVCardParameterName::Author => "AUTHOR",
            ArchivedVCardParameterName::AuthorName => "AUTHOR-NAME",
            ArchivedVCardParameterName::Created => "CREATED",
            ArchivedVCardParameterName::Derived => "DERIVED",
            ArchivedVCardParameterName::Label => "LABEL",
            ArchivedVCardParameterName::Phonetic => "PHONETIC",
            ArchivedVCardParameterName::PropId => "PROP-ID",
            ArchivedVCardParameterName::Script => "SCRIPT",
            ArchivedVCardParameterName::ServiceType => "SERVICE-TYPE",
            ArchivedVCardParameterName::Username => "USERNAME",
            ArchivedVCardParameterName::Jsptr => "JSPTR",
            ArchivedVCardParameterName::Jscomps => "JSCOMPS",
            ArchivedVCardParameterName::Other(name) => name.as_str(),
        }
    }
}

impl ArchivedVCardParameterValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ArchivedVCardParameterValue::Text(v) => Some(v.as_str()),
            ArchivedVCardParameterValue::ValueType(v) => Some(v.as_str()),
            ArchivedVCardParameterValue::Type(v) => Some(v.as_str()),
            ArchivedVCardParameterValue::Calscale(v) => Some(v.as_str()),
            ArchivedVCardParameterValue::Level(v) => Some(v.as_str()),
            ArchivedVCardParameterValue::Phonetic(v) => Some(v.as_str()),
            ArchivedVCardParameterValue::Jscomps(_)
            | ArchivedVCardParameterValue::Integer(_)
            | ArchivedVCardParameterValue::Timestamp(_)
            | ArchivedVCardParameterValue::Bool(_)
            | ArchivedVCardParameterValue::Null => None,
        }
    }

    pub fn as_phonetic(&self) -> Option<IanaType<&ArchivedVCardPhonetic, &str>> {
        match self {
            ArchivedVCardParameterValue::Phonetic(v) => Some(IanaType::Iana(v)),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_calscale(&self) -> Option<IanaType<&ArchivedCalendarScale, &str>> {
        match self {
            ArchivedVCardParameterValue::Calscale(v) => Some(IanaType::Iana(v)),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_level(&self) -> Option<IanaType<&ArchivedVCardLevel, &str>> {
        match self {
            ArchivedVCardParameterValue::Level(v) => Some(IanaType::Iana(v)),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_type(&self) -> Option<IanaType<&ArchivedVCardType, &str>> {
        match self {
            ArchivedVCardParameterValue::Type(v) => Some(IanaType::Iana(v)),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_value_type(&self) -> Option<IanaType<&ArchivedVCardValueType, &str>> {
        match self {
            ArchivedVCardParameterValue::ValueType(v) => Some(IanaType::Iana(v)),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<IanaType<bool, &str>> {
        match self {
            ArchivedVCardParameterValue::Bool(b) => Some(IanaType::Iana(*b)),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_integer(&self) -> Option<IanaType<u32, &str>> {
        match self {
            ArchivedVCardParameterValue::Integer(i) => Some(IanaType::Iana(i.to_native())),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }

    pub fn as_timestamp(&self) -> Option<IanaType<i64, &str>> {
        match self {
            ArchivedVCardParameterValue::Timestamp(t) => Some(IanaType::Iana(t.to_native())),
            ArchivedVCardParameterValue::Text(v) => Some(IanaType::Other(v.as_str())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        common::{Data, PartialDateTime},
        vcard::{
            ArchivedVCard, VCard, VCardEntry, VCardGramGender, VCardKind, VCardProperty, VCardSex,
            VCardValue,
        },
    };
    use rkyv::rancor::Error;

    #[test]
    fn archived_values_read_as_the_native_text() {
        let vcard = VCard {
            entries: vec![
                VCardEntry::new(VCardProperty::Uid).with_value(VCardValue::Kind(VCardKind::Group)),
                VCardEntry::new(VCardProperty::Version).with_value(VCardValue::Text("4.0".into())),
                VCardEntry::new(VCardProperty::Gender).with_values([
                    VCardValue::Sex(VCardSex::Female),
                    VCardValue::Text("woman".into()),
                ]),
                VCardEntry::new(VCardProperty::Gramgender)
                    .with_value(VCardValue::GramGender(VCardGramGender::Neuter)),
                VCardEntry::new(VCardProperty::N).with_values([
                    VCardValue::Text("Doe".into()),
                    VCardValue::Component(vec!["John".into(), "Paul".into()]),
                    VCardValue::Component(vec![]),
                ]),
                VCardEntry::new(VCardProperty::Other("X-VALUES".into())).with_values([
                    VCardValue::Integer(3),
                    VCardValue::Float(1.5),
                    VCardValue::Boolean(true),
                    VCardValue::PartialDateTime(PartialDateTime::default()),
                    VCardValue::Binary(Box::new(Data {
                        content_type: None,
                        data: vec![1, 2, 3],
                    })),
                ]),
            ],
        };
        let bytes = rkyv::to_bytes::<Error>(&vcard).expect("the card archives");
        let archived = rkyv::access::<ArchivedVCard, Error>(&bytes).expect("the archive validates");

        assert_eq!(archived.uid(), Some("GROUP"));
        assert_eq!(archived.uid(), vcard.uid());
        assert_eq!(archived.version(), vcard.version());
        for (entry, archived_entry) in vcard.entries.iter().zip(archived.entries.iter()) {
            for (value, archived_value) in entry.values.iter().zip(archived_entry.values.iter()) {
                assert_eq!(archived_value.as_text(), value.as_text(), "{value:?}");
            }
        }
    }
}
