#![no_main]
use calcard::{Entry, Parser};
use libfuzzer_sys::fuzz_target;

const ICAL_VCARD_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789\
    :;,=\r\n \t.+-_@/\\\"'()[]{}*#$%&!?<>|~^`";

fuzz_target!(|data: &[u8]| {
    let str_data = String::from_utf8_lossy(data).into_owned();
    let mut parser = Parser::new(&str_data);
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
    fn test_empty_input() {
        run_fuzz(b"");
    }

    #[test]
    fn test_null_bytes() {
        run_fuzz(b"\x00\x00\x00");
    }

    #[test]
    fn test_long_input() {
        let long_input = "A".repeat(100000);
        run_fuzz(long_input.as_bytes());
    }

    fn run_fuzz(data: &[u8]) {
        if let Ok(input_str) = std::str::from_utf8(data) {
            let mut parser = Parser::new(input_str);

            loop {
                match parser.entry() {
                    Entry::Eof => break,
                    _ => {}
                }
            }
        }
    }
}
