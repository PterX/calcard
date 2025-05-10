#![no_main]
use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use calcard::{Entry, Parser};

#[derive(Arbitrary, Debug)]
enum FuzzInputType {
    Random,
    ConstrainedAscii,
}

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    input_type: FuzzInputType,
    data: Vec<u8>,
}

const ICAL_VCARD_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\
    :;,=\r\n \t.+-_@/\\\"'()[]{}*#$%&!?<>|~^`";

fuzz_target!(|input: FuzzInput| {
    let test_data = match input.input_type {
        FuzzInputType::Random => String::from_utf8_lossy(&input.data).into_owned(),
        FuzzInputType::ConstrainedAscii => {
            let constrained_bytes: Vec<u8> = input
                .data
                .iter()
                .map(|&b| ICAL_VCARD_CHARS[b as usize % ICAL_VCARD_CHARS.len()])
                .collect();

            String::from_utf8(constrained_bytes).unwrap_or_default()
        }
    };

    let mut parser = Parser::new(&test_data);

    let mut iteration_count = 0;
    const MAX_ITERATIONS: usize = 1000;

    loop {
        iteration_count += 1;
        if iteration_count > MAX_ITERATIONS {
            break;
        }

        match parser.entry() {
            Entry::Eof => break,
            _ => continue,
        }
    }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_vcard() {
        let sample = r#"BEGIN:VCARD
VERSION:3.0
FN:John Doe
EMAIL:john@example.com
END:VCARD"#;

        let mut parser = Parser::new(sample);
        let mut entries = Vec::new();

        loop {
            match parser.entry() {
                Entry::Eof => break,
                entry => entries.push(entry),
            }
        }

        assert!(!entries.is_empty());
    }

    #[test]
    fn test_sample_icalendar() {
        let sample = r#"BEGIN:VCALENDAR
VERSION:2.0
PRODID:-//Test//Test Calendar//EN
BEGIN:VEVENT
DTSTART:20240101T090000Z
DTEND:20240101T100000Z
SUMMARY:Test Event
END:VEVENT
END:VCALENDAR"#;

        let mut parser = Parser::new(sample);
        let mut entries = Vec::new();

        loop {
            match parser.entry() {
                Entry::Eof => break,
                entry => entries.push(entry),
            }
        }

        assert!(!entries.is_empty());
    }
}
