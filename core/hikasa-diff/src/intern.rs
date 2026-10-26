//! Token interning: [`TokenId`], [`Interner`], [`intern_pair`] and the [`MAX_SEQUENCE_LEN`] limit.
//!
//! Ids are dense, start at 0 and are assigned in first-occurrence order over left-then-right input
//! (INV-INTERN-1), so they are a deterministic function of the input. The hash map inside [`Interner`] is only
//! ever probed, never iterated (T1), so its random hashing state cannot influence any result.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::error::DiffError;

/// An interned token (a line key, a segment, a char …). Ids are dense, starting at 0, assigned in
/// first-occurrence order over left-then-right input (INV-INTERN-1: deterministic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenId(pub u32);

/// Longest sequence any engine accepts: `i32::MAX - 2` (imara-diff asserts `len < i32::MAX`). hikasa-text
/// rejects longer documents earlier (D-43), so no user input reaches the assertion (T9).
pub const MAX_SEQUENCE_LEN: u32 = i32::MAX as u32 - 2;

/// Most distinct ids an [`Interner`] hands out: `i32::MAX - 1` (imara-diff's limit on `num_tokens`).
const MAX_DISTINCT: u32 = i32::MAX as u32 - 1;

/// Initial table capacity of [`intern_pair`] (smaller inputs get exactly their size).
const INITIAL_CAPACITY: usize = 4096;

/// Keys [`intern_pair`] interns before it sizes its table from the distinct ratio seen so far.
const RESERVE_SAMPLE: usize = 65_536;

/// Interner. The `HashMap` is internal and never iterated (T1).
#[derive(Debug, Clone)]
pub struct Interner<K: Hash + Eq> {
    /// Key → id. Only probed, never iterated.
    map: HashMap<K, TokenId>,
    /// The next id to hand out (= number of distinct keys so far).
    next: u32,
}

impl<K: Hash + Eq> Interner<K> {
    /// Empty interner.
    pub fn new() -> Self {
        Interner {
            map: HashMap::new(),
            next: 0,
        }
    }

    /// With capacity for `n` distinct keys.
    pub fn with_capacity(n: usize) -> Self {
        Interner {
            map: HashMap::with_capacity(n),
            next: 0,
        }
    }

    /// Id of `key`, assigning the next id when new. Fails with `InputTooLarge` beyond `i32::MAX - 1` ids
    /// (imara-diff's limit).
    pub fn intern(&mut self, key: K) -> Result<TokenId, DiffError> {
        match self.map.entry(key) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                if self.next >= MAX_DISTINCT {
                    return Err(DiffError::InputTooLarge {
                        tokens: u64::from(self.next) + 1,
                    });
                }
                let id = TokenId(self.next);
                self.next += 1;
                entry.insert(id);
                Ok(id)
            }
        }
    }

    /// Number of distinct keys.
    pub fn len(&self) -> u32 {
        self.next
    }

    /// Makes room for `total` distinct keys in all (no-op when there is room already).
    fn reserve_total(&mut self, total: usize) {
        self.map.reserve(total.saturating_sub(self.map.len()));
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.next == 0
    }
}

impl<K: Hash + Eq> Default for Interner<K> {
    /// [`Interner::new`].
    fn default() -> Self {
        Interner::new()
    }
}

/// Left and right token sequences that share one interner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternedPair {
    /// Left tokens.
    pub left: Vec<TokenId>,
    /// Right tokens.
    pub right: Vec<TokenId>,
    /// Number of distinct tokens.
    pub distinct: u32,
}

/// Interns two byte-key sequences (left first). The keys are borrowed (hikasa-text passes slices of its
/// per-chunk key arenas), so interning copies no key bytes. Fails with `InputTooLarge` when either sequence
/// is longer than [`MAX_SEQUENCE_LEN`].
///
/// Memory: the two id vectors (4 B per line, sized exactly from the size hints) and one hash-table entry per
/// distinct key (a 16 B key reference, the 4 B id, padding and a control byte: 25 B per bucket at 44–88 % load,
/// so 28–57 B per distinct key; ≈ 30 B on the §0.8 1M-line input). The table starts small and is sized once,
/// after 65,536 keys, from the distinct ratio seen so far times the longer side, so repetitive inputs never get
/// a table sized by their line count. The table's capacity never influences an id (T1).
///
/// ```
/// use hikasa_diff::{TokenId, intern_pair};
///
/// let left: [&[u8]; 3] = [b"a", b"b", b"a"];
/// let right: [&[u8]; 2] = [b"c", b"b"];
/// let pair = intern_pair(left, right).unwrap();
/// assert_eq!(pair.left, [TokenId(0), TokenId(1), TokenId(0)]);
/// assert_eq!(pair.right, [TokenId(2), TokenId(1)]);
/// assert_eq!(pair.distinct, 3);
/// ```
pub fn intern_pair<'k>(
    left: impl IntoIterator<Item = &'k [u8]>,
    right: impl IntoIterator<Item = &'k [u8]>,
) -> Result<InternedPair, DiffError> {
    let left = left.into_iter();
    let right = right.into_iter();
    // The length guard runs on the size hints first, so an over-long exact-size input is rejected before
    // anything is allocated or interned.
    let left_hint = left.size_hint().0;
    let right_hint = right.size_hint().0;
    check_sequence_len(left_hint)?;
    check_sequence_len(right_hint)?;
    let mut table = Table {
        interner: Interner::with_capacity(left_hint.max(right_hint).min(INITIAL_CAPACITY)),
        seen: 0,
        expected: left_hint.max(right_hint),
    };
    let left = table.sequence(left, left_hint)?;
    let right = table.sequence(right, right_hint)?;
    Ok(InternedPair {
        left,
        right,
        distinct: table.interner.len(),
    })
}

/// The interner of [`intern_pair`] and its sizing state.
struct Table<'k> {
    interner: Interner<&'k [u8]>,
    /// Keys interned so far (both sides).
    seen: usize,
    /// The longer side's length hint.
    expected: usize,
}

impl<'k> Table<'k> {
    /// Interns one sequence, enforcing [`MAX_SEQUENCE_LEN`] as it goes.
    fn sequence(
        &mut self,
        keys: impl Iterator<Item = &'k [u8]>,
        hint: usize,
    ) -> Result<Vec<TokenId>, DiffError> {
        let mut out = Vec::with_capacity(hint);
        for key in keys {
            check_sequence_len(out.len() + 1)?;
            if self.seen == RESERVE_SAMPLE {
                // Distinct keys expected in all: the ratio seen in the sample, times the longer side.
                let ratio = u64::from(self.interner.len());
                let estimate = (self.expected as u64).saturating_mul(ratio) / RESERVE_SAMPLE as u64;
                self.interner.reserve_total(estimate as usize);
            }
            self.seen += 1;
            out.push(self.interner.intern(key)?);
        }
        Ok(out)
    }
}

/// The length guard of every entry point: `len > MAX_SEQUENCE_LEN` → `InputTooLarge`.
pub(crate) fn check_sequence_len(len: usize) -> Result<(), DiffError> {
    if len > MAX_SEQUENCE_LEN as usize {
        Err(DiffError::InputTooLarge { tokens: len as u64 })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dense_and_in_first_seen_order() {
        let mut interner = Interner::new();
        assert!(interner.is_empty());
        assert_eq!(interner.intern("x"), Ok(TokenId(0)));
        assert_eq!(interner.intern("y"), Ok(TokenId(1)));
        assert_eq!(interner.intern("x"), Ok(TokenId(0)));
        assert_eq!(interner.len(), 2);
        assert!(!interner.is_empty());
        let default: Interner<u8> = Interner::default();
        assert_eq!(default.len(), 0);
    }

    #[test]
    fn the_length_guard_rejects_max_plus_one_without_allocating() {
        assert_eq!(check_sequence_len(MAX_SEQUENCE_LEN as usize), Ok(()));
        assert_eq!(
            check_sequence_len(MAX_SEQUENCE_LEN as usize + 1),
            Err(DiffError::InputTooLarge {
                tokens: u64::from(MAX_SEQUENCE_LEN) + 1
            })
        );
        // `repeat().take()` reports its exact length, so the guard fires before a single key is interned.
        let key: &[u8] = b"k";
        let too_long = std::iter::repeat_n(key, MAX_SEQUENCE_LEN as usize + 1);
        assert_eq!(
            intern_pair(too_long, std::iter::empty()),
            Err(DiffError::InputTooLarge {
                tokens: u64::from(MAX_SEQUENCE_LEN) + 1
            })
        );
        let too_long = std::iter::repeat_n(key, MAX_SEQUENCE_LEN as usize + 1);
        assert!(matches!(
            intern_pair(std::iter::empty(), too_long),
            Err(DiffError::InputTooLarge { .. })
        ));
    }

    /// Regression (intern memory): the table is sized by the distinct keys, not by the line count; a repetitive
    /// input keeps a small table.
    #[test]
    fn the_table_is_sized_by_distinct_keys() {
        let keys: Vec<Vec<u8>> = (0..1000u32).map(|k| k.to_le_bytes().to_vec()).collect();
        let lines = 300_000;
        let left = || (0..lines).map(|i| keys[i % 1000].as_slice());
        let mut table = Table {
            interner: Interner::with_capacity(INITIAL_CAPACITY),
            seen: 0,
            expected: lines,
        };
        let ids = table.sequence(left(), lines).unwrap();
        assert_eq!(table.interner.len(), 1000);
        assert_eq!(ids[1000], TokenId(0));
        assert!(
            table.interner.map.capacity() < 2 * INITIAL_CAPACITY,
            "capacity {} for 1,000 distinct keys",
            table.interner.map.capacity()
        );
        // Mostly unique keys: one reserve after the sample makes room for all of them.
        let unique: Vec<[u8; 4]> = (0..200_000u32).map(u32::to_le_bytes).collect();
        let mut table = Table {
            interner: Interner::with_capacity(INITIAL_CAPACITY),
            seen: 0,
            expected: unique.len(),
        };
        table
            .sequence(unique.iter().map(|k| k.as_slice()), unique.len())
            .unwrap();
        assert!(table.interner.map.capacity() >= 200_000);
        assert!(table.interner.map.capacity() < 2 * 262_144);
    }

    #[test]
    fn the_distinct_limit_is_enforced() {
        let mut interner: Interner<u32> = Interner::new();
        interner.next = MAX_DISTINCT;
        assert_eq!(
            interner.intern(5),
            Err(DiffError::InputTooLarge {
                tokens: u64::from(MAX_DISTINCT) + 1
            })
        );
        interner.next = MAX_DISTINCT - 1;
        assert_eq!(interner.intern(5), Ok(TokenId(MAX_DISTINCT - 1)));
        assert_eq!(interner.intern(5), Ok(TokenId(MAX_DISTINCT - 1)));
    }
}
