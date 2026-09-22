/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::jscalendar::{
    JSCAL_NAMESPACE, JSCalendarId, JSCalendarProperty, JSCalendarType, JSCalendarValue,
};
use jmap_tools::{JsonPointer, JsonPointerItem, Key, Map, Value};
use std::str::FromStr;
use uuid::fmt::Hyphenated;

pub(crate) trait JSCalendarKeyExt<I: JSCalendarId> {
    fn is_forbidden_override_key(&self) -> bool;
    fn is_ignored_override_key(&self) -> bool;
    fn is_series_converted_property(&self) -> bool;
    fn is_uuid5_of(&self, value: &[u8]) -> bool;
    fn matches_pointer_item(&self, item: &JsonPointerItem<JSCalendarProperty<I>>) -> bool;
    fn pointer_index(&self) -> Option<u64>;
}

pub(crate) trait JSCalendarMapExt {
    fn is_type_or_untyped(&self, types: &[JSCalendarType]) -> bool;
}

pub(crate) trait JSCalendarValueExt<I: JSCalendarId>: Sized {
    fn apply_jsprop(&mut self, pointer: &[JsonPointerItem<JSCalendarProperty<I>>], patch: Self);
}

pub trait JSCalendarPatch {
    fn is_instance_patch(&self) -> bool;
}

impl<I: JSCalendarId> JSCalendarKeyExt<I> for Key<'_, JSCalendarProperty<I>> {
    fn is_forbidden_override_key(&self) -> bool {
        match self {
            Key::Property(JSCalendarProperty::Pointer(pointer)) => {
                JSCalendarProperty::is_forbidden_override_pointer(pointer)
            }
            Key::Property(property) => property.is_forbidden_override_patch(),
            _ => false,
        }
    }

    fn is_ignored_override_key(&self) -> bool {
        matches!(
            self,
            Key::Property(property) if property.is_forbidden_override_patch()
                || matches!(property, JSCalendarProperty::Excluded)
        )
    }

    fn is_series_converted_property(&self) -> bool {
        let first = match self {
            Key::Property(JSCalendarProperty::Pointer(ptr)) => {
                return matches!(
                    ptr.iter()
                        .find(|item| !matches!(item, JsonPointerItem::Root)),
                    Some(JsonPointerItem::Key(Key::Property(
                        JSCalendarProperty::RecurrenceOverrides
                            | JSCalendarProperty::RecurrenceRule
                    )))
                );
            }
            Key::Property(property) => {
                return matches!(
                    property,
                    JSCalendarProperty::RecurrenceOverrides | JSCalendarProperty::RecurrenceRule
                );
            }
            Key::Borrowed(key) => key.split('/').next(),
            Key::Owned(key) => key.split('/').next(),
        };

        matches!(
            first.and_then(|name| JSCalendarProperty::<I>::from_str(name).ok()),
            Some(JSCalendarProperty::RecurrenceOverrides | JSCalendarProperty::RecurrenceRule)
        )
    }

    fn is_uuid5_of(&self, value: &[u8]) -> bool {
        let name = self.to_string();
        name.len() == Hyphenated::LENGTH && {
            let mut buffer = [0u8; Hyphenated::LENGTH];
            uuid::Uuid::new_v5(&JSCAL_NAMESPACE, value)
                .hyphenated()
                .encode_lower(&mut buffer)
                == name.as_ref()
        }
    }

    fn matches_pointer_item(&self, item: &JsonPointerItem<JSCalendarProperty<I>>) -> bool {
        match item {
            JsonPointerItem::Key(item) => item == self,
            JsonPointerItem::Number(number) => self.pointer_index() == Some(*number),
            JsonPointerItem::Root | JsonPointerItem::Wildcard | JsonPointerItem::Invalid(_) => {
                false
            }
        }
    }

    fn pointer_index(&self) -> Option<u64> {
        let name = self.as_string_key()?;
        (!name.is_empty()
            && name.bytes().all(|byte| byte.is_ascii_digit())
            && (name == "0" || !name.starts_with('0')))
        .then(|| name.parse().ok())
        .flatten()
    }
}

impl<I: JSCalendarId, B: JSCalendarId> JSCalendarMapExt
    for Map<'_, JSCalendarProperty<I>, JSCalendarValue<I, B>>
{
    fn is_type_or_untyped(&self, types: &[JSCalendarType]) -> bool {
        match self.get(&Key::Property(JSCalendarProperty::Type)) {
            None => true,
            Some(Value::Element(JSCalendarValue::Type(typ))) => types.contains(typ),
            Some(_) => false,
        }
    }
}

impl<I: JSCalendarId, B: JSCalendarId> JSCalendarPatch
    for Value<'_, JSCalendarProperty<I>, JSCalendarValue<I, B>>
{
    fn is_instance_patch(&self) -> bool {
        self.as_object().is_some_and(|patch| {
            let excluded = Key::Property(JSCalendarProperty::Excluded);
            !patch.contains_key_value(&excluded, &Value::Bool(true))
                && patch
                    .keys()
                    .any(|key| key != &excluded && !key.is_forbidden_override_key())
        })
    }
}

impl<I: JSCalendarId, B: JSCalendarId> JSCalendarValueExt<I>
    for Value<'_, JSCalendarProperty<I>, JSCalendarValue<I, B>>
{
    fn apply_jsprop(&mut self, pointer: &[JsonPointerItem<JSCalendarProperty<I>>], patch: Self) {
        let (Some((last, parents)), false) = (pointer.split_last(), patch.is_null()) else {
            return;
        };
        let mut current = self;
        for item in parents {
            let Value::Object(obj) = current else {
                return;
            };
            let Some(next) = obj
                .iter_mut()
                .find_map(|(key, value)| key.matches_pointer_item(item).then_some(value))
            else {
                return;
            };
            current = next;
        }
        let Value::Object(obj) = current else {
            return;
        };
        if obj.iter().any(|(key, _)| key.matches_pointer_item(last)) {
            return;
        }
        let key = match last {
            JsonPointerItem::Key(key) => key.clone(),
            JsonPointerItem::Number(number) => Key::Owned(number.to_string()),
            JsonPointerItem::Root | JsonPointerItem::Wildcard | JsonPointerItem::Invalid(_) => {
                return;
            }
        };
        obj.insert_unchecked(key, patch);
    }
}

impl<I: JSCalendarId> JSCalendarProperty<I> {
    pub(crate) fn is_series_property(&self) -> bool {
        matches!(
            self,
            JSCalendarProperty::RecurrenceRule
                | JSCalendarProperty::RecurrenceOverrides
                | JSCalendarProperty::RecurrenceId
                | JSCalendarProperty::RecurrenceIdTimeZone
                | JSCalendarProperty::Excluded
        )
    }

    pub(crate) fn is_member_map(&self) -> bool {
        matches!(
            self,
            JSCalendarProperty::Participants
                | JSCalendarProperty::Locations
                | JSCalendarProperty::VirtualLocations
                | JSCalendarProperty::Alerts
                | JSCalendarProperty::Links
                | JSCalendarProperty::RelatedTo
        )
    }

    pub(crate) fn member_pointer<const N: usize>(
        &self,
        keys: [Key<'static, JSCalendarProperty<I>>; N],
    ) -> Key<'static, JSCalendarProperty<I>> {
        let mut items = Vec::with_capacity(N + 1);
        items.push(JsonPointerItem::Key(Key::Property(self.clone())));
        items.extend(keys.into_iter().map(|key| match key.pointer_index() {
            Some(index) => JsonPointerItem::Number(index),
            None => JsonPointerItem::Key(key),
        }));
        Key::Property(JSCalendarProperty::Pointer(JsonPointer::new(items)))
    }
}
