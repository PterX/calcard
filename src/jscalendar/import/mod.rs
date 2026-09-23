/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use ahash::AHashMap;
use jmap_tools::{JsonPointer, Key, Value};

use crate::common::timezone::ZonedDateTime;
use crate::{
    common::{
        blob::{BlobIdFn, BlobIdGenerator, BlobIds, BlobOptions, NoBlobIds},
        timezone::Tz,
    },
    icalendar::{
        ICalendarComponentType, ICalendarEntry, ICalendarParameterName, ICalendarProperty,
    },
    jscalendar::{JSCalendarDateTime, JSCalendarId, JSCalendarProperty, JSCalendarValue},
};

pub mod convert;
pub mod params;
pub mod props;

#[derive(Default)]
#[allow(clippy::type_complexity)]
struct State<I: JSCalendarId, B: JSCalendarId> {
    component_type: ICalendarComponentType,
    entries: AHashMap<
        Key<'static, JSCalendarProperty<I>>,
        Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    >,
    ical_converted_properties: AHashMap<String, ICalendarConvertedProperty<I, B>>,
    ical_properties: Vec<Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>>,
    ical_components: Option<Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>>,
    recurrence_overrides: Vec<(JSCalendarDateTime, State<I, B>)>,
    patch_objects: Vec<(
        JsonPointer<JSCalendarProperty<I>>,
        Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    )>,
    link_ids: LinkIds,
    jsid: Option<String>,
    uid: Option<String>,
    recurrence_id: Option<ZonedDateTime>,
    recurrence_id_is_date: bool,
    due: Option<ZonedDateTime>,
    tz_start: Option<Tz>,
    tz_end: Option<Tz>,
    has_dates: bool,
    has_end: bool,
    map_component: bool,
    is_recurrence_instance: bool,
    include_ical_components: bool,
}

#[derive(Debug, Default)]
struct ICalendarConvertedProperty<I: JSCalendarId, B: JSCalendarId> {
    name: Option<ICalendarProperty>,
    params: ICalendarParams<I, B>,
}

#[derive(Debug, Default)]
#[allow(clippy::type_complexity)]
struct ICalendarParams<I: JSCalendarId, B: JSCalendarId>(
    Vec<(
        ICalendarParameterName,
        Vec<Value<'static, JSCalendarProperty<I>, JSCalendarValue<I, B>>>,
    )>,
);

#[derive(Debug, Default)]
struct LinkIds(AHashMap<String, LinkId>);

#[derive(Debug, Clone, Copy)]
struct LinkId {
    position: usize,
    next_suffix: u32,
}

#[derive(Debug, Clone)]
struct EntryState {
    entry: ICalendarEntry,
    converted_to: Option<String>,
    map_name: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ImportOptions<G = NoBlobIds> {
    include_ical_components: bool,
    return_first: bool,
    blobs: BlobOptions<G>,
}

pub(super) struct ImportContext<'a, B> {
    include_ical_components: bool,
    return_first: bool,
    blob_ids: Option<BlobIds<'a, B>>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            include_ical_components: true,
            return_first: false,
            blobs: BlobOptions::default(),
        }
    }
}

impl ImportOptions {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<G> ImportOptions<G> {
    pub fn include_ical_components(mut self, include: bool) -> Self {
        self.include_ical_components = include;
        self
    }

    pub fn return_first(mut self, return_first: bool) -> Self {
        self.return_first = return_first;
        self
    }

    pub fn with_blob_ids<B, F>(self, blob_ids: F) -> ImportOptions<BlobIdFn<F>>
    where
        F: FnMut(&[u8]) -> Option<B>,
    {
        self.with_blob_id_generator(BlobIdFn(blob_ids))
    }

    pub fn with_blob_id_generator<T>(self, blob_id_generator: T) -> ImportOptions<T> {
        ImportOptions {
            include_ical_components: self.include_ical_components,
            return_first: self.return_first,
            blobs: self.blobs.with_handler(blob_id_generator),
        }
    }

    pub(super) fn context<B: Clone>(&mut self) -> ImportContext<'_, B>
    where
        G: BlobIdGenerator<B>,
    {
        ImportContext {
            include_ical_components: self.include_ical_components,
            return_first: self.return_first,
            blob_ids: self.blobs.blob_ids(),
        }
    }
}
