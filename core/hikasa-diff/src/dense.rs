//! [`DenseRemap`]: the mandatory step before every imara-diff call (D-16, §2.14.3, INV-DENSE-1).
//!
//! imara-diff allocates O(max token id) per call (Myers counts occurrences into a vector of size max id + 1;
//! Histogram allocates `num_tokens` list heads). Thousands of calls on small sub-ranges of a large input would
//! therefore cost O(calls × distinct keys). Remapping each sub-range pair to a local alphabet `0..k` first makes
//! every call O(|A| + |B|). The tables are epoch-stamped, so starting a new remap is O(1).

use hikasa_core::CancelToken;
use imara_diff::Token;

use crate::error::DiffError;
use crate::intern::TokenId;

/// Tokens remapped between two cancellation checks of [`DenseRemap::remap_cancellable`] (§5.3).
const CHUNK: usize = 65_536;

/// Remaps the token ids of a sub-range pair to a dense local alphabet `0..k` (first-occurrence order, `a`
/// before `b`). One instance per `align` call.
#[derive(Debug, Clone)]
pub(crate) struct DenseRemap {
    /// `stamp[id] == epoch` ⇔ `id` was seen in the current remap.
    stamp: Vec<u32>,
    /// Local id of `id` in the current remap (valid only when stamped).
    local: Vec<u32>,
    /// Current epoch; 0 is never a live epoch, so fresh (zeroed) slots are never stamped.
    epoch: u32,
}

impl DenseRemap {
    /// Capacity for global ids `< distinct` (O(distinct) once per alignment). Larger ids still work: the tables
    /// grow on demand, so the remap is total.
    pub(crate) fn new(distinct: u32) -> DenseRemap {
        DenseRemap {
            stamp: vec![0; distinct as usize],
            local: vec![0; distinct as usize],
            epoch: 0,
        }
    }

    /// Writes the remapped tokens of `a` and `b` (first-occurrence order, `a` before `b`) and returns the local
    /// alphabet size `k`, which is passed to imara-diff as `num_tokens`. O(|a| + |b|).
    /// `debug_assert!` every written id is `< k`.
    pub(crate) fn remap(
        &mut self,
        a: &[TokenId],
        b: &[TokenId],
        out_a: &mut Vec<Token>,
        out_b: &mut Vec<Token>,
    ) -> u32 {
        self.begin();
        let mut k = 0u32;
        out_a.clear();
        out_b.clear();
        self.extend(a, out_a, &mut k);
        self.extend(b, out_b, &mut k);
        debug_assert!(
            out_a.iter().chain(out_b.iter()).all(|t| t.0 < k),
            "INV-DENSE-1: a remapped id is not below k = {k}"
        );
        k
    }

    /// [`DenseRemap::remap`] in chunks of `CHUNK` tokens, checking `cancel` between chunks (for large ranges,
    /// §5.3). The output is identical to `remap`.
    pub(crate) fn remap_cancellable(
        &mut self,
        a: &[TokenId],
        b: &[TokenId],
        out_a: &mut Vec<Token>,
        out_b: &mut Vec<Token>,
        cancel: &CancelToken,
    ) -> Result<u32, DiffError> {
        if a.len() + b.len() <= CHUNK {
            return Ok(self.remap(a, b, out_a, out_b));
        }
        self.begin();
        let mut k = 0u32;
        out_a.clear();
        out_b.clear();
        for chunk in a.chunks(CHUNK) {
            cancel.check()?;
            self.extend(chunk, out_a, &mut k);
        }
        for chunk in b.chunks(CHUNK) {
            cancel.check()?;
            self.extend(chunk, out_b, &mut k);
        }
        debug_assert!(
            out_a.iter().chain(out_b.iter()).all(|t| t.0 < k),
            "INV-DENSE-1: a remapped id is not below k = {k}"
        );
        Ok(k)
    }

    /// Appends the local ids of `ids` to `out`, continuing the current remap (`k` = next local id).
    fn extend(&mut self, ids: &[TokenId], out: &mut Vec<Token>, k: &mut u32) {
        out.reserve(ids.len());
        for &id in ids {
            out.push(Token(self.local_id(id, k)));
        }
    }

    /// Starts a new remap: O(1), except once every 2³² − 1 remaps, when the stamps are cleared.
    fn begin(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.stamp.fill(0);
            self.epoch = 1;
        }
    }

    /// Local id of `id`, assigning `*k` (and incrementing it) on first sight in this epoch.
    #[inline]
    fn local_id(&mut self, id: TokenId, k: &mut u32) -> u32 {
        let slot = id.0 as usize;
        if slot >= self.stamp.len() {
            self.stamp.resize(slot + 1, 0);
            self.local.resize(slot + 1, 0);
        }
        if self.stamp[slot] != self.epoch {
            self.stamp[slot] = self.epoch;
            self.local[slot] = *k;
            *k += 1;
        }
        self.local[slot]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ids(raw: &[u32]) -> Vec<TokenId> {
        raw.iter().copied().map(TokenId).collect()
    }

    #[test]
    fn remaps_in_first_occurrence_order_and_resets_per_call() {
        let mut remap = DenseRemap::new(100);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let k = remap.remap(&ids(&[90, 7, 90]), &ids(&[3, 7]), &mut a, &mut b);
        assert_eq!(k, 3);
        assert_eq!(a, [Token(0), Token(1), Token(0)]);
        assert_eq!(b, [Token(2), Token(1)]);
        // A second call starts from 0 again (epoch stamping, no clearing).
        let k = remap.remap(&ids(&[7]), &ids(&[90]), &mut a, &mut b);
        assert_eq!(k, 2);
        assert_eq!(a, [Token(0)]);
        assert_eq!(b, [Token(1)]);
        // Ids beyond the declared capacity grow the tables instead of panicking.
        let k = remap.remap(&ids(&[1000]), &ids(&[]), &mut a, &mut b);
        assert_eq!((k, a.as_slice()), (1, [Token(0)].as_slice()));
    }

    #[test]
    fn the_epoch_wraps_safely() {
        let mut remap = DenseRemap::new(4);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        remap.remap(&ids(&[1, 2]), &ids(&[]), &mut a, &mut b);
        remap.epoch = u32::MAX;
        // Wrapping clears the stamps, so the stale stamp of id 1 can never be mistaken for a live one.
        remap.stamp[1] = 1;
        let k = remap.remap(&ids(&[3, 1]), &ids(&[1]), &mut a, &mut b);
        assert_eq!(k, 2);
        assert_eq!(a, [Token(0), Token(1)]);
        assert_eq!(b, [Token(1)]);
    }

    #[test]
    fn the_cancellable_remap_is_identical_and_observes_cancellation() {
        let a: Vec<TokenId> = (0..100_000u32).map(|i| TokenId(i % 977)).collect();
        let b: Vec<TokenId> = (0..70_000u32).map(|i| TokenId(i % 1013 + 500)).collect();
        let mut remap = DenseRemap::new(2000);
        let (mut a1, mut b1, mut a2, mut b2) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let k1 = remap.remap(&a, &b, &mut a1, &mut b1);
        let k2 = remap
            .remap_cancellable(&a, &b, &mut a2, &mut b2, &CancelToken::never())
            .unwrap();
        assert_eq!((k1, &a1, &b1), (k2, &a2, &b2));
        let cancel = CancelToken::new();
        cancel.cancel();
        assert_eq!(
            remap.remap_cancellable(&a, &b, &mut a2, &mut b2, &cancel),
            Err(DiffError::Cancelled)
        );
        // Small inputs take the plain path.
        assert_eq!(
            remap.remap_cancellable(&a[..10], &b[..10], &mut a2, &mut b2, &cancel),
            Ok(remap.remap(&a[..10], &b[..10], &mut a1, &mut b1))
        );
    }

    fn sequences() -> impl Strategy<Value = (Vec<u32>, Vec<u32>)> {
        let id = prop_oneof![0u32..8, 0u32..10_000];
        (
            proptest::collection::vec(id.clone(), 0..60),
            proptest::collection::vec(id, 0..60),
        )
    }

    proptest! {
        #![proptest_config(crate::test_config::unit("tests/proptest-regressions/unit_dense.txt"))]

        /// P-ALIGN-9: `DenseRemap` preserves token equality and first-occurrence order, and every remapped id is
        /// `< k` (the debug assertion inside `remap` is active in tests).
        #[test]
        fn p_align_9_dense_remap((a_raw, b_raw) in sequences(), reuse in proptest::bool::ANY) {
            let mut remap = DenseRemap::new(16);
            let (mut a, mut b) = (Vec::new(), Vec::new());
            if reuse {
                // A previous remap of other data must not leak into this one.
                remap.remap(&ids(&b_raw), &ids(&a_raw), &mut a, &mut b);
            }
            let k = remap.remap(&ids(&a_raw), &ids(&b_raw), &mut a, &mut b);
            let global: Vec<u32> = a_raw.iter().chain(&b_raw).copied().collect();
            let local: Vec<u32> = a.iter().chain(&b).map(|t| t.0).collect();
            prop_assert_eq!(global.len(), local.len());
            // Equality is preserved in both directions.
            for x in 0..global.len() {
                for y in 0..global.len() {
                    prop_assert_eq!(global[x] == global[y], local[x] == local[y]);
                }
            }
            // First occurrences are numbered 0, 1, 2, … in order, and every id is below k.
            let mut next = 0u32;
            for &id in &local {
                prop_assert!(id < k);
                if id == next {
                    next += 1;
                } else {
                    prop_assert!(id < next, "id {} appears before {}", id, next);
                }
            }
            prop_assert_eq!(next, k);
        }
    }
}
