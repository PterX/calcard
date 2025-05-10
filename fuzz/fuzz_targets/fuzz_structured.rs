#![no_main]
use calcard::{Entry, Parser};
use libfuzzer_sys::fuzz_target;

const ICAL_VCARD_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\
    :;,=\r\n \t.+-_@/\\\"'()[]{}*#$%&!?<>|~^`";

const COMMON_PROPERTIES: &[&str] = &[
    "BEGIN",
    "END",
    "VERSION",
    "FN",
    "N",
    "EMAIL",
    "TEL",
    "ADR",
    "ORG",
    "TITLE",
    "NICKNAME",
    "URL",
    "BDAY",
    "NOTE",
    "UID",
    "REV",
    "PRODID",
    "CALSCALE",
    "METHOD",
    "DTSTART",
    "DTEND",
    "SUMMARY",
    "DESCRIPTION",
    "LOCATION",
    "RRULE",
    "EXDATE",
    "RDATE",
    "ATTENDEE",
    "ORGANIZER",
    "VCALENDAR",
    "VCARD",
    "VEVENT",
    "VTODO",
    "VJOURNAL",
    "VALARM",
];

fuzz_target!(|data: &[u8]| {
    let constrained_data: String = data
        .iter()
        .map(|&b| {
            *ICAL_VCARD_CHARS
                .get(b as usize % ICAL_VCARD_CHARS.len())
                .unwrap_or(&b'A') as char
        })
        .collect();

    let mut parser = Parser::new(&constrained_data);

    let mut iteration_count = 0;
    const MAX_ITERATIONS: usize = 10000;

    loop {
        if iteration_count >= MAX_ITERATIONS {
            break;
        }

        match parser.entry() {
            Entry::Eof => break,
            _ => {
                iteration_count += 1;
                continue;
            }
        }
    }

    let structured_input = generate_structured_input(data);
    let mut parser = Parser::new(&structured_input);

    let mut iteration_count = 0;

    loop {
        if iteration_count >= MAX_ITERATIONS {
            break;
        }

        match parser.entry() {
            Entry::Eof => break,
            _ => {
                iteration_count += 1;
                continue;
            }
        }
    }
});

fn generate_structured_input(data: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        if i % 7 == 0 && i + 4 < data.len() {
            let prop_idx = data[i] as usize % COMMON_PROPERTIES.len();
            result.push_str(COMMON_PROPERTIES[prop_idx]);
            result.push(':');
            i += 1;
        } else {
            let char_idx = data[i] as usize % ICAL_VCARD_CHARS.len();
            result.push(ICAL_VCARD_CHARS[char_idx] as char);
            i += 1;
        }

        if i % 37 == 0 {
            result.push_str("\r\n");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_structured_input() {
        let data = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let output = generate_structured_input(&data);

        assert!(COMMON_PROPERTIES.iter().any(|&prop| output.contains(prop)));

        for c in output.chars() {
            assert!(ICAL_VCARD_CHARS.contains(&(c as u8)) || c == '\r' || c == '\n');
        }
    }
}
