/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    common::{IanaParse, IanaType},
    jscontact::{
        Context, Feature, JSContactId, JSContactProperty, JSContactValue,
        export::{State, props::convert_value},
    },
    vcard::{
        VCard, VCardEntry, VCardParameter, VCardParameterName, VCardParameterValue, VCardProperty,
        VCardType, VCardValueType, ValueType,
    },
};
use jmap_tools::{Element, JsonPointer, Key, Property, Value};
use smallvec::{SmallVec, smallvec};
use std::{borrow::Cow, mem};

impl<'x, I, B> State<'x, I, B>
where
    I: JSContactId,
    B: JSContactId,
{
    pub(super) fn insert_vcard(&mut self, path: &[JSContactProperty<I>], mut entry: VCardEntry) {
        if self.converted_props_count < self.converted_props.len() {
            // Obtain propId
            let mut prop_id =
                if matches!(entry.name, VCardProperty::Member | VCardProperty::Related) {
                    entry.values.first().and_then(|v| v.as_text())
                } else {
                    entry.prop_id()
                };

            // Try mapping X-ABLabel
            if let Some(prop_id_) = prop_id {
                let mut remove_pos = None;
                for (param_pos, param) in entry.params.iter().enumerate() {
                    if let (VCardParameterName::Label, VCardParameterValue::Text(label)) =
                        (&param.name, &param.value)
                    {
                        if self.converted_props.iter().any(|(prop, _)| {
                            prop.len() == 3
                                && prop[0].to_string() == path[0].to_string()
                                && prop[1] == prop_id_
                                && prop[2] == Key::Property(JSContactProperty::Label)
                        }) {
                            self.insert_vcard(
                                &[path[0].clone(), JSContactProperty::Label],
                                VCardEntry::new(VCardProperty::Other("X-ABLabel".into()))
                                    .with_value(label.to_string()),
                            );
                            remove_pos = Some(param_pos);
                        }
                        break;
                    }
                }

                if let Some(pos) = remove_pos {
                    entry.params.swap_remove(pos);
                    prop_id =
                        if matches!(entry.name, VCardProperty::Member | VCardProperty::Related) {
                            entry.values.first().and_then(|v| v.as_text())
                        } else {
                            entry.prop_id()
                        };
                }
            }

            let skip_tz_geo = matches!(entry.name, VCardProperty::Adr);
            let mut matched_once = false;
            let may_match = path
                .first()
                .is_none_or(|head| self.converted_heads.contains_key(head));

            if may_match {
                'outer: for (keys, value) in self.converted_props.iter_mut() {
                    let is_localized_key = keys.first().is_some_and(|k| {
                        matches!(k, Key::Property(JSContactProperty::Localizations))
                    });

                    if let Some(lang) = &self.language {
                        if !is_localized_key || keys.get(1).is_none_or(|k| *k != lang.as_str()) {
                            continue;
                        }
                    } else if is_localized_key {
                        continue;
                    }
                    if matches!(value, Value::Null) {
                        continue;
                    }

                    for (pos, item) in path.iter().enumerate() {
                        if !keys
                            .iter()
                            .any(|k| matches!(k, Key::Property(p) if p == item))
                        {
                            if pos == 0 && matched_once {
                                break 'outer;
                            } else {
                                continue 'outer;
                            }
                        } else {
                            matched_once = true;
                        }
                    }

                    if prop_id
                        .map(Key::Borrowed)
                        .is_none_or(|prop_id| keys.iter().any(|k| k == &prop_id))
                        && (!skip_tz_geo
                            || !keys.iter().any(|k| {
                                matches!(
                                    k,
                                    Key::Property(
                                        JSContactProperty::TimeZone
                                            | JSContactProperty::Coordinates
                                    )
                                )
                            }))
                    {
                        entry.import_converted_properties(mem::take(value));
                        self.converted_props_count += 1;
                        break;
                    }
                }
            }
        }

        if let Some(lang) = &self.language {
            entry.params.push(VCardParameter::language(lang.clone()));
        }

        self.vcard.entries.push(entry);
    }

    pub(super) fn convert_types(
        &mut self,
        vcard_property: &VCardProperty,
        [property, id]: [&str; 2],
        bucket: JSContactProperty<I>,
        value: Value<'x, JSContactProperty<I>, JSContactValue<I, B>>,
        params: &mut Vec<VCardParameter>,
    ) {
        let mut has_mapped = false;
        let mut has_unmapped = false;
        let mut has_unquotable_key = false;

        let is_mapped =
            |key: &Key<'_, JSContactProperty<I>>,
             value: &Value<'x, JSContactProperty<I>, JSContactValue<I, B>>| {
                value.as_bool() == Some(true)
                    && bucket
                        .vcard_type(vcard_property, key.to_string().as_ref())
                        .is_some()
            };

        for (key, item) in value.as_object().into_iter().flat_map(|obj| obj.iter()) {
            let key = key.to_string();
            match bucket
                .vcard_type(vcard_property, key.as_ref())
                .filter(|_| item.as_bool() == Some(true))
            {
                Some(IanaType::Iana(typ)) => {
                    params.push(VCardParameter::typ(typ));
                    has_mapped = true;
                }
                Some(IanaType::Other(typ)) => {
                    params.push(VCardParameter::typ(typ.to_ascii_uppercase()));
                    has_mapped = true;
                }
                None => {
                    has_unmapped = true;
                    has_unquotable_key |= key
                        .chars()
                        .any(|ch| ch.is_control() || matches!(ch, '"' | '\\'));
                }
            }
        }

        if !has_unmapped {
            return;
        }

        let bucket_name = bucket.to_string();
        if has_mapped && !has_unquotable_key && self.language.is_none() {
            for (key, item) in value.into_expanded_object() {
                if !is_mapped(&key, &item) {
                    self.insert_jsprop(
                        &[property, id, bucket_name.as_ref(), key.to_string().as_ref()],
                        item,
                    );
                }
            }
        } else {
            self.insert_jsprop(
                &[property, id, bucket_name.as_ref()],
                Value::Object(value.into_expanded_object().collect()),
            );
        }
    }

    pub(super) fn insert_jsprop(
        &mut self,
        path: &[&str],
        value: Value<'x, JSContactProperty<I>, JSContactValue<I, B>>,
    ) {
        let path = if let Some(lang) = &self.language {
            JsonPointer::<JSContactProperty<I>>::encode([
                JSContactProperty::Localizations::<I>.to_string().as_ref(),
                lang.as_str(),
                JsonPointer::<JSContactProperty<I>>::encode(path).as_str(),
            ])
        } else {
            JsonPointer::<JSContactProperty<I>>::encode(path)
        };

        self.vcard.entries.push(
            VCardEntry::new(VCardProperty::Jsprop)
                .with_param(VCardParameter::jsptr(path))
                .with_value(serde_json::to_string(&value).unwrap_or_default()),
        );
    }

    pub(super) fn import_properties(
        &mut self,
        props: Vec<Value<'x, JSContactProperty<I>, JSContactValue<I, B>>>,
    ) {
        for prop in props.into_iter().flat_map(|prop| prop.into_array()) {
            let mut prop = prop.into_iter();
            let Some(name) = prop
                .next()
                .and_then(|v| v.into_string())
                .map(|name| {
                    VCardProperty::parse(name.as_bytes()).unwrap_or_else(|| {
                        let mut name = name.into_owned();
                        name.make_ascii_uppercase();
                        VCardProperty::Other(name)
                    })
                })
                .filter(|name| !matches!(name, VCardProperty::Begin | VCardProperty::End))
            else {
                continue;
            };
            let Some(params) = prop.next() else {
                continue;
            };
            let Some(value_type) = prop
                .next()
                .and_then(|v| v.into_string())
                .map(|v| VCardValueType::parse(v.as_bytes()))
            else {
                continue;
            };

            let (default_type, _) = name.default_types();
            let convert_type = value_type.map(ValueType::Vcard).unwrap_or(default_type);

            let Some(values) = prop.next().and_then(|v| match v {
                Value::Array(arr) => Some(
                    arr.into_iter()
                        .filter_map(|v| convert_value(v, &convert_type).ok())
                        .collect::<SmallVec<_>>(),
                ),
                v => convert_value(v, &convert_type).ok().map(|v| smallvec![v]),
            }) else {
                continue;
            };

            let mut entry = VCardEntry::new(name);
            entry.import_jcard_params(params);
            entry.values = values;
            if convert_type != default_type
                && let Some(value_type) = value_type
            {
                entry.params.push(VCardParameter::value(
                    IanaType::<VCardValueType, String>::Iana(value_type),
                ));
            }
            self.vcard.entries.push(entry);
        }
    }

    pub(super) fn into_vcard(self) -> VCard {
        self.vcard
    }
}

impl<I: JSContactId> JSContactProperty<I> {
    fn vcard_type<'k>(
        &self,
        property: &VCardProperty,
        key: &'k str,
    ) -> Option<IanaType<VCardType, &'k str>> {
        if key.is_empty()
            || !key
                .bytes()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
        {
            return None;
        }

        match (self, VCardType::parse(key.as_bytes())) {
            (JSContactProperty::Features, Some(typ)) => Feature::from_vcard_type(property, &typ)
                .is_some_and(|feature| feature.as_str() == key)
                .then_some(IanaType::Iana(typ)),
            (JSContactProperty::Features, None) => {
                Feature::from_vcard_type(property, &VCardType::Cell)
                    .is_some_and(|feature| feature.as_str() == key)
                    .then_some(IanaType::Iana(VCardType::Cell))
            }
            (JSContactProperty::Contexts, Some(typ)) => (Feature::from_vcard_type(property, &typ)
                .is_none()
                && !matches!(typ, VCardType::Home | VCardType::Cell)
                && (!matches!(typ, VCardType::Billing | VCardType::Delivery)
                    || property == &VCardProperty::Adr))
                .then_some(IanaType::Iana(typ)),
            (JSContactProperty::Contexts, None) if key == Context::Private.as_str() => {
                Some(IanaType::Iana(VCardType::Home))
            }
            (JSContactProperty::Contexts, None) => Some(IanaType::Other(key)),
            _ => None,
        }
    }
}

pub(crate) enum ParamValue<'x> {
    Text(Cow<'x, str>),
    Number(i64),
    Bool(bool),
}

impl<'x> ParamValue<'x> {
    pub(crate) fn try_from_value<P: Property, E: Element>(value: Value<'x, P, E>) -> Option<Self> {
        match value {
            Value::Str(s) => Some(Self::Text(s)),
            Value::Number(n) => Some(match n.as_i64() {
                Some(number) => Self::Number(number),
                None => Self::Text(Value::<P, E>::Number(n).to_string().into()),
            }),
            Value::Bool(b) => Some(Self::Bool(b)),
            Value::Element(e) => Some(Self::Text(e.to_cow().to_string().into())),
            _ => None,
        }
    }

    pub(crate) fn into_string(self) -> Cow<'x, str> {
        match self {
            Self::Text(s) => s,
            Self::Number(n) => n.to_string().into(),
            Self::Bool(b) => if b { "true" } else { "false" }.to_string().into(),
        }
    }

    pub(crate) fn into_number(self) -> Result<i64, Self> {
        match self {
            Self::Number(n) => Ok(n),
            Self::Text(s) => s.parse().map_err(|_| Self::Text(s)),
            _ => Err(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ParamValue;
    use crate::{
        common::{IanaParse, IanaString, IanaType},
        jscontact::{JSContactProperty, JSContactValue},
        vcard::{VCardEntry, VCardParameter, VCardParameterValue, VCardProperty, VCardType},
    };
    use jmap_tools::{Key, Map, Value};

    type JSValue = Value<'static, JSContactProperty<String>, JSContactValue<String, String>>;

    #[test]
    fn vcard_type_imports_back_into_the_same_key() {
        let keys = [
            "work",
            "home",
            "private",
            "billing",
            "delivery",
            "cell",
            "mobile",
            "voice",
            "fax",
            "text",
            "video",
            "pager",
            "textphone",
            "main-number",
            "friend",
            "co-worker",
            "x-car",
            "pref",
            "WORK",
            "Work",
            "x.y",
            "a,b",
            "example.com:car",
            "",
            "a b",
        ];

        for property in [VCardProperty::Tel, VCardProperty::Email] {
            for bucket in [JSContactProperty::Contexts, JSContactProperty::Features] {
                for key in keys {
                    let imports_as_key = |text: &str| {
                        let (imported_bucket, imported_key) =
                            JSContactProperty::<String>::from_vcard_type(
                                &property,
                                VCardType::parse(text.as_bytes()).map_or_else(
                                    || IanaType::Other(text.to_string()),
                                    IanaType::Iana,
                                ),
                            );
                        imported_bucket == bucket && imported_key.to_string() == key
                    };
                    let is_type_value = !key.is_empty()
                        && key
                            .bytes()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-');

                    let is_export_restricted =
                        matches!(key, "billing" | "delivery") && property != VCardProperty::Adr;
                    let is_consistent = is_export_restricted
                        || match bucket.vcard_type(&property, key) {
                            Some(IanaType::Iana(typ)) => {
                                is_type_value && imports_as_key(typ.as_str())
                            }
                            Some(IanaType::Other(typ)) => {
                                is_type_value && imports_as_key(&typ.to_ascii_uppercase())
                            }
                            None => {
                                !is_type_value
                                    || ![key.to_ascii_uppercase().as_str(), "HOME", "CELL"]
                                        .into_iter()
                                        .any(imports_as_key)
                            }
                        };
                    assert!(is_consistent, "{property:?} {bucket:?} {key:?}");
                }
            }
        }
    }

    #[test]
    fn param_numbers_do_not_wrap() {
        for (value, expected) in [
            (JSValue::Number((-3i64).into()), "-3"),
            (JSValue::Number(u64::MAX.into()), "18446744073709551615"),
            (JSValue::Number(1.5f64.into()), "1.5"),
        ] {
            assert_eq!(
                ParamValue::try_from_value(value.clone()).map(ParamValue::into_string),
                Some(expected.into()),
                "{value:?}"
            );
        }

        let mut entry = VCardEntry::new(VCardProperty::Email);
        entry.import_jcard_params(JSValue::Object(Map::from(vec![
            (Key::Borrowed("pref"), JSValue::Number((-1i64).into())),
            (
                Key::Borrowed("index"),
                JSValue::Number(4_294_967_296u64.into()),
            ),
            (Key::Borrowed("x-number"), JSValue::Number(1.5f64.into())),
        ])));
        assert_eq!(
            entry.params,
            [
                VCardParameter {
                    name: crate::vcard::VCardParameterName::Pref,
                    value: VCardParameterValue::Text("-1".to_string()),
                },
                VCardParameter {
                    name: crate::vcard::VCardParameterName::Index,
                    value: VCardParameterValue::Text("4294967296".to_string()),
                },
                VCardParameter {
                    name: crate::vcard::VCardParameterName::Other("X-NUMBER".to_string()),
                    value: VCardParameterValue::Text("1.5".to_string()),
                },
            ]
        );
    }
}
