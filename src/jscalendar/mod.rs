/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

pub mod export;
pub mod import;
pub mod parser;
pub mod types;

use crate::{
    common::{CalendarScale, IanaString, LinkRelation},
    icalendar::{
        ICalendarComponentType, ICalendarDuration, ICalendarFrequency, ICalendarMethod,
        ICalendarMonth, ICalendarSkip, ICalendarWeekday,
    },
};
use jmap_tools::{JsonPointer, Value};
use mail_parser::DateTime;
use serde::Serialize;
use std::{borrow::Cow, fmt::Debug, fmt::Display, hash::Hash, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[repr(transparent)]
pub struct JSCalendar<'x, I: JSCalendarId, B: JSCalendarId>(
    pub Value<'x, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
);

pub trait JSCalendarId:
    FromStr + Sized + Serialize + Display + Clone + Eq + Hash + Ord + Debug + Default
{
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarValue<I: JSCalendarId, B: JSCalendarId> {
    Type(JSCalendarType),
    DateTime(JSCalendarDateTime),
    Duration(ICalendarDuration),
    AlertAction(JSCalendarAlertAction),
    FreeBusyStatus(JSCalendarFreeBusyStatus),
    ParticipantKind(JSCalendarParticipantKind),
    ParticipationStatus(JSCalendarParticipationStatus),
    Privacy(JSCalendarPrivacy),
    Progress(JSCalendarProgress),
    RelativeTo(JSCalendarRelativeTo),
    ScheduleAgent(JSCalendarScheduleAgent),
    EventStatus(JSCalendarEventStatus),
    LinkRelation(LinkRelation),
    Frequency(ICalendarFrequency),
    CalendarScale(CalendarScale),
    Skip(ICalendarSkip),
    Weekday(ICalendarWeekday),
    Month(ICalendarMonth),
    Method(ICalendarMethod),
    Id(I),
    BlobId(B),
    IdReference(String),
}

impl<I: JSCalendarId, B: JSCalendarId> Default for JSCalendarValue<I, B> {
    fn default() -> Self {
        JSCalendarValue::Type(JSCalendarType::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JSCalendarDateTime {
    pub timestamp: i64,
    pub is_local: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarProperty<I: JSCalendarId> {
    // JMAP for Calendar Properties
    Id,
    BaseEventId,
    CalendarIds,
    IsDraft,
    IsOrigin,
    UtcStart,
    UtcEnd,
    UseDefaultAlerts,
    MayInviteSelf,
    MayInviteOthers,
    HideAttendees,
    BlobId,

    // JSCalendar Properties
    #[default]
    Type,
    Acknowledged,
    Action,
    Alerts,
    ByDay,
    ByHour,
    ByMinute,
    ByMonth,
    ByMonthDay,
    BySecond,
    BySetPosition,
    ByWeekNo,
    ByYearDay,
    CalendarAddress,
    Categories,
    Color,
    ContentType,
    Coordinates,
    Count,
    Created,
    Day,
    DelegatedFrom,
    DelegatedTo,
    Description,
    DescriptionContentType,
    Display,
    Due,
    Duration,
    Email,
    Entries,
    EstimatedDuration,
    Excluded,
    ExpectReply,
    Features,
    FirstDayOfWeek,
    FreeBusyStatus,
    Frequency,
    Href,
    Interval,
    InvitedBy,
    Keywords,
    Kind,
    Links,
    Locale,
    Locations,
    LocationTypes,
    MemberOf,
    Method,
    Name,
    NthOfPeriod,
    Offset,
    Participants,
    ParticipationComment,
    ParticipationStatus,
    PercentComplete,
    Priority,
    Privacy,
    ProdId,
    Progress,
    RecurrenceId,
    RecurrenceIdTimeZone,
    RecurrenceOverrides,
    Rel,
    RelatedTo,
    Relation,
    RelativeTo,
    ReplyTo,
    RequestStatus,
    Roles,
    Rscale,
    SentBy,
    ScheduleAgent,
    ScheduleForceSend,
    ScheduleSequence,
    ScheduleStatus,
    ScheduleUpdated,
    SendTo,
    Sequence,
    ShowWithoutTime,
    Size,
    Skip,
    Source,
    Start,
    Status,
    TimeZone,
    Title,
    Trigger,
    Uid,
    Until,
    Updated,
    Uri,
    VirtualLocations,
    When,
    EndTimeZone,
    MainLocationId,
    OrganizerCalendarAddress,
    RecurrenceRule,
    ICalComponent,
    Properties,
    Parameters,
    ConvertedProperties,
    ValueType,
    Components,
    DateTime(JSCalendarDateTime),
    LinkDisplay(JSCalendarLinkDisplay),
    VirtualLocationFeature(JSCalendarVirtualLocationFeature),
    ParticipantRole(JSCalendarParticipantRole),
    RelationValue(JSCalendarRelation),
    LinkRelation(LinkRelation),
    Pointer(JsonPointer<JSCalendarProperty<I>>),
    IdValue(I),
    IdReference(String),
}

impl<T> JSCalendarId for T where
    T: FromStr + Sized + Serialize + Display + Clone + Eq + Hash + Ord + Debug + Default
{
}

impl<I: JSCalendarId> From<I> for JSCalendarProperty<I> {
    fn from(id: I) -> Self {
        JSCalendarProperty::IdValue(id)
    }
}

impl<I: JSCalendarId, B: JSCalendarId> From<I> for JSCalendarValue<I, B> {
    fn from(id: I) -> Self {
        JSCalendarValue::Id(id)
    }
}

impl<'x, I: JSCalendarId, B: JSCalendarId> Default for JSCalendar<'x, I, B> {
    fn default() -> Self {
        Self(Value::Object(jmap_tools::Map::new()))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarType {
    #[default]
    Event,
    Task,
    Group,
    Alert,
    Boolean,
    Duration,
    Id,
    Int,
    LocalDateTime,
    Link,
    Location,
    NDay,
    Number,
    Participant,
    PatchObject,
    RecurrenceRule,
    Relation,
    SignedDuration,
    String,
    TimeZone,
    TimeZoneId,
    TimeZoneRule,
    UnsignedInt,
    UTCDateTime,
    VirtualLocation,
    ICalComponent,
    OffsetTrigger,
    AbsoluteTrigger,
}

// JSCalendar Enum Values for action (Context: Alert)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarAlertAction {
    Display,
    Email,
}

// JSCalendar Enum Values for display (Context: Link)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarLinkDisplay {
    Badge,
    Graphic,
    Fullsize,
    Thumbnail,
}

// JSCalendar Enum Values for features (Context: VirtualLocation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarVirtualLocationFeature {
    Audio,
    Chat,
    Feed,
    Moderator,
    Phone,
    Screen,
    Video,
}

// JSCalendar Enum Values for freeBusyStatus (Context: Event, Task)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarFreeBusyStatus {
    Free,
    Busy,
}

// JSCalendar Enum Values for kind (Context: Participant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarParticipantKind {
    Individual,
    Group,
    Resource,
    Location,
}

// JSCalendar Enum Values for participationStatus (Context: Participant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarParticipationStatus {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
    Delegated,
}

// JSCalendar Enum Values for privacy (Context: Event, Task)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarPrivacy {
    Public,
    Private,
    Secret,
}

// JSCalendar Enum Values for progress (Context: Task, Participant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarProgress {
    NeedsAction,
    InProcess,
    Completed,
    Failed,
    Cancelled,
}

// JSCalendar Enum Values for relation (Context: Relation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarRelation {
    First,
    Next,
    Child,
    Parent,
    Snooze,
}

// JSCalendar Enum Values for relativeTo (Context: OffsetTrigger, Location)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarRelativeTo {
    Start,
    End,
}

// JSCalendar Enum Values for roles (Context: Participant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarParticipantRole {
    Owner,
    Attendee,
    Optional,
    Informational,
    Chair,
    Required,
}

// JSCalendar Enum Values for scheduleAgent (Context: Participant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarScheduleAgent {
    Server,
    Client,
    None,
}

// JSCalendar Enum Values for status (Context: Event)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JSCalendarEventStatus {
    Confirmed,
    Cancelled,
    Tentative,
}

impl JSCalendarDateTime {
    pub fn new(timestamp: i64, is_local: bool) -> Self {
        Self {
            timestamp,
            is_local,
        }
    }

    pub fn to_rfc3339(&self) -> String {
        let dt = DateTime::from_timestamp(self.timestamp);
        if !self.is_local {
            dt.to_rfc3339()
        } else {
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second,
            )
        }
    }
}

impl ICalendarComponentType {
    pub fn to_jscalendar_type(&self) -> Option<JSCalendarType> {
        match &self {
            ICalendarComponentType::VCalendar => Some(JSCalendarType::Group),
            ICalendarComponentType::VEvent => Some(JSCalendarType::Event),
            ICalendarComponentType::VTodo => Some(JSCalendarType::Task),
            ICalendarComponentType::VAlarm => Some(JSCalendarType::Alert),
            ICalendarComponentType::Participant => Some(JSCalendarType::Participant),
            ICalendarComponentType::VLocation => Some(JSCalendarType::Location),
            ICalendarComponentType::Standard
            | ICalendarComponentType::Daylight
            | ICalendarComponentType::VAvailability
            | ICalendarComponentType::Available
            | ICalendarComponentType::VResource
            | ICalendarComponentType::VStatus
            | ICalendarComponentType::VJournal
            | ICalendarComponentType::VFreebusy
            | ICalendarComponentType::VTimezone
            | ICalendarComponentType::Other(_) => None,
        }
    }
}

impl JSCalendarType {
    pub fn to_icalendar_component_type(&self) -> Option<ICalendarComponentType> {
        match &self {
            JSCalendarType::Group => Some(ICalendarComponentType::VCalendar),
            JSCalendarType::Event => Some(ICalendarComponentType::VEvent),
            JSCalendarType::Task => Some(ICalendarComponentType::VTodo),
            JSCalendarType::Alert => Some(ICalendarComponentType::VAlarm),
            JSCalendarType::Participant => Some(ICalendarComponentType::Participant),
            JSCalendarType::Location => Some(ICalendarComponentType::VLocation),
            _ => None,
        }
    }
}

// 7f1e1965-ae73-4454-b088-232c90730ce2
static JSCAL_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    127, 30, 25, 101, 174, 115, 68, 84, 176, 136, 35, 44, 144, 115, 12, 226,
]);

#[inline]
pub(crate) fn uuid5(text: impl AsRef<[u8]>) -> String {
    uuid::Uuid::new_v5(&JSCAL_NAMESPACE, text.as_ref())
        .hyphenated()
        .to_string()
}

#[cfg(test)]
impl<I: JSCalendarId, B: JSCalendarId> std::fmt::Display for JSCalendar<'_, I, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = serde_json::to_string_pretty(&self.0).map_err(|_| std::fmt::Error)?;
        write!(f, "{}", s)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        icalendar::{ICalendar, ICalendarComponent, ICalendarProperty},
        jscalendar::{JSCalendar, JSCalendarProperty, JSCalendarValue},
    };
    use jmap_tools::Value;

    #[derive(Debug, Default)]
    struct Test {
        comment: String,
        test: String,
        expect: String,
        roundtrip: String,
        line_num: usize,
    }

    #[test]
    fn convert_jscalendar() {
        // Read all test files in the test directory
        for entry in std::fs::read_dir("resources/jscalendar").unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let input = std::fs::read_to_string(&path).unwrap();
            let mut test = Test::default();
            let mut cur_command = "";
            let mut cur_value = &mut test.test;

            for (line_num, line) in input.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Some(line) = line.strip_prefix("> ") {
                    let (command, comment) = line.split_once(' ').unwrap_or((line, ""));

                    match (command, cur_command) {
                        ("test", _) => {
                            if !test.test.is_empty() {
                                test.run();
                            }
                            test = Test::default();
                            cur_value = &mut test.test;
                            cur_command = "test";
                            test.comment = comment.to_string();
                            test.line_num = line_num + 1;
                        }
                        ("convert", "test") => {
                            cur_command = "convert";
                            cur_value = &mut test.expect;
                        }
                        ("convert", "convert") => {
                            cur_command = "convert";
                            cur_value = &mut test.roundtrip;
                        }
                        _ => {
                            panic!(
                                "Unexpected command '{}' in file '{}' at line {}",
                                command,
                                path.display(),
                                line_num + 1
                            );
                        }
                    }
                } else {
                    cur_value.push_str(line);
                    cur_value.push('\n');
                }
            }

            if !test.test.is_empty() {
                test.run();
            }
        }
    }

    impl Test {
        fn run(mut self) {
            if self.expect.is_empty() {
                panic!(
                    "Test '{}' at line {} has no expected output",
                    self.comment, self.line_num
                );
            }

            println!("Running test '{}' at line {}", self.comment, self.line_num);

            if is_jscalendar(&self.test) {
                fix_jscalendar(&mut self.test);
                fix_icalendar(&mut self.expect);
                let source =
                    sanitize_jscalendar(parse_jscalendar(&self.comment, self.line_num, &self.test));
                let expect =
                    sanitize_icalendar(parse_icalendar(&self.comment, self.line_num, &self.expect));
                let roundtrip = if !self.roundtrip.is_empty() {
                    fix_jscalendar(&mut self.roundtrip);
                    sanitize_jscalendar(parse_jscalendar(
                        &self.comment,
                        self.line_num,
                        &self.roundtrip,
                    ))
                } else {
                    source.clone()
                };

                let first_convert =
                    sanitize_icalendar(source.into_icalendar().unwrap_or_else(|| {
                        panic!(
                            "Failed to convert JSCalendar to iCalendar: test {} on line {}: {}",
                            self.comment, self.line_num, self.test
                        )
                    }));
                if first_convert != expect {
                    let first_convert =
                        sanitize_icalendar(ICalendar::parse(first_convert.to_string()).unwrap());

                    if first_convert != expect {
                        panic!(
                            "JSCalendar to iCalendar conversion failed: test {} on line {}, expected: {}, got: {}",
                            self.comment, self.line_num, expect, first_convert
                        );
                    }
                }
                let roundtrip_convert = sanitize_jscalendar(first_convert.into_jscalendar());
                if roundtrip_convert != roundtrip {
                    let roundtrip_convert = roundtrip_convert.to_string();
                    let roundtrip = roundtrip.to_string();

                    if roundtrip_convert != roundtrip {
                        panic!(
                            "iCalendar to JSCalendar conversion failed: test {} on line {}, expected: {}, got: {}",
                            self.comment, self.line_num, roundtrip, roundtrip_convert
                        );
                    }
                }
            } else {
                fix_icalendar(&mut self.test);
                fix_jscalendar(&mut self.expect);
                let source =
                    sanitize_icalendar(parse_icalendar(&self.comment, self.line_num, &self.test));
                let expect = sanitize_jscalendar(parse_jscalendar(
                    &self.comment,
                    self.line_num,
                    &self.expect,
                ));
                let roundtrip = if !self.roundtrip.is_empty() {
                    fix_icalendar(&mut self.roundtrip);
                    sanitize_icalendar(parse_icalendar(
                        &self.comment,
                        self.line_num,
                        &self.roundtrip,
                    ))
                } else {
                    source.clone()
                };

                let first_convert = sanitize_jscalendar(source.into_jscalendar());
                if first_convert != expect {
                    let first_convert = first_convert.to_string();
                    let expect = expect.to_string();

                    if first_convert != expect {
                        panic!(
                            "iCalendar to JSCalendar conversion failed: test {} on line {}, expected: {}, got: {}",
                            self.comment, self.line_num, expect, first_convert
                        );
                    }
                }
                let roundtrip_convert =
                    sanitize_icalendar(first_convert.into_icalendar().unwrap_or_else(|| {
                        panic!(
                            "Failed to convert JSCalendar to iCalendar: test {} on line {}: {}",
                            self.comment, self.line_num, self.test
                        )
                    }));
                if roundtrip_convert != roundtrip {
                    let roundtrip_convert = sanitize_icalendar(
                        ICalendar::parse(roundtrip_convert.to_string()).unwrap(),
                    );
                    if roundtrip_convert != roundtrip {
                        panic!(
                            "JSCalendar to iCalendar conversion failed: test {} on line {}, expected: {}, got: {}",
                            self.comment, self.line_num, roundtrip, roundtrip_convert
                        );
                    }
                }
            }
        }
    }

    fn is_jscalendar(s: &str) -> bool {
        s.starts_with("{") || s.starts_with("\"")
    }

    fn fix_icalendar(s: &mut String) {
        if s.starts_with("BEGIN:") {
            if !s.starts_with("BEGIN:VCALENDAR") {
                let mut v = "BEGIN:VCALENDAR\nVERSION:2.0\n".to_string();
                v.push_str(s);
                v.push_str("END:VCALENDAR\n");
                *s = v;
            }
        } else {
            let mut v = "BEGIN:VCALENDAR\nVERSION:2.0\nBEGIN:VEVENT\n".to_string();
            v.push_str(s);
            v.push_str("END:VEVENT\nEND:VCALENDAR\n");
            *s = v;
        }
    }

    fn fix_jscalendar(s: &mut String) {
        let (prefix, suffix) = if !s.starts_with("{") {
            ("{", "}")
        } else {
            ("", "")
        };

        if s.contains(r#""@type": "Group""#) {
            if !prefix.is_empty() {
                *s = format!("{prefix}{s}{suffix}");
            }
        } else if s.contains(r#""@type": "Event""#) || s.contains(r#""@type": "Task""#) {
            *s = format!("{{\"@type\": \"Group\", \"entries\": [{prefix}{s}{suffix}]}}");
        } else {
            *s = format!(
                "{{\"@type\": \"Group\", \"entries\": [{prefix}\"@type\": \"Event\", {s}{suffix}]}}"
            );
        }
    }

    fn parse_icalendar(test_name: &str, line_num: usize, s: &str) -> ICalendar {
        ICalendar::parse(s).unwrap_or_else(|_| {
            panic!(
                "Failed to parse iCalendar: {} on line {}, test {}",
                s, line_num, test_name
            )
        })
    }

    fn parse_jscalendar<'x>(
        test_name: &str,
        line_num: usize,
        s: &'x str,
    ) -> JSCalendar<'x, String, String> {
        JSCalendar::parse(s).unwrap_or_else(|_| {
            panic!(
                "Failed to parse JSCalendar: {} on line {}, test {}",
                s, line_num, test_name
            )
        })
    }

    fn sanitize_icalendar(mut icalendar: ICalendar) -> ICalendar {
        for component in &mut icalendar.components {
            sanitize_icalendar_component(component);
        }

        icalendar
    }

    fn sanitize_icalendar_component(component: &mut ICalendarComponent) {
        component
            .entries
            .retain(|e| !matches!(e.name, ICalendarProperty::Version));
        component
            .entries
            .sort_unstable_by(|a, b| a.name.cmp(&b.name));
    }

    fn sanitize_jscalendar(
        mut jscalendar: JSCalendar<'_, String, String>,
    ) -> JSCalendar<'_, String, String> {
        sort_jscalendar_properties(&mut jscalendar.0);
        jscalendar
    }

    fn sort_jscalendar_properties(
        value: &mut Value<'_, JSCalendarProperty<String>, JSCalendarValue<String, String>>,
    ) {
        match value {
            Value::Array(value) => {
                for item in value {
                    sort_jscalendar_properties(item);
                }
            }
            Value::Object(obj) => {
                //obj.as_mut_vec()
                //    .retain(|(k, _)| !matches!(k, Key::Property(JSCalendarProperty::Type)));
                obj.as_mut_vec()
                    .sort_unstable_by(|a, b| a.0.to_string().cmp(&b.0.to_string()));
                for (_, item) in obj.as_mut_vec() {
                    sort_jscalendar_properties(item);
                }
            }
            _ => {}
        }
    }
}
