//! Bigram similarity for closeness matching (§2.15.3): [`BigramProfile`] and [`bigram_similarity`].
//!
//! The measure is the Dice coefficient on character-bigram multisets, in integer per mille. It is symmetric,
//! lies in `0..=1000`, and is 1000 exactly when the two multisets are equal.

/// A multiset of character bigrams (sorted `u64` codes) for bigram similarity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BigramProfile {
    /// Sorted bigram codes `(c_k << 32) | c_(k+1)`; a single char `c` is `(c << 32) | 0xFFFF_FFFF`.
    grams: Vec<u64>,
    /// Number of chars of the text.
    chars: u32,
}

/// Second half of the code of a one-char text (no Unicode scalar value is this large).
const LONE: u64 = 0xFFFF_FFFF;

impl BigramProfile {
    /// Profile of `text` (by `char`s).
    ///
    /// ```
    /// use hikasa_diff::{BigramProfile, bigram_similarity};
    ///
    /// let a = BigramProfile::new("night");
    /// let b = BigramProfile::new("nacht");
    /// assert_eq!(bigram_similarity(&a, &b), 250); // 2·|{ht}| / (4 + 4)
    /// assert_eq!(bigram_similarity(&a, &a), 1000);
    /// ```
    pub fn new(text: &str) -> BigramProfile {
        let mut grams = Vec::with_capacity(text.len());
        let mut chars = 0u32;
        let mut previous: Option<char> = None;
        for c in text.chars() {
            chars = chars.saturating_add(1);
            if let Some(p) = previous {
                grams.push((u64::from(p) << 32) | u64::from(c));
            }
            previous = Some(c);
        }
        if let (1, Some(c)) = (chars, previous) {
            grams.push((u64::from(c) << 32) | LONE);
        }
        grams.sort_unstable();
        BigramProfile { grams, chars }
    }

    /// Number of chars of the profiled text.
    pub fn char_count(&self) -> u32 {
        self.chars
    }

    /// Number of bigrams in the multiset.
    pub fn bigram_count(&self) -> usize {
        self.grams.len()
    }
}

/// Dice coefficient on bigram multisets, per mille, integer arithmetic (§2.15.3):
/// `(|a| + |b| == 0) ? 1000 : ⌊2000 × |a ∩ b| / (|a| + |b|)⌋`.
pub fn bigram_similarity(a: &BigramProfile, b: &BigramProfile) -> u16 {
    let total = (a.grams.len() + b.grams.len()) as u64;
    if total == 0 {
        return 1000;
    }
    let common = multiset_intersection(&a.grams, &b.grams) as u64;
    // 2·common ≤ total, so the quotient is at most 1000.
    (2000 * common / total) as u16
}

/// Size of the intersection of two sorted multisets (merge).
fn multiset_intersection(a: &[u64], b: &[u64]) -> usize {
    let (mut i, mut j, mut common) = (0, 0, 0);
    while let (Some(x), Some(y)) = (a.get(i), b.get(j)) {
        match x.cmp(y) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    common
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_follow_the_definition() {
        assert_eq!(BigramProfile::new(""), BigramProfile::default());
        let one = BigramProfile::new("é");
        assert_eq!(one.grams, [(u64::from('é') << 32) | LONE]);
        assert_eq!(one.char_count(), 1);
        let two = BigramProfile::new("ab");
        assert_eq!(two.grams, [(u64::from('a') << 32) | u64::from('b')]);
        assert_eq!(BigramProfile::new("abab").bigram_count(), 3);
    }

    #[test]
    fn similarity_examples() {
        let s =
            |a: &str, b: &str| bigram_similarity(&BigramProfile::new(a), &BigramProfile::new(b));
        assert_eq!(s("", ""), 1000);
        assert_eq!(s("", "a"), 0);
        assert_eq!(s("a", "a"), 1000);
        assert_eq!(s("a", "b"), 0);
        assert_eq!(s("a", "ab"), 0);
        assert_eq!(s("abc", "abd"), 500);
        assert_eq!(s("aaaa", "aa"), 500); // {aa,aa,aa} ∩ {aa} = 1 → 2000·1/4
        assert_eq!(s("int x = 1;", "int x = 2;"), 777);
        assert_eq!(s("abc", "cba"), 0);
    }
}
