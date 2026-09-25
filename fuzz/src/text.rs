/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

const MAX_LINE_LEN: usize = 75;
const V21_MARKER: &str = "\r\nVERSION:2.1\r\n";
const V21_BASE64_MARKER: &str = ";ENCODING=BASE64";

pub struct Token<'x>(pub &'x str);

impl Token<'_> {
    pub fn is_plain(&self) -> bool {
        self.0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }

    pub fn is_stable_text(&self) -> bool {
        !self.0.contains('\r')
            && !self
                .0
                .as_bytes()
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a == b && a.is_ascii_whitespace()))
    }
}

pub struct Folded<'x>(pub &'x str);

impl Folded<'_> {
    pub fn check(&self, context: &str) {
        let text = self.0;
        let is_v21 = text.contains(V21_MARKER);
        let mut logical = String::new();
        let mut blank_allowed = false;
        let mut lines = text.split("\r\n").peekable();
        while let Some(line) = lines.next() {
            assert!(
                line.len() <= MAX_LINE_LEN,
                "{context}: physical line of {} octets exceeds {MAX_LINE_LEN}: {line:?}",
                line.len()
            );
            assert!(
                line != " ",
                "{context}: continuation line holding only the fold character in {text:?}"
            );
            match line.strip_prefix(' ') {
                Some(continuation) => logical.push_str(continuation),
                None if line.is_empty() => {
                    let v21_base64 = is_v21
                        && std::mem::take(&mut blank_allowed)
                        && logical.contains(V21_BASE64_MARKER);
                    assert!(
                        lines.peek().is_none() || v21_base64,
                        "{context}: empty physical line in {text:?}"
                    );
                    logical.clear();
                }
                None => {
                    logical.clear();
                    logical.push_str(line);
                    blank_allowed = true;
                }
            }
        }
    }
}
