//! Alignment inputs: [`AlignInput`], [`TrivialSet`] and the [`PairScorer`] used by closeness matching.

use crate::intern::TokenId;

/// A dense bit set of token ids that are "trivial" (cannot act as anchors; e.g. blank or all-unimportant
/// lines). Built by the caller.
///
/// Memory is one bit per id up to the largest id inserted (`max id / 8` bytes): insert interner ids (below
/// `distinct`); inserting `TokenId(u32::MAX)` allocates 512 MiB.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrivialSet {
    /// One bit per token id.
    bits: Vec<u64>,
}

impl TrivialSet {
    /// Set able to hold ids `< distinct` (larger ids still work: the set grows on insertion).
    pub fn with_capacity(distinct: u32) -> TrivialSet {
        TrivialSet {
            bits: vec![0; (distinct as usize).div_ceil(64)],
        }
    }

    /// Marks `id` (grows the set to `id / 8` bytes when needed).
    pub fn insert(&mut self, id: TokenId) {
        let word = (id.0 / 64) as usize;
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
        self.bits[word] |= 1 << (id.0 % 64);
    }

    /// Tests `id`.
    #[inline]
    pub fn contains(&self, id: TokenId) -> bool {
        self.bits
            .get((id.0 / 64) as usize)
            .is_some_and(|word| word & (1 << (id.0 % 64)) != 0)
    }
}

/// Inputs of an alignment.
#[derive(Debug, Clone, Copy)]
pub struct AlignInput<'a> {
    /// Left keys.
    pub left: &'a [TokenId],
    /// Right keys.
    pub right: &'a [TokenId],
    /// Left line weights 0..=4 (empty slice = all 0). Length must equal `left.len()` otherwise. Values above 4
    /// count as 4.
    pub left_weights: &'a [u8],
    /// Right line weights (same rules).
    pub right_weights: &'a [u8],
    /// Trivial tokens.
    pub trivial: &'a TrivialSet,
    /// Number of distinct tokens (from the interner).
    pub distinct: u32,
}

/// Similarity of two lines for closeness matching, per mille (0..=1000).
pub trait PairScorer: Sync {
    /// Similarity of left line `l` and right line `r` (indices into the full sequences). Values above 1000
    /// count as 1000.
    fn similarity(&self, l: u32, r: u32) -> u16;
}

impl<F: Fn(u32, u32) -> u16 + Sync> PairScorer for F {
    /// Calls the closure.
    fn similarity(&self, l: u32, r: u32) -> u16 {
        self(l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_set_is_total() {
        let mut set = TrivialSet::with_capacity(10);
        assert!(!set.contains(TokenId(3)));
        assert!(!set.contains(TokenId(1_000_000)));
        set.insert(TokenId(3));
        set.insert(TokenId(200));
        assert!(set.contains(TokenId(3)));
        assert!(set.contains(TokenId(200)));
        assert!(!set.contains(TokenId(4)));
        assert!(!set.contains(TokenId(199)));
        assert_eq!(TrivialSet::default(), TrivialSet::with_capacity(0));
    }

    #[test]
    fn closures_are_scorers() {
        let scorer = |l: u32, r: u32| (l * 10 + r) as u16;
        let dynamic: &dyn PairScorer = &scorer;
        assert_eq!(dynamic.similarity(3, 4), 34);
    }
}
