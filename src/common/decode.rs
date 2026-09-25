/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::common::Encoding;
use fearless_simd::{Bytes, Level, Simd, SimdFrom, dispatch, u8x16, u16x8, u32x4};
use mail_parser::decoders::charsets::map::charset_decoder;
use memchr::memchr3;

const QUOTED_PRINTABLE_CHARSET: &str = "iso-8859-1";
const INVALID: u8 = 0xff;
const STOP_BYTE: u8 = u8::MAX;
const SEXTET_MASK: u8 = 0xc0;
const SPARE_BYTES: usize = 64;

const BLOCK_LEN: usize = 16;
const BLOCKS_ENABLED: bool = cfg!(target_endian = "little");
const BLOCK_OUT_LEN: usize = 12;
const NIBBLE_LOW: [u8; BLOCK_LEN] = [
    0x15, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x13, 0x1a, 0x1b, 0x1b, 0x1b, 0x1a,
];
const NIBBLE_HIGH: [u8; BLOCK_LEN] = [
    0x10, 0x10, 0x01, 0x02, 0x04, 0x08, 0x04, 0x08, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
];
const ROLL: [u8; BLOCK_LEN] = [0, 16, 19, 4, 191, 191, 185, 185, 0, 0, 0, 0, 0, 0, 0, 0];
const PACK: [u8; BLOCK_LEN] = [2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, 3, 7, 11, 15];

static BASE64: [u8; 256] = base64_table();
static HEX: [u8; 256] = hex_table();

const fn base64_table() -> [u8; 256] {
    let mut table = [INVALID; 256];
    let mut byte = 0;
    while byte < table.len() {
        table[byte] = match byte as u8 {
            ch @ b'A'..=b'Z' => ch - b'A',
            ch @ b'a'..=b'z' => ch - b'a' + 26,
            ch @ b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => INVALID,
        };
        byte += 1;
    }
    table
}

const fn hex_table() -> [u8; 256] {
    let mut table = [INVALID; 256];
    let mut byte = 0;
    while byte < table.len() {
        table[byte] = match byte as u8 {
            ch @ b'0'..=b'9' => ch - b'0',
            ch @ b'A'..=b'F' => ch - b'A' + 10,
            ch @ b'a'..=b'f' => ch - b'a' + 10,
            _ => INVALID,
        };
        byte += 1;
    }
    table
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decoded {
    Binary(Vec<u8>),
    Text(String),
    Undecodable,
}

pub(crate) struct ValueDecoder<'x> {
    pub(crate) encoding: Encoding,
    pub(crate) charset: Option<&'x str>,
    pub(crate) binary: bool,
}

impl ValueDecoder<'_> {
    #[inline(never)]
    pub(crate) fn decode(&self, input: &[u8], payload: Option<Vec<u8>>) -> Decoded {
        let Some(bytes) = payload.or_else(|| self.encoding.decode(input)) else {
            return Decoded::Undecodable;
        };
        if self.binary {
            return Decoded::Binary(bytes);
        }
        let default_charset = match self.encoding {
            Encoding::QuotedPrintable => Some(QUOTED_PRINTABLE_CHARSET),
            Encoding::Base64 => None,
        };
        match self
            .charset
            .or(default_charset)
            .and_then(|charset| charset_decoder(charset.as_bytes()))
        {
            Some(decoder) => Decoded::Text(decoder(&bytes)),
            None => match String::from_utf8(bytes) {
                Ok(text) => Decoded::Text(text),
                Err(err) => Decoded::Binary(err.into_bytes()),
            },
        }
    }
}

pub(crate) struct Base64Decoder {
    out: Vec<u8>,
    sextets: u32,
    count: u8,
    blocks: Option<Level>,
}

impl Base64Decoder {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        let level = Level::new();
        Base64Decoder {
            out: Vec::with_capacity(capacity),
            sextets: 0,
            count: 0,
            blocks: (BLOCKS_ENABLED && !level.is_fallback()).then_some(level),
        }
    }

    #[inline(always)]
    pub(crate) fn decode_quads<'x>(&mut self, mut rest: &'x [u8]) -> &'x [u8] {
        if self.count != 0 {
            return rest;
        }
        if let Some(level) = self.blocks
            && rest.len() >= BLOCK_LEN
        {
            rest = dispatch!(level, simd => Self::decode_blocks(simd, rest, &mut self.out));
        }
        while let Some((block, tail)) = rest.split_first_chunk::<8>() {
            let [a, b, c, d, e, f, g, h] = block.map(|byte| BASE64[byte as usize] as u64);
            if (a | b | c | d | e | f | g | h) & SEXTET_MASK as u64 != 0 {
                break;
            }
            let word = (a << 42)
                | (b << 36)
                | (c << 30)
                | (d << 24)
                | (e << 18)
                | (f << 12)
                | (g << 6)
                | h;
            self.out.extend_from_slice(&word.to_be_bytes()[2..]);
            rest = tail;
        }
        while let Some((&[a, b, c, d], tail)) = rest.split_first_chunk::<4>() {
            let (a, b, c, d) = (
                BASE64[a as usize],
                BASE64[b as usize],
                BASE64[c as usize],
                BASE64[d as usize],
            );
            if (a | b | c | d) & SEXTET_MASK != 0 {
                break;
            }
            self.out
                .extend_from_slice(&[(a << 2) | (b >> 4), (b << 4) | (c >> 2), (c << 6) | d]);
            rest = tail;
        }
        rest
    }

    #[inline(always)]
    pub(crate) fn push(&mut self, ch: u8) -> bool {
        let value = BASE64[ch as usize];
        if value == INVALID {
            return false;
        }
        self.sextets = (self.sextets << 6) | value as u32;
        self.count = (self.count + 1) & 3;
        if self.count == 0 {
            self.out.extend_from_slice(&[
                (self.sextets >> 16) as u8,
                (self.sextets >> 8) as u8,
                self.sextets as u8,
            ]);
        }
        true
    }

    #[inline(always)]
    pub(crate) fn flush(&mut self) {
        match self.count {
            2 => self.out.push((self.sextets >> 4) as u8),
            3 => self
                .out
                .extend_from_slice(&[(self.sextets >> 10) as u8, (self.sextets >> 2) as u8]),
            _ => {}
        }
        self.count = 0;
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.flush();
        self.out
    }

    pub(crate) fn finish_trimmed(self) -> Vec<u8> {
        let mut out = self.finish();
        if out.capacity() > 2 * out.len() + SPARE_BYTES {
            out.shrink_to_fit();
        }
        out
    }

    #[inline(always)]
    fn decode_blocks<'x, S: Simd>(simd: S, mut input: &'x [u8], out: &mut Vec<u8>) -> &'x [u8] {
        let nibble_low = u8x16::simd_from(simd, NIBBLE_LOW);
        let nibble_high = u8x16::simd_from(simd, NIBBLE_HIGH);
        let roll = u8x16::simd_from(simd, ROLL);
        let pack = u8x16::simd_from(simd, PACK);
        let low_nibble = simd.splat_u8x16(0x0f);
        let slash = simd.splat_u8x16(b'/');
        let slash_roll = simd.splat_u8x16(1);
        let zero = simd.splat_u8x16(0);
        let low_byte = simd.splat_u16x8(0x00ff);
        let low_half = simd.splat_u32x4(0xffff);
        while let Some((block, tail)) = input.split_first_chunk::<BLOCK_LEN>() {
            let chars = u8x16::simd_from(simd, *block);
            let high = simd.shr_u8x16(chars, 4);
            let invalid = simd.and_u8x16(
                simd.swizzle_dyn_u8x16(nibble_low, simd.and_u8x16(chars, low_nibble)),
                simd.swizzle_dyn_u8x16(nibble_high, high),
            );
            if simd.any_true_mask8x16(simd.simd_gt_u8x16(invalid, zero)) {
                break;
            }
            let roll_index = simd.select_u8x16(simd.simd_eq_u8x16(chars, slash), slash_roll, high);
            let sextets = simd.add_u8x16(chars, simd.swizzle_dyn_u8x16(roll, roll_index));
            let pairs = u16x8::from_bytes(sextets);
            let pairs = simd.or_u16x8(
                simd.shl_u16x8(simd.and_u16x8(pairs, low_byte), 6),
                simd.shr_u16x8(pairs, 8),
            );
            let quads = u32x4::from_bytes(pairs.to_bytes());
            let quads = simd.or_u32x4(
                simd.shl_u32x4(simd.and_u32x4(quads, low_half), 12),
                simd.shr_u32x4(quads, 16),
            );
            let bytes: [u8; BLOCK_LEN] = simd.swizzle_dyn_u8x16(quads.to_bytes(), pack).into();
            if let Some(decoded) = bytes.get(..BLOCK_OUT_LEN) {
                out.extend_from_slice(decoded);
            }
            input = tail;
        }
        input
    }
}

impl Encoding {
    pub(crate) fn decode(self, input: &[u8]) -> Option<Vec<u8>> {
        match self {
            Encoding::Base64 => Self::decode_base64(input),
            Encoding::QuotedPrintable => Self::decode_quoted_printable(input),
        }
    }

    #[inline(never)]
    fn decode_base64(input: &[u8]) -> Option<Vec<u8>> {
        let mut decoder = Base64Decoder::with_capacity(input.len().div_ceil(4) * 3);
        let mut rest = input;
        loop {
            rest = decoder.decode_quads(rest);
            let Some((&ch, tail)) = rest.split_first() else {
                return Some(decoder.finish());
            };
            rest = tail;
            if !decoder.push(ch) {
                match ch {
                    b'=' => decoder.flush(),
                    b' ' | b'\t' | b'\r' | b'\n' => {}
                    _ => return (ch == STOP_BYTE).then(|| decoder.finish()),
                }
            }
        }
    }

    #[inline(never)]
    fn decode_quoted_printable(input: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(input.len());
        let mut rest = input;
        let mut ws_count = 0;
        let mut line_break: &[u8] = b"\n";

        loop {
            let run_len = memchr3(b'=', b'\r', b'\n', rest).unwrap_or(rest.len());
            if let Some((run, tail)) = rest.split_at_checked(run_len) {
                if !run.is_empty() {
                    out.extend_from_slice(run);
                    let trailing = run
                        .iter()
                        .rev()
                        .take_while(|ch| ch.is_ascii_whitespace())
                        .count();
                    ws_count = if trailing == run.len() {
                        ws_count + trailing
                    } else {
                        trailing
                    };
                }
                rest = tail;
            }

            let Some((&ch, tail)) = rest.split_first() else {
                return Some(out);
            };
            rest = tail;
            match ch {
                b'\r' => line_break = b"\r\n",
                b'\n' => {
                    out.truncate(out.len() - ws_count);
                    out.extend_from_slice(line_break);
                    ws_count = 0;
                }
                _ => {
                    if let Some((&[high, low], tail)) = rest.split_first_chunk::<2>() {
                        let (high, low) = (HEX[high as usize], HEX[low as usize]);
                        if (high | low) & 0xf0 == 0 {
                            out.push((high << 4) | low);
                            ws_count = 0;
                            rest = tail;
                            continue;
                        }
                    }

                    let mut high = None;
                    loop {
                        let Some((&ch, tail)) = rest.split_first() else {
                            return Some(out);
                        };
                        rest = tail;
                        match ch {
                            b'=' => return None,
                            b'\r' => line_break = b"\r\n",
                            b'\n' if high.is_none() => {
                                ws_count = 0;
                                break;
                            }
                            b'\n' => {
                                out.truncate(out.len() - ws_count);
                                out.extend_from_slice(line_break);
                                ws_count = 0;
                            }
                            _ => {
                                let value = HEX[ch as usize];
                                match high {
                                    None if value != INVALID => high = Some(value),
                                    None if ch.is_ascii_whitespace() => {}
                                    None => return None,
                                    Some(_) if value == INVALID => return None,
                                    Some(high) => {
                                        out.push((high << 4) | value);
                                        ws_count = 0;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Decoded, ValueDecoder};
    use crate::{
        common::{Data, Encoding, xorshift::XorShift},
        icalendar::{ICalendar, ICalendarValue},
        vcard::{VCard, VCardValue},
    };
    use mail_parser::decoders::{
        base64::base64_decode as reference_base64_decode,
        quoted_printable::quoted_printable_decode as reference_quoted_printable_decode,
    };

    const RANDOM_INPUTS: usize = 2_000;
    const BASE64_ALPHABET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn input(rng: &mut XorShift, alphabet: &[u8], pieces: &[&[u8]]) -> Vec<u8> {
        let len = match rng.below(10) {
            0 => rng.below(4000),
            1..=3 => rng.below(200),
            _ => rng.below(24),
        };
        let density = 1 + rng.below(64);
        let mut bytes = Vec::with_capacity(len + 8);
        while bytes.len() < len {
            match rng.below(density) {
                0 => bytes.push(rng.next() as u8),
                1 => bytes.extend_from_slice(pieces[rng.below(pieces.len())]),
                _ => bytes.push(alphabet[rng.below(alphabet.len())]),
            }
        }
        bytes
    }

    fn assert_base64(bytes: &[u8]) {
        assert_eq!(
            Encoding::Base64.decode(bytes),
            reference_base64_decode(bytes),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }

    fn assert_quoted_printable(bytes: &[u8]) {
        assert_eq!(
            Encoding::QuotedPrintable.decode(bytes),
            reference_quoted_printable_decode(bytes),
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
    }

    #[test]
    fn base64_decode_matches_mail_parser() {
        const PIECES: &[&[u8]] = &[
            b"=",
            b"==",
            b" ",
            b"\t",
            b"\r\n",
            b"\n ",
            b"\r\n\t",
            b"-",
            b"_",
            b"\\",
            b"\xff",
            b"\xc3\xa9",
            b":",
            b";",
            b",",
            b".",
        ];
        let mut rng = XorShift::new(0x0ba5_e640_0000_0001);
        for _ in 0..RANDOM_INPUTS {
            assert_base64(&input(&mut rng, BASE64_ALPHABET, PIECES));
        }
        for byte in 0..=u8::MAX {
            for prefix in [&b""[..], b"Q", b"QU", b"QUJ", b"QUJD"] {
                for suffix in [&b""[..], b"=", b"QUJD", b"Q"] {
                    assert_base64(&[prefix, &[byte], suffix].concat());
                }
            }
        }
    }

    #[test]
    fn base64_decode_matches_mail_parser_on_value_shapes() {
        let mut rng = XorShift::new(0x1ca1_0006);
        for _ in 0..RANDOM_INPUTS {
            let mut bytes = Vec::new();
            match rng.below(4) {
                0 => {
                    for _ in 0..rng.below(600) {
                        match rng.below(200) {
                            0 => bytes.extend_from_slice(b"\r\n "),
                            1 => bytes.push(*rng.pick(b":;,=\"\\^. \t\r\n")),
                            _ => bytes.push(*rng.pick(BASE64_ALPHABET)),
                        }
                    }
                    bytes.extend(std::iter::repeat_n(b'=', rng.below(3)));
                }
                1 => bytes.extend((0..rng.below(80)).map(|_| rng.next() as u8)),
                2 => {
                    for _ in 0..rng.below(200) {
                        bytes.push(*rng.pick(
                            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/= \t\r\n\xff-_",
                        ));
                    }
                }
                _ => {
                    for position in 0..rng.below(400) {
                        if position % 4 == 0 && rng.one_in(50) {
                            bytes.push(*rng.pick(b"= \r\n\xff\x00"));
                        }
                        bytes.push(*rng.pick(BASE64_ALPHABET));
                    }
                }
            }
            assert_base64(&bytes);
        }
    }

    #[test]
    fn base64_block_decode_matches_mail_parser_in_every_lane() {
        const VALID: &[u8] = b"QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0NTY3ODkrLw==";
        for byte in 0..=u8::MAX {
            for position in 0..VALID.len() {
                for skip in 0..4 {
                    let mut bytes = VALID.get(skip..).unwrap_or_default().to_vec();
                    if let Some(slot) = bytes.get_mut(position) {
                        *slot = byte;
                    }
                    assert_base64(&bytes);
                }
            }
        }
    }

    #[test]
    fn base64_decode_matches_mail_parser_across_block_boundaries() {
        for len in (0..=40).chain([47, 48, 49, 63, 64, 65]) {
            let run = BASE64_ALPHABET
                .iter()
                .copied()
                .cycle()
                .take(len)
                .collect::<Vec<_>>();
            for suffix in [
                &b""[..],
                b"=",
                b"==",
                b"A=",
                b"AB=",
                b"ABC=",
                b"=QUJD",
                b"\r\n ",
                b"\t",
                b"-",
                b"\xff",
                b"\xc3\xa9",
            ] {
                assert_base64(&[run.as_slice(), suffix].concat());
            }
            for position in 0..=len {
                let (head, tail) = run.split_at(position);
                for insert in [&b"="[..], b"==", b"\r\n ", b"-", b"\xff"] {
                    assert_base64(&[head, insert, tail].concat());
                }
            }
        }
    }

    #[test]
    fn base64_decode_flushes_tails_and_padding_like_mail_parser() {
        for (input, expected) in [
            (&b"QUJDRA"[..], Some(&b"ABCD"[..])),
            (b"QUI", Some(b"AB")),
            (b"QQ", Some(b"A")),
            (b"Q", Some(b"")),
            (b"QUJDQ=", Some(b"ABC")),
            (b"QQ=", Some(b"A")),
            (b"QUI=", Some(b"AB")),
            (b"QQ==QUJD", Some(b"AABC")),
            (b"/9", Some(b"\xff")),
            (b"QU\xff", Some(b"A")),
            (b"QUJ\xffQUJD", Some(b"AB")),
            (b"QU*", None),
        ] {
            assert_eq!(
                Encoding::Base64.decode(input).as_deref(),
                expected,
                "{input:?}"
            );
        }
    }

    fn calendar_value(value: &str) -> Option<ICalendarValue> {
        ICalendar::parse(format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nATTACH;ENCODING=BASE64;VALUE=BINARY:{value}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        ))
        .expect("the calendar parses")
        .components
        .get(1)
        .and_then(|component| component.entries.first())
        .and_then(|entry| entry.values.first())
        .cloned()
    }

    fn card_bytes(line: &str) -> Option<Vec<u8>> {
        VCard::parse(format!(
            "BEGIN:VCARD\r\nVERSION:4.0\r\n{line}\r\nEND:VCARD\r\n"
        ))
        .expect("the card parses")
        .entries
        .iter()
        .find_map(|entry| match entry.values.first() {
            Some(VCardValue::Binary(data)) => Some(data.data.clone()),
            _ => None,
        })
    }

    #[test]
    fn base64_value_without_padding_keeps_its_last_bytes() {
        for (value, expected) in [
            ("QUJDRA", &b"ABCD"[..]),
            ("QUJD\r\n RA", b"ABCD"),
            ("QU\r\n I", b"AB"),
        ] {
            assert_eq!(
                calendar_value(value),
                Some(ICalendarValue::Binary(expected.to_vec())),
                "{value:?}"
            );
        }
        assert_eq!(
            card_bytes("PHOTO;ENCODING=b:QUI").as_deref(),
            Some(&b"AB"[..])
        );
        assert_eq!(
            card_bytes("O;ENCODING=BASE64:/9").as_deref(),
            Some(&b"\xff"[..])
        );
        assert_eq!(
            Data::try_parse(b"data:;base64,QUJDRA")
                .map(|data| data.data)
                .as_deref(),
            Some(&b"ABCD"[..])
        );
    }

    #[test]
    fn base64_padding_after_a_single_character_adds_no_byte() {
        for value in ["QUJDQ=", "QUJD\r\n Q=", "QUJDQ\r\n ="] {
            assert_eq!(
                calendar_value(value),
                Some(ICalendarValue::Binary(b"ABC".to_vec())),
                "{value:?}"
            );
        }
    }

    #[test]
    fn quoted_printable_decode_matches_mail_parser() {
        const ALPHABET: &[u8] = b"abcdefXYZ0123456789ABCDEF =\t\r\n\x0c.;:,-_\\\"'";
        const PIECES: &[&[u8]] = &[
            b"=\r\n",
            b"=\n",
            b"=0D=0A",
            b"=C3=A9",
            b"=c3=bc",
            b"=3D",
            b"=20",
            b"=09",
            b"= \n",
            b"=\t\r\n",
            b"=G1",
            b"=A",
            b"==",
            b"=\r=\n",
            b"  \r\n",
            b"\t\n",
            b"\xc3\xa9",
            b"\xff",
            b"=\r\nA",
            b" =\n",
            b"=4",
        ];
        let mut rng = XorShift::new(0x0e0e_0e0e_0000_0001);
        for _ in 0..RANDOM_INPUTS {
            assert_quoted_printable(&input(&mut rng, ALPHABET, PIECES));
        }
        for first in 0..=u8::MAX {
            for second in [b'0', b'a', b'F', b'g', b' ', b'\r', b'\n', b'=', 0xff] {
                for prefix in [&b""[..], b"a", b"a ", b"="] {
                    assert_quoted_printable(&[prefix, b"=", &[first], &[second], b" x\n"].concat());
                }
            }
        }
    }

    #[test]
    fn quoted_printable_decode_matches_mail_parser_across_block_boundaries() {
        let text = b"abcdefghijklmnopqrstuvwxyz0123456789"
            .iter()
            .copied()
            .cycle()
            .take(80)
            .collect::<Vec<_>>();
        for position in 0..=text.len() {
            let (head, tail) = text.split_at(position);
            for piece in [
                &b"=\r\n"[..],
                b"=\n",
                b"= \r\n",
                b" \r\n",
                b" \t\n",
                b"\r\n",
                b"=41",
                b"=4",
                b"=",
                b"=G1",
                b"==41",
                b"=\r=\n",
                b"=0D=0A",
            ] {
                assert_quoted_printable(&[head, piece, tail].concat());
            }
        }
    }

    #[test]
    fn value_decoder_applies_the_charset_and_falls_back_to_binary() {
        let text = |text: &str| Decoded::Text(text.to_string());
        let binary = |bytes: &[u8]| Decoded::Binary(bytes.to_vec());
        for (encoding, charset, is_binary, input, payload, expected) in [
            (Encoding::Base64, None, true, "QUJD", None, binary(b"ABC")),
            (Encoding::Base64, None, false, "QUJD", None, text("ABC")),
            (Encoding::Base64, None, false, "/w==", None, binary(&[0xff])),
            (
                Encoding::Base64,
                Some("iso-8859-1"),
                false,
                "Y2Fm6Q==",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::Base64,
                Some("x-unknown"),
                false,
                "Y2Fmw6k=",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::Base64,
                None,
                true,
                "QU*D",
                None,
                Decoded::Undecodable,
            ),
            (
                Encoding::Base64,
                None,
                true,
                "",
                Some(vec![1, 2, 3]),
                binary(&[1, 2, 3]),
            ),
            (
                Encoding::Base64,
                None,
                false,
                "QU*D",
                Some(b"ABC".to_vec()),
                text("ABC"),
            ),
            (
                Encoding::QuotedPrintable,
                None,
                false,
                "caf=E9",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::QuotedPrintable,
                Some("UTF-8"),
                false,
                "caf=C3=\r\n=A9",
                None,
                text("caf\u{e9}"),
            ),
            (
                Encoding::QuotedPrintable,
                Some("utf-8"),
                false,
                "caf=C3",
                None,
                binary(b"caf\xc3"),
            ),
            (
                Encoding::QuotedPrintable,
                None,
                true,
                "=00=FF",
                None,
                binary(&[0, 0xff]),
            ),
            (
                Encoding::QuotedPrintable,
                None,
                false,
                "=G1",
                None,
                Decoded::Undecodable,
            ),
        ] {
            let decoder = ValueDecoder {
                encoding,
                charset,
                binary: is_binary,
            };
            assert_eq!(
                decoder.decode(input.as_bytes(), payload),
                expected,
                "{encoding:?} {charset:?} {input:?}"
            );
        }
    }
}
