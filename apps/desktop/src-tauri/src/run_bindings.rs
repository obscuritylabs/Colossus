//! Bounded native discovery metadata. A miss requires authenticated revalidation.
use std::collections::{HashMap, VecDeque};

pub(crate) const MAX_BINDINGS: usize = 4_096;

pub(crate) struct RunBindings<T> {
    entries: HashMap<String, T>,
    order: VecDeque<String>,
}

impl<T> Default for RunBindings<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<T> RunBindings<T> {
    pub(crate) fn get(&mut self, key: &str) -> Option<&T> {
        if self.entries.contains_key(key) {
            self.order.retain(|entry| entry != key);
            self.order.push_back(key.to_owned());
        }
        self.entries.get(key)
    }

    pub(crate) fn insert(&mut self, key: String, value: T) {
        self.order.retain(|entry| entry != &key);
        if !self.entries.contains_key(&key)
            && self.entries.len() == MAX_BINDINGS
            && let Some(oldest) = self.order.pop_front()
        {
            self.entries.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paging_evicts_old_metadata_without_clearing_recent_controls() {
        let mut bindings = RunBindings::default();
        bindings.insert("active".into(), true);
        for index in 0..MAX_BINDINGS * 2 {
            assert_eq!(bindings.get("active"), Some(&true));
            bindings.insert(index.to_string(), false);
        }
        assert_eq!(bindings.len(), MAX_BINDINGS);
        assert_eq!(bindings.order.len(), MAX_BINDINGS);
        assert_eq!(bindings.get("active"), Some(&true));
        assert!(bindings.get("0").is_none());
        bindings.clear();
        assert!(bindings.is_empty());
        assert!(bindings.order.is_empty());
    }
}
