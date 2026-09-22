/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    jscontact::{Context, Feature, JSContact, JSContactProperty, JSContactValue},
    vcard::{VCard, VCardParameterName},
};
use jmap_tools::{Key, Value};
use serde_json::Value as JsonValue;

type JSValue = Value<'static, JSContactProperty<String>, JSContactValue<String, String>>;
type TypedKey = Result<JSContactProperty<String>, String>;

fn import(vcard: &str) -> JSContact<'static, String, String> {
    VCard::parse(vcard)
        .expect("valid vCard")
        .into_jscontact::<String, String>()
}

fn export(json: &str) -> String {
    JSContact::<String, String>::parse(json)
        .expect("valid JSContact")
        .into_vcard()
        .expect("JSContact exports to vCard")
        .to_string()
}

fn normalize(value: &JsonValue) -> JsonValue {
    let mut value = value.clone();
    if let Some(obj) = value.as_object_mut() {
        for key in ["vCard", "name", "version", "@type"] {
            obj.remove(key);
        }
    }
    value
}

fn typed_keys(
    card: &JSValue,
    property: JSContactProperty<String>,
    id: &str,
    bucket: JSContactProperty<String>,
) -> Option<Vec<TypedKey>> {
    let keys = card
        .as_object_and_get(&Key::Property(property))?
        .as_object_and_get(&Key::Borrowed(id))?
        .as_object_and_get(&Key::Property(bucket))?
        .as_object()?
        .keys()
        .map(|key| match key {
            Key::Property(property) => Ok(property.clone()),
            key => Err(key.to_string().into_owned()),
        })
        .collect();
    Some(keys)
}

#[test]
fn rfc9555_2_3_22_type_imports_typed_contexts() {
    let card = import(concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
        "EMAIL;PROP-ID=e1;TYPE=home,work:jane@example.com\r\n",
        "ADR;PROP-ID=a1;TYPE=billing,delivery:;;1 Main St;Town;;;\r\n",
        "TEL;PROP-ID=t1;TYPE=home,x-car,\"example.com:boat\":tel:+1-555-555-5555\r\n",
        "IMPP;PROP-ID=i1;TYPE=private,cell:xmpp:jane@example.com\r\n",
        "ORG-DIRECTORY;PROP-ID=d1;TYPE=work:https://dir.example.com\r\n",
        "END:VCARD\r\n"
    ));

    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Emails,
            "e1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Private)),
            Ok(JSContactProperty::Context(Context::Work)),
        ]),
        "RFC 9555 Section 2.3.22: home and work convert to the private and work contexts"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Addresses,
            "a1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Billing)),
            Ok(JSContactProperty::Context(Context::Delivery)),
        ]),
        "RFC 9553 Section 2.5.1: billing and delivery address contexts"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "t1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Private)),
            Err("x-car".to_string()),
            Err("example.com:boat".to_string()),
        ]),
        "RFC 9553 Section 1.8.2: unknown and vendor-specific contexts stay untyped"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "t1",
            JSContactProperty::Features
        ),
        None,
        "RFC 9555 Section 2.7.6: features are only set for TEL-specific TYPE values"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::OnlineServices,
            "i1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Private)),
            Err("mobile".to_string()),
        ]),
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Directories,
            "d1",
            JSContactProperty::Contexts
        ),
        Some(vec![Ok(JSContactProperty::Context(Context::Work))]),
        "RFC 9555 Section 2.10.4: the TYPE parameter of ORG-DIRECTORY converts to contexts"
    );
}

#[test]
fn rfc9555_2_7_6_tel_type_imports_typed_features() {
    let card = import(concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
        "TEL;PROP-ID=t1;TYPE=cell,voice,fax,text,video,pager,textphone,main-number,work:",
        "tel:+1-555-555-5555\r\n",
        "END:VCARD\r\n"
    ));

    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "t1",
            JSContactProperty::Features
        ),
        Some(vec![
            Ok(JSContactProperty::Feature(Feature::Mobile)),
            Ok(JSContactProperty::Feature(Feature::Voice)),
            Ok(JSContactProperty::Feature(Feature::Fax)),
            Ok(JSContactProperty::Feature(Feature::Text)),
            Ok(JSContactProperty::Feature(Feature::Video)),
            Ok(JSContactProperty::Feature(Feature::Pager)),
            Ok(JSContactProperty::Feature(Feature::TextPhone)),
            Ok(JSContactProperty::Feature(Feature::MainNumber)),
        ]),
        "RFC 9555 Section 2.7.6, Table 3"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "t1",
            JSContactProperty::Contexts
        ),
        Some(vec![Ok(JSContactProperty::Context(Context::Work))]),
    );
}

#[test]
fn rfc9555_3_1_1_types_without_a_matching_type_value_round_trip_as_jsprop() {
    let json = r#"{
        "@type": "Card",
        "version": "1.0",
        "name": {"full": "Jane"},
        "phones": {
            "p1": {"number": "tel:1", "features": {"example.com:pager": true}},
            "p2": {
                "number": "tel:2",
                "features": {"voice": true, "example.com:pager": true, "private": true},
                "contexts": {"example.com:car": true, "voice": true}
            }
        },
        "emails": {
            "e1": {"address": "a@example.com", "contexts": {"home": true, "cell": true}}
        },
        "onlineServices": {
            "o1": {"uri": "xmpp:a@example.com", "contexts": {"work": true, "Work": true}}
        },
        "addresses": {
            "a1": {"full": "1 Main St", "contexts": {"billing": true, "example.com:office": true}}
        },
        "directories": {
            "d1": {"kind": "directory", "uri": "https://d.example.com", "contexts": {"work": true, "example.com:dir": true}},
            "d2": {"kind": "entry", "uri": "https://e.example.com", "contexts": {"work": true}}
        },
        "titles": {
            "t1": {"kind": "title", "name": "Boss", "contexts": {"work": true}}
        }
    }"#;
    let exported = export(json);
    let card = import(&exported);

    let expected: JsonValue = serde_json::from_str(json).expect("valid JSON");
    let reimported: JsonValue = serde_json::from_str(&card.to_string_pretty()).expect("valid JSON");
    assert_eq!(normalize(&reimported), normalize(&expected), "{exported}");

    for unexpected in [
        "EXAMPLE.COM",
        "TYPE=HOME",
        "CELL",
        "PRIVATE",
        "SOURCE;TYPE",
        "TITLE;TYPE",
    ] {
        assert!(!exported.contains(unexpected), "{unexpected}\n{exported}");
    }
    for expected in [
        "TEL;TYPE=VOICE;PROP-ID=p2:tel:2",
        "JSPROP;JSPTR=\"phones/p1/features\":{\"example.com:pager\":true}",
        "JSPROP;JSPTR=\"addresses/a1/contexts/example.com:office\":true",
        "ORG-DIRECTORY;TYPE=WORK;PROP-ID=d1:https://d.example.com",
        "JSPROP;JSPTR=\"directories/d1/contexts/example.com:dir\":true",
        "JSPROP;JSPTR=\"directories/d2/contexts\":{\"work\":true}",
        "JSPROP;JSPTR=\"titles/t1/contexts\":{\"work\":true}",
    ] {
        assert!(exported.contains(expected), "{expected}\n{exported}");
    }

    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "p1",
            JSContactProperty::Features
        ),
        Some(vec![Err("example.com:pager".to_string())]),
        "RFC 9553 Section 1.8.2: vendor-specific features stay features\n{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "p2",
            JSContactProperty::Features
        ),
        Some(vec![
            Ok(JSContactProperty::Feature(Feature::Voice)),
            Err("example.com:pager".to_string()),
            Err("private".to_string()),
        ]),
        "{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Phones,
            "p2",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Err("example.com:car".to_string()),
            Err("voice".to_string()),
        ]),
        "{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Emails,
            "e1",
            JSContactProperty::Contexts
        ),
        Some(vec![Err("home".to_string()), Err("cell".to_string())]),
        "{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::OnlineServices,
            "o1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Work)),
            Err("Work".to_string()),
        ]),
        "{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Directories,
            "d1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Work)),
            Err("example.com:dir".to_string()),
        ]),
        "RFC 9555 Section 2.10.4\n{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Directories,
            "d2",
            JSContactProperty::Contexts
        ),
        Some(vec![Ok(JSContactProperty::Context(Context::Work))]),
        "RFC 6350 Section 6.1.3: SOURCE has no TYPE parameter\n{exported}"
    );
    assert_eq!(
        typed_keys(
            &card.0,
            JSContactProperty::Titles,
            "t1",
            JSContactProperty::Contexts
        ),
        Some(vec![Ok(JSContactProperty::Context(Context::Work))]),
        "RFC 9553 Section 2.2.5: Title has no contexts property\n{exported}"
    );
}

fn localized_view(card: &JsonValue, language: &str) -> JsonValue {
    let mut view = normalize(card);
    if let Some(obj) = view.as_object_mut() {
        obj.remove("localizations");
    }
    for (pointer, patch) in card
        .pointer(&format!("/localizations/{language}"))
        .and_then(JsonValue::as_object)
        .into_iter()
        .flatten()
    {
        let (parent, member) = pointer.rsplit_once('/').unwrap_or(("", pointer.as_str()));
        if let Some(parent) = view
            .pointer_mut(&format!("/{parent}"))
            .and_then(JsonValue::as_object_mut)
        {
            parent.insert(member.to_string(), patch.clone());
        }
    }
    view
}

#[test]
fn rfc9553_1_4_3_localized_contexts_and_features_form_a_valid_patch_object() {
    let json = r#"{
        "@type": "Card",
        "version": "1.0",
        "name": {"full": "Jane"},
        "addresses": {"a1": {"full": "1 Main St", "contexts": {"work": true}}},
        "phones": {"p1": {"number": "tel:1", "features": {"voice": true}}},
        "nicknames": {"n1": {"name": "Nick", "contexts": {"work": true}}},
        "localizations": {
            "fr": {
                "addresses/a1": {
                    "full": "1 rue Principale",
                    "contexts": {"example.com:x": true, "Work": true, "work": true}
                },
                "phones/p1": {
                    "number": "tel:2",
                    "features": {"voice": true, "example.com:pager": true},
                    "contexts": {"private": true}
                },
                "nicknames/n1": {"name": "Nicolas", "contexts": {"x-car": true}}
            }
        }
    }"#;
    let exported = export(json);
    let reimported: JsonValue =
        serde_json::from_str(&import(&exported).to_string_pretty()).expect("valid JSON");
    let expected: JsonValue = serde_json::from_str(json).expect("valid JSON");

    let patches = reimported
        .pointer("/localizations/fr")
        .and_then(JsonValue::as_object)
        .expect("localized patches");
    for pointer in patches.keys() {
        let prefix = format!("{pointer}/");
        assert!(
            !patches.keys().any(|other| other.starts_with(&prefix)),
            "RFC 9553 Section 1.4.3: {pointer} is a prefix of another patch\n{exported}\n{reimported:#}"
        );
    }
    let (reimported, expected) = (
        localized_view(&reimported, "fr"),
        localized_view(&expected, "fr"),
    );
    for pointer in ["/addresses", "/nicknames"] {
        assert_eq!(
            reimported.pointer(pointer),
            expected.pointer(pointer),
            "{pointer}\n{exported}"
        );
    }
    for expected in [
        "ADR;LABEL=\"1 rue Principale\";TYPE=WORK;PROP-ID=a1;LANGUAGE=fr:",
        "JSPROP;JSPTR=\"localizations/fr/addresses~1a1~1contexts\":",
        "JSPROP;JSPTR=\"localizations/fr/phones~1p1~1features\":",
        "NICKNAME;TYPE=X-CAR;PROP-ID=n1;LANGUAGE=fr:Nicolas",
    ] {
        assert!(exported.contains(expected), "{expected}\n{exported}");
    }
}

#[test]
fn rfc6350_5_6_contexts_that_are_not_type_values_round_trip_as_jsprop() {
    let json = r#"{
        "@type": "Card",
        "version": "1.0",
        "name": {"full": "Jane"},
        "emails": {
            "e1": {"address": "a@example.com", "contexts": {"example.com:a,b": true}},
            "e2": {
                "address": "b@example.com",
                "contexts": {"work": true, "example.com:a;b": true, "example.com:a b": true, "x-car": true}
            },
            "e3": {"address": "c@example.com", "contexts": {"work": true, "a\u0001b": true, "a\\b": true}},
            "e4": {"address": "d@example.com", "contexts": {"WORK": true, "x.y": true, "": true}}
        },
        "cryptoKeys": {
            "k1": {"uri": "https://k.example.com", "contexts": {"cell": true, "example.com:a,b": true}}
        }
    }"#;
    let exported = export(json);
    let card = import(&exported);

    let expected: JsonValue = serde_json::from_str(json).expect("valid JSON");
    let reimported: JsonValue = serde_json::from_str(&card.to_string_pretty()).expect("valid JSON");
    assert_eq!(normalize(&reimported), normalize(&expected), "{exported}");

    let vcard = VCard::parse(&exported).expect("valid vCard");
    for value in vcard
        .entries
        .iter()
        .flat_map(|entry| entry.params.iter())
        .filter(|param| param.name == VCardParameterName::Type)
        .map(|param| param.value.as_text().unwrap_or_default())
    {
        assert!(
            !value.is_empty()
                && value
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-'),
            "RFC 6350 Section 5.6: {value:?} is not a TYPE value\n{exported}"
        );
    }
}

#[test]
fn rfc6350_5_6_quoted_type_list_keeps_unknown_values() {
    let vcard = VCard::parse(concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
        "EMAIL;PROP-ID=e1;TYPE=\"work, x-foo\",home:jane@example.com\r\n",
        "END:VCARD\r\n"
    ))
    .expect("valid vCard");
    let types = vcard
        .entries
        .iter()
        .flat_map(|entry| entry.params.iter())
        .filter(|param| param.name == VCardParameterName::Type)
        .filter_map(|param| param.value.as_text())
        .collect::<Vec<_>>();
    assert_eq!(types, ["WORK", "x-foo", "HOME"], "{vcard}");

    assert_eq!(
        typed_keys(
            &vcard.into_jscontact::<String, String>().0,
            JSContactProperty::Emails,
            "e1",
            JSContactProperty::Contexts
        ),
        Some(vec![
            Ok(JSContactProperty::Context(Context::Work)),
            Err("x-foo".to_string()),
            Ok(JSContactProperty::Context(Context::Private)),
        ]),
    );
}

#[test]
fn rfc8620_1_5_repeated_type_values_do_not_repeat_members() {
    let card = import(concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
        "TEL;PROP-ID=t1;TYPE=home,private;TYPE=cell,CELL:tel:1\r\n",
        "RELATED;TYPE=friend,FRIEND:urn:uuid:f81d4fae-7dec-11d0-a765-00a0c91e6bf6\r\n",
        "END:VCARD\r\n"
    ));
    let json = serde_json::to_string(&card.0).expect("serializable");
    for member in ["\"private\"", "\"mobile\"", "\"friend\""] {
        assert_eq!(
            json.matches(member).count(),
            1,
            "RFC 8620 Section 1.5 (I-JSON): {member}\n{json}"
        );
    }
}

#[test]
fn rfc9553_2_2_1_1_an_empty_n_does_not_create_a_name() {
    let card = import("BEGIN:VCARD\r\nVERSION:4.0\r\nN:;;;;\r\nEND:VCARD\r\n");
    let json = serde_json::to_string(&card.0).expect("serializable");
    assert!(
        !json.contains("\"name\""),
        "RFC 9553 Section 2.2.1.1: the components property MUST be set if the full property is not set\n{json}"
    );

    for (vcard, expected) in [
        (
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\nN:;;;;\r\nEND:VCARD\r\n",
            "\"full\"",
        ),
        (
            "BEGIN:VCARD\r\nVERSION:4.0\r\nN:Doe;Jane;;;\r\nEND:VCARD\r\n",
            "\"components\"",
        ),
    ] {
        let json = serde_json::to_string(&import(vcard).0).expect("serializable");
        assert!(json.contains(expected), "{vcard}\n{json}");
    }
}

#[test]
fn rfc9553_1_5_1_contexts_that_are_not_true_are_preserved() {
    let vcard = export(
        r#"{"@type": "Card", "version": "1.0", "name": {"full": "T"},
            "emails": {"e1": {"@type": "EmailAddress", "address": "a@b.c",
                "contexts": {"work": false, "private": true}}},
            "phones": {"p1": {"@type": "Phone", "number": "tel:+1",
                "contexts": {"work": false}, "features": {"voice": false}}}}"#,
    );
    assert!(vcard.contains("EMAIL;TYPE=HOME;"), "{vcard}");
    assert!(
        vcard.contains(r#"JSPROP;JSPTR="emails/e1/contexts/work":false"#),
        "RFC 9553 Section 1.5.1: the values in a set MUST be true, so a false entry has no TYPE\n{vcard}"
    );
    assert!(
        vcard.contains(r#"JSPROP;JSPTR="phones/p1/contexts":{"work":false}"#),
        "{vcard}"
    );

    let json = serde_json::to_string(&import(&vcard).0).expect("serializable");
    for expected in [r#""work":false"#, r#""private":true"#, r#""voice":false"#] {
        assert!(json.contains(expected), "missing {expected}\n{json}");
    }
}
