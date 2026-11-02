use std::collections::VecDeque;

pub(crate) struct LruCache<K, V> {
    capacity: usize,
    entries: VecDeque<(K, V)>,
}

impl<K: PartialEq, V: Clone> LruCache<K, V> {
    pub(crate) fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "cache capacity must be greater than zero");
        Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn get_or_insert_with(&mut self, key: K, create: impl FnOnce(&K) -> V) -> V {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(cached_key, _)| cached_key == &key)
        {
            let entry = self
                .entries
                .remove(index)
                .expect("cached entry index should remain valid");
            let value = entry.1.clone();
            self.entries.push_back(entry);
            return value;
        }

        let value = create(&key);
        if self.entries.len() == self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, value.clone()));
        value
    }
}
