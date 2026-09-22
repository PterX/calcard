/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::common::timezone::ZonedDateTime;
use crate::{
    common::{
        blob::{BlobOptions, BlobResolver, BlobResolverFn, NoBlobIds, ResolvedBlobs},
        export::{EmbeddedBudget, ExportError, RejectedPatch},
        timezone::Tz,
    },
    icalendar::{ICalendar, ICalendarComponentType},
    jscalendar::{JSCalendarId, JSCalendarProperty, JSCalendarValue, RecurrenceOverrides},
};
use jmap_tools::{Key, Map, Value};
use std::hash::Hash;

pub mod convert;
pub mod params;
pub mod props;

struct State<'x, I: JSCalendarId, B: JSCalendarId> {
    entries: Map<'x, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    default_component_type: ICalendarComponentType,
    uid: Option<&'x str>,
    tz: Option<Tz>,
    tz_end: Option<Tz>,
    tz_rid: Option<Tz>,
    start: Option<ZonedDateTime>,
    recurrence_id: Option<ZonedDateTime>,
    is_date: bool,
}

#[allow(clippy::type_complexity)]
struct ConvertedComponent<'x, I: JSCalendarId, B: JSCalendarId> {
    pub(super) name: ICalendarComponentType,
    pub(super) converted_props: Vec<(
        Vec<Key<'x, JSCalendarProperty<I>>>,
        Value<'x, JSCalendarProperty<I>, JSCalendarValue<I, B>>,
    )>,
    pub(super) converted_props_count: usize,
    pub(super) properties: Vec<Value<'x, JSCalendarProperty<I>, JSCalendarValue<I, B>>>,
    pub(super) components: Vec<Value<'x, JSCalendarProperty<I>, JSCalendarValue<I, B>>>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExportOptions<R = NoBlobIds> {
    recurrence_overrides: RecurrenceOverrides,
    max_expansions: usize,
    blobs: BlobOptions<R>,
}

pub(super) struct ExportContext<'a, B> {
    recurrence_overrides: RecurrenceOverrides,
    max_expansions: usize,
    blobs: Option<ResolvedBlobs<'a, B>>,
    budget: EmbeddedBudget,
    error: Option<ExportError>,
    rejected_patches: Vec<RejectedPatch>,
}

const DEFAULT_MAX_EXPANSIONS: usize = 3000;

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            recurrence_overrides: RecurrenceOverrides::Full,
            max_expansions: DEFAULT_MAX_EXPANSIONS,
            blobs: BlobOptions::default(),
        }
    }
}

impl ExportOptions {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<R> ExportOptions<R> {
    pub fn recurrence_overrides(mut self, recurrence_overrides: RecurrenceOverrides) -> Self {
        self.recurrence_overrides = recurrence_overrides;
        self
    }

    pub fn max_expansions(mut self, max_expansions: usize) -> Self {
        self.max_expansions = max_expansions;
        self
    }

    pub fn with_blob_resolver<B, F>(self, blob_resolver: F) -> ExportOptions<BlobResolverFn<F>>
    where
        F: FnMut(&B) -> Option<Vec<u8>>,
    {
        self.with_resolver(BlobResolverFn(blob_resolver))
    }

    pub fn with_resolver<T>(self, blob_resolver: T) -> ExportOptions<T> {
        ExportOptions {
            recurrence_overrides: self.recurrence_overrides,
            max_expansions: self.max_expansions,
            blobs: self.blobs.with_handler(blob_resolver),
        }
    }

    pub fn max_embedded_size(mut self, max_embedded_size: usize) -> Self {
        self.blobs = self.blobs.with_max_embedded_size(max_embedded_size);
        self
    }

    pub(super) fn context<B: Clone + Eq + Hash>(&mut self) -> ExportContext<'_, B>
    where
        R: BlobResolver<B>,
    {
        ExportContext {
            recurrence_overrides: self.recurrence_overrides,
            max_expansions: self.max_expansions,
            budget: self.blobs.budget(),
            blobs: self.blobs.resolved_blobs(),
            error: None,
            rejected_patches: Vec::new(),
        }
    }
}

impl<B> ExportContext<'_, B> {
    fn fail(&mut self, error: ExportError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn has_failed(&self) -> bool {
        self.error.is_some()
    }

    fn reject_patch(&mut self, recurrence_id: String, pointer: String) {
        self.rejected_patches.push(RejectedPatch {
            recurrence_id,
            pointer,
        });
    }

    fn embed(&mut self, data: &[u8]) -> bool {
        match self.budget.take(data) {
            Err(error) => {
                self.blobs = None;
                self.fail(error);
                false
            }
            Ok(()) => true,
        }
    }

    fn into_result(self, ical: ICalendar) -> Result<(ICalendar, Vec<RejectedPatch>), ExportError> {
        match self.error {
            Some(error) => Err(error),
            None if ical.has_calendar_components() => Ok((ical, self.rejected_patches)),
            None => Err(ExportError::NoComponents),
        }
    }
}
