/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{Entry, Parser};

const MAX_ENTRIES: usize = 1_000;

pub struct EntryStream<'x> {
    parser: Parser<'x>,
    remaining: usize,
}

impl<'x> EntryStream<'x> {
    pub fn new(text: &'x str, strict: bool) -> Self {
        let parser = Parser::new(text);
        Self {
            parser: if strict { parser.strict() } else { parser },
            remaining: MAX_ENTRIES,
        }
    }
}

impl Iterator for EntryStream<'_> {
    type Item = Entry;

    fn next(&mut self) -> Option<Entry> {
        self.remaining = self.remaining.checked_sub(1)?;
        match self.parser.entry() {
            Entry::Eof => {
                self.remaining = 0;
                None
            }
            entry => Some(entry),
        }
    }
}
