//! Stacks that keep their first entries inline.
//!
//! The readers keep a little state per open container and per key of an open
//! object. A typical message (a JSON-RPC envelope, an LSP request) never has
//! more than a few of either open at once, so keeping the first few inline
//! means reading it allocates nothing for that state; a deeper or wider
//! document spills the rest to the heap.

use alloc::vec::Vec;

/// A stack of `Copy` entries whose first `N` live inline; entries past `N`
/// spill to a vector.
///
/// Entry `i` is `inline[i]` for `i < N` and `spill[i - N]` after: the spill
/// is used only while the inline part is full.
#[derive(Clone, Debug)]
pub(crate) struct SmallStack<T, const N: usize> {
    len: usize,
    inline: [T; N],
    spill: Vec<T>,
}

impl<T: Copy, const N: usize> SmallStack<T, N> {
    /// An empty stack; `fill` only initializes the inline slots.
    pub(crate) const fn new(fill: T) -> Self {
        Self {
            len: 0,
            inline: [fill; N],
            spill: Vec::new(),
        }
    }

    /// The number of entries.
    #[inline]
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    /// Adds `value` on top.
    #[inline]
    pub(crate) fn push(&mut self, value: T) {
        match self.inline.get_mut(self.len) {
            Some(slot) => *slot = value,
            None => self.spill.push(value),
        }
        self.len += 1;
    }

    /// Removes the top entry.
    #[inline]
    pub(crate) fn pop(&mut self) -> Option<T> {
        let top = self.len.checked_sub(1)?;
        self.len = top;
        match self.inline.get(top) {
            Some(&value) => Some(value),
            None => self.spill.pop(),
        }
    }

    /// The top entry.
    #[inline]
    pub(crate) fn last_mut(&mut self) -> Option<&mut T> {
        let top = self.len.checked_sub(1)?;
        if top < N {
            self.inline.get_mut(top)
        } else {
            self.spill.last_mut()
        }
    }

    /// Keeps the first `len` entries.
    #[inline]
    pub(crate) fn truncate(&mut self, len: usize) {
        if len < self.len {
            self.spill.truncate(len.saturating_sub(N));
            self.len = len;
        }
    }

    /// The entries from `from` on, as the inline part and the spilled part.
    #[inline]
    pub(crate) fn from(&self, from: usize) -> (&[T], &[T]) {
        let inline = self.inline.get(from.min(N)..self.len.min(N)).unwrap_or(&[]);
        let spill = self.spill.get(from.saturating_sub(N)..).unwrap_or(&[]);
        (inline, spill)
    }

    /// The entries from `from` on, in order.
    #[inline]
    pub(crate) fn iter_from(&self, from: usize) -> impl Iterator<Item = &T> {
        let (inline, spill) = self.from(from);
        inline.iter().chain(spill)
    }
}

/// A stack of flags, one bit per entry, the first 64 inline.
#[derive(Clone, Debug)]
pub(crate) struct BitStack {
    len: usize,
    low: u64,
    high: Vec<u64>,
}

impl BitStack {
    /// An empty stack.
    pub(crate) const fn new() -> Self {
        Self {
            len: 0,
            low: 0,
            high: Vec::new(),
        }
    }

    /// The number of entries.
    #[inline]
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    /// Adds `flag` on top.
    #[inline]
    pub(crate) fn push(&mut self, flag: bool) {
        let at = self.len;
        let bit = 1u64 << (at % 64);
        let word = if at < 64 {
            &mut self.low
        } else {
            let index = at / 64 - 1;
            if index == self.high.len() {
                self.high.push(0);
            }
            match self.high.get_mut(index) {
                Some(word) => word,
                None => return,
            }
        };
        if flag {
            *word |= bit;
        } else {
            *word &= !bit;
        }
        self.len += 1;
    }

    /// Removes the top entry.
    #[inline]
    pub(crate) fn pop(&mut self) {
        self.len = self.len.saturating_sub(1);
    }

    /// The top entry.
    #[inline]
    pub(crate) fn last(&self) -> Option<bool> {
        let at = self.len.checked_sub(1)?;
        let word = if at < 64 {
            self.low
        } else {
            *self.high.get(at / 64 - 1)?
        };
        Some((word >> (at % 64)) & 1 == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_small_stack_spills_past_its_inline_entries_and_back() {
        let mut s: SmallStack<usize, 3> = SmallStack::new(0);
        for i in 0..10 {
            s.push(i);
        }
        assert_eq!(s.len(), 10);
        assert_eq!(
            s.iter_from(2).copied().collect::<Vec<_>>(),
            [2, 3, 4, 5, 6, 7, 8, 9]
        );
        assert_eq!(s.iter_from(5).copied().collect::<Vec<_>>(), [5, 6, 7, 8, 9]);
        if let Some(top) = s.last_mut() {
            *top = 90;
        }
        assert_eq!(s.pop(), Some(90));
        s.truncate(4);
        assert_eq!(s.iter_from(0).copied().collect::<Vec<_>>(), [0, 1, 2, 3]);
        s.truncate(2);
        assert_eq!(s.iter_from(0).copied().collect::<Vec<_>>(), [0, 1]);
        if let Some(top) = s.last_mut() {
            *top = 10;
        }
        s.push(7);
        s.push(8);
        assert_eq!(s.iter_from(1).copied().collect::<Vec<_>>(), [10, 7, 8]);
        assert_eq!(
            (s.pop(), s.pop(), s.pop(), s.pop(), s.pop()),
            (Some(8), Some(7), Some(10), Some(0), None)
        );
        s.truncate(5);
        assert_eq!(s.len(), 0);
        assert_eq!(s.from(0), (&[][..], &[][..]));
    }

    #[test]
    fn test_bit_stack_keeps_every_flag_at_any_depth() {
        let mut s = BitStack::new();
        assert_eq!(s.last(), None);
        let flag = |i: usize| i % 3 == 0 || i % 7 == 1;
        for i in 0..300 {
            s.push(flag(i));
            assert_eq!(s.last(), Some(flag(i)));
        }
        for i in (0..300).rev() {
            assert_eq!(s.last(), Some(flag(i)), "{i}");
            s.pop();
        }
        assert_eq!((s.len(), s.last()), (0, None));
        // Pushed again after popping, the bits are overwritten, not or-ed.
        for i in 0..200 {
            s.push(!flag(i));
        }
        assert_eq!(s.last(), Some(!flag(199)));
        s.pop();
        assert_eq!(s.last(), Some(!flag(198)));
    }
}
