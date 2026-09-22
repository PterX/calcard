/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    common::{CalendarScale, IanaParse, IanaType, PartialDateTime, parser::Integer},
    icalendar::Uri,
    jscontact::{
        JSContactGrammaticalGender, JSContactId, JSContactKind, JSContactLevel,
        JSContactPhoneticSystem, JSContactProperty, JSContactValue,
    },
    vcard::{
        VCardGramGender, VCardKind, VCardLevel, VCardPhonetic, VCardSex, VCardValue,
        VCardValueType, ValueType,
    },
};
use jmap_tools::{JsonPointerItem, Key, Map, Value};
use std::{borrow::Cow, iter::Peekable, vec::IntoIter};

pub(super) fn build_path<'x, I, B>(
    obj: &mut Value<'x, JSContactProperty<I>, JSContactValue<I, B>>,
    mut ptr: Peekable<IntoIter<JsonPointerItem<JSContactProperty<I>>>>,
    value: Value<'x, JSContactProperty<I>, JSContactValue<I, B>>,
) -> Option<Value<'x, JSContactProperty<I>, JSContactValue<I, B>>>
where
    I: JSContactId,
    B: JSContactId,
{
    if let Some(item) = ptr.next() {
        match item {
            JsonPointerItem::Root | JsonPointerItem::Wildcard | JsonPointerItem::Invalid(_) => {}
            JsonPointerItem::Key(key) => match obj {
                Value::Object(obj) => {
                    return build_path(
                        obj.insert_or_get_mut(
                            key,
                            if matches!(ptr.peek(), Some(JsonPointerItem::Key(_))) {
                                Value::Object(Map::from(Vec::new()))
                            } else {
                                Value::Array(Vec::new())
                            },
                        ),
                        ptr,
                        value,
                    );
                }
                Value::Null => {
                    *obj = Value::Object(Map::from(vec![(key, Value::Null)]));
                    return build_path(
                        &mut obj
                            .as_object_mut()
                            .unwrap()
                            .as_mut_vec()
                            .last_mut()
                            .unwrap()
                            .1,
                        ptr,
                        value,
                    );
                }
                _ => {}
            },
            JsonPointerItem::Number(idx) => {
                if let Some(arr) = obj.as_array_mut() {
                    if (idx as usize) < arr.len() {
                        return build_path(&mut arr[idx as usize], ptr, value);
                    }

                    if idx < 20 {
                        arr.resize_with(idx as usize + 1, || Value::Null);
                        return build_path(&mut arr[idx as usize], ptr, value);
                    }
                }
            }
        }
        Some(value)
    } else {
        *obj = value;
        None
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn convert_anniversary<I, B>(
    value: Value<'_, JSContactProperty<I>, JSContactValue<I, B>>,
) -> Result<
    (PartialDateTime, Option<CalendarScale>),
    Value<'_, JSContactProperty<I>, JSContactValue<I, B>>,
>
where
    I: JSContactId,
    B: JSContactId,
{
    let mut date = PartialDateTime::default();
    let mut calendar_scale = None;
    let mut is_valid = true;
    let Some(object) = value.as_object() else {
        return Err(value);
    };

    for (key, value) in object.as_vec() {
        match key {
            Key::Property(JSContactProperty::Day) => {
                if let Value::Number(day) = value {
                    date.day = day
                        .as_u64()
                        .filter(|day| (1..=31).contains(day))
                        .map(|day| day as u8);
                    is_valid &= date.day.is_some();
                }
            }
            Key::Property(JSContactProperty::Month) => {
                if let Value::Number(month) = value {
                    date.month = month
                        .as_u64()
                        .filter(|month| (1..=12).contains(month))
                        .map(|month| month as u8);
                    is_valid &= date.month.is_some();
                }
            }
            Key::Property(JSContactProperty::Year) => {
                if let Value::Number(year) = value {
                    date.year = year
                        .as_u64()
                        .filter(|year| *year <= 9999)
                        .map(|year| year as u16);
                    is_valid &= date.year.is_some();
                }
            }
            Key::Property(JSContactProperty::CalendarScale) => {
                if let Value::Element(JSContactValue::CalendarScale(scale)) = value {
                    calendar_scale = Some(scale.clone());
                }
            }
            Key::Property(JSContactProperty::Utc) => {
                if let Value::Element(JSContactValue::Timestamp(timestamp)) = value {
                    return Ok((PartialDateTime::from_utc_timestamp(*timestamp), None));
                }
            }
            _ => {}
        }
    }

    if is_valid && (date.year.is_some() || date.month.is_some() || date.day.is_some()) {
        Ok((date, calendar_scale))
    } else {
        Err(value)
    }
}

pub(super) fn convert_value<'x, I, B>(
    value: Value<'x, JSContactProperty<I>, JSContactValue<I, B>>,
    value_type: &'_ ValueType,
) -> Result<VCardValue, Value<'x, JSContactProperty<I>, JSContactValue<I, B>>>
where
    I: JSContactId,
    B: JSContactId,
{
    match value {
        Value::Element(e) => match e {
            JSContactValue::Timestamp(t) => Ok(VCardValue::PartialDateTime(
                PartialDateTime::from_utc_timestamp(t),
            )),
            JSContactValue::GrammaticalGender(g) => Ok(VCardValue::GramGender(match g {
                JSContactGrammaticalGender::Animate => VCardGramGender::Animate,
                JSContactGrammaticalGender::Common => VCardGramGender::Common,
                JSContactGrammaticalGender::Feminine => VCardGramGender::Feminine,
                JSContactGrammaticalGender::Inanimate => VCardGramGender::Inanimate,
                JSContactGrammaticalGender::Masculine => VCardGramGender::Masculine,
                JSContactGrammaticalGender::Neuter => VCardGramGender::Neuter,
            })),
            JSContactValue::Kind(k) => match k {
                JSContactKind::Individual => Ok(VCardValue::Kind(VCardKind::Individual)),
                JSContactKind::Group => Ok(VCardValue::Kind(VCardKind::Group)),
                JSContactKind::Location => Ok(VCardValue::Kind(VCardKind::Location)),
                JSContactKind::Org => Ok(VCardValue::Kind(VCardKind::Org)),
                JSContactKind::Application => Ok(VCardValue::Kind(VCardKind::Application)),
                JSContactKind::Device => Ok(VCardValue::Kind(VCardKind::Device)),
                _ => Err(Value::Element(JSContactValue::Kind(k))),
            },
            JSContactValue::Level(_)
            | JSContactValue::Type(_)
            | JSContactValue::Relation(_)
            | JSContactValue::PhoneticSystem(_)
            | JSContactValue::CalendarScale(_)
            | JSContactValue::BlobId(_)
            | JSContactValue::Id(_)
            | JSContactValue::IdReference(_) => Err(Value::Element(e)),
        },
        Value::Str(s) => {
            match value_type {
                ValueType::Kind => {
                    if let Some(kind) = VCardKind::parse(s.as_ref().as_bytes()) {
                        return Ok(VCardValue::Kind(kind));
                    }
                }
                ValueType::Sex => {
                    if let Some(sex) = VCardSex::parse(s.as_ref().as_bytes()) {
                        return Ok(VCardValue::Sex(sex));
                    }
                }
                ValueType::GramGender => {
                    if let Some(gender) = VCardGramGender::parse(s.as_ref().as_bytes()) {
                        return Ok(VCardValue::GramGender(gender));
                    }
                }
                ValueType::Vcard(typ) => match typ {
                    VCardValueType::Boolean => {
                        if s.eq_ignore_ascii_case("true") {
                            return Ok(VCardValue::Boolean(true));
                        } else if s.eq_ignore_ascii_case("false") {
                            return Ok(VCardValue::Boolean(false));
                        }
                    }
                    VCardValueType::Date => {
                        let mut dt = PartialDateTime::default();
                        dt.parse_vcard_date(&mut s.as_ref().as_bytes().iter().peekable());
                        if !dt.is_null() {
                            return Ok(VCardValue::PartialDateTime(dt));
                        }
                    }
                    VCardValueType::DateAndOrTime => {
                        let mut dt = PartialDateTime::default();
                        dt.parse_vcard_date_and_or_time(
                            &mut s.as_ref().as_bytes().iter().peekable(),
                        );
                        if !dt.is_null() {
                            return Ok(VCardValue::PartialDateTime(dt));
                        }
                    }
                    VCardValueType::DateTime => {
                        let mut dt = PartialDateTime::default();
                        dt.parse_vcard_date_time(&mut s.as_ref().as_bytes().iter().peekable());
                        if !dt.is_null() {
                            return Ok(VCardValue::PartialDateTime(dt));
                        }
                    }
                    VCardValueType::Time => {
                        let mut dt = PartialDateTime::default();
                        dt.parse_vcard_time(&mut s.as_ref().as_bytes().iter().peekable(), false);
                        if !dt.is_null() {
                            return Ok(VCardValue::PartialDateTime(dt));
                        }
                    }
                    VCardValueType::Timestamp => {
                        let mut dt = PartialDateTime::default();
                        if dt.parse_timestamp(&mut s.as_ref().as_bytes().iter().peekable(), true) {
                            return Ok(VCardValue::PartialDateTime(dt));
                        }
                    }
                    VCardValueType::UtcOffset => {
                        let mut dt = PartialDateTime::default();
                        dt.parse_zone(&mut s.as_ref().as_bytes().iter().peekable());
                        if !dt.is_null() {
                            return Ok(VCardValue::PartialDateTime(dt));
                        }
                    }
                    VCardValueType::Float => {
                        if let Ok(float) = s.as_ref().parse::<f64>()
                            && float.is_finite()
                        {
                            return Ok(VCardValue::Float(float));
                        }
                    }
                    VCardValueType::Integer => {
                        if let Some(integer) = Integer::parse(s.as_ref().as_bytes()) {
                            return Ok(VCardValue::Integer(integer.0));
                        }
                    }
                    VCardValueType::Uri => {
                        return Ok(match Uri::parse(s) {
                            Uri::Data(data) => VCardValue::Binary(data),
                            Uri::Location(text) => VCardValue::Text(text),
                        });
                    }
                    VCardValueType::LanguageTag | VCardValueType::Text => (),
                },
            }

            Ok(VCardValue::Text(s.into_owned()))
        }
        Value::Bool(b) => Ok(VCardValue::Boolean(b)),
        Value::Number(n) => match n.as_i64() {
            Some(integer) => Ok(VCardValue::Integer(integer)),
            None => n
                .as_f64()
                .filter(|float| n.is_f64() && float.is_finite())
                .map(VCardValue::Float)
                .ok_or(Value::Number(n)),
        },
        value => Err(value),
    }
}

pub(super) trait U32Value {
    fn as_u32(&self) -> Option<u32>;
    fn as_pref(&self) -> Option<u32>;
    fn as_index(&self) -> Option<u32>;
}

impl<I, B> U32Value for Value<'_, JSContactProperty<I>, JSContactValue<I, B>>
where
    I: JSContactId,
    B: JSContactId,
{
    fn as_u32(&self) -> Option<u32> {
        self.as_u64().and_then(|value| u32::try_from(value).ok())
    }

    fn as_pref(&self) -> Option<u32> {
        self.as_u32().filter(|pref| (1..=100).contains(pref))
    }

    fn as_index(&self) -> Option<u32> {
        self.as_u32().filter(|index| *index >= 1)
    }
}

pub(super) fn map_kind<T, I, B>(
    value: &Value<'_, JSContactProperty<I>, JSContactValue<I, B>>,
    types: impl IntoIterator<Item = (JSContactKind, T)>,
) -> Option<T>
where
    I: JSContactId,
    B: JSContactId,
{
    value
        .as_object()
        .and_then(|obj| obj.get(&Key::Property(JSContactProperty::Kind)))
        .and_then(|v| match v {
            Value::Element(JSContactValue::Kind(kind)) => {
                types.into_iter().find_map(|(js_kind, vcard_property)| {
                    if js_kind == *kind {
                        Some(vcard_property)
                    } else {
                        None
                    }
                })
            }

            _ => None,
        })
}

impl<I, B> TryFrom<Value<'_, JSContactProperty<I>, JSContactValue<I, B>>>
    for IanaType<VCardPhonetic, String>
where
    I: JSContactId,
    B: JSContactId,
{
    type Error = ();
    fn try_from(
        value: Value<'_, JSContactProperty<I>, JSContactValue<I, B>>,
    ) -> Result<Self, Self::Error> {
        match value {
            Value::Element(JSContactValue::PhoneticSystem(system)) => {
                Ok(IanaType::Iana(match system {
                    JSContactPhoneticSystem::Ipa => VCardPhonetic::Ipa,
                    JSContactPhoneticSystem::Jyut => VCardPhonetic::Jyut,
                    JSContactPhoneticSystem::Piny => VCardPhonetic::Piny,
                    JSContactPhoneticSystem::Script => VCardPhonetic::Script,
                }))
            }
            Value::Str(text) => match VCardPhonetic::parse(text.as_ref().as_bytes()) {
                Some(phonetic) => Ok(IanaType::Iana(phonetic)),
                None => Ok(IanaType::Other(text.to_ascii_uppercase())),
            },
            _ => Err(()),
        }
    }
}

impl<I, B> TryFrom<Value<'_, JSContactProperty<I>, JSContactValue<I, B>>>
    for IanaType<VCardLevel, String>
where
    I: JSContactId,
    B: JSContactId,
{
    type Error = ();

    fn try_from(
        value: Value<'_, JSContactProperty<I>, JSContactValue<I, B>>,
    ) -> Result<Self, Self::Error> {
        match value {
            Value::Element(JSContactValue::Level(level)) => Ok(IanaType::Iana(match level {
                JSContactLevel::High => VCardLevel::High,
                JSContactLevel::Low => VCardLevel::Low,
                JSContactLevel::Medium => VCardLevel::Medium,
            })),
            Value::Str(text) => match VCardLevel::parse(text.as_ref().as_bytes()) {
                Some(level) => Ok(IanaType::Iana(level)),
                None => Ok(IanaType::Other(text.to_ascii_uppercase())),
            },
            _ => Err(()),
        }
    }
}

pub(super) fn find_text_param<'x, I, B>(
    value: &'x Value<'x, JSContactProperty<I>, JSContactValue<I, B>>,
    name: &str,
) -> Option<Cow<'x, str>>
where
    I: JSContactId,
    B: JSContactId,
{
    value
        .as_object()
        .and_then(|obj| obj.get(&Key::Property(JSContactProperty::Parameters)))
        .and_then(|obj| obj.as_object())
        .and_then(|obj| obj.get_ignore_case(name))
        .and_then(|obj| obj.as_str())
}

#[cfg(test)]
mod tests {
    use super::{U32Value, convert_anniversary, convert_value};
    use crate::{
        jscontact::{JSContactProperty, JSContactValue},
        vcard::{VCardValue, VCardValueType, ValueType},
    };
    use jmap_tools::{Key, Map, Value};

    type JSValue = Value<'static, JSContactProperty<String>, JSContactValue<String, String>>;

    #[test]
    fn convert_value_numbers_do_not_wrap() {
        for (value, expected) in [
            (
                JSValue::Number((-5i64).into()),
                Some(VCardValue::Integer(-5)),
            ),
            (
                JSValue::Number(i64::MAX.into()),
                Some(VCardValue::Integer(i64::MAX)),
            ),
            (JSValue::Number(u64::MAX.into()), None),
            (JSValue::Number(1.5f64.into()), Some(VCardValue::Float(1.5))),
            (JSValue::Number(f64::NAN.into()), None),
            (JSValue::Number(f64::NEG_INFINITY.into()), None),
        ] {
            assert_eq!(
                convert_value(value.clone(), &ValueType::Vcard(VCardValueType::Integer)).ok(),
                expected,
                "{value:?}"
            );
        }

        for text in ["NaN", "inf", "-infinity"] {
            assert_eq!(
                convert_value(
                    JSValue::Str(text.into()),
                    &ValueType::Vcard(VCardValueType::Float)
                )
                .ok(),
                Some(VCardValue::Text(text.to_string())),
                "RFC 6350 Section 4.6: {text} is not a float value"
            );
        }
    }

    #[test]
    fn u32_values_do_not_wrap() {
        for (value, expected) in [
            (JSValue::Number(1u64.into()), Some(1)),
            (JSValue::Number(u32::MAX.into()), Some(u32::MAX)),
            (JSValue::Number((-1i64).into()), None),
            (JSValue::Number((u64::from(u32::MAX) + 2).into()), None),
            (JSValue::Number(1.0f64.into()), None),
        ] {
            assert_eq!(value.as_u32(), expected, "{value:?}");
        }
    }

    #[test]
    fn convert_anniversary_numbers_do_not_wrap() {
        let date = |parts: &[(JSContactProperty<String>, JSValue)]| {
            JSValue::Object(Map::from(
                parts
                    .iter()
                    .map(|(key, value)| (Key::Property(key.clone()), value.clone()))
                    .collect::<Vec<_>>(),
            ))
        };

        for parts in [
            [
                (JSContactProperty::Month, JSValue::Number(2u64.into())),
                (JSContactProperty::Day, JSValue::Number(257u64.into())),
            ],
            [
                (JSContactProperty::Year, JSValue::Number(2000u64.into())),
                (JSContactProperty::Month, JSValue::Number((-1i64).into())),
            ],
            [
                (JSContactProperty::Year, JSValue::Number(70_000u64.into())),
                (JSContactProperty::Month, JSValue::Number(1u64.into())),
            ],
            [
                (JSContactProperty::Month, JSValue::Number(2u64.into())),
                (JSContactProperty::Day, JSValue::Number(40u64.into())),
            ],
            [
                (JSContactProperty::Year, JSValue::Number(2000u64.into())),
                (JSContactProperty::Month, JSValue::Number(13u64.into())),
            ],
            [
                (JSContactProperty::Year, JSValue::Number(2000u64.into())),
                (JSContactProperty::Month, JSValue::Number(0u64.into())),
            ],
            [
                (JSContactProperty::Month, JSValue::Number(2u64.into())),
                (JSContactProperty::Day, JSValue::Number(0u64.into())),
            ],
            [
                (JSContactProperty::Year, JSValue::Number(12_345u64.into())),
                (JSContactProperty::Month, JSValue::Number(1u64.into())),
            ],
        ] {
            assert!(
                convert_anniversary(date(&parts)).is_err(),
                "RFC 9553 Section 2.8.1: {parts:?}"
            );
        }

        let (converted, _) = convert_anniversary(date(&[
            (JSContactProperty::Year, JSValue::Number(1953u64.into())),
            (JSContactProperty::Month, JSValue::Number(4u64.into())),
            (JSContactProperty::Day, JSValue::Number(15u64.into())),
        ]))
        .expect("valid partial date");
        assert_eq!(
            (converted.year, converted.month, converted.day),
            (Some(1953), Some(4), Some(15))
        );
    }
}
