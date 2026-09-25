/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    common::{Data, IanaString, IanaType, jsprop::text::IntoAsciiLowercase},
    icalendar::{
        ICalendarDisplayType, ICalendarEntry, ICalendarFeatureType, ICalendarParameter,
        ICalendarParameterName, ICalendarParameterValue, ICalendarParticipationRole,
        ICalendarParticipationStatus, ICalendarProperty, ICalendarRelated, ICalendarUserTypes,
        ICalendarValue, ICalendarValueType, Uri,
    },
    jscalendar::{
        JSCalendarId, JSCalendarLinkDisplay, JSCalendarParticipantKind, JSCalendarParticipantRole,
        JSCalendarParticipationStatus, JSCalendarProperty, JSCalendarRelativeTo, JSCalendarValue,
        JSCalendarVirtualLocationFeature,
        ext::JSCalendarObjectExt,
        import::{ICalendarParams, ParamValues, PropertyMap},
    },
};
use jmap_tools::{Key, Map, Value};
use std::{borrow::Cow, mem};

pub(super) trait ExtractParams<I: JSCalendarId, B: JSCalendarId> {
    fn extract_params(
        &mut self,
        entry: &mut ICalendarEntry,
        extract: &[ICalendarParameterName],
    ) -> Option<String>;
}

pub(super) trait ParamTarget<I: JSCalendarId, B: JSCalendarId> {
    fn set(
        &mut self,
        property: JSCalendarProperty<I>,
        value: Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    );

    fn set_flag(
        &mut self,
        property: JSCalendarProperty<I>,
        flag: Key<'static, JSCalendarProperty<I>>,
    );
}

impl<I: JSCalendarId, B: JSCalendarId> ParamTarget<I, B> for PropertyMap<I, B> {
    fn set(
        &mut self,
        property: JSCalendarProperty<I>,
        value: Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    ) {
        self.insert(Key::Property(property), value);
    }

    fn set_flag(
        &mut self,
        property: JSCalendarProperty<I>,
        flag: Key<'static, JSCalendarProperty<I>>,
    ) {
        if let Some(flags) = self
            .entry(Key::Property(property))
            .or_insert_with(Value::new_object)
            .as_object_mut()
        {
            flags.upsert(flag, Value::Bool(true));
        }
    }
}

impl<I: JSCalendarId, B: JSCalendarId> ParamTarget<I, B>
    for Map<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>
{
    fn set(
        &mut self,
        property: JSCalendarProperty<I>,
        value: Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    ) {
        self.upsert(Key::Property(property), value);
    }

    fn set_flag(
        &mut self,
        property: JSCalendarProperty<I>,
        flag: Key<'static, JSCalendarProperty<I>>,
    ) {
        if let Some(flags) = self
            .upsert_or_get_mut(Key::Property(property), Value::new_object)
            .as_object_mut()
        {
            flags.upsert(flag, Value::Bool(true));
        }
    }
}

impl<I: JSCalendarId, B: JSCalendarId, T: ParamTarget<I, B>> ExtractParams<I, B> for T {
    fn extract_params(
        &mut self,
        entry: &mut ICalendarEntry,
        extract: &[ICalendarParameterName],
    ) -> Option<String> {
        let mut jsid = None;
        let is_styled_description = matches!(entry.name, ICalendarProperty::StyledDescription);
        let is_conference = matches!(entry.name, ICalendarProperty::Conference);

        entry.params.retain_mut(|param| {
            if !extract.contains(&param.name) {
                return true;
            }

            match param.name {
                ICalendarParameterName::Cn => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set(JSCalendarProperty::Name, Value::Str(text));
                    }
                }
                ICalendarParameterName::Cutype => {
                    let kind = match param.take_value() {
                        ICalendarParameterValue::Text(value) => Value::Str(value.into()),
                        ICalendarParameterValue::Cutype(value) => match value {
                            ICalendarUserTypes::Individual => {
                                Value::Element(JSCalendarValue::ParticipantKind(
                                    JSCalendarParticipantKind::Individual,
                                ))
                            }
                            ICalendarUserTypes::Group => Value::Element(
                                JSCalendarValue::ParticipantKind(JSCalendarParticipantKind::Group),
                            ),
                            ICalendarUserTypes::Resource => {
                                Value::Element(JSCalendarValue::ParticipantKind(
                                    JSCalendarParticipantKind::Resource,
                                ))
                            }
                            ICalendarUserTypes::Room => {
                                Value::Element(JSCalendarValue::ParticipantKind(
                                    JSCalendarParticipantKind::Location,
                                ))
                            }
                            ICalendarUserTypes::Unknown => {
                                Value::Str(value.as_str().to_lowercase().into())
                            }
                        },
                        _ => return false,
                    };
                    self.set(JSCalendarProperty::Kind, kind);
                }
                ICalendarParameterName::DelegatedFrom => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set_flag(JSCalendarProperty::DelegatedFrom, Key::from(text));
                    }
                }
                ICalendarParameterName::DelegatedTo => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set_flag(JSCalendarProperty::DelegatedTo, Key::from(text));
                    }
                }
                ICalendarParameterName::Email => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set(JSCalendarProperty::Email, Value::Str(text));
                    }
                }
                ICalendarParameterName::Rsvp => {
                    if let Some(boolean) = param.value.as_bool() {
                        self.set(JSCalendarProperty::ExpectReply, Value::Bool(boolean));
                    }
                }
                ICalendarParameterName::Member => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set_flag(JSCalendarProperty::MemberOf, Key::from(text));
                    }
                }
                ICalendarParameterName::Partstat => {
                    let status = match param.take_value() {
                        ICalendarParameterValue::Partstat(value) => {
                            Value::Element(JSCalendarValue::ParticipationStatus(match value {
                                ICalendarParticipationStatus::NeedsAction => {
                                    JSCalendarParticipationStatus::NeedsAction
                                }
                                ICalendarParticipationStatus::Declined => {
                                    JSCalendarParticipationStatus::Declined
                                }
                                ICalendarParticipationStatus::Tentative => {
                                    JSCalendarParticipationStatus::Tentative
                                }
                                ICalendarParticipationStatus::Delegated => {
                                    JSCalendarParticipationStatus::Delegated
                                }
                                ICalendarParticipationStatus::Completed
                                | ICalendarParticipationStatus::InProcess
                                | ICalendarParticipationStatus::Failed
                                | ICalendarParticipationStatus::Accepted => {
                                    JSCalendarParticipationStatus::Accepted
                                }
                            }))
                        }
                        ICalendarParameterValue::Text(value) => Value::Str(value.into()),
                        _ => return false,
                    };
                    self.set(JSCalendarProperty::ParticipationStatus, status);
                }
                ICalendarParameterName::Role => {
                    let role = match param.take_value() {
                        ICalendarParameterValue::Role(value) => {
                            Key::Property(JSCalendarProperty::ParticipantRole(match value {
                                ICalendarParticipationRole::Chair => {
                                    JSCalendarParticipantRole::Chair
                                }
                                ICalendarParticipationRole::ReqParticipant => {
                                    JSCalendarParticipantRole::Required
                                }
                                ICalendarParticipationRole::OptParticipant => {
                                    JSCalendarParticipantRole::Optional
                                }
                                ICalendarParticipationRole::NonParticipant => {
                                    JSCalendarParticipantRole::Informational
                                }
                                ICalendarParticipationRole::Owner => {
                                    JSCalendarParticipantRole::Owner
                                }
                            }))
                        }
                        ICalendarParameterValue::Text(value) => Key::Owned(value),
                        _ => return false,
                    };
                    self.set_flag(JSCalendarProperty::Roles, role);
                }
                ICalendarParameterName::SentBy => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set(JSCalendarProperty::SentBy, Value::Str(text));
                    }
                }
                ICalendarParameterName::Fmttype => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set(
                            if is_styled_description {
                                JSCalendarProperty::DescriptionContentType
                            } else {
                                JSCalendarProperty::ContentType
                            },
                            Value::Str(text),
                        );
                    }
                }
                ICalendarParameterName::Label | ICalendarParameterName::Filename => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set(
                            if is_conference {
                                JSCalendarProperty::Name
                            } else {
                                JSCalendarProperty::Title
                            },
                            Value::Str(text),
                        );
                    }
                }
                ICalendarParameterName::Size => match param.value.as_integer() {
                    Some(number) => {
                        self.set(JSCalendarProperty::Size, Value::Number(number.into()));
                    }
                    None => return true,
                },
                ICalendarParameterName::Linkrel => match param.take_value() {
                    ICalendarParameterValue::Linkrel(linkrel) => {
                        self.set(
                            JSCalendarProperty::Rel,
                            Value::Element(JSCalendarValue::LinkRelation(linkrel)),
                        );
                    }
                    ICalendarParameterValue::Text(value) => {
                        self.set(JSCalendarProperty::Rel, Value::Str(value.into()));
                    }
                    value => {
                        param.value = value;
                        return true;
                    }
                },
                ICalendarParameterName::Related => match param.take_value() {
                    ICalendarParameterValue::Related(related) => {
                        self.set(
                            JSCalendarProperty::RelativeTo,
                            Value::Element(JSCalendarValue::RelativeTo(match related {
                                ICalendarRelated::Start => JSCalendarRelativeTo::Start,
                                ICalendarRelated::End => JSCalendarRelativeTo::End,
                            })),
                        );
                    }
                    value => {
                        param.value = value;
                        return true;
                    }
                },
                ICalendarParameterName::Feature => {
                    let feature = match param.take_value() {
                        ICalendarParameterValue::Feature(value) => {
                            Key::Property(JSCalendarProperty::VirtualLocationFeature(match value {
                                ICalendarFeatureType::Audio => {
                                    JSCalendarVirtualLocationFeature::Audio
                                }
                                ICalendarFeatureType::Chat => {
                                    JSCalendarVirtualLocationFeature::Chat
                                }
                                ICalendarFeatureType::Feed => {
                                    JSCalendarVirtualLocationFeature::Feed
                                }
                                ICalendarFeatureType::Moderator => {
                                    JSCalendarVirtualLocationFeature::Moderator
                                }
                                ICalendarFeatureType::Phone => {
                                    JSCalendarVirtualLocationFeature::Phone
                                }
                                ICalendarFeatureType::Screen => {
                                    JSCalendarVirtualLocationFeature::Screen
                                }
                                ICalendarFeatureType::Video => {
                                    JSCalendarVirtualLocationFeature::Video
                                }
                            }))
                        }
                        ICalendarParameterValue::Text(value) => Key::Owned(value),
                        _ => return false,
                    };
                    self.set_flag(JSCalendarProperty::Features, feature);
                }
                ICalendarParameterName::Language => {
                    if let Some(text) = param.take_value().into_text() {
                        self.set(JSCalendarProperty::Locale, Value::Str(text));
                    }
                }
                ICalendarParameterName::Display => {
                    let display = match param.take_value() {
                        ICalendarParameterValue::Display(value) => {
                            Key::Property(JSCalendarProperty::LinkDisplay(match value {
                                ICalendarDisplayType::Badge => JSCalendarLinkDisplay::Badge,
                                ICalendarDisplayType::Graphic => JSCalendarLinkDisplay::Graphic,
                                ICalendarDisplayType::Fullsize => JSCalendarLinkDisplay::Fullsize,
                                ICalendarDisplayType::Thumbnail => JSCalendarLinkDisplay::Thumbnail,
                            }))
                        }
                        ICalendarParameterValue::Text(value) => Key::Owned(value),
                        value => {
                            param.value = value;
                            return true;
                        }
                    };
                    self.set_flag(JSCalendarProperty::Display, display);
                }
                ICalendarParameterName::Jsid => {
                    jsid = param.take_value().into_text().map(Cow::into_owned);
                }
                ICalendarParameterName::Range
                | ICalendarParameterName::Reltype
                | ICalendarParameterName::Dir
                | ICalendarParameterName::Fbtype
                | ICalendarParameterName::ScheduleAgent
                | ICalendarParameterName::ScheduleForceSend
                | ICalendarParameterName::ScheduleStatus
                | ICalendarParameterName::Tzid
                | ICalendarParameterName::Value
                | ICalendarParameterName::ManagedId
                | ICalendarParameterName::Order
                | ICalendarParameterName::Schema
                | ICalendarParameterName::Derived
                | ICalendarParameterName::Gap
                | ICalendarParameterName::Jsptr
                | ICalendarParameterName::Other(_)
                | ICalendarParameterName::Altrep => {}
            }
            false
        });

        jsid
    }
}

impl ICalendarParameter {
    fn take_value(&mut self) -> ICalendarParameterValue {
        mem::replace(&mut self.value, ICalendarParameterValue::Null)
    }
}

impl<I: JSCalendarId, B: JSCalendarId> ICalendarParams<I, B> {
    pub(super) fn push(
        &mut self,
        name: ICalendarParameterName,
        value: Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    ) {
        match self.0.iter_mut().find(|(param, _)| param == &name) {
            Some((_, values)) => values.push(value),
            None => self.0.push((name, ParamValues::One(value))),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn into_jscalendar_value(
        self,
    ) -> Option<Map<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>> {
        (!self.0.is_empty()).then(|| {
            self.0
                .into_iter()
                .map(|(param, values)| {
                    let value = match values {
                        ParamValues::One(value) => value,
                        ParamValues::Many(values) => Value::Array(values),
                    };
                    (
                        Key::Owned(param.into_string().into_ascii_lowercase()),
                        value,
                    )
                })
                .collect()
        })
    }
}

impl<I: JSCalendarId, B: JSCalendarId> ParamValues<I, B> {
    fn push(&mut self, value: Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>) {
        match self {
            ParamValues::Many(values) => values.push(value),
            ParamValues::One(first) => {
                let first = mem::replace(first, Value::Null);
                *self = ParamValues::Many(vec![first, value]);
            }
        }
    }
}

impl ICalendarValue {
    pub(super) fn into_jscalendar_value<I: JSCalendarId, B: JSCalendarId>(
        self,
        value_type: Option<&IanaType<ICalendarValueType, String>>,
    ) -> Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>> {
        match self {
            ICalendarValue::Text(v) => Value::Str(v.into()),
            ICalendarValue::Integer(v) => Value::Number(v.into()),
            ICalendarValue::Float(v) => Value::Number(v.into()),
            ICalendarValue::Boolean(v) => Value::Bool(v),
            ICalendarValue::PartialDateTime(v) => {
                let mut out = String::new();
                let _ = v.format_as_ical(
                    &mut out,
                    value_type.and_then(|v| v.iana()).unwrap_or(
                        match (v.has_date(), v.has_time(), v.has_zone()) {
                            (true, true, _) => &ICalendarValueType::DateTime,
                            (true, _, _) => &ICalendarValueType::Date,
                            (_, true, _) => &ICalendarValueType::Time,
                            (_, _, true) => &ICalendarValueType::UtcOffset,
                            _ => &ICalendarValueType::Text,
                        },
                    ),
                );
                Value::Str(out.into())
            }
            ICalendarValue::Binary(v) => Value::Str(
                Uri::Data(Box::new(Data {
                    content_type: None,
                    data: v,
                }))
                .into_unwrapped_string()
                .into(),
            ),
            ICalendarValue::Uri(v) => Value::Str(v.into_unwrapped_string().into()),
            ICalendarValue::Duration(v) => Value::Str(v.to_string().into()),
            ICalendarValue::RecurrenceRule(v) => Value::Str(v.to_string().into()),
            ICalendarValue::Period(v) => Value::Str(v.to_string().into()),
            ICalendarValue::CalendarScale(v) => Value::Str(v.as_str().into()),
            ICalendarValue::Method(v) => Value::Str(v.as_str().into()),
            ICalendarValue::Classification(v) => Value::Str(v.as_str().into()),
            ICalendarValue::Status(v) => Value::Str(v.as_str().into()),
            ICalendarValue::Transparency(v) => Value::Str(v.as_str().into()),
            ICalendarValue::Action(v) => Value::Str(v.as_str().into()),
            ICalendarValue::BusyType(v) => Value::Str(v.as_str().into()),
            ICalendarValue::ParticipantType(v) => Value::Str(v.as_str().into()),
            ICalendarValue::ResourceType(v) => Value::Str(v.as_str().into()),
            ICalendarValue::Proximity(v) => Value::Str(v.as_str().into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExtractParams, ICalendarParams, IntoAsciiLowercase};
    use crate::{
        icalendar::{
            ICalendarEntry, ICalendarParameter, ICalendarParameterName, ICalendarParameterValue,
            ICalendarProperty,
        },
        jscalendar::{JSCalendarProperty, JSCalendarValue},
    };
    use jmap_tools::{Map, Value};
    use std::borrow::Cow;

    type TestMap = Map<'static, JSCalendarProperty<String>, JSCalendarValue<String, String>>;

    #[test]
    fn params_group_values_by_name_in_first_occurrence_order() {
        let empty: Option<TestMap> = ICalendarParams::default().into_jscalendar_value();
        assert!(empty.is_none());

        let mut params = ICalendarParams::default();
        assert!(params.is_empty());
        for (name, value) in [
            (ICalendarParameterName::Cn, Value::Str(Cow::Borrowed("a"))),
            (ICalendarParameterName::Role, Value::Str(Cow::Borrowed("b"))),
            (ICalendarParameterName::Cn, Value::Str(Cow::Borrowed("c"))),
            (
                ICalendarParameterName::Other("X-Mixed-Case".to_string()),
                Value::Bool(true),
            ),
            (ICalendarParameterName::Cn, Value::Null),
            (
                ICalendarParameterName::Other(String::new()),
                Value::Str(Cow::Borrowed("e")),
            ),
            (
                ICalendarParameterName::Other("X-\u{c4}\u{d6}\u{dc}-\u{e9}".to_string()),
                Value::Array(vec![Value::Str(Cow::Borrowed("x"))]),
            ),
        ] {
            params.push(name, value);
        }
        assert!(!params.is_empty());
        let params: Option<TestMap> = params.into_jscalendar_value();
        assert_eq!(
            format!("{params:?}"),
            concat!(
                r#"Some(ObjectAsVec([(Owned("cn"), Array [Str("a"), Str("c"), Null]), "#,
                r#"(Owned("role"), Str("b")), (Owned("x-mixed-case"), Bool(true)), "#,
                r#"(Owned(""), Str("e")), (Owned("x-"#,
                "\u{c4}\u{d6}\u{dc}-\u{e9}",
                r#""), Array [Str("x")])]))"#
            )
        );
    }

    #[test]
    fn lowercase_matches_to_ascii_lowercase() {
        let pieces = [
            "A", "b", "-", "\u{c9}", "\u{e9}", "\u{df}", "Z", "0", "\u{130}", "\u{1c5}",
        ];
        let texts = pieces
            .iter()
            .flat_map(|first| pieces.iter().map(move |second| format!("{first}{second}")))
            .chain(pieces.iter().map(|piece| piece.to_string()))
            .chain([String::new(), pieces.concat(), pieces.concat().repeat(3)]);
        for text in texts {
            let expected = text.to_ascii_lowercase();
            assert_eq!(
                Cow::<str>::Owned(text.clone()).into_ascii_lowercase(),
                expected
            );
            let borrowed: &'static str = Box::leak(text.into_boxed_str());
            assert_eq!(Cow::Borrowed(borrowed).into_ascii_lowercase(), expected);
        }
    }

    #[test]
    fn extract_params_keeps_unconverted_parameters_in_place() {
        use ICalendarParameterName as Name;
        use ICalendarParameterValue as ParamValue;
        let text = |text: &str| ParamValue::Text(text.to_string());
        let params = [
            (true, Name::Other("X-A".to_string()), text("1")),
            (false, Name::Cn, text("Doe")),
            (true, Name::Size, text("abc")),
            (false, Name::Cutype, ParamValue::Integer(1)),
            (false, Name::Fmttype, text("text/plain")),
            (true, Name::Linkrel, ParamValue::Integer(3)),
            (false, Name::Partstat, ParamValue::Null),
            (true, Name::Related, text("middle")),
            (false, Name::Jsid, text("id1")),
            (true, Name::Display, ParamValue::Bool(true)),
            (false, Name::Label, text("Room")),
            (false, Name::Size, ParamValue::Integer(10)),
            (false, Name::Role, ParamValue::Bool(false)),
            (true, Name::Other("X-B".to_string()), text("2")),
            (false, Name::Feature, ParamValue::Integer(2)),
            (false, Name::Tzid, text("Europe/Berlin")),
            (false, Name::DelegatedTo, text("mailto:a@b")),
        ];
        let mut entry = ICalendarEntry::new(ICalendarProperty::Conference);
        entry.params = params
            .iter()
            .map(|(_, name, value)| ICalendarParameter::new(name.clone(), value.clone()))
            .collect();
        let mut target = TestMap::new();
        let jsid = target.extract_params(
            &mut entry,
            &[
                Name::Cn,
                Name::Cutype,
                Name::Size,
                Name::Fmttype,
                Name::Linkrel,
                Name::Partstat,
                Name::Related,
                Name::Jsid,
                Name::Display,
                Name::Label,
                Name::Role,
                Name::Feature,
                Name::Tzid,
                Name::DelegatedTo,
            ],
        );
        assert_eq!(jsid.as_deref(), Some("id1"));
        assert_eq!(
            entry.params,
            params
                .into_iter()
                .filter(|(kept, ..)| *kept)
                .map(|(_, name, value)| ICalendarParameter::new(name, value))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            format!("{target:?}"),
            concat!(
                r#"ObjectAsVec([(Property(Name), Str("Room")), "#,
                r#"(Property(ContentType), Str("text/plain")), (Property(Size), Number(10)), "#,
                r#"(Property(DelegatedTo), Object ObjectAsVec([(Owned("mailto:a@b"), Bool(true))]))])"#
            )
        );
    }
}
