/*
 * SPDX-FileCopyrightText: 2020 Stalwart Labs LLC <hello@stalw.art>
 *
 * SPDX-License-Identifier: Apache-2.0 OR MIT
 */

use ahash::{AHashMap, RandomState};
use std::{borrow::Borrow, hash::Hash, vec::IntoIter};

pub(crate) const INDEX_THRESHOLD: usize = 16;

#[derive(Debug)]
pub(crate) struct OrderedMap<K, V> {
    entries: Vec<(K, V)>,
    index: Option<Box<HashIndex>>,
}

#[derive(Debug)]
struct HashIndex {
    state: RandomState,
    positions: AHashMap<u64, usize>,
}

impl<K, V> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            index: None,
        }
    }
}

impl<K: Hash + Eq, V> OrderedMap<K, V> {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(key, value)| (key, value))
    }

    pub(crate) fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.position(key).is_some()
    }

    pub(crate) fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let position = self.position(key)?;
        self.entries.get_mut(position).map(|(_, value)| value)
    }

    pub(crate) fn push(&mut self, key: K, value: V) {
        if let Some(index) = &mut self.index {
            index.insert(&key, self.entries.len());
        }
        self.entries.push((key, value));
        if self.index.is_none() {
            self.index = HashIndex::build(&self.entries);
        }
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        match self.get_mut(&key) {
            Some(existing) => *existing = value,
            None => self.push(key, value),
        }
    }

    pub(crate) fn insert_if_absent(&mut self, key: K, value: V) {
        if !self.contains_key(&key) {
            self.push(key, value);
        }
    }

    pub(crate) fn upsert(
        &mut self,
        key: K,
        value: impl FnOnce() -> V,
        update: impl FnOnce(&mut V),
    ) {
        match self.get_mut(&key) {
            Some(existing) => update(existing),
            None => {
                let mut value = value();
                update(&mut value);
                self.push(key, value);
            }
        }
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        let len = self.entries.len();
        self.entries.retain_mut(|(key, value)| keep(key, value));
        if self.entries.len() != len {
            self.index = HashIndex::build(&self.entries);
        }
    }

    fn position<Q>(&self, key: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let Some(index) = &self.index else {
            return self.scan(key);
        };
        let position = index.get(key)?;
        match self.entries.get(position) {
            Some((item, _)) if item.borrow() == key => Some(position),
            _ => self.scan(key),
        }
    }

    fn scan<Q>(&self, key: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.entries
            .iter()
            .position(|(item, _)| item.borrow() == key)
    }
}

impl<K, V> IntoIterator for OrderedMap<K, V> {
    type Item = (K, V);
    type IntoIter = IntoIter<(K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl HashIndex {
    #[inline]
    fn build<K: Hash, V>(entries: &[(K, V)]) -> Option<Box<Self>> {
        (entries.len() > INDEX_THRESHOLD).then(|| Self::index(entries))
    }

    #[cold]
    #[inline(never)]
    fn index<K: Hash, V>(entries: &[(K, V)]) -> Box<Self> {
        let mut index = Box::new(HashIndex {
            state: RandomState::new(),
            positions: AHashMap::with_capacity(entries.len()),
        });
        for (position, (key, _)) in entries.iter().enumerate() {
            index.insert(key, position);
        }
        index
    }

    fn insert<Q: Hash + ?Sized>(&mut self, key: &Q, position: usize) {
        self.positions
            .entry(self.state.hash_one(key))
            .or_insert(position);
    }

    fn get<Q: Hash + ?Sized>(&self, key: &Q) -> Option<usize> {
        self.positions.get(&self.state.hash_one(key)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{INDEX_THRESHOLD, OrderedMap};
    use crate::common::xorshift::XorShift;
    use ahash::AHashMap;
    use std::{
        cell::Cell,
        hash::{Hash, Hasher},
    };

    const SEQUENCES: usize = 3_000;
    const MAX_OPERATIONS: usize = 96;
    const MAX_KEYS: usize = 160;
    const FIXED_KEYS: [&str; 6] = ["title", "start", "duration", "timeZone", "", "a/b~c"];

    impl XorShift {
        fn keys(&mut self) -> Vec<String> {
            let count = 1 + self.below(MAX_KEYS);
            (0..count)
                .map(|_| match self.below(4) {
                    0 => format!("participants/{:x}", self.next()),
                    1 => format!("participants/{:x}/roles", self.below(8)),
                    2 => format!("locations/{}", self.below(64)),
                    _ => self.pick(&FIXED_KEYS).to_string(),
                })
                .collect()
        }
    }

    #[derive(Default)]
    struct Reference {
        order: Vec<String>,
        values: AHashMap<String, u64>,
    }

    impl Reference {
        fn insert(&mut self, key: &str, value: u64) {
            if self.values.insert(key.to_string(), value).is_none() {
                self.order.push(key.to_string());
            }
        }

        fn insert_if_absent(&mut self, key: &str, value: u64) {
            if !self.values.contains_key(key) {
                self.insert(key, value);
            }
        }

        fn retain(&mut self, keep: impl Fn(&str) -> bool) {
            self.order.retain(|key| keep(key));
            self.values.retain(|key, _| keep(key));
        }

        fn list(&self) -> Vec<(String, u64)> {
            self.order
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        self.values.get(key).copied().unwrap_or(u64::MAX),
                    )
                })
                .collect()
        }
    }

    #[test]
    fn ordered_map_matches_an_insertion_ordered_hash_map() {
        let mut rng = XorShift::new(0x0c0e_27ed_da7a_2026);
        let mut indexed_steps = 0;
        for _ in 0..SEQUENCES {
            let keys = rng.keys();
            let mut map = OrderedMap::<String, u64>::default();
            let mut reference = Reference::default();
            for step in 0..1 + rng.below(MAX_OPERATIONS) {
                let key = keys.get(rng.below(keys.len())).map_or("", String::as_str);
                let value = step as u64;
                match rng.below(7) {
                    0 => {
                        map.insert(key.to_string(), value);
                        reference.insert(key, value);
                    }
                    1 => {
                        map.insert_if_absent(key.to_string(), value);
                        reference.insert_if_absent(key, value);
                    }
                    2 => {
                        let modulus = 2 + rng.below(5);
                        let rest = rng.below(modulus);
                        let keep = |key: &str| key.len() % modulus != rest;
                        map.retain(|key, _| keep(key));
                        reference.retain(keep);
                    }
                    3 => {
                        assert_eq!(
                            map.get_mut(key).map(|value| *value),
                            reference.values.get(key).copied(),
                            "{key}"
                        );
                        assert_eq!(map.contains_key(key), reference.values.contains_key(key));
                    }
                    4 => {
                        map.upsert(key.to_string(), || value, |existing| *existing += 1);
                        match reference.values.get(key).copied() {
                            Some(existing) => reference.insert(key, existing + 1),
                            None => reference.insert(key, value + 1),
                        }
                    }
                    _ => match map.get_mut(key) {
                        Some(existing) => {
                            *existing = value;
                            reference.insert(key, value);
                        }
                        None => {
                            map.push(key.to_string(), value);
                            reference.insert(key, value);
                        }
                    },
                }
                assert_eq!(map.len(), reference.order.len());
                assert_eq!(map.is_empty(), reference.order.is_empty());
                assert_eq!(map.index.is_some(), map.len() > INDEX_THRESHOLD);
                indexed_steps += usize::from(map.index.is_some());
            }
            assert_eq!(
                map.iter()
                    .map(|(key, value)| (key.clone(), *value))
                    .collect::<Vec<_>>(),
                reference.list()
            );
            assert_eq!(map.into_iter().collect::<Vec<_>>(), reference.list());
        }
        assert!(indexed_steps > 0);
    }

    #[test]
    fn ordered_map_resolves_hash_collisions_by_scanning() {
        let mut map = OrderedMap::<String, u64>::default();
        let keys = (0..=INDEX_THRESHOLD)
            .map(|position| format!("key{position}"))
            .collect::<Vec<_>>();
        for (position, key) in keys.iter().enumerate() {
            map.push(key.clone(), position as u64);
        }
        let [first_key, second_key, ..] = keys.as_slice() else {
            panic!("two keys expected");
        };
        let Some(index) = map.index.as_mut() else {
            panic!("index expected past the threshold");
        };
        let first = index.state.hash_one(first_key.as_str());
        index.positions.insert(first, 1);
        assert_eq!(map.position(first_key.as_str()), Some(0));
        assert_eq!(map.position(second_key.as_str()), Some(1));
        assert_eq!(map.position("missing"), None);
    }

    struct CountedKey<'x> {
        text: String,
        calls: &'x Cell<usize>,
    }

    impl CountedKey<'_> {
        fn count_call(&self) {
            self.calls.set(self.calls.get() + 1);
        }
    }

    impl PartialEq for CountedKey<'_> {
        fn eq(&self, other: &Self) -> bool {
            self.count_call();
            self.text == other.text
        }
    }

    impl Eq for CountedKey<'_> {}

    impl Hash for CountedKey<'_> {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.count_call();
            self.text.hash(state);
        }
    }

    #[test]
    fn ordered_map_lookups_are_constant_past_the_threshold() {
        for len in [INDEX_THRESHOLD, 1_000, 16_000] {
            let calls = Cell::new(0);
            let key = |position: usize| CountedKey {
                text: format!("k{position}"),
                calls: &calls,
            };
            let mut map = OrderedMap::default();
            for position in 0..len {
                map.upsert(key(position), || 0, |count| *count += 1);
            }
            for position in 0..len {
                assert!(map.contains_key(&key(position)));
            }
            let bound = 2 * len * (INDEX_THRESHOLD + 1);
            assert!(
                calls.get() <= bound,
                "{len} keys took {} key comparisons and hashes",
                calls.get()
            );
        }
    }
}
