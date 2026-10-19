//! Finding repeated keys in one object, in `O(n log n)` at worst.
//!
//! Shared by the tree builder (which applies every [`DuplicateKeys`](crate::DuplicateKeys)
//! policy) and the readers' state machine (which refuses duplicates under
//! the default policy). Keys are compared as decoded text, so `"a"` and an
//! escaped spelling of it are the same key. Each function takes the
//! object's keys as `key(i)` for `i` in `0..n`.

use alloc::vec::Vec;

/// Objects up to this many keys are checked pairwise.
const PAIRWISE: usize = 16;

/// Whether any of the `n` keys repeats. Small objects are checked pairwise,
/// comparing one-word fingerprints and the strings only when those match;
/// larger ones by sorting indices, which also leaves `order` sorted by
/// (key, position) for [`first_repeat`] and [`for_each_run`]. After `true`,
/// `order` is always sorted.
pub(crate) fn has_duplicates<'k>(
    n: usize,
    key: &impl Fn(usize) -> &'k str,
    order: &mut Vec<usize>,
) -> bool {
    if n < 2 {
        return false;
    }
    if n <= PAIRWISE {
        // At most 120 one-word comparisons.
        let mut prints = [0u64; PAIRWISE];
        for (i, slot) in prints.iter_mut().take(n).enumerate() {
            *slot = fingerprint(key(i));
        }
        let prints = prints.get(..n).unwrap_or(&[]);
        let any = prints.iter().enumerate().skip(1).any(|(i, a)| {
            prints
                .iter()
                .take(i)
                .enumerate()
                .any(|(j, b)| a == b && key(i) == key(j))
        });
        if !any {
            return false;
        }
    }
    order.clear();
    order.extend(0..n);
    order.sort_unstable_by(|&a, &b| key(a).cmp(key(b)).then(a.cmp(&b)));
    order.windows(2).any(|w| match w {
        [a, b] => key(*a) == key(*b),
        _ => false,
    })
}

/// A cheap summary of a key: equal keys have equal fingerprints. The length
/// and the first eight bytes, which tell most distinct keys apart.
#[inline]
fn fingerprint(key: &str) -> u64 {
    let bytes = key.as_bytes();
    let mut head = [0u8; 8];
    for (slot, &b) in head.iter_mut().zip(bytes) {
        *slot = b;
    }
    u64::from_le_bytes(head) ^ (bytes.len() as u64).rotate_left(56)
}

/// Calls `f(run)` for each run of equal keys in the sorted `order` (each run
/// sorted by position, the first being the first occurrence).
pub(crate) fn for_each_run<'k>(
    key: &impl Fn(usize) -> &'k str,
    order: &[usize],
    mut f: impl FnMut(&[usize]),
) {
    let mut start = 0;
    while start < order.len() {
        let first = key(order.get(start).copied().unwrap_or(0));
        let mut end = start + 1;
        while order.get(end).is_some_and(|&i| key(i) == first) {
            end += 1;
        }
        if end - start > 1 {
            f(order.get(start..end).unwrap_or(&[]));
        }
        start = end;
    }
}

/// The position of the earliest key that repeats an earlier one, given the
/// sorted `order`.
pub(crate) fn first_repeat<'k>(key: &impl Fn(usize) -> &'k str, order: &[usize]) -> usize {
    let mut earliest = usize::MAX;
    for_each_run(key, order, |run| {
        if let Some(&second) = run.get(1) {
            earliest = earliest.min(second);
        }
    });
    earliest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pairwise_and_sorted_paths_agree() {
        let keys = ["a", "abcdefgh1", "abcdefgh2", "b", "abcdefgh1"];
        let key = |i: usize| keys.get(i).copied().unwrap_or("");
        let mut order = Vec::new();
        assert!(has_duplicates(keys.len(), &key, &mut order));
        assert_eq!(first_repeat(&key, &order), 4);
        assert!(!has_duplicates(4, &key, &mut order));
        assert!(!has_duplicates(1, &key, &mut order));

        let many: Vec<alloc::string::String> = (0..40).map(|i| alloc::format!("k{i}")).collect();
        let key = |i: usize| many.get(i).map_or("", |s| s.as_str());
        assert!(!has_duplicates(many.len(), &key, &mut order));
        let mut repeated = many.clone();
        repeated.push("k7".into());
        repeated.push("k3".into());
        let key = |i: usize| repeated.get(i).map_or("", |s| s.as_str());
        assert!(has_duplicates(repeated.len(), &key, &mut order));
        assert_eq!(first_repeat(&key, &order), 40);
        let mut runs = 0;
        for_each_run(&key, &order, |run| {
            runs += 1;
            assert_eq!(run.len(), 2);
        });
        assert_eq!(runs, 2);
    }
}
