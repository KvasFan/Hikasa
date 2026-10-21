//! Deterministic benchmark inputs (§6.1: seeded xorshift64*, never committed), shared by the benches and by
//! the timing and allocation tests (`#[path]` include).
#![allow(dead_code)]

use hikasa_diff::{Algorithm, AlignInput, AlignOptions, PairScorer, TokenId, TrivialSet};

/// Seeded xorshift64*.
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// An owned alignment input.
pub struct Generated {
    pub left: Vec<TokenId>,
    pub right: Vec<TokenId>,
    pub trivial: TrivialSet,
    pub distinct: u32,
    /// Number of anchor-less gaps by construction (pathological inputs), else 0.
    pub gaps: usize,
}

impl Generated {
    pub fn input(&self) -> AlignInput<'_> {
        AlignInput {
            left: &self.left,
            right: &self.right,
            left_weights: &[],
            right_weights: &[],
            trivial: &self.trivial,
            distinct: self.distinct,
        }
    }

    /// Tokens on both sides.
    pub fn tokens(&self) -> usize {
        self.left.len() + self.right.len()
    }
}

/// Number of "common" line keys (blank, braces, …); key 0 is blank and trivial.
const COMMON_KEYS: u32 = 16;

/// `lines` lines (≈ 48-byte source lines stand-in): 85 % unique keys, 15 % from 16 common keys (one of them
/// trivial). The right side copies the left one with change clusters of 5–15 lines starting at 1 line in 1000
/// (replacements, insertions and deletions in equal shares), so about 1 % of the lines change, in clusters.
pub fn clustered_changes(lines: usize, seed: u64) -> Generated {
    let mut rng = XorShift(seed);
    let mut next = COMMON_KEYS;
    let mut fresh = |rng: &mut XorShift| {
        let _ = rng;
        next += 1;
        TokenId(next - 1)
    };
    let mut left = Vec::with_capacity(lines);
    for _ in 0..lines {
        if rng.below(100) < 15 {
            left.push(TokenId(rng.below(u64::from(COMMON_KEYS)) as u32));
        } else {
            left.push(fresh(&mut rng));
        }
    }
    let mut right = Vec::with_capacity(lines + lines / 50);
    let mut i = 0;
    while i < left.len() {
        if rng.below(1000) == 0 {
            let len = 5 + rng.below(11) as usize;
            match rng.below(3) {
                0 => {
                    for _ in 0..len.min(left.len() - i) {
                        right.push(fresh(&mut rng));
                    }
                    i += len;
                }
                1 => {
                    for _ in 0..len {
                        right.push(fresh(&mut rng));
                    }
                }
                _ => i += len,
            }
        } else {
            right.push(left[i]);
            i += 1;
        }
    }
    let distinct = next;
    let mut trivial = TrivialSet::with_capacity(distinct);
    trivial.insert(TokenId(0));
    Generated {
        left,
        right,
        trivial,
        distinct,
        gaps: 0,
    }
}

/// `gaps` anchor-less gaps of `gap_len` lines per side, drawn independently on both sides from two repetitive
/// keys (think `}` and `x;`), separated by unique anchor lines present on both sides.
pub fn pathological(gaps: usize, gap_len: usize, seed: u64) -> Generated {
    let mut rng = XorShift(seed);
    let mut left = Vec::with_capacity(gaps * (gap_len + 1));
    let mut right = Vec::with_capacity(gaps * (gap_len + 1));
    for g in 0..gaps {
        let anchor = TokenId(2 + g as u32);
        left.push(anchor);
        right.push(anchor);
        for _ in 0..gap_len {
            left.push(TokenId(rng.below(2) as u32));
            right.push(TokenId(rng.below(2) as u32));
        }
    }
    Generated {
        left,
        right,
        trivial: TrivialSet::with_capacity(2 + gaps as u32),
        distinct: 2 + gaps as u32,
        gaps,
    }
}

/// `lines` random lines per side over `alphabet` symbols, none trivial: imara-diff's roughly quadratic worst
/// case, and the input that exercises Standard's bounded fallback (§2.14.5).
pub fn random_lines(lines: usize, alphabet: u32, seed: u64) -> Generated {
    let mut rng = XorShift(seed);
    let mut side = || -> Vec<TokenId> {
        (0..lines)
            .map(|_| TokenId(rng.below(u64::from(alphabet)) as u32))
            .collect()
    };
    let (left, right) = (side(), side());
    Generated {
        left,
        right,
        trivial: TrivialSet::with_capacity(alphabet),
        distinct: alphabet,
        gaps: 0,
    }
}

/// A cheap deterministic similarity (a hash of the two keys) for closeness in benches: the real bigram scorer is
/// hikasa-text's business, here only the alignment cost is measured.
pub struct HashScorer<'g>(pub &'g Generated);

impl PairScorer for HashScorer<'_> {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        let a = u64::from(self.0.left[l as usize].0);
        let b = u64::from(self.0.right[r as usize].0);
        let h = (a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b).wrapping_mul(0xD6E8_FEB8_6659_FD93);
        ((h >> 33) % 1001) as u16
    }
}

/// The default options (D-04: skew 300, closeness 500 ‰) for `algorithm`.
pub fn options(algorithm: Algorithm) -> AlignOptions {
    AlignOptions::with_algorithm(algorithm)
}
