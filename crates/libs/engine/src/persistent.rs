//! A persistent map from small integer keys, as a dynamic-range persistent segment tree.
//!
//! Inserting returns a new map that shares every untouched subtree with the old one, so each
//! version costs O(log n) and old versions stay valid. The range grows by doubling to fit the
//! largest key. The value tree keeps one version per node, mapping each value id to the depth of
//! the node that defines it (design §6.1).

use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PersistentMap<V> {
    root: Option<Arc<Slot<V>>>,
    /// Keys `0..=end` fit; `end + 1` is a power of two.
    end: usize,
}

#[derive(Debug)]
enum Slot<V> {
    Leaf(V),
    Split {
        left: Option<Arc<Slot<V>>>,
        right: Option<Arc<Slot<V>>>,
    },
}

impl<V: Copy> Default for PersistentMap<V> {
    fn default() -> Self {
        Self { root: None, end: 0 }
    }
}

impl<V: Copy> PersistentMap<V> {
    /// The value at `key`, if set.
    pub fn get(&self, key: usize) -> Option<V> {
        if key > self.end {
            return None;
        }
        let (mut slot, mut begin, mut end) = (self.root.as_deref()?, 0, self.end);
        loop {
            match slot {
                Slot::Leaf(value) => return Some(*value),
                Slot::Split { left, right } => {
                    let mid = begin + (end - begin) / 2;
                    let next = match key <= mid {
                        true => {
                            end = mid;
                            left
                        }
                        false => {
                            begin = mid + 1;
                            right
                        }
                    };
                    slot = next.as_deref()?;
                }
            }
        }
    }

    /// This map with `key` set to `value`.
    pub fn insert(&self, key: usize, value: V) -> Self {
        let (mut root, mut end) = (self.root.clone(), self.end);
        // Double the range until `key` fits; the old tree becomes the left half.
        while key > end {
            end = 2 * end + 1;
            root = root.map(|root| {
                Arc::new(Slot::Split {
                    left: Some(root),
                    right: None,
                })
            });
        }
        Self {
            root: Some(Self::set(root.as_ref(), 0, end, key, value)),
            end,
        }
    }

    fn set(
        slot: Option<&Arc<Slot<V>>>,
        begin: usize,
        end: usize,
        key: usize,
        value: V,
    ) -> Arc<Slot<V>> {
        if begin == end {
            return Arc::new(Slot::Leaf(value));
        }
        let (left, right) = match slot.map(Arc::as_ref) {
            Some(Slot::Split { left, right }) => (left.clone(), right.clone()),
            _ => (None, None),
        };
        let mid = begin + (end - begin) / 2;
        Arc::new(match key <= mid {
            true => Slot::Split {
                left: Some(Self::set(left.as_ref(), begin, mid, key, value)),
                right,
            },
            false => Slot::Split {
                left,
                right: Some(Self::set(right.as_ref(), mid + 1, end, key, value)),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_independent() {
        let empty = PersistentMap::default();
        let one = empty.insert(3, 'a');
        let two = one.insert(100, 'b');
        let three = two.insert(3, 'c');
        assert_eq!(empty.get(3), None);
        assert_eq!((one.get(3), one.get(100)), (Some('a'), None));
        assert_eq!((two.get(3), two.get(100)), (Some('a'), Some('b')));
        assert_eq!((three.get(3), three.get(100)), (Some('c'), Some('b')));
        assert_eq!(three.get(4), None);
        assert_eq!(three.get(usize::MAX), None);
    }

    #[test]
    fn matches_a_plain_map() {
        let mut map = PersistentMap::default();
        let mut plain = std::collections::HashMap::new();
        let mut key = 7usize;
        for step in 0..2000 {
            key = (key * 31 + 17) % 997;
            map = map.insert(key, step);
            plain.insert(key, step);
        }
        for key in 0..1100 {
            assert_eq!(map.get(key), plain.get(&key).copied(), "key {key}");
        }
    }
}
