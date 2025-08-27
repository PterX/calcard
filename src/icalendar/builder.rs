/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use crate::{
    common::PartialDateTime,
    icalendar::{
        ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarParameter,
        ICalendarParameterName, ICalendarParameterValue, ICalendarProperty, ICalendarValue,
        ICalendarValueType,
    },
};
use ahash::{AHashMap, AHashSet};

impl ICalendar {
    pub fn remove_component_ids(&mut self, component_ids: &[u16]) {
        // Validate component IDs
        let max_component_id = self.components.len() as u16;
        let mut remove_component_ids = AHashSet::from_iter(
            component_ids
                .iter()
                .filter(|id| **id < max_component_id)
                .cloned(),
        );

        // Add sub-components to the set
        for (component_id, component) in self.components.iter().enumerate() {
            if remove_component_ids.contains(&(component_id as u16)) {
                remove_component_ids.extend(&component.component_ids);
            }
        }

        if !remove_component_ids.is_empty() {
            let id_mappings = (0..max_component_id)
                .filter(|i| !remove_component_ids.contains(i))
                .enumerate()
                .map(|(new_id, old_id)| (old_id, new_id as u16))
                .collect::<AHashMap<_, _>>();

            for (component_id, mut component) in
                std::mem::replace(&mut self.components, Vec::with_capacity(id_mappings.len()))
                    .into_iter()
                    .enumerate()
            {
                if !remove_component_ids.contains(&(component_id as u16)) {
                    let component_ids = component
                        .component_ids
                        .iter()
                        .filter_map(|id| id_mappings.get(id).cloned())
                        .collect();
                    component.component_ids = component_ids;
                    self.components.push(component);
                }
            }
        }
    }

    pub fn copy_timezones(&mut self, other: &ICalendar) {
        for component in &other.components {
            if component.component_type == ICalendarComponentType::VTimezone {
                let tz_component_id = self.components.len();
                self.components[0]
                    .component_ids
                    .insert(1, tz_component_id as u16);
                self.components.push(ICalendarComponent {
                    component_type: ICalendarComponentType::VTimezone,
                    entries: component.entries.clone(),
                    component_ids: vec![],
                });
                for component_id in &component.component_ids {
                    let item_id = self.components.len() as u16;
                    let item = &other.components[*component_id as usize];
                    self.components.push(ICalendarComponent {
                        component_type: item.component_type.clone(),
                        entries: item.entries.clone(),
                        component_ids: vec![],
                    });
                    self.components[tz_component_id].component_ids.push(item_id);
                }
            }
        }
    }
}

impl ICalendarComponent {
    pub fn add_dtstamp(&mut self, dt_stamp: PartialDateTime) {
        self.entries.push(ICalendarEntry {
            name: ICalendarProperty::Dtstamp,
            params: vec![],
            values: vec![ICalendarValue::PartialDateTime(Box::new(dt_stamp))],
        });
    }

    pub fn add_sequence(&mut self, sequence: i64) {
        self.entries.push(ICalendarEntry {
            name: ICalendarProperty::Sequence,
            params: vec![],
            values: vec![ICalendarValue::Integer(sequence)],
        });
    }

    pub fn add_uid(&mut self, uid: &str) {
        self.entries.push(ICalendarEntry {
            name: ICalendarProperty::Uid,
            params: vec![],
            values: vec![ICalendarValue::Text(uid.to_string())],
        });
    }

    pub fn add_property(&mut self, name: ICalendarProperty, value: ICalendarValue) {
        self.entries.push(ICalendarEntry {
            name,
            params: vec![],
            values: vec![value],
        });
    }

    pub fn add_property_with_params(
        &mut self,
        name: ICalendarProperty,
        params: impl IntoIterator<Item = ICalendarParameter>,
        value: ICalendarValue,
    ) {
        self.entries.push(ICalendarEntry {
            name,
            params: params.into_iter().collect(),
            values: vec![value],
        });
    }
}

impl ICalendarEntry {
    pub fn new(name: ICalendarProperty) -> Self {
        Self {
            name,
            params: vec![],
            values: vec![],
        }
    }

    pub fn with_params(mut self, params: Vec<ICalendarParameter>) -> Self {
        self.params = params;
        self
    }

    pub fn with_value(mut self, value: impl Into<ICalendarValue>) -> Self {
        self.values.push(value.into());
        self
    }

    pub fn with_values(mut self, values: Vec<ICalendarValue>) -> Self {
        self.values = values;
        self
    }

    pub fn with_param(mut self, param: impl Into<ICalendarParameter>) -> Self {
        self.params.push(param.into());
        self
    }

    pub fn add_param(&mut self, param: impl Into<ICalendarParameter>) {
        self.params.push(param.into());
    }

    pub fn with_param_opt(mut self, param: Option<impl Into<ICalendarParameter>>) -> Self {
        if let Some(param) = param {
            self.params.push(param.into());
        }
        self
    }

    pub fn is_type(&self, typ: &ICalendarValueType) -> bool {
        self.parameters(&ICalendarParameterName::Value)
            .any(|p| matches!(p, ICalendarParameterValue::Value(v) if v == typ))
    }
}

#[cfg(test)]
mod tests {
    use crate::icalendar::ICalendar;

    #[test]
    fn remove_component_ids() {
        let mut ical =
            ICalendar::parse(std::fs::read_to_string("resources/ical/007.ics").unwrap()).unwrap();
        ical.remove_component_ids(&[2, 5, 7]);

        let max_component_id = ical.components.len() as u16;
        for component in &ical.components {
            for id in &component.component_ids {
                assert!(*id < max_component_id);
            }
        }

        assert_eq!(
            ical.to_string().replace("\r\n", "\n"),
            r#"BEGIN:VCALENDAR
PRODID:-//Google Inc//Google Calendar 70.9054//EN
VERSION:2.0
CALSCALE:GREGORIAN
METHOD:PUBLISH
X-WR-CALNAME:ANONYMOUS
X-WR-TIMEZONE:America/Denver
BEGIN:VTIMEZONE
TZID:America/Denver
X-LIC-LOCATION:America/Denver
BEGIN:STANDARD
TZOFFSETFROM:-0600
TZOFFSETTO:-0700
TZNAME:MST
DTSTART:19701101T020000
RRULE:FREQ=YEARLY;BYDAY=1SU;BYMONTH=11
END:STANDARD
END:VTIMEZONE
BEGIN:VEVENT
DTSTART;TZID=America/Denver:20200903T094000
DTEND;TZID=America/Denver:20200903T095000
RRULE:FREQ=WEEKLY;BYDAY=FR,MO,TH,TU,WE
EXDATE;TZID=America/Denver:20201015T094000
DTSTAMP:20201124T214551Z
UID:078tk5i2t4all3kk0jcoi3mdmd@google.com
CREATED:20200903T032927Z
DESCRIPTION:
LAST-MODIFIED:20200903T032927Z
LOCATION:
SEQUENCE:0
STATUS:CONFIRMED
SUMMARY:Brain Break
TRANSP:OPAQUE
END:VEVENT
BEGIN:VEVENT
DTSTART;TZID=America/Denver:20200903T095000
DTEND;TZID=America/Denver:20200903T104000
RRULE:FREQ=WEEKLY;BYDAY=FR,MO,TH,TU,WE
EXDATE;TZID=America/Denver:20201015T095000
DTSTAMP:20201124T214551Z
UID:11le1ep09hvog7dbotn6foj38e@google.com
CREATED:20200903T032956Z
DESCRIPTION:
LAST-MODIFIED:20200903T032957Z
LOCATION:
SEQUENCE:0
STATUS:CONFIRMED
SUMMARY:Academic Time
TRANSP:OPAQUE
END:VEVENT
BEGIN:VEVENT
DTSTART;TZID=Europe/Budapest:20201102T072000
DTEND;TZID=Europe/Budapest:20201102T072000
EXDATE;TZID=Europe/Budapest:20201221T072000
EXDATE;TZID=Europe/Budapest:20201222T072000
EXDATE;TZID=Europe/Budapest:20201223T072000
EXDATE;TZID=Europe/Budapest:20201224T072000
EXDATE;TZID=Europe/Budapest:20201225T072000
EXDATE;TZID=Europe/Budapest:20201228T072000
EXDATE;TZID=Europe/Budapest:20201229T072000
EXDATE;TZID=Europe/Budapest:20201230T072000
EXDATE;TZID=Europe/Budapest:20201231T072000
EXDATE;TZID=Europe/Budapest:20210101T072000
RRULE;X-BUSYMAC-REGENERATE=TRASH:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR
DTSTAMP:20201230T095550Z
UID:*Masked away*
CREATED:20201102T054749Z
DESCRIPTION:
LAST-MODIFIED:20201221T100211Z
LOCATION:
SEQUENCE:1
STATUS:CONFIRMED
SUMMARY:Invalid RRULE property
TRANSP:OPAQUE
X-BUSYMAC-LASTMODBY:*Masked away*
END:VEVENT
END:VCALENDAR
"#
        )
    }
}
