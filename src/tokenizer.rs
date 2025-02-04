use std::{
    borrow::Cow,
    iter::{Enumerate, Peekable},
    slice::Iter,
};

pub struct Parser<'x> {
    input: &'x [u8],
    iter: Peekable<Enumerate<Iter<'x, u8>>>,
    strict: bool,
    stop_colon: bool,
    stop_semicolon: bool,
    stop_comma: bool,
    stop_equal: bool,
    stop_slash: bool,
    unfold_qp: bool,
    unquote: bool,
}

pub(crate) struct Token<'x> {
    text: Cow<'x, [u8]>,
    start: usize,
    end: usize,
    stop_char: u8,
}

impl<'x> Parser<'x> {
    pub fn new(input: &'x [u8]) -> Self {
        Self {
            input,
            iter: input.iter().enumerate().peekable(),
            strict: false,
            stop_colon: true,
            stop_semicolon: true,
            stop_comma: true,
            stop_equal: true,
            stop_slash: false,
            unfold_qp: false,
            unquote: true,
        }
    }

    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    pub fn qp_value(&mut self) {
        self.stop_colon = false;
        self.stop_semicolon = true;
        self.stop_comma = false;
        self.stop_equal = false;
        self.stop_slash = false;
        self.unfold_qp = true;
        self.unquote = false;
    }

    fn try_unfold(&mut self) -> bool {
        if let Some((_, next)) = self.iter.peek() {
            if **next == b' ' || **next == b'\t' {
                self.iter.next();
                return true;
            }
        }
        false
    }

    pub(crate) fn token(&mut self) -> Option<Token<'x>> {
        let mut offset_start = usize::MAX;
        let mut offset_end = usize::MAX;
        let mut in_quote = false;
        let last_char;
        let mut buf: Vec<u8> = vec![];

        'outer: loop {
            let (idx, ch) = if let Some(next) = self.iter.next() {
                next
            } else if offset_start != usize::MAX {
                last_char = b'\n';
                break;
            } else {
                return None;
            };

            match ch {
                b' ' | b'\t' => {
                    if in_quote || buf.last().is_some_and(|ch| !ch.is_ascii_whitespace()) {
                        if offset_start == usize::MAX {
                            offset_start = idx;
                        }
                        offset_end = idx;

                        if !buf.is_empty() {
                            buf.push(*ch);
                        }
                    }
                }
                b'\r' => {}
                b'\n' => {
                    if self.unfold_qp
                        && buf.last().or_else(|| self.input.get(offset_end)).copied() == Some(b'=')
                    {
                        offset_end = idx;

                        if !buf.is_empty() {
                            buf.push(*ch);
                        }
                    } else if self.try_unfold() {
                        if buf.is_empty() && offset_start != usize::MAX {
                            buf.extend_from_slice(&self.input[offset_start..=offset_end]);
                        }
                    } else if offset_start != usize::MAX {
                        last_char = b'\n';
                        break;
                    }
                }
                b'\\' => {
                    let mut next_ch = b'\\';
                    let mut next_offset_end = idx;
                    while let Some((idx, ch)) = self.iter.next() {
                        match ch {
                            b' ' | b'\t' | b'\r' => {}
                            b'\n' => {
                                if self.try_unfold() {
                                    if let Some((idx, ch)) = self.iter.next() {
                                        next_ch = *ch;
                                        next_offset_end = idx;
                                        break;
                                    }
                                } else {
                                    last_char = b'\n';
                                    offset_end = idx - 1;
                                    break 'outer;
                                }
                            }
                            _ => {
                                next_ch = *ch;
                                next_offset_end = idx;
                                break;
                            }
                        }
                    }
                    if offset_start != usize::MAX {
                        if buf.is_empty() {
                            buf.extend_from_slice(&self.input[offset_start..=offset_end]);
                        }
                    } else {
                        offset_start = next_offset_end;
                    }
                    buf.push(match next_ch {
                        b'n' | b'N' => b'\n',
                        b't' | b'T' => b'\t',
                        b'r' | b'R' => b'\r',
                        _ => next_ch,
                    });
                    offset_end = next_offset_end;
                }
                b'"' if self.unquote => {
                    in_quote = !in_quote;
                }
                b':' if !in_quote && self.stop_colon => {
                    last_char = b':';
                    break;
                }
                b';' if !in_quote && self.stop_semicolon => {
                    last_char = b';';
                    break;
                }
                b',' if !in_quote && self.stop_comma => {
                    last_char = b',';
                    break;
                }
                b'=' if !in_quote && self.stop_equal => {
                    last_char = b'=';
                    break;
                }
                b'/' if !in_quote && self.stop_slash => {
                    last_char = b'/';
                    break;
                }
                _ => {
                    if offset_start == usize::MAX {
                        offset_start = idx;
                    }
                    offset_end = idx;

                    if !buf.is_empty() {
                        buf.push(*ch);
                    }
                }
            }
        }

        if buf.is_empty() {
            if offset_start != usize::MAX {
                Some(Token {
                    text: Cow::Borrowed(&self.input[offset_start..=offset_end]),
                    start: offset_start,
                    end: offset_end,
                    stop_char: last_char,
                })
            } else {
                Some(Token {
                    text: Cow::Borrowed(b"".as_ref()),
                    start: offset_start,
                    end: offset_end,
                    stop_char: last_char,
                })
            }
        } else {
            Some(Token {
                text: Cow::Owned(buf),
                start: offset_start,
                end: offset_end,
                stop_char: last_char,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum TextOwner {
        Borrowed(&'static str),
        Owned(String),
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
                    (TextOwner::Borrowed("NOTE"), ':'),
                    (
                        TextOwner::Owned(
                            "This is a long description that exists on a long line.".into(),
                        ),
                        '\n',
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
                    (TextOwner::Borrowed("this is a text value"), '\n'),
                    (TextOwner::Borrowed("this is one value"), ','),
                    (TextOwner::Borrowed("this is another"), '\n'),
                    (
                        TextOwner::Owned("this is a single value, with a comma encoded".into()),
                        '\n',
                    ),
                ],
                b"".as_slice(),
            ),
            (
                concat!("N;ALTID=1;LANGUAGE=en:Yamada;Taro;;;"),
                vec![
                    (TextOwner::Borrowed("N"), ';'),
                    (TextOwner::Borrowed("ALTID"), '='),
                    (TextOwner::Borrowed("1"), ';'),
                    (TextOwner::Borrowed("LANGUAGE"), '='),
                    (TextOwner::Borrowed("en"), ':'),
                    (TextOwner::Borrowed("Yamada"), ';'),
                    (TextOwner::Borrowed("Taro"), ';'),
                    (TextOwner::Borrowed(""), ';'),
                    (TextOwner::Borrowed(""), ';'),
                ],
                b"".as_slice(),
            ),
            (
                concat!("N;SORT-AS=\"Mann,James\":de Mann;Henry,James;;"),
                vec![
                    (TextOwner::Borrowed("N"), ';'),
                    (TextOwner::Borrowed("SORT-AS"), '='),
                    (TextOwner::Borrowed("Mann,James"), ':'),
                    (TextOwner::Borrowed("de Mann"), ';'),
                    (TextOwner::Borrowed("Henry"), ','),
                    (TextOwner::Borrowed("James"), ';'),
                    (TextOwner::Borrowed(""), ';'),
                ],
                b"".as_slice(),
            ),
            (
                concat!("  hello\\ nworld\\\\"),
                vec![(TextOwner::Owned("hello\nworld\\".into()), '\n')],
                b"".as_slice(),
            ),
            (
                concat!(
                    "X-ABC-MMSUBJ;VALUE=URI;FMTTYPE=audio/basic:http://www.example.\n",
                    " org/mysubj.au"
                ),
                vec![
                    (TextOwner::Borrowed("X-ABC-MMSUBJ"), ';'),
                    (TextOwner::Borrowed("VALUE"), '='),
                    (TextOwner::Borrowed("URI"), ';'),
                    (TextOwner::Borrowed("FMTTYPE"), '='),
                    (TextOwner::Borrowed("audio/basic"), ':'),
                    (TextOwner::Borrowed("http"), ':'),
                    (TextOwner::Owned("//www.example.org/mysubj.au".into()), '\n'),
                ],
                b"".as_slice(),
            ),
            (
                concat!("RDATE;VALUE=DATE:19970304,19970504,19970704,19970904"),
                vec![
                    (TextOwner::Borrowed("RDATE"), ';'),
                    (TextOwner::Borrowed("VALUE"), '='),
                    (TextOwner::Borrowed("DATE"), ':'),
                    (TextOwner::Borrowed("19970304"), ','),
                    (TextOwner::Borrowed("19970504"), ','),
                    (TextOwner::Borrowed("19970704"), ','),
                    (TextOwner::Borrowed("19970904"), '\n'),
                ],
                b"".as_slice(),
            ),
            (
                concat!(" BEGIN; ::\n \n \n test"),
                vec![
                    (TextOwner::Borrowed("BEGIN"), ';'),
                    (TextOwner::Borrowed(""), ':'),
                    (TextOwner::Borrowed(""), ':'),
                    (TextOwner::Borrowed("test"), '\n'),
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
                    (TextOwner::Borrowed("DESCRIPTION"), ';'),
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
                        '\n',
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
                    (TextOwner::Borrowed("ATTACH"), ';'),
                    (TextOwner::Borrowed("FMTTYPE"), '='),
                    (TextOwner::Borrowed("text/plain"), ';'),
                    (TextOwner::Borrowed("ENCODING"), '='),
                    (TextOwner::Borrowed("BASE64"), ';'),
                    (TextOwner::Borrowed("VALUE"), '='),
                    (TextOwner::Borrowed("BINARY"), ':'),
                    (
                        TextOwner::Owned(
                            "VGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZy4".into(),
                        ),
                        '\n',
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
                    (TextOwner::Borrowed("DESCRIPTION"), ';'),
                    (TextOwner::Borrowed("ALTREP"), '='),
                    (TextOwner::Borrowed("cid:part1.0001@example.org"), ':'),
                    (
                        TextOwner::Owned(
                            "The Fall'98 WildWizards Conference - - Las Vegas, NV, USA".to_string(),
                        ),
                        '\n',
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
                    (TextOwner::Borrowed("ATTENDEE"), ';'),
                    (TextOwner::Borrowed("DELEGATED-FROM"), '='),
                    (TextOwner::Borrowed("mailto:jsmith@example.com"), ':'),
                    (TextOwner::Borrowed("mailto"), ':'),
                    (TextOwner::Borrowed("jdoe@example.com"), '\n'),
                ],
                b"".as_slice(),
            ),
            (
                concat!(
                    "ATTENDEE;DELEGATED-TO=\"mailto:jdoe@example.com\",\"mailto:jqpublic\n",
                    " @example.com\":mailto:jsmith@example.com"
                ),
                vec![
                    (TextOwner::Borrowed("ATTENDEE"), ';'),
                    (TextOwner::Borrowed("DELEGATED-TO"), '='),
                    (TextOwner::Borrowed("mailto:jdoe@example.com"), ','),
                    (TextOwner::Owned("mailto:jqpublic@example.com".into()), ':'),
                    (TextOwner::Borrowed("mailto"), ':'),
                    (TextOwner::Borrowed("jsmith@example.com"), '\n'),
                ],
                b"".as_slice(),
            ),
            (
                concat!("CATEGORIES:cat1  ,  cat2,   cat3"),
                vec![
                    (TextOwner::Borrowed("CATEGORIES"), ':'),
                    (TextOwner::Borrowed("cat1"), ','),
                    (TextOwner::Borrowed("cat2"), ','),
                    (TextOwner::Borrowed("cat3"), '\n'),
                ],
                b"".as_slice(),
            ),
            (
                concat!("SUMMARY:Meeting\n", "\n\n", "BEGIN:VALARM"),
                vec![
                    (TextOwner::Borrowed("SUMMARY"), ':'),
                    (TextOwner::Borrowed("Meeting"), '\n'),
                    (TextOwner::Borrowed("BEGIN"), ':'),
                    (TextOwner::Borrowed("VALARM"), '\n'),
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
                    (TextOwner::Borrowed("FN"), ';'),
                    (TextOwner::Borrowed("CHARSET"), '='),
                    (TextOwner::Borrowed("UTF-8"), ';'),
                    (TextOwner::Borrowed("ENCODING"), '='),
                    (TextOwner::Borrowed("QUOTED-PRINTABLE"), ':'),
                    (
                        TextOwner::Borrowed(concat!(
                            "=D0=B3=D0=BE=D1=80=20=D0=97=D0=B0=D0=BE=D1=80",
                            "=D1=81=D0=81=D0=\n=BA=\n=D1=96=\n xyz =\n=D1=96"
                        )),
                        '\n',
                    ),
                ],
                b"".as_slice(),
            ),
            (
                concat!("\\"),
                vec![(TextOwner::Owned("\\".into()), '\n')],
                b"".as_slice(),
            ),
            (
                concat!("\\n"),
                vec![(TextOwner::Owned("\n".into()), '\n')],
                b"".as_slice(),
            ),
            (
                concat!("\\nhello"),
                vec![(TextOwner::Owned("\nhello".into()), '\n')],
                b"".as_slice(),
            ),
            (concat!(""), vec![], b"".as_slice()),
        ] {
            let mut parser = Parser::new(input.as_bytes());
            let mut tokens = vec![];
            for ch in disable_stop {
                match ch {
                    b'=' => {
                        parser.stop_equal = false;
                    }
                    b':' => {
                        parser.stop_colon = false;
                    }
                    _ => {}
                }
            }

            while let Some(token) = parser.token() {
                if token.text.eq_ignore_ascii_case(b"quoted-printable") {
                    parser.qp_value();
                }
                let text = match token.text {
                    Cow::Borrowed(text) => TextOwner::Borrowed(std::str::from_utf8(text).unwrap()),
                    Cow::Owned(text) => TextOwner::Owned(String::from_utf8(text).unwrap()),
                };
                tokens.push((text, char::from(token.stop_char)));
            }
            assert_eq!(tokens, expected);
        }
    }
}
