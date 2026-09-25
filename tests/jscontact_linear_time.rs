/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{jscontact::JSContact, vcard::VCard};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const SMALL: usize = 4_000;
const LARGE: usize = 16_000;
const MAX_RATIO: f64 = 9.0;

struct Timing(fn(usize) -> Box<dyn Fn()>);

impl Timing {
    fn fastest(&self, count: usize) -> Duration {
        let run = (self.0)(count);
        (0..3)
            .map(|_| {
                let start = Instant::now();
                run();
                start.elapsed()
            })
            .min()
            .unwrap_or_default()
    }

    fn assert_linear(&self, name: &str) {
        let small = self.fastest(SMALL);
        let large = self.fastest(LARGE);
        let ratio = large.as_secs_f64() / small.as_secs_f64().max(f64::MIN_POSITIVE);
        assert!(
            ratio < MAX_RATIO,
            "{name}: 4x the input took {ratio:.1}x the time ({small:?} -> {large:?})"
        );
    }
}

fn card_lines(count: usize, line: impl Fn(usize) -> String) -> VCard {
    let mut input = String::from("BEGIN:VCARD\r\nVERSION:4.0\r\nFN:x\r\n");
    for index in 0..count {
        input.push_str(&line(index));
    }
    input.push_str("END:VCARD\r\n");
    VCard::parse(&input).expect("valid card")
}

#[test]
#[ignore]
fn distinct_languages_import_in_linear_time() {
    Timing(|count| {
        let card = card_lines(count, |index| {
            format!("NOTE;LANGUAGE=l{index}:note {index}\r\n")
        });
        Box::new(move || {
            black_box(card.clone().into_jscontact::<String, String>());
        })
    })
    .assert_linear("LANGUAGE");
}

#[test]
#[ignore]
fn distinct_languages_export_in_linear_time() {
    Timing(|count| {
        let contact = card_lines(count, |index| {
            format!("NOTE;LANGUAGE=l{index}:note {index}\r\n")
        })
        .into_jscontact::<String, String>();
        Box::new(move || {
            black_box(contact.clone().into_vcard().expect("exported card"));
        })
    })
    .assert_linear("LANGUAGE export");
}

#[test]
#[ignore]
fn distinct_localized_property_names_import_in_linear_time() {
    Timing(|count| {
        let card = card_lines(count, |index| {
            format!("X-P{index};LANGUAGE=en:v {index}\r\n")
        });
        Box::new(move || {
            black_box(card.clone().into_jscontact::<String, String>());
        })
    })
    .assert_linear("X- names with LANGUAGE");
}

#[test]
#[ignore]
fn distinct_name_alt_ids_import_in_linear_time() {
    Timing(|count| {
        let card = card_lines(count, |index| format!("N;ALTID={index}:a{index};b;;;\r\n"));
        Box::new(move || {
            black_box(card.clone().into_jscontact::<String, String>());
        })
    })
    .assert_linear("N ALTID");
}

#[test]
#[ignore]
fn distinct_converted_localizations_import_in_linear_time() {
    Timing(|count| {
        let card = card_lines(count, |index| {
            format!("NOTE;LANGUAGE=l{index};X-A=b:note {index}\r\n")
        });
        Box::new(move || {
            black_box(card.clone().into_jscontact::<String, String>());
        })
    })
    .assert_linear("LANGUAGE with parameters");
}

#[test]
#[ignore]
fn distinct_parameters_of_one_entry_import_in_linear_time() {
    Timing(|count| {
        let parameters: String = (0..count)
            .map(|index| format!(";X-P{index}=v{index}"))
            .collect();
        let card = VCard::parse(format!(
            "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:x\r\nNOTE{parameters}:note\r\nEND:VCARD\r\n"
        ))
        .expect("valid card");
        Box::new(move || {
            black_box(card.clone().into_jscontact::<String, String>());
        })
    })
    .assert_linear("parameters of one entry");
}

#[test]
#[ignore]
fn distinct_converted_property_heads_export_in_linear_time() {
    Timing(|count| {
        let members: Vec<String> = (0..count)
            .map(|index| format!(r#""addressBookIds/a{index}":{{"name":"note"}}"#))
            .collect();
        let json = format!(
            r#"{{"@type":"Card","version":"1.0","name":{{"full":"x"}},"vCard":{{"convertedProperties":{{{}}}}}}}"#,
            members.join(",")
        );
        let contact = JSContact::<String, String>(
            JSContact::<String, String>::parse(&json)
                .expect("valid contact")
                .0
                .into_owned(),
        );
        Box::new(move || {
            black_box(contact.clone().into_vcard().expect("exported card"));
        })
    })
    .assert_linear("converted property heads");
}
