/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    Parser,
    common::{
        Encoding, IanaParse,
        decode::Base64Decoder,
        scan::{self, Mask},
    },
};
use memchr::memchr;
use std::ops::Deref;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Token<'x> {
    pub(crate) text: TokenText<'x>,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) stop_char: StopChar,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TokenText<'x> {
    Borrowed(&'x str),
    Owned(String),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum StopChar {
    Colon,
    Semicolon,
    Comma,
    Equal,
    Dot,
    Lf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mode(u16);

impl Mode {
    pub(crate) const STOP_COLON: Mode = Mode(scan::COLON);
    pub(crate) const STOP_SEMICOLON: Mode = Mode(scan::SEMICOLON);
    pub(crate) const STOP_COMMA: Mode = Mode(scan::COMMA);
    pub(crate) const STOP_EQUAL: Mode = Mode(scan::EQUAL);
    pub(crate) const STOP_DOT: Mode = Mode(scan::DOT);
    pub(crate) const UNFOLD_QP: Mode = Mode(1 << 11);
    pub(crate) const UNFOLD_B64: Mode = Mode(1 << 12);
    pub(crate) const UNQUOTE: Mode = Mode(scan::QUOTE);
    pub(crate) const SKIP_WS: Mode = Mode(scan::WHITESPACE);
    pub(crate) const STRIP_CTL: Mode = Mode(scan::CONTROL);
    pub(crate) const UNESCAPE_CARET: Mode = Mode(scan::CARET);
    pub(crate) const UNESCAPE_BACKSLASH: Mode = Mode(scan::BACKSLASH);

    pub(crate) const INITIAL: Mode = Mode(
        Mode::STOP_COLON.0
            | Mode::STOP_SEMICOLON.0
            | Mode::STOP_COMMA.0
            | Mode::STOP_EQUAL.0
            | Mode::UNQUOTE.0
            | Mode::UNESCAPE_BACKSLASH.0,
    );
    const IANA_TOKEN: Mode = Mode(
        Mode::STOP_COLON.0
            | Mode::STOP_SEMICOLON.0
            | Mode::STOP_COMMA.0
            | Mode::STOP_EQUAL.0
            | Mode::UNQUOTE.0
            | Mode::SKIP_WS.0
            | Mode::UNESCAPE_BACKSLASH.0,
    );
    const SINGLE_VALUE: Mode = Mode::UNESCAPE_BACKSLASH;
    const MULTI_VALUE_COMMA: Mode =
        Mode(Mode::STOP_COMMA.0 | Mode::SKIP_WS.0 | Mode::UNESCAPE_BACKSLASH.0);
    const MULTI_VALUE_SEMICOLON: Mode = Mode(Mode::STOP_SEMICOLON.0 | Mode::UNESCAPE_BACKSLASH.0);
    const MULTI_VALUE_SEMICOLON_AND_COMMA: Mode =
        Mode(Mode::STOP_SEMICOLON.0 | Mode::STOP_COMMA.0 | Mode::UNESCAPE_BACKSLASH.0);
    const PARAM_VALUE: Mode = Mode(
        Mode::STOP_COLON.0
            | Mode::STOP_SEMICOLON.0
            | Mode::STOP_COMMA.0
            | Mode::UNQUOTE.0
            | Mode::SKIP_WS.0
            | Mode::STRIP_CTL.0
            | Mode::UNESCAPE_CARET.0
            | Mode::UNESCAPE_BACKSLASH.0,
    );
    const RRULE_VALUE: Mode = Mode(
        Mode::STOP_COLON.0
            | Mode::STOP_COMMA.0
            | Mode::STOP_EQUAL.0
            | Mode::STOP_SEMICOLON.0
            | Mode::SKIP_WS.0
            | Mode::UNESCAPE_BACKSLASH.0,
    );

    const PLAIN_VALUE: Mode = Mode(Mode::UNESCAPE_BACKSLASH.0 | Mode::UNFOLD_B64.0);

    const UNFOLD: u16 = Self::UNFOLD_QP.0 | Self::UNFOLD_B64.0;
    const UNQUOTED_ONLY: u16 =
        scan::WHITESPACE | scan::COLON | scan::SEMICOLON | scan::COMMA | scan::EQUAL | scan::DOT;

    #[inline(always)]
    pub(crate) const fn contains(self, flag: Mode) -> bool {
        self.0 & flag.0 == flag.0
    }

    #[inline(always)]
    pub(crate) fn set(&mut self, flag: Mode, enabled: bool) {
        if enabled {
            self.0 |= flag.0;
        } else {
            self.0 &= !flag.0;
        }
    }

    #[inline(always)]
    const fn unquoted_mask(self) -> Mask {
        Mask::new(self.0 & !Self::UNFOLD)
    }

    #[inline(always)]
    const fn quoted_mask(self) -> Mask {
        Mask::new(self.0 & !Self::UNFOLD & !Self::UNQUOTED_ONLY)
    }
}

impl<'x> Parser<'x> {
    pub(crate) fn expect_iana_token(&mut self) {
        self.mode = Mode::IANA_TOKEN;
    }

    pub(crate) fn expect_single_value(&mut self) {
        self.mode = Mode::SINGLE_VALUE;
    }

    pub(crate) fn expect_multi_value_comma(&mut self) {
        self.mode = Mode::MULTI_VALUE_COMMA;
    }

    pub(crate) fn expect_multi_value_semicolon(&mut self) {
        self.mode = Mode::MULTI_VALUE_SEMICOLON;
    }

    pub(crate) fn expect_multi_value_semicolon_and_comma(&mut self) {
        self.mode = Mode::MULTI_VALUE_SEMICOLON_AND_COMMA;
    }

    pub(crate) fn expect_param_value(&mut self) {
        self.mode = Mode::PARAM_VALUE;
    }

    pub(crate) fn expect_rrule_value(&mut self) {
        self.mode = Mode::RRULE_VALUE;
    }

    fn caret_escape(&self, idx: usize) -> Option<(usize, char)> {
        let mut bytes = self.input.get(idx + 1..)?.iter().zip(idx + 1..);
        while let Some((ch, pos)) = bytes.next() {
            match ch {
                b'\r' if matches!(self.input.get(pos + 1), Some(b'\n')) => {}
                b'\n' => {
                    bytes.next().filter(|(ch, _)| matches!(ch, b' ' | b'\t'))?;
                }
                b'^' => return Some((pos, '^')),
                b'n' => return Some((pos, '\n')),
                b'\'' => return Some((pos, '"')),
                _ => return None,
            }
        }
        None
    }

    fn is_base64_continuation(&self, idx: usize) -> bool {
        let mut has_data = false;

        for ch in self.input.get(idx + 1..).unwrap_or_default() {
            match ch {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' | b'=' => has_data = true,
                b'\r' => {}
                b'\n' => break,
                _ => return false,
            }
        }

        has_data
    }

    #[inline(always)]
    fn borrowed(&self, start: usize, end: usize) -> TokenText<'x> {
        TokenText::Borrowed(self.source.get(start..end).unwrap_or_default())
    }

    pub(crate) fn token(&mut self) -> Option<Token<'x>> {
        let input = self.input;
        let start = self.pos;
        let mask = self.mode.unquoted_mask();
        let rest = input.get(start..).unwrap_or_default();
        let run = mask.plain_len(rest);
        let end = start + run;
        let Some(&ch) = rest.get(run) else {
            if run == 0 {
                return None;
            }
            self.pos = end;
            self.last_token_end = end - 1;
            return Some(Token {
                text: self.borrowed(start, end),
                start,
                end: end - 1,
                stop_char: StopChar::Lf,
            });
        };
        let (stop_char, next) = match mask.class_of(ch) {
            scan::COLON => (StopChar::Colon, end + 1),
            scan::SEMICOLON => (StopChar::Semicolon, end + 1),
            scan::COMMA => (StopChar::Comma, end + 1),
            scan::EQUAL => (StopChar::Equal, end + 1),
            scan::DOT => (StopChar::Dot, end + 1),
            scan::EOL => {
                let lf = if ch == b'\n' {
                    end
                } else if input.get(end + 1) == Some(&b'\n') {
                    end + 1
                } else {
                    return self.token_slow(start, run);
                };
                if matches!(input.get(lf + 1), Some(b' ' | b'\t'))
                    || (run != 0
                        && self.mode.contains(Mode::UNFOLD_QP)
                        && input.get(end - 1) == Some(&b'='))
                    || self.mode.contains(Mode::UNFOLD_B64)
                {
                    return self.token_slow(start, run);
                }
                (StopChar::Lf, lf + 1)
            }
            scan::QUOTE if run == 0 => return self.quoted_token(start),
            _ => return self.token_slow(start, run),
        };
        self.pos = next;
        Some(if run != 0 {
            self.last_token_end = end - 1;
            Token {
                text: self.borrowed(start, end),
                start,
                end: end - 1,
                stop_char,
            }
        } else {
            Token {
                text: TokenText::Borrowed(""),
                start: next - 1,
                end: next - 1,
                stop_char,
            }
        })
    }

    #[inline(always)]
    fn quoted_token(&mut self, quote: usize) -> Option<Token<'x>> {
        let input = self.input;
        let inner_start = quote + 1;
        let inner = input.get(inner_start..).unwrap_or_default();
        let inner_len = self.mode.quoted_mask().plain_len(inner);
        let close = inner_start + inner_len;
        let stop_char = match input.get(close..close + 2) {
            Some(&[b'"', stop]) => match self.mode.unquoted_mask().class_of(stop) {
                scan::COLON => StopChar::Colon,
                scan::SEMICOLON => StopChar::Semicolon,
                scan::COMMA => StopChar::Comma,
                scan::EQUAL => StopChar::Equal,
                scan::DOT => StopChar::Dot,
                _ => return self.token_slow(quote, 0),
            },
            _ => return self.token_slow(quote, 0),
        };
        self.pos = close + 2;
        Some(if inner_len != 0 {
            self.last_token_end = close - 1;
            Token {
                text: self.borrowed(inner_start, close),
                start: inner_start,
                end: close - 1,
                stop_char,
            }
        } else {
            Token {
                text: TokenText::Borrowed(""),
                start: close + 1,
                end: close + 1,
                stop_char,
            }
        })
    }

    #[inline(never)]
    fn token_slow(&mut self, start: usize, run: usize) -> Option<Token<'x>> {
        let input = self.input;
        let source = self.source;
        let unquoted_mask = self.mode.unquoted_mask();
        let quoted_mask = self.mode.quoted_mask();
        let mut mask = unquoted_mask;
        let mut pos = start + run;
        let mut offset_start = if run != 0 { start } else { usize::MAX };
        let mut offset_end = if run != 0 {
            start + run - 1
        } else {
            usize::MAX
        };
        let mut last_idx = 0;
        let mut in_quote = false;
        let mut buf = OwnedText::default();
        let mut skipped_ws = usize::MAX;

        let stop_char = 'outer: loop {
            let Some(&ch) = input.get(pos) else {
                if offset_start != usize::MAX {
                    break StopChar::Lf;
                }
                self.pos = pos;
                return None;
            };
            let idx = pos;
            pos += 1;
            last_idx = idx;

            'special: {
                let content = match mask.class_of(ch) {
                    scan::EOL => {
                        let idx = if ch == b'\n' {
                            idx
                        } else if input.get(pos) == Some(&b'\n') {
                            last_idx = pos;
                            pos += 1;
                            last_idx
                        } else {
                            break 'special;
                        };
                        if self.mode.contains(Mode::UNFOLD_QP)
                            && buf.last().or_else(|| input.get(offset_end).copied()) == Some(b'=')
                        {
                            offset_end = idx;

                            if !buf.is_empty() {
                                buf.push('\n');
                            }
                        } else if let Some(b' ' | b'\t') = input.get(pos) {
                            pos += 1;
                            if buf.is_empty() && offset_start != usize::MAX {
                                buf.start(source.get(offset_start..=offset_end));
                            }

                            if skipped_ws != usize::MAX && !buf.is_empty() {
                                buf.restore_whitespace(input.get(skipped_ws..idx));
                            }
                            skipped_ws = usize::MAX;
                        } else if self.mode.contains(Mode::UNFOLD_B64)
                            && self.is_base64_continuation(idx)
                        {
                            if buf.is_empty() && offset_start != usize::MAX {
                                buf.start(source.get(offset_start..=offset_end));
                            }
                            skipped_ws = usize::MAX;
                        } else {
                            break 'outer StopChar::Lf;
                        }
                        break 'special;
                    }
                    scan::WHITESPACE => {
                        if !buf.last().is_some_and(|last| last != ch) {
                            if skipped_ws == usize::MAX {
                                skipped_ws = idx;
                            }
                            break 'special;
                        }
                        if ch == b'\t' { '\t' } else { ' ' }
                    }
                    scan::BACKSLASH => {
                        let mut next_ch = b'\\';
                        let mut next_offset_end = idx;
                        while let Some(&escaped) = input.get(pos) {
                            let escaped_idx = pos;
                            pos += 1;
                            match escaped {
                                b'\t' | b'\r' => {}
                                b'\n' => {
                                    if let Some(b' ' | b'\t') = input.get(pos) {
                                        pos += 1;
                                        if let Some(&folded) = input.get(pos) {
                                            next_ch = folded;
                                            next_offset_end = pos;
                                            pos += 1;
                                            break;
                                        }
                                    } else {
                                        offset_end = escaped_idx - 1;
                                        break 'outer StopChar::Lf;
                                    }
                                }
                                _ => {
                                    next_ch = escaped;
                                    next_offset_end = escaped_idx;
                                    break;
                                }
                            }
                        }
                        if offset_start != usize::MAX {
                            if buf.is_empty() {
                                buf.start(source.get(offset_start..=offset_end));
                            }

                            if skipped_ws != usize::MAX {
                                buf.restore_whitespace(input.get(skipped_ws..idx));
                            }
                        } else {
                            offset_start = next_offset_end;
                        }
                        skipped_ws = usize::MAX;
                        match next_ch {
                            b'n' | b'N' => buf.push('\n'),
                            b'r' | b'R' => buf.push('\r'),
                            ascii if ascii.is_ascii() => buf.push(char::from(ascii)),
                            _ => {
                                if let Some(escaped) = source
                                    .get(next_offset_end..)
                                    .and_then(|rest| rest.chars().next())
                                {
                                    buf.push(escaped);
                                    pos = next_offset_end + escaped.len_utf8();
                                    next_offset_end = pos - 1;
                                }
                            }
                        }
                        offset_end = next_offset_end;
                        break 'special;
                    }
                    scan::QUOTE => {
                        in_quote = !in_quote;
                        mask = if in_quote { quoted_mask } else { unquoted_mask };
                        break 'special;
                    }
                    scan::COLON => break 'outer StopChar::Colon,
                    scan::SEMICOLON => break 'outer StopChar::Semicolon,
                    scan::COMMA => break 'outer StopChar::Comma,
                    scan::EQUAL => break 'outer StopChar::Equal,
                    scan::DOT => break 'outer StopChar::Dot,
                    scan::CONTROL => {
                        if buf.is_empty() && offset_start != usize::MAX {
                            buf.start(source.get(offset_start..=offset_end));
                        }
                        break 'special;
                    }
                    scan::CARET => {
                        if let Some((escape_idx, decoded)) = self.caret_escape(idx) {
                            if offset_start != usize::MAX {
                                if buf.is_empty() {
                                    buf.start(source.get(offset_start..=offset_end));
                                }
                                if skipped_ws != usize::MAX {
                                    buf.restore_whitespace(input.get(skipped_ws..idx));
                                }
                            } else {
                                offset_start = escape_idx;
                            }
                            skipped_ws = usize::MAX;
                            buf.push(decoded);
                            offset_end = escape_idx;
                            pos = escape_idx + 1;
                            break 'special;
                        }
                        '^'
                    }
                    _ => {
                        pos = idx;
                        break 'special;
                    }
                };

                if offset_start == usize::MAX {
                    offset_start = idx;
                }
                offset_end = idx;
                skipped_ws = usize::MAX;

                if !buf.is_empty() {
                    buf.push(content);
                }
            }

            let run = mask.plain_len(input.get(pos..).unwrap_or_default());
            if run != 0 {
                if offset_start == usize::MAX {
                    offset_start = pos;
                }
                if !buf.is_empty() {
                    buf.push_str(source.get(pos..pos + run));
                }
                pos += run;
                offset_end = pos - 1;
                skipped_ws = usize::MAX;
            }
        };

        self.pos = pos;

        if offset_start != usize::MAX {
            self.last_token_end = offset_end;
        }

        let (text, start, end) = if !buf.is_empty() {
            (buf.into_text(), offset_start, offset_end)
        } else if offset_start != usize::MAX {
            (
                TokenText::Borrowed(source.get(offset_start..=offset_end).unwrap_or_default()),
                offset_start,
                offset_end,
            )
        } else {
            (TokenText::Borrowed(""), last_idx, last_idx)
        };
        Some(Token {
            text,
            start,
            end,
            stop_char,
        })
    }

    #[inline]
    pub(crate) fn token_until_lf(&mut self, last_stop_char: &mut StopChar) -> Option<Token<'x>> {
        if last_stop_char != &StopChar::Lf {
            self.token()
                .inspect(|token| *last_stop_char = token.stop_char)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn parse_value_until_lf<T>(
        &mut self,
        separator: StopChar,
        last_stop_char: &mut StopChar,
    ) -> Option<Option<T>>
    where
        T: IanaParse + 'static,
    {
        if *last_stop_char != separator {
            self.token_until_lf(last_stop_char)
                .map(|token| T::parse(token.text.as_ref()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub(crate) fn value_token(
        &mut self,
        encoding: Option<Encoding>,
        payload: &mut Option<Vec<u8>>,
    ) -> Option<Token<'x>> {
        if encoding == Some(Encoding::Base64) {
            self.base64_token(payload)
        } else {
            self.token()
        }
    }

    #[inline(never)]
    fn base64_token(&mut self, payload: &mut Option<Vec<u8>>) -> Option<Token<'x>> {
        if let Some((token, next, bytes)) = self.folded_base64() {
            self.pos = next;
            self.last_token_end = token.end;
            *payload = Some(bytes);
            return Some(token);
        }
        self.token()
    }

    fn folded_base64(&self) -> Option<(Token<'x>, usize, Vec<u8>)> {
        if self.mode.0 & !Mode::PLAIN_VALUE.0 != 0 {
            return None;
        }
        let start = self.pos;
        let value = self.input.get(start..)?;
        let mut decoder = Base64Decoder::with_capacity(FoldedLines(value).len_hint() / 4 * 3);
        let mut first = usize::MAX;
        let mut end = 0;
        let mut rest = value;
        let next = loop {
            let offset = value.len() - rest.len();
            rest = decoder.decode_quads(rest);
            let consumed = value.len() - rest.len();
            if consumed != offset {
                first = first.min(offset);
                end = consumed;
            }
            let Some((&ch, tail)) = rest.split_first() else {
                break consumed;
            };
            rest = match ch {
                b'\r' | b'\n' => {
                    let after = match rest {
                        [b'\n', after @ ..] | [b'\r', b'\n', after @ ..] => after,
                        _ => return None,
                    };
                    match after {
                        [b' ' | b'\t', after @ ..] => after,
                        _ => {
                            let next = value.len() - after.len();
                            if self.mode.contains(Mode::UNFOLD_B64)
                                && self.is_base64_continuation(start + next - 1)
                            {
                                return None;
                            }
                            break next;
                        }
                    }
                }
                _ => {
                    if !decoder.push(ch) {
                        match ch {
                            b'=' => decoder.flush(),
                            b' ' | b'\t' => {}
                            _ => return None,
                        }
                    }
                    first = first.min(consumed);
                    end = consumed + 1;
                    tail
                }
            };
        };
        if first >= end {
            return None;
        }
        let (first, end) = (start + first, start + end);
        Some((
            Token {
                text: TokenText::Borrowed(self.source.get(first..end)?),
                start: first,
                end: end - 1,
                stop_char: StopChar::Lf,
            },
            start + next,
            decoder.finish_trimmed(),
        ))
    }

    pub(crate) fn seek_lf(&mut self) -> bool {
        loop {
            match self.token() {
                Some(Token {
                    stop_char: StopChar::Lf,
                    ..
                }) => return true,
                None => return false,
                _ => {}
            }
        }
    }

    pub(crate) fn seek_value_or_eol(&mut self) -> StopChar {
        loop {
            match self.token() {
                Some(Token {
                    stop_char: StopChar::Colon,
                    ..
                }) => return StopChar::Colon,
                Some(Token {
                    stop_char: StopChar::Lf,
                    ..
                })
                | None => return StopChar::Lf,
                _ => {}
            }
        }
    }

    pub(crate) fn seek_param_value_or_eol(&mut self) -> StopChar {
        loop {
            match self.token() {
                Some(Token {
                    stop_char: stop_char @ (StopChar::Colon | StopChar::Semicolon | StopChar::Equal),
                    ..
                }) => return stop_char,
                Some(Token {
                    stop_char: StopChar::Lf,
                    ..
                })
                | None => return StopChar::Lf,
                _ => {}
            }
        }
    }
}

struct FoldedLines<'x>(&'x [u8]);

impl FoldedLines<'_> {
    #[inline(never)]
    fn len_hint(&self) -> usize {
        let value = self.0;
        let Some(first) = memchr(b'\n', value) else {
            return value.len();
        };
        let lines = value.get(first + 1..).unwrap_or_default();
        let Some(period) = lines
            .first()
            .filter(|ch| matches!(ch, b' ' | b'\t'))
            .and_then(|_| memchr(b'\n', lines))
            .map(|len| len + 1)
        else {
            return first + 1;
        };
        let folded = |line: usize| {
            lines
                .chunks_exact(period)
                .nth(line)
                .is_some_and(|line| matches!(line, [b' ' | b'\t', .., b'\n']))
        };
        let mut known = 0;
        let mut step = 1;
        while folded(known + step) {
            known += step;
            step *= 2;
        }
        while step > 1 {
            step /= 2;
            if folded(known + step) {
                known += step;
            }
        }
        let last = first + (known + 1) * period;
        match value.get(last + 1..) {
            Some(line @ [b' ' | b'\t', ..]) => {
                memchr(b'\n', line).map_or(value.len(), |len| last + 2 + len)
            }
            _ => last + 1,
        }
    }
}

#[derive(Default)]
struct OwnedText(String);

impl OwnedText {
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline(always)]
    fn last(&self) -> Option<u8> {
        self.0.as_bytes().last().copied()
    }

    #[inline(always)]
    fn push(&mut self, ch: char) {
        self.0.push(ch);
    }

    #[inline(always)]
    fn push_str(&mut self, run: Option<&str>) {
        self.0.push_str(run.unwrap_or_default());
    }

    #[inline(always)]
    fn start(&mut self, prefix: Option<&str>) {
        self.push_str(prefix);
    }

    #[inline(always)]
    fn into_text<'x>(self) -> TokenText<'x> {
        TokenText::Owned(self.0)
    }

    #[inline(always)]
    fn restore_whitespace(&mut self, skipped: Option<&[u8]>) {
        self.0.extend(
            skipped
                .unwrap_or_default()
                .iter()
                .filter_map(|ch| match ch {
                    b' ' => Some(' '),
                    b'\t' => Some('\t'),
                    _ => None,
                }),
        );
    }
}

impl TokenText<'_> {
    #[inline(always)]
    pub(crate) fn as_str(&self) -> &str {
        match self {
            TokenText::Borrowed(text) => text,
            TokenText::Owned(text) => text,
        }
    }

    #[inline(always)]
    pub(crate) fn into_string(self) -> String {
        match self {
            TokenText::Borrowed(text) => text.to_owned(),
            TokenText::Owned(text) => text,
        }
    }
}

impl Deref for TokenText<'_> {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        self.as_str().as_bytes()
    }
}

impl AsRef<[u8]> for TokenText<'_> {
    #[inline(always)]
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl From<String> for TokenText<'_> {
    fn from(text: String) -> Self {
        TokenText::Owned(text)
    }
}

impl<'x> Token<'x> {
    pub fn into_string(self) -> String {
        self.text.into_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::decoders::base64::base64_decode;

    #[derive(Debug, PartialEq, Eq)]
    enum TextOwner<'x> {
        Borrowed(&'x str),
        Owned(String),
    }

    type Expected<'x> = (TextOwner<'x>, StopChar, usize, usize);

    fn with(mut mode: Mode, flag: Mode, enabled: bool) -> Mode {
        mode.set(flag, enabled);
        mode
    }

    fn run(len: usize) -> String {
        "abcdefghijklmnopqrstuvwxyz0123456789"
            .chars()
            .cycle()
            .take(len)
            .collect()
    }

    fn tokens(input: &str, mode: Mode) -> Vec<Expected<'_>> {
        let mut parser = Parser::new(input);
        parser.mode = mode;
        let mut tokens = vec![];
        while let Some(token) = parser.token() {
            let (stop_char, start, end) = (token.stop_char, token.start, token.end);
            let borrowed = match &token.text {
                TokenText::Borrowed(text) => Some(*text),
                TokenText::Owned(_) => None,
            };
            let text = token.into_string();
            let text = match borrowed {
                Some(borrowed) => {
                    assert_eq!(borrowed, text, "{input:?}");
                    TextOwner::Borrowed(borrowed)
                }
                None => TextOwner::Owned(text),
            };
            tokens.push((text, stop_char, start, end));
        }
        assert_eq!(parser.pos, input.len(), "{input:?}");
        tokens
    }

    #[test]
    fn test_tokenizer_edge_cases() {
        use StopChar::*;
        use TextOwner::{Borrowed as B, Owned as O};
        let multi_qp = with(Mode::MULTI_VALUE_COMMA, Mode::UNFOLD_QP, true);
        let single_b64 = with(Mode::SINGLE_VALUE, Mode::UNFOLD_B64, true);
        for (input, mode, expected) in [
            (
                ";;:",
                Mode::IANA_TOKEN,
                vec![
                    (B(""), Semicolon, 0, 0),
                    (B(""), Semicolon, 1, 1),
                    (B(""), Colon, 2, 2),
                ],
            ),
            (
                "\u{e9}:\u{fc};\u{65e5}\u{672c},x",
                Mode::IANA_TOKEN,
                vec![
                    (B("\u{e9}"), Colon, 0, 1),
                    (B("\u{fc}"), Semicolon, 3, 4),
                    (B("\u{65e5}\u{672c}"), Comma, 6, 11),
                    (B("x"), Lf, 13, 13),
                ],
            ),
            (
                "  X-A ;B",
                Mode::IANA_TOKEN,
                vec![(B("X-A"), Semicolon, 2, 4), (B("B"), Lf, 7, 7)],
            ),
            (
                "\"\":x",
                Mode::PARAM_VALUE,
                vec![(B(""), Colon, 2, 2), (B("x"), Lf, 3, 3)],
            ),
            (
                "\"a,b\"c;d",
                Mode::PARAM_VALUE,
                vec![(B("a,b\"c"), Semicolon, 1, 5), (B("d"), Lf, 7, 7)],
            ),
            ("\"abc", Mode::PARAM_VALUE, vec![(B("abc"), Lf, 1, 3)]),
            (
                "a^'b^nc^^d^x;e",
                Mode::PARAM_VALUE,
                vec![
                    (O("a\"b\nc^d^x".into()), Semicolon, 0, 11),
                    (B("e"), Lf, 13, 13),
                ],
            ),
            (
                "\"a\r\n b\":c",
                Mode::PARAM_VALUE,
                vec![(O("ab".into()), Colon, 1, 5), (B("c"), Lf, 8, 8)],
            ),
            (
                "a\u{1}b\u{7f}c,d",
                Mode::PARAM_VALUE,
                vec![(O("abc".into()), Comma, 0, 4), (B("d"), Lf, 6, 6)],
            ),
            (
                "abc\rdef\n",
                Mode::SINGLE_VALUE,
                vec![(B("abc\rdef"), Lf, 0, 6)],
            ),
            ("abc\r", Mode::SINGLE_VALUE, vec![(B("abc"), Lf, 0, 2)]),
            ("\n", Mode::SINGLE_VALUE, vec![(B(""), Lf, 0, 0)]),
            (
                "a\\\r\n b\n",
                Mode::SINGLE_VALUE,
                vec![(O("ab".into()), Lf, 0, 5)],
            ),
            (
                "a\\\nb",
                Mode::SINGLE_VALUE,
                vec![(B("a\\"), Lf, 0, 1), (B("b"), Lf, 3, 3)],
            ),
            (
                "a , b,,c\\,d\n",
                Mode::MULTI_VALUE_COMMA,
                vec![
                    (B("a"), Comma, 0, 0),
                    (B("b"), Comma, 4, 4),
                    (B(""), Comma, 6, 6),
                    (O("c,d".into()), Lf, 7, 10),
                ],
            ),
            (
                "=41=\r\n=42,C\r\n",
                multi_qp,
                vec![(B("=41=\r\n=42"), Comma, 0, 8), (B("C"), Lf, 10, 10)],
            ),
            ("=41=\n\r\n", multi_qp, vec![(B("=41=\n"), Lf, 0, 4)]),
            (
                "QUJD\r\nREVG\r\nX:Y",
                single_b64,
                vec![(O("QUJDREVG".into()), Lf, 0, 9), (B("X:Y"), Lf, 12, 14)],
            ),
            (
                "QUJD\r\n\r\nREVG",
                single_b64,
                vec![(B("QUJD"), Lf, 0, 3), (B("REVG"), Lf, 8, 11)],
            ),
            (
                "FREQ=DAILY;BYDAY=MO,TU\r\n",
                Mode::RRULE_VALUE,
                vec![
                    (B("FREQ"), Equal, 0, 3),
                    (B("DAILY"), Semicolon, 5, 9),
                    (B("BYDAY"), Equal, 11, 15),
                    (B("MO"), Comma, 17, 18),
                    (B("TU"), Lf, 20, 21),
                ],
            ),
            (
                "caf\u{e9}\\,\u{4e2d}\\n\u{1f600}x",
                Mode::SINGLE_VALUE,
                vec![(O("caf\u{e9},\u{4e2d}\n\u{1f600}x".into()), Lf, 0, 16)],
            ),
            (
                "a\\\u{e9}b",
                Mode::SINGLE_VALUE,
                vec![(O("a\u{e9}b".into()), Lf, 0, 4)],
            ),
            (
                "\\\u{4e2d}\\\u{1f600}",
                Mode::SINGLE_VALUE,
                vec![(O("\u{4e2d}\u{1f600}".into()), Lf, 1, 8)],
            ),
            (
                "a\\\r\n \u{e9}b",
                Mode::SINGLE_VALUE,
                vec![(O("a\u{e9}b".into()), Lf, 0, 7)],
            ),
            (
                "\u{e9}\r\n \u{4e2d}\\;\u{1f600}",
                Mode::SINGLE_VALUE,
                vec![(O("\u{e9}\u{4e2d};\u{1f600}".into()), Lf, 0, 13)],
            ),
            (
                "x\\\t\u{e9}",
                Mode::SINGLE_VALUE,
                vec![(O("x\u{e9}".into()), Lf, 0, 4)],
            ),
            (
                "\u{e9} \\, \u{e9},\u{4e2d}\\\\",
                Mode::MULTI_VALUE_COMMA,
                vec![
                    (O("\u{e9} , \u{e9}".into()), Comma, 0, 7),
                    (O("\u{4e2d}\\".into()), Lf, 9, 13),
                ],
            ),
            (
                "\u{1f600}\\;\u{e9};x",
                Mode::MULTI_VALUE_SEMICOLON,
                vec![
                    (O("\u{1f600};\u{e9}".into()), Semicolon, 0, 7),
                    (B("x"), Lf, 9, 9),
                ],
            ),
            (
                "^\u{e9}^n\u{4e2d}^'x;y",
                Mode::PARAM_VALUE,
                vec![
                    (O("^\u{e9}\n\u{4e2d}\"x".into()), Semicolon, 0, 10),
                    (B("y"), Lf, 12, 12),
                ],
            ),
            (
                "\"\u{e9}^^\u{1f600}\";x",
                Mode::PARAM_VALUE,
                vec![
                    (O("\u{e9}^\u{1f600}".into()), Semicolon, 1, 8),
                    (B("x"), Lf, 11, 11),
                ],
            ),
            (
                "\u{e9}\u{1}\u{4e2d},x",
                Mode::PARAM_VALUE,
                vec![
                    (O("\u{e9}\u{4e2d}".into()), Comma, 0, 5),
                    (B("x"), Lf, 7, 7),
                ],
            ),
            (
                "\u{e9}^\r\n n\u{4e2d}:x",
                Mode::PARAM_VALUE,
                vec![
                    (O("\u{e9}\n\u{4e2d}".into()), Colon, 0, 9),
                    (B("x"), Lf, 11, 11),
                ],
            ),
            (
                "a \t\\,b,c",
                Mode::MULTI_VALUE_COMMA,
                vec![(O("a \t,b".into()), Comma, 0, 5), (B("c"), Lf, 7, 7)],
            ),
            (
                "a\\,b\t\t\r\n c",
                Mode::MULTI_VALUE_COMMA,
                vec![(O("a,b\t\tc".into()), Lf, 0, 9)],
            ),
            (
                "a \t^nb;c",
                Mode::PARAM_VALUE,
                vec![(O("a \t\nb".into()), Semicolon, 0, 5), (B("c"), Lf, 7, 7)],
            ),
        ] {
            assert_eq!(tokens(input, mode), expected, "{input:?}");
        }
    }

    fn value_token_matches_token(input: &str, mode: Mode, encoding: Option<Encoding>) -> bool {
        let mut fused = Parser::new(input);
        fused.mode = mode;
        let mut plain = Parser::new(input);
        plain.mode = mode;
        let mut payload = None;
        let token = fused.value_token(encoding, &mut payload);
        let expected = plain.token();
        assert_eq!(
            (fused.pos, fused.last_token_end),
            (plain.pos, plain.last_token_end),
            "{input:?}"
        );
        match (token, payload, expected) {
            (Some(token), Some(bytes), Some(expected)) => {
                assert_eq!(
                    (token.start, token.end, token.stop_char),
                    (expected.start, expected.end, expected.stop_char),
                    "{input:?}"
                );
                assert_eq!(Some(bytes), base64_decode(&expected.text), "{input:?}");
                let content = |text: &[u8]| {
                    text.iter()
                        .filter(|ch| !matches!(ch, b' ' | b'\t' | b'\r' | b'\n'))
                        .copied()
                        .collect::<Vec<_>>()
                };
                assert_eq!(content(&token.text), content(&expected.text), "{input:?}");
                true
            }
            (token, None, expected) => {
                assert_eq!(token, expected, "{input:?}");
                false
            }
            (token, payload, expected) => {
                panic!("{input:?}: {token:?} {payload:?} {expected:?}")
            }
        }
    }

    #[test]
    fn test_value_token_folded_base64() {
        const VALUE: &str = concat!(
            "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5",
            "ejAxMjM0NTY3ODkrLw=="
        );
        let raw = with(Mode::SINGLE_VALUE, Mode::UNESCAPE_BACKSLASH, false);
        let mut values = vec![];
        for value in [
            VALUE,
            VALUE.trim_end_matches('='),
            "QUJDRA",
            "QUI",
            "QUJDQ=",
            "QQ==QUI",
        ] {
            for fold in ["\r\n ", "\n\t", "\r\n  "] {
                for position in 0..=value.len() {
                    let (head, tail) = value.split_at(position);
                    values.push(format!("{head}{fold}{tail}"));
                }
            }
            for fold in ["\r\n ", "\n "] {
                for width in 1..value.len() {
                    let lines = value
                        .as_bytes()
                        .chunks(width)
                        .map(String::from_utf8_lossy)
                        .collect::<Vec<_>>();
                    values.push(lines.join(fold));
                }
            }
        }
        for value in &values {
            for end in ["\r\nEND:VCARD\r\n", "\nX:Y", ""] {
                let input = format!("{value}{end}");
                for mode in [Mode::SINGLE_VALUE, Mode::PLAIN_VALUE, raw] {
                    assert!(
                        value_token_matches_token(&input, mode, Some(Encoding::Base64)),
                        "{input:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_value_token_falls_back_to_the_tokenizer() {
        let base64 = Some(Encoding::Base64);
        for (input, mode, encoding, fused) in [
            ("QUJD\\nREVG\r\nX:Y", Mode::SINGLE_VALUE, base64, false),
            ("QUJD\rREVG\r\nX:Y", Mode::SINGLE_VALUE, base64, false),
            ("QU*D\r\nX:Y", Mode::SINGLE_VALUE, base64, false),
            ("QUJD\r\nREVG\r\nX:Y", Mode::PLAIN_VALUE, base64, false),
            ("QUJD\r\nREVG\r\nX:Y", Mode::SINGLE_VALUE, base64, true),
            ("\r\nX:Y", Mode::PLAIN_VALUE, base64, false),
            ("", Mode::PLAIN_VALUE, base64, false),
            ("  \r\nX:Y", Mode::PLAIN_VALUE, base64, true),
            ("QUJD,REVG\r\n", Mode::MULTI_VALUE_COMMA, base64, false),
            ("QUJD\r\n REVG\r\n", Mode::SINGLE_VALUE, None, false),
            (
                "QUJD\r\n REVG\r\n",
                Mode::SINGLE_VALUE,
                Some(Encoding::QuotedPrintable),
                false,
            ),
            ("QUJD\r\n REVG\r\n", Mode::SINGLE_VALUE, base64, true),
        ] {
            assert_eq!(
                value_token_matches_token(input, mode, encoding),
                fused,
                "{input:?}"
            );
        }
    }

    #[test]
    fn test_tokenizer_run_boundaries() {
        let raw = with(Mode::SINGLE_VALUE, Mode::UNESCAPE_BACKSLASH, false);
        for len in [
            1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129,
        ] {
            let run = run(len);
            let run = run.as_str();
            let raw_text = format!("{run}\\n{run}");
            for (input, mode, expected) in [
                (
                    format!("{run}:{run};{run}"),
                    Mode::IANA_TOKEN,
                    vec![
                        (TextOwner::Borrowed(run), StopChar::Colon, 0, len - 1),
                        (
                            TextOwner::Borrowed(run),
                            StopChar::Semicolon,
                            len + 1,
                            2 * len,
                        ),
                        (
                            TextOwner::Borrowed(run),
                            StopChar::Lf,
                            2 * len + 2,
                            3 * len + 1,
                        ),
                    ],
                ),
                (
                    format!("\"{run}\";{run}:"),
                    Mode::PARAM_VALUE,
                    vec![
                        (TextOwner::Borrowed(run), StopChar::Semicolon, 1, len),
                        (
                            TextOwner::Borrowed(run),
                            StopChar::Colon,
                            len + 3,
                            2 * len + 2,
                        ),
                    ],
                ),
                (
                    format!("{run}\\,{run}\r\n"),
                    Mode::SINGLE_VALUE,
                    vec![(
                        TextOwner::Owned(format!("{run},{run}")),
                        StopChar::Lf,
                        0,
                        2 * len + 1,
                    )],
                ),
                (
                    format!("{run}\\n{run}\n"),
                    raw,
                    vec![(
                        TextOwner::Borrowed(raw_text.as_str()),
                        StopChar::Lf,
                        0,
                        2 * len + 1,
                    )],
                ),
                (
                    format!("{run}\r\n {run}\r\n"),
                    Mode::SINGLE_VALUE,
                    vec![(
                        TextOwner::Owned(format!("{run}{run}")),
                        StopChar::Lf,
                        0,
                        2 * len + 2,
                    )],
                ),
                (
                    format!("{run}\n\t{run}"),
                    Mode::SINGLE_VALUE,
                    vec![(
                        TextOwner::Owned(format!("{run}{run}")),
                        StopChar::Lf,
                        0,
                        2 * len + 1,
                    )],
                ),
                (
                    format!("{run}\r\n "),
                    Mode::SINGLE_VALUE,
                    vec![(TextOwner::Owned(run.to_string()), StopChar::Lf, 0, len - 1)],
                ),
                (
                    format!("{run}\\"),
                    Mode::SINGLE_VALUE,
                    vec![(TextOwner::Owned(format!("{run}\\")), StopChar::Lf, 0, len)],
                ),
            ] {
                assert_eq!(tokens(&input, mode), expected, "{input:?}");
            }
        }
    }

    #[test]
    fn test_tokenizer() {
        for (input, expected, disable_stop) in [
            (
                concat!(
                    "NOTE:This is a long descrip\n",
                    " tion that exists o\n",
                    " n a long line.",
                ),
                vec![
                    (TextOwner::Borrowed("NOTE"), StopChar::Colon),
                    (
                        TextOwner::Owned(
                            "This is a long description that exists on a long line.".into(),
                        ),
                        StopChar::Lf,
                    ),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "this is a text value\n",
                    "this is one value,this is another\n",
                    "this is a single value\\, with a comma encoded\n"
                ),
                vec![
                    (TextOwner::Borrowed("this is a text value"), StopChar::Lf),
                    (TextOwner::Borrowed("this is one value"), StopChar::Comma),
                    (TextOwner::Borrowed("this is another"), StopChar::Lf),
                    (
                        TextOwner::Owned("this is a single value, with a comma encoded".into()),
                        StopChar::Lf,
                    ),
                ],
                b"".as_slice(),
            ),
            (
                "N;ALTID=1;LANGUAGE=en:Yamada;Taro;;;",
                vec![
                    (TextOwner::Borrowed("N"), StopChar::Semicolon),
                    (TextOwner::Borrowed("ALTID"), StopChar::Equal),
                    (TextOwner::Borrowed("1"), StopChar::Semicolon),
                    (TextOwner::Borrowed("LANGUAGE"), StopChar::Equal),
                    (TextOwner::Borrowed("en"), StopChar::Colon),
                    (TextOwner::Borrowed("Yamada"), StopChar::Semicolon),
                    (TextOwner::Borrowed("Taro"), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                ],
                b"".as_slice(),
            ),
            (
                "N;SORT-AS=\"Mann,James\":de Mann;Henry,James;;",
                vec![
                    (TextOwner::Borrowed("N"), StopChar::Semicolon),
                    (TextOwner::Borrowed("SORT-AS"), StopChar::Equal),
                    (TextOwner::Borrowed("Mann,James"), StopChar::Colon),
                    (TextOwner::Borrowed("de Mann"), StopChar::Semicolon),
                    (TextOwner::Borrowed("Henry"), StopChar::Comma),
                    (TextOwner::Borrowed("James"), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                ],
                b"".as_slice(),
            ),
            (
                "  hello\\ nworld\\\\",
                vec![(TextOwner::Owned("hello nworld\\".into()), StopChar::Lf)],
                b"".as_slice(),
            ),
            (
                concat!(
                    "X-ABC-MMSUBJ;VALUE=URI;FMTTYPE=audio/basic:http://www.example.\n",
                    " org/mysubj.au"
                ),
                vec![
                    (TextOwner::Borrowed("X-ABC-MMSUBJ"), StopChar::Semicolon),
                    (TextOwner::Borrowed("VALUE"), StopChar::Equal),
                    (TextOwner::Borrowed("URI"), StopChar::Semicolon),
                    (TextOwner::Borrowed("FMTTYPE"), StopChar::Equal),
                    (TextOwner::Borrowed("audio/basic"), StopChar::Colon),
                    (TextOwner::Borrowed("http"), StopChar::Colon),
                    (
                        TextOwner::Owned("//www.example.org/mysubj.au".into()),
                        StopChar::Lf,
                    ),
                ],
                b"".as_slice(),
            ),
            (
                "RDATE;VALUE=DATE:19970304,19970504,19970704,19970904",
                vec![
                    (TextOwner::Borrowed("RDATE"), StopChar::Semicolon),
                    (TextOwner::Borrowed("VALUE"), StopChar::Equal),
                    (TextOwner::Borrowed("DATE"), StopChar::Colon),
                    (TextOwner::Borrowed("19970304"), StopChar::Comma),
                    (TextOwner::Borrowed("19970504"), StopChar::Comma),
                    (TextOwner::Borrowed("19970704"), StopChar::Comma),
                    (TextOwner::Borrowed("19970904"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                " BEGIN; ::\n \n \n test",
                vec![
                    (TextOwner::Borrowed("BEGIN"), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Colon),
                    (TextOwner::Borrowed(""), StopChar::Colon),
                    (TextOwner::Borrowed("test"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "DESCRIPTION;Sunday - Partly cloudy with a 20 percent chance of snow show",
                    "ers. Highs in the lower to mid 40s.\\n<a href=\\\"http://www.wunderground.c",
                    "om/US/WA/Leavenworth.html\\\">More Information</a>"
                ),
                vec![
                    (TextOwner::Borrowed("DESCRIPTION"), StopChar::Semicolon),
                    (
                        TextOwner::Owned(
                            concat!(
                                "Sunday - Partly cloudy with a 20 percent ",
                                "chance of snow showers. Highs in the lower ",
                                "to mid 40s.\n<a href=\"http://www.wunderground.com",
                                "/US/WA/Leavenworth.html\">More Information</a>"
                            )
                            .into(),
                        ),
                        StopChar::Lf,
                    ),
                ],
                b"=:".as_slice(),
            ),
            (
                concat!(
                    "ATTACH;FMTTYPE=text/plain;ENCODING=BASE64;VALUE=BINARY:VGhlIH\n",
                    " F1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4"
                ),
                vec![
                    (TextOwner::Borrowed("ATTACH"), StopChar::Semicolon),
                    (TextOwner::Borrowed("FMTTYPE"), StopChar::Equal),
                    (TextOwner::Borrowed("text/plain"), StopChar::Semicolon),
                    (TextOwner::Borrowed("ENCODING"), StopChar::Equal),
                    (TextOwner::Borrowed("BASE64"), StopChar::Semicolon),
                    (TextOwner::Borrowed("VALUE"), StopChar::Equal),
                    (TextOwner::Borrowed("BINARY"), StopChar::Colon),
                    (
                        TextOwner::Owned(
                            "VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4".into(),
                        ),
                        StopChar::Lf,
                    ),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "DESCRIPTION;ALTREP=\"cid:part1.0001@example.org\":The Fall'98 Wild\n",
                    " Wizards Conference - - Las Vegas\\, NV\\, USA"
                ),
                vec![
                    (TextOwner::Borrowed("DESCRIPTION"), StopChar::Semicolon),
                    (TextOwner::Borrowed("ALTREP"), StopChar::Equal),
                    (
                        TextOwner::Borrowed("cid:part1.0001@example.org"),
                        StopChar::Colon,
                    ),
                    (
                        TextOwner::Owned(
                            "The Fall'98 WildWizards Conference - - Las Vegas, NV, USA".to_string(),
                        ),
                        StopChar::Lf,
                    ),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "ATTENDEE;DELEGATED-FROM=\"mailto:jsmith@example.com\":mailto:\n",
                    " jdoe@example.com"
                ),
                vec![
                    (TextOwner::Borrowed("ATTENDEE"), StopChar::Semicolon),
                    (TextOwner::Borrowed("DELEGATED-FROM"), StopChar::Equal),
                    (
                        TextOwner::Borrowed("mailto:jsmith@example.com"),
                        StopChar::Colon,
                    ),
                    (TextOwner::Borrowed("mailto"), StopChar::Colon),
                    (TextOwner::Borrowed("jdoe@example.com"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "ATTENDEE;DELEGATED-TO=\"mailto:jdoe@example.com\",\"mailto:jqpublic\n",
                    " @example.com\":mailto:jsmith@example.com"
                ),
                vec![
                    (TextOwner::Borrowed("ATTENDEE"), StopChar::Semicolon),
                    (TextOwner::Borrowed("DELEGATED-TO"), StopChar::Equal),
                    (
                        TextOwner::Borrowed("mailto:jdoe@example.com"),
                        StopChar::Comma,
                    ),
                    (
                        TextOwner::Owned("mailto:jqpublic@example.com".into()),
                        StopChar::Colon,
                    ),
                    (TextOwner::Borrowed("mailto"), StopChar::Colon),
                    (TextOwner::Borrowed("jsmith@example.com"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                "CATEGORIES:cat1  ,  cat2,   cat3",
                vec![
                    (TextOwner::Borrowed("CATEGORIES"), StopChar::Colon),
                    (TextOwner::Borrowed("cat1"), StopChar::Comma),
                    (TextOwner::Borrowed("cat2"), StopChar::Comma),
                    (TextOwner::Borrowed("cat3"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                concat!("SUMMARY:Meeting\n", "\n\n", "BEGIN:VALARM"),
                vec![
                    (TextOwner::Borrowed("SUMMARY"), StopChar::Colon),
                    (TextOwner::Borrowed("Meeting"), StopChar::Lf),
                    (TextOwner::Borrowed(""), StopChar::Lf),
                    (TextOwner::Borrowed(""), StopChar::Lf),
                    (TextOwner::Borrowed("BEGIN"), StopChar::Colon),
                    (TextOwner::Borrowed("VALARM"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "FN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=D0=B3=D0=BE=D1=80=20",
                    "=D0=97=D0=B0=D0=BE=D1=80=D1=81=D0=81=D0=\n",
                    "=BA=\n",
                    "=D1=96=\n",
                    " xyz =\n",
                    "=D1=96\n"
                ),
                vec![
                    (TextOwner::Borrowed("FN"), StopChar::Semicolon),
                    (TextOwner::Borrowed("CHARSET"), StopChar::Equal),
                    (TextOwner::Borrowed("UTF-8"), StopChar::Semicolon),
                    (TextOwner::Borrowed("ENCODING"), StopChar::Equal),
                    (TextOwner::Borrowed("QUOTED-PRINTABLE"), StopChar::Colon),
                    (
                        TextOwner::Borrowed(concat!(
                            "=D0=B3=D0=BE=D1=80=20=D0=97=D0=B0=D0=BE=D1=80",
                            "=D1=81=D0=81=D0=\n=BA=\n=D1=96=\n xyz =\n=D1=96"
                        )),
                        StopChar::Lf,
                    ),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "ADR;LABEL=\"Mr. John Q. Public, Esq.\\nMail Drop: TNE QB\\n123\n",
                    " Main Street\\nAny Town, CA  91921-1234\\nU.S.A.\":\n",
                    " ;;123 Main Street;Any Town;CA;91921-1234;U.S.A."
                ),
                vec![
                    (TextOwner::Borrowed("ADR"), StopChar::Semicolon),
                    (TextOwner::Borrowed("LABEL"), StopChar::Equal),
                    (
                        TextOwner::Owned(
                            concat!(
                                "Mr. John Q. Public, Esq.\nMail Drop: TNE QB\n",
                                "123Main Street\nAny Town, CA  91921-1234\nU.S.A."
                            )
                            .into(),
                        ),
                        StopChar::Colon,
                    ),
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                    (TextOwner::Borrowed("123 Main Street"), StopChar::Semicolon),
                    (TextOwner::Borrowed("Any Town"), StopChar::Semicolon),
                    (TextOwner::Borrowed("CA"), StopChar::Semicolon),
                    (TextOwner::Borrowed("91921-1234"), StopChar::Semicolon),
                    (TextOwner::Borrowed("U.S.A."), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            (
                "\\",
                vec![(TextOwner::Owned("\\".into()), StopChar::Lf)],
                b"".as_slice(),
            ),
            (
                "\\n",
                vec![(TextOwner::Owned("\n".into()), StopChar::Lf)],
                b"".as_slice(),
            ),
            (
                "\\nhello",
                vec![(TextOwner::Owned("\nhello".into()), StopChar::Lf)],
                b"".as_slice(),
            ),
            (
                ";;\nEND:VCARD\n",
                vec![
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Semicolon),
                    (TextOwner::Borrowed(""), StopChar::Lf),
                    (TextOwner::Borrowed("END"), StopChar::Colon),
                    (TextOwner::Borrowed("VCARD"), StopChar::Lf),
                ],
                b"".as_slice(),
            ),
            ("", vec![], b"".as_slice()),
        ] {
            let mut parser = Parser::new(input);
            let mut tokens = vec![];
            parser.mode.set(Mode::SKIP_WS, true);
            for ch in disable_stop {
                match ch {
                    b'=' => {
                        parser.mode.set(Mode::STOP_EQUAL, false);
                    }
                    b':' => {
                        parser.mode.set(Mode::STOP_COLON, false);
                    }
                    _ => {}
                }
            }

            while let Some(token) = parser.token() {
                if token.text.eq_ignore_ascii_case(b"quoted-printable") {
                    parser.mode.set(Mode::STOP_COLON, false);
                    parser.mode.set(Mode::STOP_COMMA, false);
                    parser.mode.set(Mode::STOP_EQUAL, false);
                    parser.mode.set(Mode::UNQUOTE, false);
                    parser.mode.set(Mode::UNFOLD_QP, true);
                    parser.mode.set(Mode::STOP_SEMICOLON, true);
                    parser.mode.set(Mode::SKIP_WS, false);
                }
                let text = match token.text {
                    TokenText::Borrowed(text) => TextOwner::Borrowed(text),
                    TokenText::Owned(text) => TextOwner::Owned(text),
                };
                tokens.push((text, token.stop_char));
            }
            assert_eq!(tokens, expected, "failed for input: {:?}", input);
        }
    }

    fn folded_values(head: &dyn Fn(usize) -> String, fill: &str, lines: [usize; 3]) -> String {
        let fill = fill.repeat(80);
        let [first, continuations @ ..] = lines;
        let mut input = String::from("BEGIN:VCALENDAR\nBEGIN:VEVENT\n");
        for index in 0..2000 {
            let head = head(index);
            input.push_str(&head);
            input.push_str(fill.get(..first - 1 - head.len()).unwrap_or_default());
            input.push('\n');
            for line in continuations {
                input.push(' ');
                input.push_str(fill.get(..line - 2).unwrap_or_default());
                input.push('\n');
            }
        }
        input.push_str("END:VEVENT\nEND:VCALENDAR\n");
        input
    }

    #[test]
    fn test_folded_values_keep_their_own_length() {
        use crate::icalendar::{ICalendar, ICalendarValue};

        let text = |index: usize| format!("DESCRIPTION;X-N={index:06}:");
        let binary = |index: usize| format!("ATTACH;X-N={index:06};ENCODING=BASE64;VALUE=BINARY:");
        let regular = [76, 76, 76];
        let misaligned = [61, 64, 3];
        for (head, fill, lines) in [
            (&text as &dyn Fn(usize) -> String, "b", regular),
            (&binary, "QUJD", regular),
            (&text, "b", misaligned),
            (&binary, "QUJD", misaligned),
        ] {
            let input = folded_values(head, fill, lines);
            let ical = ICalendar::parse(&input).expect("valid calendar");
            let (len, capacity) = ical
                .components
                .iter()
                .flat_map(|component| &component.entries)
                .flat_map(|entry| &entry.values)
                .map(|value| match value {
                    ICalendarValue::Text(text) => (text.len(), text.capacity()),
                    ICalendarValue::Binary(bytes) => (bytes.len(), bytes.capacity()),
                    _ => (0, 0),
                })
                .fold((0, 0), |(len, capacity), (l, c)| (len + l, capacity + c));
            assert!(len > 0, "{lines:?}");
            assert!(
                capacity <= 4 * len,
                "{len} bytes of values keep {capacity} bytes for a {} byte document {lines:?}",
                input.len()
            );
        }
    }
}
