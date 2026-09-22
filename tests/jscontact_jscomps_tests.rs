/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{Entry, Parser};
use serde_json::{Value as JsonValue, json};

fn import(input: &str) -> JsonValue {
    let Entry::VCard(vcard) = Parser::new(input).entry() else {
        panic!("expected a vCard: {input:?}");
    };
    serde_json::from_str(&vcard.into_jscontact::<String, String>().to_string_pretty())
        .expect("valid JSON")
}

#[test]
fn rfc9555_3_3_1_jscomps_without_positional_entries_is_not_ordered() {
    for (input, pointer, components) in [
        (
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\nN;JSCOMPS=\";s,-\":",
            "/name",
            None,
        ),
        (
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\nADR;PROP-ID=a1;JSCOMPS=\";s,-\":",
            "/addresses/a1",
            None,
        ),
        (
            concat!(
                "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
                "N;JSCOMPS=\"s,-;s,+\":Doe;Jane;;;;;\r\n",
                "END:VCARD\r\n"
            ),
            "/name",
            Some(json!([
                {"kind": "surname", "value": "Doe"},
                {"kind": "given", "value": "Jane"}
            ])),
        ),
        (
            concat!(
                "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Jane\r\n",
                "ADR;PROP-ID=a1;JSCOMPS=\"s,-;s,+\":;;Main St;Town;;;\r\n",
                "END:VCARD\r\n"
            ),
            "/addresses/a1",
            Some(json!([
                {"kind": "name", "value": "Main St"},
                {"kind": "locality", "value": "Town"}
            ])),
        ),
    ] {
        let card = import(input);
        let object = card
            .pointer(pointer)
            .and_then(JsonValue::as_object)
            .unwrap_or_else(|| panic!("missing {pointer} for {input:?}\n{card:#}"));

        assert_eq!(
            object.get("components"),
            components.as_ref(),
            "RFC 9553 Sections 2.2.1.1 and 2.5.1.1: {input:?}\n{card:#}"
        );
        for member in ["isOrdered", "defaultSeparator"] {
            assert!(
                !object.contains_key(member),
                "RFC 9555 Section 3.3.1: {member} for {input:?}\n{card:#}"
            );
        }
    }
}
