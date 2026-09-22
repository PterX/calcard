/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    common::export::ExportError,
    jscontact::{JSContact, export::ExportOptions, import::ImportOptions},
    vcard::{VCard, VCardProperty, VCardValue, VCardVersion},
};
use jmap_tools::{Key, Value};

const BLOB_VCARD4: &str = concat!(
    "BEGIN:VCARD\r\n",
    "VERSION:4.0\r\n",
    "FN:Jane Doe\r\n",
    "PHOTO:data:image/png;base64\\,iVBORw0KGgo=\r\n",
    "LOGO;PREF=1:data:image/jpeg;base64,/9j/4A==\r\n",
    "KEY:data:application/pgp-keys;base64,UklGRgD/\r\n",
    "END:VCARD\r\n"
);

const BLOB_VCARD3: &str = concat!(
    "BEGIN:VCARD\r\n",
    "VERSION:3.0\r\n",
    "FN:Jane Doe\r\n",
    "PHOTO;ENCODING=b;TYPE=JPEG:/9j/4A==\r\n",
    "SOUND;ENCODING=b;TYPE=WAVE:aGVsbG8gd29ybGQ=\r\n",
    "END:VCARD\r\n"
);

const BLOB_VCARD21: &str = concat!(
    "BEGIN:VCARD\r\n",
    "VERSION:2.1\r\n",
    "FN:Jane Doe\r\n",
    "PHOTO;TYPE=JPEG;ENCODING=BASE64:/9j/4A==\r\n",
    "LOGO;TYPE=GIF;BASE64:R0lGODlhAQABAAAAACw=\r\n",
    "END:VCARD\r\n"
);

type MediaEntry = (VCardProperty, Vec<u8>, Option<String>);

fn media_entries(vcard: &VCard) -> Vec<MediaEntry> {
    let mut entries = vcard
        .entries
        .iter()
        .filter_map(|entry| match entry.values.first() {
            Some(VCardValue::Binary(data))
                if matches!(
                    entry.name,
                    VCardProperty::Photo | VCardProperty::Logo | VCardProperty::Sound
                ) =>
            {
                Some((
                    entry.name.clone(),
                    data.data.clone(),
                    data.content_type.clone(),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    entries.sort_unstable();
    entries
}

fn import_blobs(vcard: &str) -> (JSContact<'static, String, String>, Vec<Vec<u8>>) {
    let mut blobs: Vec<Vec<u8>> = Vec::new();
    let jscontact = VCard::parse(vcard)
        .unwrap()
        .into_jscontact_with::<String, String, _>(ImportOptions::new().with_blob_ids(|data| {
            blobs.push(data.to_vec());
            Some(format!("blob{}", blobs.len() - 1))
        }))
        .expect("converts");
    (jscontact, blobs)
}

fn resolve(blobs: &[Vec<u8>], blob_id: &str) -> Option<Vec<u8>> {
    blob_id
        .strip_prefix("blob")
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| blobs.get(index))
        .cloned()
}

#[test]
fn blob_id_media_roundtrip() {
    for (input, num_blobs, version) in [
        (BLOB_VCARD4, 2, VCardVersion::V4_0),
        (BLOB_VCARD3, 2, VCardVersion::V3_0),
        (BLOB_VCARD21, 2, VCardVersion::V2_1),
    ] {
        let original = VCard::parse(input).unwrap();
        let (jscontact, blobs) = import_blobs(input);

        let json = jscontact.to_string_pretty();
        assert_eq!(blobs.len(), num_blobs, "{json}");
        assert_eq!(jscontact.blob_ids().count(), num_blobs, "{json}");
        let media = jscontact
            .0
            .as_object_and_get(&Key::Property(calcard::jscontact::JSContactProperty::Media))
            .and_then(|media| media.as_object())
            .unwrap();
        assert_eq!(media.len(), num_blobs, "{json}");
        for item in media.values() {
            assert!(
                matches!(
                    item.as_object_and_get(&Key::Property(
                        calcard::jscontact::JSContactProperty::BlobId
                    )),
                    Some(Value::Element(calcard::jscontact::JSContactValue::BlobId(
                        _
                    )))
                ),
                "{json}"
            );
            assert!(
                item.as_object_and_get(&Key::Property(
                    calcard::jscontact::JSContactProperty::MediaType
                ))
                .is_some(),
                "{json}"
            );
            assert!(
                item.as_object_and_get(&Key::Property(calcard::jscontact::JSContactProperty::Uri))
                    .is_none(),
                "{json}"
            );
        }

        let reparsed = JSContact::<String, String>::parse(&json).unwrap();
        for exported in [jscontact, reparsed] {
            let vcard = exported
                .into_vcard_with(
                    ExportOptions::new()
                        .with_blob_resolver(|blob_id: &String| resolve(&blobs, blob_id)),
                )
                .unwrap();
            assert_eq!(media_entries(&vcard), media_entries(&original), "{vcard}");

            let mut serialized = String::new();
            vcard.write_to(&mut serialized, version).unwrap();
            assert_eq!(
                media_entries(&VCard::parse(&serialized).unwrap()),
                media_entries(&original),
                "{serialized}"
            );
            if version == VCardVersion::V2_1 {
                assert!(serialized.contains("ENCODING=BASE64"), "{serialized}");
                assert!(!serialized.contains("ENCODING=b"), "{serialized}");
            }
        }
    }
}

#[test]
fn blob_id_media_type_is_always_set() {
    let (jscontact, _) = import_blobs(concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:T\r\n",
        "PHOTO:data:;base64,iVBORw0KGgoAAAAN\r\n",
        "SOUND:data:;base64,T2dnUwACAAAA\r\n",
        "LOGO:data:;base64,AAECAwQF\r\n",
        "END:VCARD\r\n"
    ));
    let json: serde_json::Value = serde_json::from_str(&jscontact.to_string_pretty()).unwrap();
    let mut media_types = json["media"]
        .as_object()
        .unwrap()
        .values()
        .filter_map(|media| media["mediaType"].as_str())
        .collect::<Vec<_>>();
    media_types.sort_unstable();
    assert_eq!(
        media_types,
        ["application/octet-stream", "audio/ogg", "image/png"],
        "{json}"
    );
}

#[test]
fn blob_id_media_generator_declines() {
    let jscontact = VCard::parse(BLOB_VCARD4)
        .unwrap()
        .into_jscontact_with::<String, String, _>(
            ImportOptions::new().with_blob_ids(|data| (data.len() > 4).then(|| "png".to_string())),
        )
        .expect("converts");
    let json = jscontact.to_string_pretty();
    assert_eq!(jscontact.blob_ids().count(), 1, "{json}");
    assert!(
        json.contains("\"uri\": \"data:image/jpeg;base64,/9j/4A==\""),
        "{json}"
    );
}

#[test]
fn blob_id_media_unresolved_blob_is_an_export_error() {
    let (jscontact, blobs) = import_blobs(BLOB_VCARD4);

    assert_eq!(
        jscontact
            .clone()
            .into_vcard_with(ExportOptions::new().with_blob_resolver(|_: &String| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "blob0".to_string()
        }),
        "RFC 9610 Section 3: a Media blobId that cannot be resolved has no vCard representation"
    );
    assert!(
        jscontact
            .clone()
            .into_vcard_with(ExportOptions::new().with_blob_resolver(|blob_id: &String| {
                (blob_id == "blob0")
                    .then(|| resolve(&blobs, blob_id))
                    .flatten()
            }))
            .is_err()
    );
    assert!(
        jscontact
            .into_vcard_with(
                ExportOptions::new()
                    .with_blob_resolver(|blob_id: &String| resolve(&blobs, blob_id))
            )
            .is_ok()
    );

    assert_eq!(
        JSContact::<String, String>::parse(
            r#"{"@type": "Card", "version": "1.0", "name": {"full": "T"},
            "media": {"p": {"@type": "Media", "kind": "photo", "blobId": "missing", "mediaType": "image/png",
                            "contexts": {"private": true}, "example.com:x": 1}}}"#,
        )
        .expect("valid JSContact")
        .into_vcard_with(ExportOptions::new().with_blob_resolver(|_: &String| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "missing".to_string()
        })
    );
}

#[test]
fn media_embedded_size_budget_is_an_export_error() {
    let (jscontact, blobs) = import_blobs(BLOB_VCARD4);

    assert_eq!(
        jscontact.into_vcard_with(
            ExportOptions::new()
                .max_embedded_size(4)
                .with_blob_resolver(|blob_id: &String| resolve(&blobs, blob_id))
        ),
        Err(ExportError::EmbeddedSizeExceeded { max: 4 })
    );
}

#[test]
fn crypto_keys_count_toward_the_embedded_size_budget() {
    let card = r#"{"@type": "Card", "version": "1.0", "name": {"full": "T"},
            "cryptoKeys": {"k1": {"@type": "CryptoKey", "uri": "data:application/pgp-keys;base64,aGVsbG8="},
                           "k2": {"@type": "CryptoKey", "uri": "data:application/pgp-keys;base64,aGVsbG8="}}}"#;
    let export = |max_embedded_size: usize| {
        JSContact::<String, String>::parse(card)
            .expect("valid JSContact")
            .into_vcard_with(ExportOptions::new().max_embedded_size(max_embedded_size))
    };

    assert_eq!(export(4), Err(ExportError::EmbeddedSizeExceeded { max: 4 }));
    let vcard = export(5).expect("the same key is charged once").to_string();
    assert_eq!(vcard.matches("aGVsbG8=").count(), 2, "{vcard}");
}

#[test]
fn blob_id_media_with_uri() {
    let jscontact = JSContact::<String, String>::parse(
        r#"{"@type": "Card", "version": "1.0", "name": {"full": "T"},
            "media": {
                "p": {"kind": "photo", "blobId": "blob0", "uri": "https://example.com/p.png", "mediaType": "image/png"},
                "l": {"kind": "logo", "blobId": "blob0", "uri": "data:image/png;base64,AAAA", "mediaType": "image/png"}
            }}"#,
    )
    .expect("valid JSContact");
    let blobs = vec![vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]];
    let vcard = jscontact
        .into_vcard_with(
            ExportOptions::new().with_blob_resolver(|blob_id: &String| resolve(&blobs, blob_id)),
        )
        .expect("JSContact exports to vCard");
    let rendered = vcard.to_string();

    assert_eq!(
        media_entries(&vcard),
        [
            (
                VCardProperty::Photo,
                blobs[0].clone(),
                Some("image/png".to_string())
            ),
            (
                VCardProperty::Logo,
                blobs[0].clone(),
                Some("image/png".to_string())
            ),
        ],
        "{rendered}"
    );
    assert!(
        rendered.contains("JSPROP;JSPTR=\"media/p/uri\":\"https://example.com/p.png\""),
        "RFC 9610 Section 3: the blob is the property value and the uri is kept\n{rendered}"
    );
    assert!(!rendered.contains("media/l/uri"), "{rendered}");

    let (reimported, reimported_blobs) = import_blobs(&rendered);
    let json: serde_json::Value =
        serde_json::from_str(&reimported.to_string_pretty()).expect("valid JSON");
    assert_eq!(reimported_blobs, blobs, "{json}");
    assert_eq!(
        json["media"]["p"]["uri"].as_str(),
        Some("https://example.com/p.png"),
        "{json}"
    );
    assert_eq!(
        json["media"]["p"]["blobId"].as_str(),
        Some("blob0"),
        "{json}"
    );
    assert!(json["media"]["l"].get("uri").is_none(), "{json}");
    assert_eq!(
        json["media"]["l"]["blobId"].as_str(),
        Some("blob0"),
        "{json}"
    );
}

#[test]
fn blob_id_media_type_parameter_without_value() {
    let (jscontact, _) = import_blobs(concat!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:T\r\n",
        "PHOTO;PROP-ID=p1;MEDIATYPE:data:image/png;base64,iVBORw0KGgo=\r\n",
        "LOGO;PROP-ID=l1;MEDIATYPE=:data:image/png;base64,iVBORw0KGgoAAAA=\r\n",
        "SOUND;PROP-ID=s1;MEDIATYPE=;MEDIATYPE=audio/ogg:data:;base64,T2dnUwACAAAA\r\n",
        "END:VCARD\r\n"
    ));
    let json = serde_json::to_string(&jscontact.0).expect("serializable");
    assert_eq!(
        json.matches("\"mediaType\"").count(),
        3,
        "RFC 8620 Section 1.5 (I-JSON)\n{json}"
    );
    let json: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    for (id, media_type) in [
        ("p1", "image/png"),
        ("l1", "image/png"),
        ("s1", "audio/ogg"),
    ] {
        assert_eq!(
            json["media"][id]["mediaType"].as_str(),
            Some(media_type),
            "RFC 9610 Section 3: a Media with a blobId has a mediaType\n{json}"
        );
    }
}

#[test]
fn crypto_key_data_uri_exports_inline_binary() {
    let uri = "data:application/pgp-keys;base64,LS0tLS1CRUdJTiBQR1A=";
    let vcard = JSContact::<String, String>::parse(&format!(
        r#"{{"@type": "Card", "version": "1.0", "name": {{"full": "T"}},
            "cryptoKeys": {{"k1": {{"uri": "{uri}"}}, "k2": {{"uri": "https://example.com/k.asc"}}}}}}"#
    ))
    .expect("valid JSContact")
    .into_vcard()
    .expect("JSContact exports to vCard");

    for (version, expected) in [
        (
            VCardVersion::V3_0,
            "KEY;PROP-ID=k1;ENCODING=b;TYPE=PGP:LS0tLS1CRUdJTiBQR1A=",
        ),
        (
            VCardVersion::V2_1,
            "KEY;PROP-ID=k1;ENCODING=BASE64;TYPE=PGP:LS0tLS1CRUdJTiBQR1A=\r\n\r\n",
        ),
        (
            VCardVersion::V4_0,
            "KEY;PROP-ID=k1:data:application/pgp-keys;base64\\,LS0tLS1CRUdJTiBQR1A=",
        ),
    ] {
        let mut serialized = String::new();
        vcard
            .write_to(&mut serialized, version)
            .expect("serializable vCard");
        assert!(serialized.contains(expected), "{expected}\n{serialized}");
        assert!(
            serialized.contains("KEY;PROP-ID=k2:https://example.com/k.asc"),
            "{serialized}"
        );

        let json: serde_json::Value = serde_json::from_str(
            &VCard::parse(&serialized)
                .expect("valid vCard")
                .into_jscontact::<String, String>()
                .to_string_pretty(),
        )
        .expect("valid JSON");
        assert_eq!(
            json["cryptoKeys"]["k1"]["uri"].as_str(),
            Some(uri),
            "{serialized}\n{json}"
        );
    }
}

#[test]
fn blob_id_media_without_options() {
    for input in [BLOB_VCARD4, BLOB_VCARD3] {
        let default = VCard::parse(input)
            .unwrap()
            .into_jscontact::<String, String>();
        let json = default.to_string_pretty();
        assert!(!json.contains("blobId"), "{json}");
        assert_eq!(default.blob_ids().count(), 0);
        assert!(
            json.contains("\"uri\": \"data:image/jpeg;base64,/9j/4A==\""),
            "{json}"
        );
    }

    let card = r#"{"@type": "Card", "version": "1.0", "name": {"full": "T"},
            "media": {"p": {"@type": "Media", "kind": "photo", "blobId": "blob3", "mediaType": "image/png"}}}"#;
    let exported = JSContact::<String, String>::parse(card)
        .unwrap()
        .into_vcard()
        .expect("RFC 9610 Section 3: a blobId is the normal shape of a JMAP card")
        .to_string();
    assert!(exported.contains("blob3"), "{exported}");
    let roundtrip = VCard::parse(&exported)
        .unwrap()
        .into_jscontact::<String, String>()
        .to_string_pretty();
    assert!(roundtrip.contains(r#""blobId": "blob3""#), "{roundtrip}");

    assert_eq!(
        JSContact::<String, String>::parse(card)
            .unwrap()
            .into_vcard_with(ExportOptions::new().with_blob_resolver(|_: &String| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "blob3".to_string()
        }),
        "a resolver that cannot resolve a blobId is an export error"
    );
}

#[test]
fn blob_ids_include_localizations() {
    let jscontact = JSContact::<String, String>::parse(
        r#"{"@type": "Card", "version": "1.0",
            "media": {"p": {"@type": "Media", "kind": "photo", "blobId": "base", "mediaType": "image/png"}},
            "localizations": {"fr": {"media/p/blobId": "localized", "media/q": {"@type": "Media", "kind": "logo", "blobId": "nested"}}}}"#,
    )
    .unwrap();
    let mut blob_ids = jscontact.blob_ids().map(String::as_str).collect::<Vec<_>>();
    blob_ids.sort_unstable();
    assert_eq!(blob_ids, ["base", "localized", "nested"]);
}

#[derive(Default)]
struct MovedBinaries {
    buffers: Vec<*const u8>,
    calls: usize,
}

impl calcard::common::blob::BlobIdGenerator<String> for MovedBinaries {
    fn blob_id(
        &mut self,
        data: Vec<u8>,
        _: Option<&str>,
    ) -> calcard::common::blob::BlobIdOutcome<String> {
        self.buffers.push(data.as_ptr());
        self.calls += 1;
        calcard::common::blob::BlobIdOutcome::Generated(format!("blob{}", self.calls))
    }
}

#[test]
fn media_binaries_are_moved_into_the_generator() {
    let vcard = VCard::parse(BLOB_VCARD4).expect("valid vCard");
    let mut expected = vcard
        .blob_binaries()
        .map(<[u8]>::as_ptr)
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 2);

    let mut generator = MovedBinaries::default();
    vcard
        .into_jscontact_with::<String, String, _>(
            ImportOptions::new().with_blob_id_generator(&mut generator),
        )
        .expect("converts");

    let mut buffers = generator.buffers;
    buffers.sort_unstable();
    expected.sort_unstable();
    assert_eq!(
        buffers, expected,
        "a binary whose size occurs once is moved into the generator instead of copied"
    );
}

#[test]
fn identical_media_binaries_call_the_generator_once() {
    let vcard = VCard::parse(concat!(
        "BEGIN:VCARD\r\n",
        "VERSION:4.0\r\n",
        "FN:Twins\r\n",
        "PHOTO:data:image/png;base64,iVBORw0KGgo=\r\n",
        "LOGO:data:image/png;base64,iVBORw0KGgo=\r\n",
        "SOUND:data:audio/ogg;base64,T2dnUwA=\r\n",
        "END:VCARD\r\n"
    ))
    .expect("valid vCard");

    let mut generator = MovedBinaries::default();
    let jscontact = vcard
        .into_jscontact_with::<String, String, _>(
            ImportOptions::new().with_blob_id_generator(&mut generator),
        )
        .expect("converts");

    assert_eq!(
        generator.calls, 2,
        "one generator call per distinct content"
    );
    assert_eq!(jscontact.blob_ids().count(), 3);
}

#[test]
fn an_unparseable_media_blob_id_is_an_export_error() {
    let json = r#"{"@type": "Card", "version": "1.0", "name": {"full": "T"},
        "media": {"p": {"@type": "Media", "kind": "photo", "blobId": "not-a-blob", "mediaType": "image/png"}}}"#;

    assert_eq!(
        JSContact::<String, u32>::parse(json)
            .unwrap()
            .into_vcard_with(ExportOptions::new().with_blob_resolver(|_: &u32| None)),
        Err(ExportError::UnresolvedBlob {
            blob_id: "not-a-blob".to_string()
        })
    );
}
