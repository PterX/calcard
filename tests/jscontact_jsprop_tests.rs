/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    jscontact::{Context, Feature, JSContact, JSContactProperty, JSContactValue},
    vcard::VCard,
};
use jmap_tools::{Key, Value};
use serde_json::Value as JsonValue;

type JSValue = Value<'static, JSContactProperty<String>, JSContactValue<String, String>>;
type TypedKey = Result<JSContactProperty<String>, String>;

const JSPROP_REFERENCE: &str = "RFC 9555 Section 3.2.1 (JSPROP values form a PatchObject); RFC 9553 Section 1.4.3 (patch values MUST be valid for the property being set)";

const JSPROP_VCARD: &str = concat!(
    "BEGIN:VCARD\r\n",
    "VERSION:4.0\r\n",
    "FN:Jane Doe\r\n",
    "EMAIL;PROP-ID=e1:jane@example.com\r\n",
    "TEL;PROP-ID=t1:tel:+1-555-555-5555\r\n",
    "JSPROP;JSPTR=\"emails/e1/contexts\":{\"private\":true\\,\"work\":true}\r\n",
    "JSPROP;JSPTR=\"phones/t1/features\":{\"mobile\":true\\,\"voice\":true}\r\n",
    "END:VCARD\r\n"
);

fn import(vcard: &str) -> JSContact<'static, String, String> {
    VCard::parse(vcard)
        .expect("valid vCard")
        .into_jscontact::<String, String>()
}

fn normalize(jscontact: &JSContact<'_, String, String>) -> JsonValue {
    let mut value: JsonValue =
        serde_json::from_str(&jscontact.to_string_pretty()).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("vCard");
    }
    value
}

fn typed_keys(card: &JSValue, path: &[Key<'static, JSContactProperty<String>>]) -> Vec<TypedKey> {
    path.iter()
        .fold(card, |value, key| {
            value
                .as_object_and_get(key)
                .unwrap_or_else(|| panic!("{JSPROP_REFERENCE}: missing {key:?} in {value:?}"))
        })
        .as_object()
        .expect("object value")
        .keys()
        .map(|key| match key {
            Key::Property(property) => Ok(property.clone()),
            key => Err(key.to_string().into_owned()),
        })
        .collect()
}

#[test]
fn jsprop_contexts_and_features_restore_typed_keys() {
    let imported = import(JSPROP_VCARD);
    let exported = imported
        .clone()
        .into_vcard()
        .expect("JSContact exports to vCard")
        .to_string();
    let reimported = import(&exported);

    assert_eq!(
        normalize(&reimported),
        normalize(&imported),
        "{JSPROP_REFERENCE}\n{exported}"
    );
    assert!(!exported.contains("JSPROP"), "{exported}");

    assert_eq!(
        typed_keys(
            &imported.0,
            &[
                Key::Property(JSContactProperty::Emails),
                Key::Borrowed("e1"),
                Key::Property(JSContactProperty::Contexts),
            ]
        ),
        [
            Ok(JSContactProperty::Context(Context::Private)),
            Ok(JSContactProperty::Context(Context::Work)),
        ],
        "{JSPROP_REFERENCE}; RFC 9553 Sections 1.5.1 and 2.3.1\n{exported}"
    );
    assert_eq!(
        typed_keys(
            &imported.0,
            &[
                Key::Property(JSContactProperty::Phones),
                Key::Borrowed("t1"),
                Key::Property(JSContactProperty::Features),
            ]
        ),
        [
            Ok(JSContactProperty::Feature(Feature::Mobile)),
            Ok(JSContactProperty::Feature(Feature::Voice)),
        ],
        "{JSPROP_REFERENCE}; RFC 9553 Section 2.3.3\n{exported}"
    );
}
