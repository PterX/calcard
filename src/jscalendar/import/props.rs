/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    common::{
        Data, IanaString, IanaType, LinkRelation,
        blob::GeneratedBlobId,
        timezone::{Tz, TzTimestamp},
    },
    icalendar::{
        ICalendar, ICalendarEntry, ICalendarParameterName, ICalendarParameterValue,
        ICalendarProperty, ICalendarValue, ICalendarValueType, Uri,
    },
    jscalendar::{
        JSCalendarDateTime, JSCalendarId, JSCalendarPrivacy, JSCalendarProperty, JSCalendarType,
        JSCalendarValue,
        ext::{JSCalendarKeyExt, JSCalendarValueExt},
        import::{
            EntryState, ICalendarConvertedProperty, ICalendarParams, InstanceTimeZone, LinkId,
            LinkIds, State, params::ExtractParams,
        },
        overrides::OverrideDiff,
        uuid5,
    },
};
use chrono::DateTime;
use jmap_tools::{JsonPointer, JsonPointerItem, Key, Map, Property, Value};
use std::{borrow::Cow, collections::hash_map::Entry};

impl<I: JSCalendarId, B: JSCalendarId> State<I, B> {
    pub(super) fn map_named_entry(
        &mut self,
        entry: &mut EntryState,
        extract: &[ICalendarParameterName],
        top_property_name: JSCalendarProperty<I>,
        values: impl IntoIterator<
            Item = (
                Key<'static, JSCalendarProperty<I>>,
                Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
            ),
        >,
    ) {
        self.map_named_entry_with_id(entry, extract, top_property_name, values, None)
    }

    pub(super) fn map_named_entry_with_id(
        &mut self,
        entry: &mut EntryState,
        extract: &[ICalendarParameterName],
        top_property_name: JSCalendarProperty<I>,
        values: impl IntoIterator<
            Item = (
                Key<'static, JSCalendarProperty<I>>,
                Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
            ),
        >,
        default_id: Option<String>,
    ) {
        // Obtain main property and value
        let mut values = values.into_iter().peekable();
        let (property, value) = match values.peek() {
            Some((property, Value::Str(s))) => (property, s.as_ref()),
            Some((property, _)) => (property, "unknown"),
            _ => {
                panic!("Cannot generate jsid without a value");
            }
        };

        // Obtain or calculate JSID
        let mut parameters = Map::from(Vec::new());
        let is_link = top_property_name == JSCalendarProperty::Links;
        let js_id = match parameters.extract_params(&mut entry.entry, extract) {
            Some(js_id) => js_id,
            None => {
                let js_id = default_id.unwrap_or_else(|| uuid5(value));
                if is_link {
                    self.link_ids.unique(js_id)
                } else {
                    js_id
                }
            }
        };

        // Set converted props
        entry.set_converted_to::<I>(&[
            top_property_name.to_cow().as_ref(),
            js_id.as_str(),
            property.to_string().as_ref(),
        ]);

        let Some(objects) = self
            .entries
            .entry(Key::Property(top_property_name))
            .or_insert_with(Value::new_object)
            .as_object_mut()
        else {
            return;
        };
        let Some(obj) = (if is_link {
            self.link_ids.entry(objects, js_id)
        } else {
            Some(objects.insert_or_get_mut(Key::Owned(js_id), Value::new_object()))
        })
        .and_then(Value::as_object_mut) else {
            return;
        };

        for (key, value) in values.chain(parameters.into_vec()) {
            if let Some(current_value) = obj.get_mut(&key) {
                match (value, current_value) {
                    (Value::Object(new_obj), Value::Object(existing_obj)) => {
                        for (key, value) in new_obj.into_vec() {
                            existing_obj.insert(key, value);
                        }
                    }
                    (value, current_value) => {
                        *current_value = value;
                    }
                }
            } else {
                obj.insert_unchecked(key, value);
            }
        }
    }

    pub(super) fn map_blob_link(
        &mut self,
        entry: &mut EntryState,
        generated: GeneratedBlobId<B>,
        content_type: Option<String>,
        default_id: Option<String>,
        value_type: ICalendarValueType,
    ) {
        let (rel, extract): (_, &[ICalendarParameterName]) = match (&entry.entry.name, value_type) {
            (ICalendarProperty::Image, ICalendarValueType::Binary) => (
                LinkRelation::Icon,
                &[
                    ICalendarParameterName::Display,
                    ICalendarParameterName::Fmttype,
                    ICalendarParameterName::Filename,
                    ICalendarParameterName::Linkrel,
                    ICalendarParameterName::Jsid,
                ],
            ),
            (ICalendarProperty::Image, _) => (
                LinkRelation::Icon,
                &[
                    ICalendarParameterName::Display,
                    ICalendarParameterName::Fmttype,
                    ICalendarParameterName::Filename,
                    ICalendarParameterName::Jsid,
                ],
            ),
            (_, ICalendarValueType::Binary) => (
                LinkRelation::Enclosure,
                &[
                    ICalendarParameterName::Fmttype,
                    ICalendarParameterName::Filename,
                    ICalendarParameterName::Linkrel,
                    ICalendarParameterName::Jsid,
                ],
            ),
            _ => (
                LinkRelation::Enclosure,
                &[
                    ICalendarParameterName::Fmttype,
                    ICalendarParameterName::Filename,
                    ICalendarParameterName::Jsid,
                ],
            ),
        };
        entry.entry.params.retain(|param| {
            !matches!(
                param.name,
                ICalendarParameterName::Value | ICalendarParameterName::Size
            )
        });

        self.map_named_entry_with_id(
            entry,
            extract,
            JSCalendarProperty::Links,
            [
                Some((
                    Key::Property(JSCalendarProperty::BlobId),
                    Value::Element(JSCalendarValue::BlobId(generated.blob_id)),
                )),
                Some((
                    Key::Property(JSCalendarProperty::Type),
                    Value::Element(JSCalendarValue::Type(JSCalendarType::Link)),
                )),
                Some((
                    Key::Property(JSCalendarProperty::Rel),
                    Value::Element(JSCalendarValue::LinkRelation(rel)),
                )),
                Some((
                    Key::Property(JSCalendarProperty::Size),
                    Value::Number((generated.size as u64).into()),
                )),
                content_type.map(|content_type| {
                    (
                        Key::Property(JSCalendarProperty::ContentType),
                        Value::Str(content_type.into()),
                    )
                }),
            ]
            .into_iter()
            .flatten(),
            default_id,
        );
        entry.set_map_name();
    }

    pub(super) fn insert_recurrence_override(
        &mut self,
        key: Key<'static, JSCalendarProperty<I>>,
        value: Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    ) {
        self.entries
            .entry(Key::Property(JSCalendarProperty::RecurrenceOverrides))
            .or_insert_with(Value::new_object)
            .as_object_mut()
            .unwrap()
            .insert(key, value);
    }

    pub(super) fn add_period_conversion_prop(&mut self, converted_to: String) {
        if self.include_ical_components {
            let mut params = ICalendarParams::default();
            params.push(
                ICalendarParameterName::Value,
                Value::Str(ICalendarValueType::Period.as_str().into()),
            );
            self.ical_converted_properties
                .entry(converted_to)
                .or_insert(ICalendarConvertedProperty {
                    name: Some(ICalendarProperty::Rdate),
                    params,
                });
        }
    }

    pub(super) fn add_conversion_props(&mut self, mut entry: EntryState) {
        if self.include_ical_components {
            if let Some(converted_to) = entry.converted_to.take() {
                if entry.map_name || !entry.entry.params.is_empty() {
                    let mut value_type = None;

                    match self.ical_converted_properties.entry(converted_to) {
                        Entry::Occupied(mut conv_prop) => {
                            entry.jcal_parameters(&mut conv_prop.get_mut().params, &mut value_type);
                        }
                        Entry::Vacant(conv_prop) => {
                            let mut params = ICalendarParams::default();
                            entry.jcal_parameters(&mut params, &mut value_type);
                            if let Some(value_type) = value_type {
                                params.push(
                                    ICalendarParameterName::Value,
                                    Value::Str(value_type.into_string()),
                                );
                            }
                            if !params.is_empty() || entry.map_name {
                                conv_prop.insert(ICalendarConvertedProperty {
                                    name: if entry.map_name {
                                        Some(entry.entry.name)
                                    } else {
                                        None
                                    },
                                    params,
                                });
                            }
                        }
                    }
                }
            } else {
                self.ical_properties.push(entry.into_jcal());
            }
        }
    }

    pub(super) fn into_object(
        mut self,
    ) -> Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>> {
        let mut ical_obj = Map::from(Vec::new());
        if !self.ical_converted_properties.is_empty() {
            let mut converted_properties =
                Map::from(Vec::with_capacity(self.ical_converted_properties.len()));

            for (converted_to, props) in self.ical_converted_properties {
                let mut obj = Map::from(Vec::with_capacity(2));
                if let Some(params) = props.params.into_jscalendar_value() {
                    obj.insert(
                        Key::Property(JSCalendarProperty::Parameters),
                        Value::Object(params),
                    );
                }
                if let Some(name) = props.name {
                    obj.insert(
                        Key::Property(JSCalendarProperty::Name),
                        Value::Str(name.into_string().to_ascii_lowercase().into()),
                    );
                }

                converted_properties.insert_unchecked(Key::Owned(converted_to), Value::Object(obj));
            }

            ical_obj.insert_unchecked(
                Key::Property(JSCalendarProperty::ConvertedProperties),
                Value::Object(converted_properties),
            );
        }

        if !self.ical_properties.is_empty() {
            ical_obj.insert_unchecked(
                Key::Property(JSCalendarProperty::Properties),
                Value::Array(self.ical_properties),
            );
        }

        if let Some(components) = self.ical_components {
            ical_obj.insert_unchecked(Key::Property(JSCalendarProperty::Components), components);
        }

        if !ical_obj.is_empty() || self.map_component {
            ical_obj.insert_unchecked(
                Key::Property(JSCalendarProperty::Name),
                Value::Str(self.component_type.as_str().to_ascii_lowercase().into()),
            );
            self.entries.insert(
                Key::Property(JSCalendarProperty::ICalendar),
                Value::Object(ical_obj),
            );
        }

        if self.has_dates {
            /*
             If a date-time value does not have a timezone, then the timezone is not set
             in JSCalendar.
            */

            if (!self.is_recurrence_instance || self.time_zone == InstanceTimeZone::Keep)
                && let Some(tz) = self.tz_start.and_then(|tz| tz.name())
            {
                self.entries
                    .insert(Key::Property(JSCalendarProperty::TimeZone), Value::Str(tz));
            }

            if self.tz_end.is_some()
                && self.tz_start.is_some()
                && self.tz_end != self.tz_start
                && let Some(tz) = self.tz_end.and_then(|tz| tz.name())
            {
                self.entries.insert(
                    Key::Property(JSCalendarProperty::EndTimeZone),
                    Value::Str(tz),
                );
            }

            if let Some(recurrence_id) = self.recurrence_id {
                self.entries.insert(
                    Key::Property(JSCalendarProperty::RecurrenceId),
                    Value::Element(JSCalendarValue::DateTime(JSCalendarDateTime::new(
                        recurrence_id.to_naive_timestamp(),
                        true,
                    ))),
                );
                if !self.is_recurrence_instance
                    && let Some(tz) = recurrence_id
                        .timezone()
                        .to_resolved()
                        .and_then(|tz| tz.name())
                {
                    self.entries.insert(
                        Key::Property(JSCalendarProperty::RecurrenceIdTimeZone),
                        Value::Str(tz),
                    );
                }
            }
        }

        if let Some(uid) = self.uid {
            self.entries.insert(
                Key::Property(JSCalendarProperty::Uid),
                Value::Str(uid.into()),
            );
        }

        if !self.is_recurrence_instance
            && let Some(component_type) = self.component_type.to_jscalendar_type()
        {
            self.entries.insert(
                Key::Property(JSCalendarProperty::Type),
                Value::Element(JSCalendarValue::Type(component_type)),
            );
        }

        let mut obj = Value::Object(self.entries.into_iter().collect());
        let (override_patches, patches): (Vec<_>, Vec<_>) =
            self.patch_objects.into_iter().partition(|(pointer, _)| {
                !self.recurrence_overrides.is_empty()
                    && matches!(
                        pointer.first(),
                        Some(JsonPointerItem::Key(Key::Property(
                            JSCalendarProperty::RecurrenceOverrides
                        )))
                    )
            });
        for (pointer, patch) in patches {
            obj.apply_jsprop(pointer.as_slice(), patch);
        }

        if !self.recurrence_overrides.is_empty()
            && let Value::Object(base) = &mut obj
        {
            let overrides_key = Key::Property(JSCalendarProperty::RecurrenceOverrides);
            let mut overrides = match base
                .as_vec()
                .iter()
                .position(|(key, _)| key == &overrides_key)
            {
                Some(pos) => base.as_mut_vec().remove(pos).1,
                None => Value::new_object(),
            };
            if let Value::Object(overrides) = &mut overrides {
                let diff = OverrideDiff::new(base);
                for (recurrence_id, recurrence) in self.recurrence_overrides {
                    let instance = recurrence.into_object().into_object().unwrap_or_default();
                    overrides.insert(
                        Key::Property(JSCalendarProperty::DateTime(recurrence_id)),
                        Value::Object(diff.diff(recurrence_id, instance)),
                    );
                }
            }
            base.insert_unchecked(overrides_key, overrides);
        }

        for (pointer, patch) in override_patches {
            obj.apply_jsprop(pointer.as_slice(), patch);
        }

        obj
    }

    pub(super) fn set_map_component(&mut self) {
        self.map_component = true;
    }

    pub(super) fn set_is_recurrence_instance(&mut self, time_zone: InstanceTimeZone) {
        self.is_recurrence_instance = true;
        self.time_zone = time_zone;
    }

    pub(super) fn remove_start_at(&mut self, recurrence_id: JSCalendarDateTime, tz: Option<Tz>) {
        let start_key = Key::Property(JSCalendarProperty::Start);
        if self.tz_start == tz
            && matches!(
                self.entries.get(&start_key),
                Some(Value::Element(JSCalendarValue::DateTime(start)))
                    if start.timestamp == recurrence_id.timestamp
            )
        {
            self.entries.remove(&start_key);
        }
    }

    pub(super) fn recurrence_sequence(&self) -> Option<i64> {
        self.recurrence_id.map(|_| {
            self.entries
                .get(&Key::Property(JSCalendarProperty::Sequence))
                .and_then(Value::as_i64)
                .unwrap_or_default()
        })
    }

    pub(super) fn privacy(&self) -> Option<JSCalendarPrivacy> {
        match self
            .entries
            .get(&Key::Property(JSCalendarProperty::Privacy))?
        {
            Value::Element(JSCalendarValue::Privacy(privacy)) => Some(*privacy),
            _ => Some(JSCalendarPrivacy::Private),
        }
    }

    pub(super) fn raise_privacy(&mut self, privacy: JSCalendarPrivacy) {
        if self.privacy().unwrap_or(JSCalendarPrivacy::Public) < privacy {
            self.entries.insert(
                Key::Property(JSCalendarProperty::Privacy),
                Value::Element(JSCalendarValue::Privacy(privacy)),
            );
            self.patch_objects.retain(|(pointer, _)| {
                !matches!(
                    pointer.first(),
                    Some(JsonPointerItem::Key(Key::Property(
                        JSCalendarProperty::Privacy
                    )))
                )
            });
        }
    }

    pub(super) fn merge_privacy(&mut self, privacy: JSCalendarPrivacy) {
        let privacy = self
            .privacy()
            .map_or(privacy, |current| current.max(privacy));
        self.entries.insert(
            Key::Property(JSCalendarProperty::Privacy),
            Value::Element(JSCalendarValue::Privacy(privacy)),
        );
    }

    pub(super) fn remove_forbidden_override_patches(&mut self) {
        self.entries.retain(|key, _| {
            !matches!(key, Key::Property(property) if property.is_forbidden_override_patch())
        });
        self.patch_objects
            .retain(|(pointer, _)| !JSCalendarProperty::is_forbidden_override_pointer(pointer));
        self.ical_converted_properties.retain(|converted_to, _| {
            !Key::<JSCalendarProperty<I>>::Borrowed(converted_to).is_series_converted_property()
        });
    }

    pub(super) fn find_participant_by_address(&self, address: &str) -> Option<String> {
        self.entries
            .get(&Key::Property(JSCalendarProperty::Participants))?
            .as_object()?
            .iter()
            .find_map(|(key, value)| {
                match value
                    .as_object()?
                    .get(&Key::Property(JSCalendarProperty::CalendarAddress))?
                {
                    Value::Str(value) if value == address => Some(key.to_string().into_owned()),
                    _ => None,
                }
            })
    }

    #[inline]
    pub(super) fn get_mut_object_or_insert(
        &mut self,
        key: JSCalendarProperty<I>,
    ) -> &mut Map<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>> {
        self.entries
            .entry(Key::Property(key))
            .or_insert_with(|| Value::Object(Map::from(Vec::new())))
            .as_object_mut()
            .unwrap()
    }
}

impl JSCalendarDateTime {
    pub(super) fn local_in(dt: DateTime<Tz>, tz: Option<Tz>) -> Self {
        JSCalendarDateTime::new(
            if dt.timezone().is_floating() || tz == Some(dt.timezone()) {
                dt.to_naive_timestamp()
            } else {
                dt.with_timezone(&tz.unwrap_or_default())
                    .to_naive_timestamp()
            },
            true,
        )
    }
}

impl EntryState {
    pub(super) fn new(entry: ICalendarEntry) -> Self {
        Self {
            entry,
            converted_to: None,
            map_name: false,
        }
    }

    pub(super) fn set_converted_to<I: JSCalendarId>(&mut self, converted_to: &[&str]) {
        self.converted_to = Some(JsonPointer::<JSCalendarProperty<I>>::encode(converted_to));
    }

    pub(super) fn set_map_name(&mut self) {
        self.map_name = true;
    }

    pub(super) fn into_jcal<I: JSCalendarId, B: JSCalendarId>(
        mut self,
    ) -> Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>> {
        let mut value_type = None;
        let mut params = ICalendarParams::default();

        self.jcal_parameters(&mut params, &mut value_type);

        let values = if self.entry.values.len() == 1 {
            self.entry
                .values
                .into_iter()
                .next()
                .unwrap()
                .into_jscalendar_value(value_type.as_ref())
        } else {
            let mut values = Vec::with_capacity(self.entry.values.len());
            for value in self.entry.values {
                values.push(value.into_jscalendar_value(value_type.as_ref()));
            }
            Value::Array(values)
        };
        Value::Array(vec![
            Value::Str(self.entry.name.as_str().to_ascii_lowercase().into()),
            Value::Object(
                params
                    .into_jscalendar_value()
                    .unwrap_or(Map::from(Vec::new())),
            ),
            Value::Str(
                value_type
                    .map(|v| v.into_string())
                    .unwrap_or(Cow::Borrowed("unknown")),
            ),
            values,
        ])
    }

    pub(super) fn jcal_parameters<I: JSCalendarId, B: JSCalendarId>(
        &mut self,
        params: &mut ICalendarParams<I, B>,
        value_type: &mut Option<IanaType<ICalendarValueType, String>>,
    ) {
        if self.entry.params.is_empty() {
            return;
        }
        let (default_type, _) = self.entry.name.default_types();
        let default_type = default_type.unwrap_ical();

        for param in std::mem::take(&mut self.entry.params) {
            if matches!(param.name, ICalendarParameterName::Value) {
                if let Some(v) = param
                    .value
                    .into_value_type()
                    .filter(|v| !v.is_iana_and(|v| v == &default_type))
                {
                    *value_type = Some(v);
                }
            } else if let (ICalendarParameterName::Range, ICalendarParameterValue::Bool(true)) =
                (&param.name, &param.value)
            {
                params.push(param.name, Value::Str("THISANDFUTURE".into()));
            } else if let Some(value) = param.value.into_text() {
                params.push(param.name, Value::Str(value));
            }
        }
    }
}

impl LinkIds {
    fn unique(&mut self, base: String) -> String {
        let Some(mut suffix) = self.0.get(&base).map(|link| link.next_suffix) else {
            return base;
        };
        let unique = loop {
            let candidate = format!("{base}-{suffix}");
            suffix = suffix.saturating_add(1);
            if !self.0.contains_key(&candidate) {
                break candidate;
            }
        };
        if let Some(link) = self.0.get_mut(&base) {
            link.next_suffix = suffix;
        }
        unique
    }

    fn entry<'x, I: JSCalendarId, B: JSCalendarId>(
        &mut self,
        links: &'x mut Map<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
        key: String,
    ) -> Option<&'x mut Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>> {
        let position = match self.0.get(&key) {
            Some(link) => link.position,
            None => {
                let position = links.len();
                links.insert_unchecked(Key::Owned(key.clone()), Value::new_object());
                self.0.insert(
                    key,
                    LinkId {
                        position,
                        next_suffix: 2,
                    },
                );
                position
            }
        };
        links.as_mut_vec().get_mut(position).map(|(_, value)| value)
    }
}

impl ICalendar {
    pub(super) fn binary_link_sizes(&self) -> impl Iterator<Item = usize> + '_ {
        self.blob_binaries().map(<[u8]>::len)
    }
}

impl ICalendarEntry {
    pub fn participant_id(&self) -> Option<Cow<'_, str>> {
        if let Some(jsid) = self.jsid() {
            return Some(Cow::Borrowed(jsid));
        }
        match self.values.first()? {
            ICalendarValue::Text(address) => Some(Cow::Owned(uuid5(address))),
            ICalendarValue::Uri(Uri::Location(address)) => Some(Cow::Owned(uuid5(address))),
            ICalendarValue::Uri(uri) => Some(Cow::Owned(uuid5(uri.to_unwrapped_string()))),
            _ => None,
        }
    }
}

pub(super) struct ICalendarBinary {
    pub(super) data: Vec<u8>,
    pub(super) content_type: Option<String>,
    pub(super) is_data_uri: bool,
}

impl ICalendarBinary {
    pub(super) fn into_value(self) -> ICalendarValue {
        if self.is_data_uri {
            ICalendarValue::Uri(Uri::Data(Data {
                content_type: self.content_type,
                data: self.data,
            }))
        } else {
            ICalendarValue::Binary(self.data)
        }
    }
}

impl ICalendarValue {
    pub(super) fn is_binary(&self) -> bool {
        self.binary_bytes().is_some()
    }

    pub(super) fn into_binary(self) -> Option<ICalendarBinary> {
        match self {
            ICalendarValue::Binary(data) => Some(ICalendarBinary {
                data,
                content_type: None,
                is_data_uri: false,
            }),
            ICalendarValue::Uri(Uri::Data(data)) => Some(ICalendarBinary {
                data: data.data,
                content_type: data.content_type,
                is_data_uri: true,
            }),
            _ => None,
        }
    }

    pub(super) fn uri_to_string(self, media_type: Option<String>) -> Self {
        match self {
            ICalendarValue::Uri(uri) => ICalendarValue::Text(uri.into_unwrapped_string()),
            ICalendarValue::Binary(data) => ICalendarValue::Text(
                Uri::Data(Data {
                    content_type: media_type,
                    data,
                })
                .into_unwrapped_string(),
            ),
            other => other,
        }
    }
}
