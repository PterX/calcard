/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    common::IanaType,
    jscontact::{
        Context, Feature, JSContactId, JSContactKind, JSContactLevel, JSContactPhoneticSystem,
        JSContactProperty, JSContactValue,
        import::{ExtractedParams, VCardParams},
    },
    vcard::{
        VCardLevel, VCardParameter, VCardParameterName, VCardParameterValue, VCardPhonetic,
        VCardProperty, VCardType,
    },
};
use jmap_tools::{Key, Map, Value};
use std::str::FromStr;

impl Feature {
    pub(crate) fn from_vcard_type(property: &VCardProperty, typ: &VCardType) -> Option<Self> {
        match (property, typ) {
            (VCardProperty::Tel, VCardType::Cell) => Some(Feature::Mobile),
            (VCardProperty::Tel, VCardType::Fax) => Some(Feature::Fax),
            (VCardProperty::Tel, VCardType::MainNumber) => Some(Feature::MainNumber),
            (VCardProperty::Tel, VCardType::Pager) => Some(Feature::Pager),
            (VCardProperty::Tel, VCardType::Text) => Some(Feature::Text),
            (VCardProperty::Tel, VCardType::Textphone) => Some(Feature::TextPhone),
            (VCardProperty::Tel, VCardType::Video) => Some(Feature::Video),
            (VCardProperty::Tel, VCardType::Voice) => Some(Feature::Voice),
            _ => None,
        }
    }
}

impl<I: JSContactId> JSContactProperty<I> {
    pub(crate) fn from_vcard_type(
        property: &VCardProperty,
        typ: IanaType<VCardType, String>,
    ) -> (Self, Key<'static, Self>) {
        let feature = match &typ {
            IanaType::Iana(typ) => Feature::from_vcard_type(property, typ),
            IanaType::Other(_) => None,
        };
        if let Some(feature) = feature {
            return (
                JSContactProperty::Features,
                Key::Property(JSContactProperty::Feature(feature)),
            );
        }

        let key = match typ {
            IanaType::Iana(VCardType::Home) => {
                Key::Property(JSContactProperty::Context(Context::Private))
            }
            IanaType::Iana(VCardType::Work) => {
                Key::Property(JSContactProperty::Context(Context::Work))
            }
            IanaType::Iana(VCardType::Billing) => {
                Key::Property(JSContactProperty::Context(Context::Billing))
            }
            IanaType::Iana(VCardType::Delivery) => {
                Key::Property(JSContactProperty::Context(Context::Delivery))
            }
            IanaType::Iana(VCardType::Cell) => Key::Borrowed("mobile"),
            typ => {
                let key = typ.into_string().to_ascii_lowercase();
                match Context::from_str(&key) {
                    Ok(context) => Key::Property(JSContactProperty::Context(context)),
                    Err(()) => Key::Owned(key),
                }
            }
        };
        (JSContactProperty::Contexts, key)
    }
}

impl VCardParameter {
    pub(super) fn as_media_type(&self) -> Option<&str> {
        match (&self.name, &self.value) {
            (VCardParameterName::Mediatype, VCardParameterValue::Text(media_type))
                if !media_type.is_empty() =>
            {
                Some(media_type)
            }
            _ => None,
        }
    }
}

impl ExtractedParams {
    pub(super) fn prop_id(&mut self) -> Option<String> {
        self.prop_id.take()
    }

    pub(super) fn alt_id(&mut self) -> Option<String> {
        self.alt_id.take()
    }

    pub(super) fn language(&mut self) -> Option<String> {
        self.language.take()
    }

    pub(super) fn types(&mut self) -> Vec<IanaType<VCardType, String>> {
        std::mem::take(&mut self.types)
    }

    #[allow(clippy::type_complexity)]
    pub(super) fn into_iter<I: JSContactId, B: JSContactId>(
        mut self,
        property: &VCardProperty,
    ) -> impl Iterator<
        Item = (
            Key<'static, JSContactProperty<I>>,
            Value<'static, JSContactProperty<I>, JSContactValue<I, B>>,
        ),
    > {
        let mut contexts: Option<
            Vec<(
                Key<'static, JSContactProperty<I>>,
                Value<'static, JSContactProperty<I>, JSContactValue<I, B>>,
            )>,
        > = None;
        let mut features: Option<
            Vec<(
                Key<'static, JSContactProperty<I>>,
                Value<'static, JSContactProperty<I>, JSContactValue<I, B>>,
            )>,
        > = None;

        for typ in std::mem::take(&mut self.types) {
            let (bucket, key) = JSContactProperty::<I>::from_vcard_type(property, typ);
            let keys = if bucket == JSContactProperty::Features {
                features.get_or_insert_default()
            } else {
                contexts.get_or_insert_default()
            };
            if keys.iter().all(|(existing, _)| existing != &key) {
                keys.push((key, Value::Bool(true)));
            }
        }

        let author = if self.author.is_some() || self.author_name.is_some() {
            Value::Object(Map::from_iter(
                [
                    self.author_name.map(|name| {
                        (
                            Key::Property(JSContactProperty::Name),
                            Value::Str(name.into()),
                        )
                    }),
                    self.author.map(|author| {
                        (
                            Key::Property(JSContactProperty::Uri),
                            Value::Str(author.into()),
                        )
                    }),
                ]
                .into_iter()
                .flatten(),
            ))
            .into()
        } else {
            None
        };

        [
            (
                Key::Property(JSContactProperty::Contexts),
                contexts.map(|v| Value::Object(Map::from(v))),
            ),
            (
                Key::Property(JSContactProperty::Features),
                features.map(|v| Value::Object(Map::from(v))),
            ),
            (
                Key::Property(JSContactProperty::Language),
                self.language.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::Pref),
                self.pref.map(|v| Value::Number((v as u64).into())),
            ),
            (Key::Property(JSContactProperty::Author), author),
            (
                Key::Property(JSContactProperty::MediaType),
                self.media_type.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::PhoneticSystem),
                self.phonetic_system.map(|v| match v {
                    IanaType::Iana(value) => {
                        Value::Element(JSContactValue::PhoneticSystem(match value {
                            VCardPhonetic::Ipa => JSContactPhoneticSystem::Ipa,
                            VCardPhonetic::Jyut => JSContactPhoneticSystem::Jyut,
                            VCardPhonetic::Piny => JSContactPhoneticSystem::Piny,
                            VCardPhonetic::Script => JSContactPhoneticSystem::Script,
                        }))
                    }
                    IanaType::Other(value) => Value::Str(value.into()),
                }),
            ),
            (
                Key::Property(JSContactProperty::PhoneticScript),
                self.phonetic_script.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::CalendarScale),
                self.calscale.map(|value| match value {
                    IanaType::Iana(value) => Value::Element(JSContactValue::CalendarScale(value)),
                    IanaType::Other(value) => Value::Str(value.into()),
                }),
            ),
            (
                Key::Property(JSContactProperty::SortAs),
                self.sort_as.map(|sort_as| {
                    if matches!(property, VCardProperty::N) {
                        if let Some((surname, given)) =
                            sort_as.split_once(',').and_then(|(surname, given)| {
                                let surname = surname.trim();
                                let given = given.trim();

                                if surname.is_empty() || given.is_empty() {
                                    None
                                } else {
                                    Some((surname.to_string(), given.to_string()))
                                }
                            })
                        {
                            Value::Object(Map::from_iter(vec![
                                (
                                    Key::Property(JSContactProperty::SortAsKind(
                                        JSContactKind::Surname,
                                    )),
                                    Value::Str(surname.into()),
                                ),
                                (
                                    Key::Property(JSContactProperty::SortAsKind(
                                        JSContactKind::Given,
                                    )),
                                    Value::Str(given.into()),
                                ),
                            ]))
                        } else {
                            Value::Object(Map::from_iter(vec![(
                                Key::Property(JSContactProperty::SortAsKind(
                                    JSContactKind::Surname,
                                )),
                                Value::Str(sort_as.into()),
                            )]))
                        }
                    } else {
                        Value::Str(sort_as.into())
                    }
                }),
            ),
            (
                Key::Property(JSContactProperty::Coordinates),
                self.geo.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::TimeZone),
                self.tz.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::ListAs),
                self.index.map(|v| Value::Number((v as u64).into())),
            ),
            (
                Key::Property(JSContactProperty::Level),
                self.level.map(|value| match value {
                    IanaType::Iana(value) => Value::Element(JSContactValue::Level(match value {
                        VCardLevel::Beginner | VCardLevel::Low => JSContactLevel::Low,
                        VCardLevel::Average | VCardLevel::Medium => JSContactLevel::Medium,
                        VCardLevel::Expert | VCardLevel::High => JSContactLevel::High,
                    })),
                    IanaType::Other(value) => Value::Str(value.into()),
                }),
            ),
            (
                Key::Property(JSContactProperty::CountryCode),
                self.country_code.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::Created),
                self.created
                    .map(|v| Value::Element(JSContactValue::Timestamp(v))),
            ),
            (
                Key::Property(if matches!(property, VCardProperty::Adr) {
                    JSContactProperty::Full
                } else {
                    JSContactProperty::Label
                }),
                self.label.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::Service),
                self.service_type.map(Into::into).map(Value::Str),
            ),
            (
                Key::Property(JSContactProperty::User),
                self.username.map(Into::into).map(Value::Str),
            ),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
    }
}

impl<I: JSContactId, B: JSContactId> VCardParams<I, B> {
    pub(super) fn into_jscontact_value(
        self,
    ) -> Option<Map<'static, JSContactProperty<I>, JSContactValue<I, B>>> {
        if !self.0.is_empty() {
            let mut obj = Map::from(Vec::with_capacity(self.0.len()));

            for (param, value) in self.0 {
                let value = if value.len() > 1 {
                    Value::Array(value)
                } else {
                    value.into_iter().next().unwrap()
                };
                obj.insert_unchecked(Key::Owned(param.into_string().to_ascii_lowercase()), value);
            }
            Some(obj)
        } else {
            None
        }
    }
}
