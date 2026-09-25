/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use calcard::{
    common::{IanaParse, LinkRelation},
    icalendar::{ICalendar, ICalendarDuration, ICalendarMonth},
    jscalendar::{JSCalendarDateTime, JSCalendarId, JSCalendarProperty, JSCalendarValue},
    jscontact::{JSContactId, JSContactProperty, JSContactValue},
    vcard::VCard,
};
use jmap_tools::{Element, JsonPointer, JsonPointerItem, Key, Property, Value};
use serde::{Serialize, Serializer, ser::SerializeMap};
use std::{
    collections::HashSet,
    fmt::{self, Debug, Display},
    io,
    mem::discriminant,
    str::FromStr,
};

const ROUNDS: usize = 2_000;
const CALENDAR_PROPERTY_VARIANTS: usize = 125;
const CALENDAR_UNIT_PROPERTIES: usize = 116;
const CONTACT_PROPERTY_VARIANTS: usize = 83;
const CONTACT_UNIT_PROPERTIES: usize = 77;
const CALENDAR_VALUE_VARIANTS: usize = 22;
const CONTACT_VALUE_VARIANTS: usize = 11;
const MIN_PER_VARIANT: usize = 10;

const SOURCES: &[&str] = &[
    include_str!("../src/common/mod.rs"),
    include_str!("../src/common/types.rs"),
    include_str!("../src/icalendar/types.rs"),
    include_str!("../src/jscalendar/types.rs"),
    include_str!("../src/jscontact/mod.rs"),
    include_str!("../src/jscontact/types.rs"),
];

const PIECES: &[&str] = &[
    "",
    "a",
    "k1",
    "~",
    "~0",
    "~1",
    "/",
    "//",
    "*",
    "0",
    "07",
    "18446744073709551615",
    "18446744073709551616",
    "\"",
    "\\",
    "\n",
    "\r\n",
    "\t",
    "\u{0}",
    "\u{1f}",
    "\u{7f}",
    "\u{e9}",
    "\u{65e5}\u{672c}",
    "\u{1f389}",
    "#",
    "#x",
    "@type",
    "2025-03-05T09:00:00",
    "2025-03-05T09:00:00Z",
    "participants",
    "recurrenceOverrides",
    "calendarIds",
    "convertedProperties",
    "localizations",
    "addressBookIds",
    "links",
    "blobId",
    "title",
];

const TIMESTAMPS: &[i64] = &[
    0,
    -1,
    1,
    86_399,
    86_400,
    951_782_400,
    1_741_165_200,
    -62_135_596_800,
    -62_135_596_801,
    -62_167_219_200,
    -62_167_219_201,
    253_402_300_799,
    253_402_300_800,
    2_066_062_521_599,
    2_066_062_521_600,
    i64::MIN,
    i64::MIN + 1,
    i64::MAX,
    i64::MAX - 1,
];

const DURATION_PARTS: &[u32] = &[
    1,
    9,
    10,
    59,
    60,
    99,
    100,
    3600,
    86_400,
    u32::MAX - 1,
    u32::MAX,
];

const CALENDAR: &str = concat!(
    "BEGIN:VCALENDAR\r\n",
    "VERSION:2.0\r\n",
    "PRODID:-//calcard//serialize_text//EN\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:e1c3a6d2-1f55-4b8e-9d0c-3f1f0b7c2a10\r\n",
    "DTSTAMP:20250101T000000Z\r\n",
    "DTSTART;TZID=Europe/Paris:20230101T130000\r\n",
    "DURATION:P1DT2H3M4S\r\n",
    "RRULE:FREQ=YEARLY;BYMONTH=2,3;BYDAY=MO,-1FR\r\n",
    "RDATE;VALUE=PERIOD:20231223T150000Z/PT2H,20231224T150000Z/PT3H\r\n",
    "EXDATE;TZID=Europe/Paris:20230301T130000\r\n",
    "SUMMARY:caf\u{e9} \"quoted\" \\\\ back\r\n",
    "ORGANIZER;CN=\"Jane \\\"J\\\" Doe\":mailto:jane@example.com\r\n",
    "ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED;X-NOTE=\u{65e5}\u{672c}:mailto:bob@example.com\r\n",
    "ATTACH;FMTTYPE=text/plain:https://example.com/a~b/c\r\n",
    "RELATED-TO;RELTYPE=PARENT:parent-uid\r\n",
    "X-CUSTOM;X-PARAM=1:value\r\n",
    "BEGIN:VALARM\r\n",
    "ACTION:DISPLAY\r\n",
    "TRIGGER:-PT15M\r\n",
    "END:VALARM\r\n",
    "END:VEVENT\r\n",
    "BEGIN:VEVENT\r\n",
    "UID:e1c3a6d2-1f55-4b8e-9d0c-3f1f0b7c2a10\r\n",
    "RECURRENCE-ID;TZID=Europe/Paris:20230201T130000\r\n",
    "DTSTART;TZID=Europe/Paris:20230202T140000\r\n",
    "SUMMARY:moved\r\n",
    "END:VEVENT\r\n",
    "END:VCALENDAR\r\n"
);

const CONTACT: &str = concat!(
    "BEGIN:VCARD\r\n",
    "VERSION:4.0\r\n",
    "UID:urn:uuid:4fbe8971-0bc3-424c-9c26-36c3e1eff6b1\r\n",
    "FN:Jane \"J\" Doe\r\n",
    "N:Doe;Jane;;;\r\n",
    "BDAY:19800412\r\n",
    "ANNIVERSARY:20100101T120000Z\r\n",
    "REV:20250101T000000Z\r\n",
    "EMAIL;TYPE=work;PREF=1:jane@example.com\r\n",
    "TEL;VALUE=uri;TYPE=cell:tel:+1-555-0100\r\n",
    "ADR;TYPE=home:;;1 Main St;Town;;12345;US\r\n",
    "TITLE;ALTID=1;LANGUAGE=en:Boss\r\n",
    "TITLE;ALTID=1;LANGUAGE=fr:Patron\r\n",
    "NOTE:caf\u{e9} \\\\ back\r\n",
    "PHOTO:data:image/png;base64,iVBORw0KGgo=\r\n",
    "X-CUSTOM;X-PARAM=1:value\r\n",
    "END:VCARD\r\n"
);

type CalProp<I> = JSCalendarProperty<I>;
type CalElem<I, B> = JSCalendarValue<I, B>;
type ConProp<I> = JSContactProperty<I>;
type ConElem<I, B> = JSContactValue<I, B>;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<'x, T>(&mut self, items: &'x [T]) -> &'x T {
        let pos = self.below(items.len());
        items.get(pos).expect("non-empty pool")
    }

    fn text(&mut self, words: &[&str]) -> String {
        let mut text = String::new();
        for _ in 0..self.below(6) {
            match self.below(8) {
                0 | 1 => text.push_str(self.pick(words)),
                2 => text.push_str(&"x".repeat(self.below(300))),
                3 => text.push(char::from_u32(self.next() as u32 % 0x11_0000).unwrap_or('?')),
                _ => text.push_str(self.pick(PIECES)),
            }
        }
        text
    }

    fn timestamp(&mut self) -> i64 {
        match self.below(6) {
            0 => *self.pick(TIMESTAMPS),
            1 => self.next() as i64,
            2 => (self.next() % 20_000_000_000_000) as i64 - 10_000_000_000_000,
            _ => (self.next() % 4_102_444_800) as i64,
        }
    }

    fn duration_part(&mut self) -> u32 {
        match self.below(6) {
            0..=2 => 0,
            3 => *self.pick(DURATION_PARTS),
            _ => self.next() as u32 >> self.below(32),
        }
    }

    fn duration(&mut self) -> ICalendarDuration {
        ICalendarDuration {
            neg: self.one_in(2),
            weeks: self.duration_part(),
            days: self.duration_part(),
            hours: self.duration_part(),
            minutes: self.duration_part(),
            seconds: self.duration_part(),
        }
    }

    fn date_time(&mut self) -> JSCalendarDateTime {
        JSCalendarDateTime::new(self.timestamp(), self.one_in(2))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
struct PieceId(u64);

impl Display for PieceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:x}\"\\{}\u{e9}", self.0, self.0 % 97)?;
        f.write_str("\n/~")
    }
}

impl FromStr for PieceId {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(PieceId).map_err(|_| ())
    }
}

trait TestId: JSCalendarId + JSContactId {
    fn random(rng: &mut Rng, words: &[&str]) -> Self;
}

impl TestId for String {
    fn random(rng: &mut Rng, words: &[&str]) -> Self {
        rng.text(words)
    }
}

impl TestId for u64 {
    fn random(rng: &mut Rng, _: &[&str]) -> Self {
        rng.next() >> rng.below(64)
    }
}

impl TestId for PieceId {
    fn random(rng: &mut Rng, _: &[&str]) -> Self {
        PieceId(rng.next() >> rng.below(64))
    }
}

struct DefaultKey<'x, P>(&'x P);

impl<P: Property> Serialize for DefaultKey<'_, P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_cow())
    }
}

struct HookedKey<'x, P>(&'x P);

impl<P: Property> Serialize for HookedKey<'_, P> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize_text(serializer)
    }
}

struct DefaultElement<'x, E>(&'x E);

impl<E: Element> Serialize for DefaultElement<'_, E> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_cow())
    }
}

struct HookedElement<'x, E>(&'x E);

impl<E: Element> Serialize for HookedElement<'_, E> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize_text(serializer)
    }
}

struct AsKey<'x, T>(&'x T);

impl<T: Serialize> Serialize for AsKey<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(self.0, &0u8)?;
        map.end()
    }
}

struct Limited {
    written: Vec<u8>,
    limit: usize,
}

impl io::Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = self.limit.saturating_sub(self.written.len());
        if room == 0 && !buf.is_empty() {
            return Err(io::Error::other("writer full"));
        }
        let (accepted, _) = buf.split_at(buf.len().min(room));
        self.written.extend_from_slice(accepted);
        Ok(accepted.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| err.to_string())
}

fn limited(value: &impl Serialize, limit: usize) -> (Vec<u8>, Result<(), String>) {
    let mut writer = Limited {
        written: Vec::new(),
        limit,
    };
    let result = serde_json::to_writer(&mut writer, value).map_err(|err| err.to_string());
    (writer.written, result)
}

fn compare(new: &impl Serialize, old: &impl Serialize, rng: &mut Rng, context: &dyn Debug) {
    let expected = json(old);
    assert_eq!(json(new), expected, "{context:?}");
    assert_eq!(json(&AsKey(new)), json(&AsKey(old)), "{context:?}");
    assert_eq!(
        serde_json::to_string_pretty(&AsKey(new)).ok(),
        serde_json::to_string_pretty(&AsKey(old)).ok(),
        "{context:?}"
    );
    assert_eq!(
        serde_json::to_value(new).ok(),
        serde_json::to_value(old).ok(),
        "{context:?}"
    );
    assert_eq!(
        serde_json::to_value(AsKey(new)).ok(),
        serde_json::to_value(AsKey(old)).ok(),
        "{context:?}"
    );
    if rng.one_in(4) {
        let limit = rng.below(expected.map_or(0, |text| text.len()) + 2);
        assert_eq!(limited(new, limit), limited(old, limit), "{context:?}");
    }
}

fn compare_key<P: Property>(property: &P, rng: &mut Rng) {
    compare(&HookedKey(property), &DefaultKey(property), rng, property);
}

fn compare_element<E: Element>(element: &E, rng: &mut Rng) {
    compare(
        &HookedElement(element),
        &DefaultElement(element),
        rng,
        element,
    );
}

fn compare_document<P: Property, E: Element>(value: &Value<'_, P, E>, expected_texts: &[&str]) {
    let expected = serde_json::Value::from(value);
    for text in [
        serde_json::to_string(value).expect("serializes"),
        serde_json::to_string_pretty(value).expect("serializes"),
    ] {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).ok(),
            Some(expected.clone()),
            "{text}"
        );
        for expected_text in expected_texts {
            assert!(text.contains(expected_text), "{expected_text} in {text}");
        }
    }
}

fn vocabulary() -> Vec<&'static str> {
    let mut words: Vec<&'static str> = SOURCES
        .iter()
        .flat_map(|source| source.split('"').skip(1).step_by(2))
        .filter(|word| !word.is_empty() && word.len() <= 64)
        .collect();
    words.sort_unstable();
    words.dedup();
    words
}

struct Calendar<I: JSCalendarId, B: JSCalendarId> {
    words: Vec<&'static str>,
    units: Vec<CalProp<I>>,
    named: Vec<CalProp<I>>,
    parents: Vec<CalProp<I>>,
    statics: Vec<Vec<CalElem<I, B>>>,
}

impl<I: TestId, B: TestId> Calendar<I, B> {
    fn new() -> Self {
        let words = vocabulary();
        let units: Vec<CalProp<I>> = words
            .iter()
            .filter_map(|word| CalProp::from_str(word).ok())
            .collect();
        let named_parents = [
            CalProp::Display,
            CalProp::Features,
            CalProp::Roles,
            CalProp::Relation,
        ];
        let named = named_parents
            .iter()
            .flat_map(|parent| {
                words.iter().filter_map(move |word| {
                    CalProp::try_parse(Some(&Key::Property(parent.clone())), word)
                })
            })
            .filter(|property| !units.contains(property))
            .chain(
                words
                    .iter()
                    .filter_map(|word| LinkRelation::parse(word.as_bytes()))
                    .map(CalProp::LinkRelation),
            )
            .collect();
        let static_parents = [
            CalProp::Type,
            CalProp::Action,
            CalProp::FreeBusyStatus,
            CalProp::Kind,
            CalProp::ParticipationStatus,
            CalProp::Privacy,
            CalProp::Progress,
            CalProp::RelativeTo,
            CalProp::ScheduleAgent,
            CalProp::Status,
            CalProp::Rel,
            CalProp::Frequency,
            CalProp::Rscale,
            CalProp::Skip,
            CalProp::Day,
            CalProp::Method,
        ];
        let statics: Vec<Vec<CalElem<I, B>>> = static_parents
            .iter()
            .map(|parent| {
                words
                    .iter()
                    .filter_map(|word| {
                        CalElem::try_parse::<()>(&Key::Property(parent.clone()), word)
                    })
                    .collect()
            })
            .collect();
        assert!(statics.iter().all(|pool| !pool.is_empty()));
        assert_eq!(
            units.iter().map(discriminant).collect::<HashSet<_>>().len(),
            CALENDAR_UNIT_PROPERTIES
        );
        let parents = [
            CalProp::RecurrenceOverrides,
            CalProp::ConvertedProperties,
            CalProp::CalendarIds,
            CalProp::Participants,
            CalProp::Links,
            CalProp::Display,
            CalProp::Roles,
        ]
        .into_iter()
        .collect();
        Calendar {
            words,
            units,
            named,
            parents,
            statics,
        }
    }

    fn property(&self, rng: &mut Rng, depth: usize) -> CalProp<I> {
        match rng.below(10) {
            0..=2 => rng.pick(&self.units).clone(),
            3 => rng.pick(&self.named).clone(),
            4 => CalProp::DateTime(rng.date_time()),
            5 | 6 if depth < 3 => CalProp::Pointer(self.pointer(rng, depth + 1)),
            7 => CalProp::IdValue(I::random(rng, &self.words)),
            8 => CalProp::IdReference(rng.text(&self.words)),
            _ => CalProp::try_parse(None, &rng.text(&self.words)).unwrap_or(CalProp::Title),
        }
    }

    fn pointer(&self, rng: &mut Rng, depth: usize) -> JsonPointer<CalProp<I>> {
        if rng.one_in(2) {
            let text = (0..rng.below(6))
                .map(|_| {
                    if rng.one_in(3) {
                        rng.pick(&self.words).to_string()
                    } else {
                        rng.text(&self.words)
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            match rng.below(3) {
                0 => JsonPointer::parse(&text),
                1 => match CalProp::try_parse(
                    Some(&Key::Property(rng.pick(&self.parents).clone())),
                    &text,
                ) {
                    Some(CalProp::Pointer(pointer)) => pointer,
                    _ => JsonPointer::parse(&text),
                },
                _ => JsonPointer::new(vec![JsonPointerItem::Key(Key::Owned(
                    "y".repeat(100 + rng.below(40)),
                ))]),
            }
        } else {
            let items = (0..rng.below(6))
                .map(|_| match rng.below(7) {
                    0 => JsonPointerItem::Root,
                    1 => JsonPointerItem::Wildcard,
                    2 => JsonPointerItem::Invalid(rng.text(&self.words)),
                    3 => JsonPointerItem::Number(rng.next() >> rng.below(64)),
                    4 => JsonPointerItem::Key(Key::Owned(rng.text(&self.words))),
                    5 => JsonPointerItem::Key(Key::Borrowed(rng.pick(PIECES))),
                    _ => JsonPointerItem::Key(Key::Property(self.property(rng, depth))),
                })
                .collect();
            JsonPointer::new(items)
        }
    }

    fn element(&self, rng: &mut Rng) -> CalElem<I, B> {
        match rng.below(self.statics.len() + 6) {
            0 => CalElem::DateTime(rng.date_time()),
            1 => CalElem::Duration(rng.duration()),
            2 => CalElem::Month(ICalendarMonth::new(rng.next() as u8, rng.one_in(2))),
            3 => CalElem::Id(I::random(rng, &self.words)),
            4 => CalElem::BlobId(B::random(rng, &self.words)),
            5 => CalElem::IdReference(rng.text(&self.words)),
            kind => rng
                .pick(self.statics.get(kind - 6).expect("static pool"))
                .clone(),
        }
    }
}

trait Ordinal {
    fn ordinal(&self) -> usize;
}

impl<I: JSCalendarId, B: JSCalendarId> Ordinal for CalElem<I, B> {
    fn ordinal(&self) -> usize {
        match self {
            JSCalendarValue::Type(_) => 0,
            JSCalendarValue::DateTime(_) => 1,
            JSCalendarValue::Duration(_) => 2,
            JSCalendarValue::AlertAction(_) => 3,
            JSCalendarValue::FreeBusyStatus(_) => 4,
            JSCalendarValue::ParticipantKind(_) => 5,
            JSCalendarValue::ParticipationStatus(_) => 6,
            JSCalendarValue::Privacy(_) => 7,
            JSCalendarValue::Progress(_) => 8,
            JSCalendarValue::RelativeTo(_) => 9,
            JSCalendarValue::ScheduleAgent(_) => 10,
            JSCalendarValue::EventStatus(_) => 11,
            JSCalendarValue::LinkRelation(_) => 12,
            JSCalendarValue::Frequency(_) => 13,
            JSCalendarValue::CalendarScale(_) => 14,
            JSCalendarValue::Skip(_) => 15,
            JSCalendarValue::Weekday(_) => 16,
            JSCalendarValue::Month(_) => 17,
            JSCalendarValue::Method(_) => 18,
            JSCalendarValue::Id(_) => 19,
            JSCalendarValue::BlobId(_) => 20,
            JSCalendarValue::IdReference(_) => 21,
        }
    }
}

impl<I: JSContactId, B: JSContactId> Ordinal for ConElem<I, B> {
    fn ordinal(&self) -> usize {
        match self {
            JSContactValue::Id(_) => 0,
            JSContactValue::BlobId(_) => 1,
            JSContactValue::IdReference(_) => 2,
            JSContactValue::Timestamp(_) => 3,
            JSContactValue::Type(_) => 4,
            JSContactValue::GrammaticalGender(_) => 5,
            JSContactValue::Kind(_) => 6,
            JSContactValue::Level(_) => 7,
            JSContactValue::Relation(_) => 8,
            JSContactValue::PhoneticSystem(_) => 9,
            JSContactValue::CalendarScale(_) => 10,
        }
    }
}

fn check_calendar<I: TestId, B: TestId>(seed: u64, rounds: usize) {
    let calendar = Calendar::<I, B>::new();
    let mut rng = Rng(seed);
    let mut properties = HashSet::new();
    let mut values = [0usize; CALENDAR_VALUE_VARIANTS];
    for property in calendar.units.iter().chain(&calendar.named) {
        properties.insert(discriminant(property));
        compare_key(property, &mut rng);
    }
    for element in calendar.statics.iter().flatten() {
        if let Some(count) = values.get_mut(element.ordinal()) {
            *count += 1;
        }
        compare_element(element, &mut rng);
    }
    for _ in 0..rounds {
        let property = calendar.property(&mut rng, 0);
        properties.insert(discriminant(&property));
        compare_key(&property, &mut rng);
        let element = calendar.element(&mut rng);
        if let Some(count) = values.get_mut(element.ordinal()) {
            *count += 1;
        }
        compare_element(&element, &mut rng);
    }
    assert_eq!(properties.len(), CALENDAR_PROPERTY_VARIANTS);
    assert!(
        values.iter().all(|count| *count >= MIN_PER_VARIANT),
        "{values:?}"
    );
}

struct Contact<I: JSContactId, B: JSContactId> {
    words: Vec<&'static str>,
    units: Vec<ConProp<I>>,
    named: Vec<ConProp<I>>,
    parents: Vec<ConProp<I>>,
    statics: Vec<Vec<ConElem<I, B>>>,
}

impl<I: TestId, B: TestId> Contact<I, B> {
    fn new() -> Self {
        let words = vocabulary();
        let units: Vec<ConProp<I>> = words
            .iter()
            .filter_map(|word| ConProp::from_str(word).ok())
            .collect();
        let named_parents = [ConProp::Contexts, ConProp::Features, ConProp::SortAs];
        let named = named_parents
            .iter()
            .flat_map(|parent| {
                words.iter().filter_map(move |word| {
                    ConProp::try_parse(Some(&Key::Property(parent.clone())), word)
                })
            })
            .filter(|property| !units.contains(property))
            .collect();
        let static_parents = [
            ConProp::Type,
            ConProp::CalendarScale,
            ConProp::Kind,
            ConProp::GrammaticalGender,
            ConProp::PhoneticSystem,
            ConProp::Relation,
            ConProp::Level,
        ];
        let statics: Vec<Vec<ConElem<I, B>>> = static_parents
            .iter()
            .map(|parent| {
                words
                    .iter()
                    .filter_map(|word| {
                        ConElem::try_parse::<()>(&Key::Property(parent.clone()), word)
                    })
                    .collect()
            })
            .collect();
        assert!(statics.iter().all(|pool| !pool.is_empty()));
        assert_eq!(
            units.iter().map(discriminant).collect::<HashSet<_>>().len(),
            CONTACT_UNIT_PROPERTIES
        );
        let parents = [
            ConProp::ConvertedProperties,
            ConProp::Localizations,
            ConProp::AddressBookIds,
            ConProp::Phones,
            ConProp::Contexts,
        ]
        .into_iter()
        .collect();
        Contact {
            words,
            units,
            named,
            parents,
            statics,
        }
    }

    fn property(&self, rng: &mut Rng, depth: usize) -> ConProp<I> {
        match rng.below(9) {
            0..=2 => rng.pick(&self.units).clone(),
            3 => rng.pick(&self.named).clone(),
            4 | 5 if depth < 3 => ConProp::Pointer(self.pointer(rng, depth + 1)),
            6 => ConProp::IdValue(I::random(rng, &self.words)),
            7 => ConProp::IdReference(rng.text(&self.words)),
            _ => ConProp::try_parse(None, &rng.text(&self.words)).unwrap_or(ConProp::Name),
        }
    }

    fn pointer(&self, rng: &mut Rng, depth: usize) -> JsonPointer<ConProp<I>> {
        if rng.one_in(2) {
            let text = (0..rng.below(6))
                .map(|_| {
                    if rng.one_in(3) {
                        rng.pick(&self.words).to_string()
                    } else {
                        rng.text(&self.words)
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            match rng.below(3) {
                0 => JsonPointer::parse(&text),
                1 => match ConProp::try_parse(
                    Some(&Key::Property(rng.pick(&self.parents).clone())),
                    &text,
                ) {
                    Some(ConProp::Pointer(pointer)) => pointer,
                    _ => JsonPointer::parse(&text),
                },
                _ => JsonPointer::new(vec![JsonPointerItem::Key(Key::Owned(
                    "y".repeat(100 + rng.below(40)),
                ))]),
            }
        } else {
            let items = (0..rng.below(6))
                .map(|_| match rng.below(7) {
                    0 => JsonPointerItem::Root,
                    1 => JsonPointerItem::Wildcard,
                    2 => JsonPointerItem::Invalid(rng.text(&self.words)),
                    3 => JsonPointerItem::Number(rng.next() >> rng.below(64)),
                    4 => JsonPointerItem::Key(Key::Owned(rng.text(&self.words))),
                    5 => JsonPointerItem::Key(Key::Borrowed(rng.pick(PIECES))),
                    _ => JsonPointerItem::Key(Key::Property(self.property(rng, depth))),
                })
                .collect();
            JsonPointer::new(items)
        }
    }

    fn element(&self, rng: &mut Rng) -> ConElem<I, B> {
        match rng.below(self.statics.len() + 4) {
            0 => ConElem::Timestamp(rng.timestamp()),
            1 => ConElem::Id(I::random(rng, &self.words)),
            2 => ConElem::BlobId(B::random(rng, &self.words)),
            3 => ConElem::IdReference(rng.text(&self.words)),
            kind => rng
                .pick(self.statics.get(kind - 4).expect("static pool"))
                .clone(),
        }
    }
}

fn check_contact<I: TestId, B: TestId>(seed: u64, rounds: usize) {
    let contact = Contact::<I, B>::new();
    let mut rng = Rng(seed);
    let mut properties = HashSet::new();
    let mut values = [0usize; CONTACT_VALUE_VARIANTS];
    for property in contact.units.iter().chain(&contact.named) {
        properties.insert(discriminant(property));
        compare_key(property, &mut rng);
    }
    for element in contact.statics.iter().flatten() {
        if let Some(count) = values.get_mut(element.ordinal()) {
            *count += 1;
        }
        compare_element(element, &mut rng);
    }
    for _ in 0..rounds {
        let property = contact.property(&mut rng, 0);
        properties.insert(discriminant(&property));
        compare_key(&property, &mut rng);
        let element = contact.element(&mut rng);
        if let Some(count) = values.get_mut(element.ordinal()) {
            *count += 1;
        }
        compare_element(&element, &mut rng);
    }
    assert_eq!(properties.len(), CONTACT_PROPERTY_VARIANTS);
    assert!(
        values.iter().all(|count| *count >= MIN_PER_VARIANT),
        "{values:?}"
    );
}

#[test]
fn calendar_text_matches_default_path() {
    check_calendar::<String, String>(0x5EED_0001, ROUNDS);
    check_calendar::<u64, PieceId>(0x5EED_0002, ROUNDS / 4);
    check_calendar::<PieceId, u64>(0x5EED_0003, ROUNDS / 4);
}

#[test]
fn contact_text_matches_default_path() {
    check_contact::<String, String>(0x5EED_0004, ROUNDS);
    check_contact::<u64, PieceId>(0x5EED_0005, ROUNDS / 4);
    check_contact::<PieceId, u64>(0x5EED_0006, ROUNDS / 4);
}

#[test]
fn pointer_text_around_the_stack_buffer() {
    let mut rng = Rng(0x5EED_0007);
    for len in 0..=300 {
        for pointer in [
            JsonPointer::<CalProp<String>>::new(vec![JsonPointerItem::Key(Key::Owned(
                "p".repeat(len),
            ))]),
            JsonPointer::new(vec![
                JsonPointerItem::Key(Key::Property(CalProp::Participants)),
                JsonPointerItem::Key(Key::Owned("\u{e9}".repeat(len / 2))),
                JsonPointerItem::Key(Key::Owned("~/".repeat(len % 7))),
            ]),
        ] {
            compare_key(&CalProp::Pointer(pointer), &mut rng);
        }
        let pointer = JsonPointer::<ConProp<String>>::new(vec![JsonPointerItem::Key(Key::Owned(
            "c".repeat(len),
        ))]);
        compare_key(&ConProp::Pointer(pointer), &mut rng);
    }
}

#[test]
fn documents_serialize_like_their_to_cow_text() {
    let calendar = ICalendar::parse(CALENDAR)
        .expect("valid iCalendar")
        .into_jscalendar::<String, String>();
    compare_document(
        &calendar.0,
        &[
            "\"recurrenceOverrides/2023-12-23T16:00:00\"",
            "\"2023-01-01T13:00:00\"",
            "\"P1DT2H3M4S\"",
            "\"-PT15M\"",
        ],
    );
    let contact = VCard::parse(CONTACT)
        .expect("valid vCard")
        .into_jscontact::<String, String>();
    compare_document(&contact.0, &["\"2025-01-01T00:00:00Z\""]);
}
