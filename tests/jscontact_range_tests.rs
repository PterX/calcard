/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{jscontact::JSContact, vcard::VCard};

fn card(members: &str) -> String {
    format!(
        r#"{{"@type": "Card", "version": "1.0", "uid": "u1", "name": {{"full": "T"}}, {members}}}"#
    )
}

fn export(members: &str) -> String {
    JSContact::<String, String>::parse(&card(members))
        .expect("valid JSContact")
        .into_vcard()
        .expect("exports to vCard")
        .to_string()
}

fn roundtrip(vcard: &str) -> String {
    VCard::parse(vcard)
        .expect("valid vCard")
        .into_jscontact::<String, String>()
        .to_string_pretty()
}

#[test]
fn rfc6350_5_3_pref_outside_1_to_100_becomes_a_jsprop() {
    for members in [
        r#""emails": {"e1": {"@type": "EmailAddress", "address": "a@b.c", "pref": 500}}"#,
        r#""phones": {"p1": {"@type": "Phone", "number": "tel:+1", "pref": 0}}"#,
        r#""media": {"m1": {"@type": "Media", "kind": "photo", "uri": "https://e.com/a.png", "pref": 101}}"#,
        r#""addresses": {"a1": {"@type": "Address", "full": "X", "pref": 500}}"#,
        r#""nicknames": {"n1": {"@type": "Nickname", "name": "Jo", "pref": 500}}"#,
        r#""titles": {"t1": {"@type": "Title", "name": "Boss", "pref": 500}}"#,
        r#""onlineServices": {"o1": {"@type": "OnlineService", "uri": "https://e.com/u", "pref": 500}}"#,
        r#""calendars": {"c1": {"@type": "Calendar", "kind": "calendar", "uri": "https://e.com/c", "pref": 500}}"#,
        r#""links": {"l1": {"@type": "Link", "uri": "https://e.com/l", "pref": 500}}"#,
        r#""directories": {"d1": {"@type": "Directory", "kind": "directory", "uri": "https://e.com/d", "pref": 500}}"#,
        r#""cryptoKeys": {"k1": {"@type": "CryptoKey", "uri": "https://e.com/k", "pref": 500}}"#,
        r#""speakToAs": {"@type": "SpeakToAs", "pronouns": {"k1": {"@type": "Pronouns", "pronouns": "they", "pref": 500}}}"#,
    ] {
        let exported = export(members);
        assert!(
            !exported.contains("PREF="),
            "RFC 6350 Section 5.3: PREF is an integer between 1 and 100\n{members}\n{exported}"
        );
        assert!(exported.contains("JSPROP"), "{members}\n{exported}");
        assert!(
            roundtrip(&exported).contains("\"pref\""),
            "the value survives the round trip\n{members}\n{exported}"
        );
    }

    let exported = export(
        r#""emails": {"e1": {"@type": "EmailAddress", "address": "a@b.c", "pref": 1}},
           "phones": {"p1": {"@type": "Phone", "number": "tel:+1", "pref": 100}}"#,
    );
    assert!(exported.contains("EMAIL;PREF=1;"), "{exported}");
    assert!(exported.contains("TEL;PREF=100;"), "{exported}");
}

#[test]
fn rfc6715_3_1_index_must_be_strictly_positive() {
    for (members, expected) in [
        (
            r#""directories": {"d1": {"@type": "Directory", "kind": "directory", "uri": "https://e.com/d", "listAs": 0}}"#,
            "0",
        ),
        (
            r#""directories": {"d1": {"@type": "Directory", "kind": "directory", "uri": "https://e.com/d", "listAs": 4294967296}}"#,
            "4294967296",
        ),
    ] {
        let exported = export(members);
        assert!(
            !exported.contains("INDEX="),
            "RFC 6715 Section 3.1: INDEX values must be strictly positive\n{exported}"
        );
        assert!(
            roundtrip(&exported).contains(expected),
            "the value survives the round trip\n{exported}"
        );
    }

    let exported = export(
        r#""directories": {"d1": {"@type": "Directory", "kind": "directory", "uri": "https://e.com/d", "listAs": 2}}"#,
    );
    assert!(exported.contains("INDEX=2"), "{exported}");
}

#[test]
fn rfc9553_2_8_1_out_of_range_anniversary_dates_become_a_jsprop() {
    for (members, expected) in [
        (
            r#""anniversaries": {"a1": {"@type": "Anniversary", "kind": "birth", "date": {"@type": "PartialDate", "month": 2, "day": 40}}}"#,
            "\"day\": 40",
        ),
        (
            r#""anniversaries": {"a1": {"@type": "Anniversary", "kind": "birth", "date": {"@type": "PartialDate", "year": 2000, "month": 13}}}"#,
            "\"month\": 13",
        ),
        (
            r#""anniversaries": {"a1": {"@type": "Anniversary", "kind": "birth", "date": {"@type": "PartialDate", "year": 12345, "month": 1, "day": 2}}}"#,
            "\"year\": 12345",
        ),
        (
            r#""anniversaries": {"a1": {"@type": "Anniversary", "kind": "birth", "date": {"@type": "PartialDate", "year": 2000, "month": 0}}}"#,
            "\"month\": 0",
        ),
    ] {
        let exported = export(members);
        assert!(
            !exported.contains("BDAY:") && !exported.contains("BDAY;"),
            "RFC 9553 Section 2.8.1: 1 <= month <= 12 and 1 <= day <= 31\n{exported}"
        );
        assert!(
            roundtrip(&exported).contains(expected),
            "the value survives the round trip\n{exported}"
        );
    }

    let exported = export(
        r#""anniversaries": {"a1": {"@type": "Anniversary", "kind": "birth", "date": {"@type": "PartialDate", "year": 1953, "month": 4, "day": 15}}}"#,
    );
    assert!(exported.contains("BDAY"), "{exported}");
    assert!(exported.contains("19530415"), "{exported}");
}
