//! The closeness scorer (§2.15.2 and §2.15.3 as amended in wave 3): the similarity of two lines whose keys
//! differ, from bounded samples of their important text, under a deterministic work budget.
//!
//! ```text
//! sample(line)  = the first SAMPLE_CHARS (256) chars of the line's important text (the key without markers;
//!                 the raw line on the identity path)
//! sim(a, b)     = 999                          if the samples are equal after removing whitespace
//!               = ⌊2000·|A ∩ B| / (|A| + |B|)⌋ if either sample has fewer than 3 chars (A, B = char multisets;
//!                                                `}` vs `};` = 666)
//!               = bigram Dice (§2.15.3)         otherwise
//!               capped at 999 (1000 stays reserved for equal keys, which hikasa-diff scores itself)
//! cost          = 16 + chars(a) + chars(b) units per call; 16 + 4·chars units per profile built; 4 units for a
//!                 call whose upper bound (from the lengths alone) is below the threshold, which returns 0: such a
//!                 pair is never allowed, so the DP's result is the same as with its exact similarity
//! budget        = SCORER_UNITS_PER_LINE·(n + m) + SCORER_BASE_UNITS units per alignment; once spent, every
//!                 further call returns 0 (the remaining lines of the gap stay unpaired) and compare() adds
//!                 warning closeness_budget_exhausted
//! ```
//!
//! Integer-only and symmetric. Every quantity is a count (never time), and hikasa-diff calls the scorer in a
//! fixed order (one DP at a time, gaps in document order, on the calling thread), so the budget's effect is
//! reproducible (T6). One call costs O(SAMPLE_CHARS) whatever the line length, so the DP's cancellation checks
//! (every 4,096 cells) stay ≤ a few ms apart; the scorer also returns 0 at once after cancellation.
//!
//! Profiles are cached in one window per side: a contiguous run of line indices (the inner side of a DP fits,
//! ≤ 2,000 lines because a gap has at most 4,000,000 cells), rebuilt from scratch when a request leaves it.
//! Memory: ≤ 2 × WINDOW profiles of ≤ 3 KiB (a ≤ 1 KiB sample and ≤ 255 bigram codes) ≈ 12 MiB worst case.

use std::sync::Mutex;

use hikasa_core::cancel::CancelToken;
use hikasa_diff::PairScorer;

use crate::classify::KeyShape;
use crate::compare::keys::KeyStore;

/// Chars of a line's important text that its profile covers.
pub(crate) const SAMPLE_CHARS: usize = 256;

/// Profiles kept per side.
const WINDOW: usize = 2048;

/// Budget units per line of both documents.
pub(crate) const SCORER_UNITS_PER_LINE: u64 = 16;

/// Budget units of every alignment.
pub(crate) const SCORER_BASE_UNITS: u64 = 40_000_000;

/// The work budget of one alignment of `n` and `m` lines.
pub(crate) fn scorer_budget(n: u64, m: u64) -> u64 {
    SCORER_UNITS_PER_LINE
        .saturating_mul(n.saturating_add(m))
        .saturating_add(SCORER_BASE_UNITS)
}

/// The profile of one line's sample.
struct Profile {
    /// The sample (≤ SAMPLE_CHARS chars).
    sample: Box<str>,
    /// Its char count.
    chars: u32,
    /// Its char count without whitespace.
    squeezed: u32,
    /// Its bigram multiset (§2.15.3): sorted codes `(c_k << 32) | c_(k+1)`, exactly the codes of
    /// `hikasa_diff::BigramProfile` (a one-char sample never reaches the bigram measure).
    grams: Box<[u64]>,
}

impl Profile {
    fn new(sample: &str) -> Profile {
        let mut grams = Vec::with_capacity(sample.len());
        let mut previous: Option<char> = None;
        let mut chars = 0u32;
        let mut squeezed = 0u32;
        for c in sample.chars() {
            chars += 1;
            squeezed += u32::from(!c.is_whitespace());
            if let Some(p) = previous {
                grams.push((u64::from(p) << 32) | u64::from(c));
            }
            previous = Some(c);
        }
        grams.sort_unstable();
        Profile {
            sample: sample.into(),
            chars,
            squeezed,
            grams: grams.into_boxed_slice(),
        }
    }
}

/// A contiguous run of profiled lines of one side.
#[derive(Default)]
struct Window {
    base: u32,
    profiles: Vec<Profile>,
}

impl Window {
    /// The slot of `line`, if cached.
    fn get(&self, line: u32) -> Option<usize> {
        let k = line.checked_sub(self.base)? as usize;
        (k < self.profiles.len()).then_some(k)
    }

    /// Caches `profile` for `line`: appended when it continues the run (and the run has room), else the run
    /// restarts at `line`.
    fn put(&mut self, line: u32, profile: Profile) -> usize {
        let continues = self.base as usize + self.profiles.len() == line as usize
            && self.profiles.len() < WINDOW;
        if !continues {
            self.profiles.clear();
            self.base = line;
        }
        self.profiles.push(profile);
        self.profiles.len() - 1
    }
}

/// Mutable scorer state (the scorer is called from one thread at a time; the mutex only makes it `Sync`).
#[derive(Default)]
struct State {
    left: Window,
    right: Window,
    spent: u64,
    exhausted: bool,
    scratch: String,
}

/// The closeness scorer over the keys of both sides.
pub(crate) struct ProfileScorer<'k> {
    left: &'k KeyStore,
    right: &'k KeyStore,
    shape: KeyShape,
    budget: u64,
    threshold: u16,
    cancel: &'k CancelToken,
    state: Mutex<State>,
}

impl<'k> ProfileScorer<'k> {
    /// A scorer for the closeness threshold `threshold` (per mille) with the budget of
    /// `scorer_budget(lines of both sides)`.
    pub(crate) fn new(
        left: &'k KeyStore,
        right: &'k KeyStore,
        shape: KeyShape,
        threshold: u16,
        cancel: &'k CancelToken,
    ) -> ProfileScorer<'k> {
        ProfileScorer {
            left,
            right,
            shape,
            budget: scorer_budget(u64::from(left.len()), u64::from(right.len())),
            threshold,
            cancel,
            state: Mutex::new(State::default()),
        }
    }

    /// True when the budget ran out (some pairs were scored 0 without being compared).
    pub(crate) fn exhausted(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .exhausted
    }
}

/// Charges `cost` units; false (and exhausted) when the budget cannot cover them.
fn charge(spent: &mut u64, exhausted: &mut bool, budget: u64, cost: u64) -> bool {
    match spent.checked_add(cost) {
        Some(total) if total <= budget => {
            *spent = total;
            true
        }
        _ => {
            *exhausted = true;
            false
        }
    }
}

/// The slot of `line` in `window`, building its profile when needed; `None` when the budget ran out.
fn slot(
    window: &mut Window,
    scratch: &mut String,
    spent: (&mut u64, &mut bool),
    budget: u64,
    keys: &KeyStore,
    shape: KeyShape,
    line: u32,
) -> Option<usize> {
    if let Some(k) = window.get(line) {
        return Some(k);
    }
    shape.sample(keys.get(line), SAMPLE_CHARS, scratch);
    let cost = 16 + 4 * scratch.chars().count() as u64;
    let (spent, exhausted) = spent;
    if !charge(spent, exhausted, budget, cost) {
        return None;
    }
    Some(window.put(line, Profile::new(scratch)))
}

impl PairScorer for ProfileScorer<'_> {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        if self.cancel.is_cancelled() {
            return 0;
        }
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = &mut *guard;
        if state.exhausted {
            return 0;
        }
        let State {
            left,
            right,
            spent,
            exhausted,
            scratch,
        } = state;
        let Some(a) = slot(
            left,
            scratch,
            (spent, exhausted),
            self.budget,
            self.left,
            self.shape,
            l,
        ) else {
            return 0;
        };
        let Some(b) = slot(
            right,
            scratch,
            (spent, exhausted),
            self.budget,
            self.right,
            self.shape,
            r,
        ) else {
            return 0;
        };
        let (a, b) = (&left.profiles[a], &right.profiles[b]);
        if upper_bound(a, b) < self.threshold {
            // Never allowed whatever its exact value: 0 gives the same DP result.
            let _ = charge(spent, exhausted, self.budget, 4);
            return 0;
        }
        let cost = 16 + u64::from(a.chars) + u64::from(b.chars);
        if !charge(spent, exhausted, self.budget, cost) {
            return 0;
        }
        similarity(a, b)
    }
}

/// `sim(a, b)` of the module documentation (keys differ).
fn similarity(a: &Profile, b: &Profile) -> u16 {
    if squeezed_equal(&a.sample, &b.sample) {
        return 999;
    }
    if a.chars < 3 || b.chars < 3 {
        return multiset_dice(&a.sample, &b.sample).min(999);
    }
    bigram_dice(&a.grams, &b.grams).min(999)
}

/// `hikasa_diff::bigram_similarity` of two sorted bigram multisets: `⌊2000·|a ∩ b| / (|a| + |b|)⌋` (1000 when
/// both are empty). The intersection is a branch-free merge (the result is the same; random text makes the
/// branches of a plain merge unpredictable).
fn bigram_dice(a: &[u64], b: &[u64]) -> u16 {
    let total = a.len() + b.len();
    if total == 0 {
        return 1000;
    }
    let (mut i, mut j, mut common) = (0usize, 0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (x, y) = (a[i], b[j]);
        common += usize::from(x == y);
        i += usize::from(x <= y);
        j += usize::from(y <= x);
    }
    (2000 * common / total) as u16
}

/// An upper bound of `similarity(a, b)` from the lengths alone: 999 when the samples may be equal after removing
/// whitespace (equal squeezed lengths); else the Dice bound `⌊2000·min / (x + y)⌋` of the measure that applies
/// (char counts for short samples, bigram counts otherwise).
fn upper_bound(a: &Profile, b: &Profile) -> u16 {
    if a.squeezed == b.squeezed {
        return 999;
    }
    let (x, y) = if a.chars < 3 || b.chars < 3 {
        (a.chars as usize, b.chars as usize)
    } else {
        (a.grams.len(), b.grams.len())
    };
    if x + y == 0 {
        return 999;
    }
    (2000 * x.min(y) / (x + y)).min(999) as u16
}

/// `a` and `b` are equal after removing whitespace.
fn squeezed_equal(a: &str, b: &str) -> bool {
    let mut x = a.chars().filter(|c| !c.is_whitespace());
    let mut y = b.chars().filter(|c| !c.is_whitespace());
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (p, q) if p == q => {}
            _ => return false,
        }
    }
}

/// Dice coefficient of the char multisets of `a` and `b`, per mille (one of them has at most 2 chars).
fn multiset_dice(a: &str, b: &str) -> u16 {
    let (na, nb) = (a.chars().count(), b.chars().count());
    if na + nb == 0 {
        return 1000;
    }
    let (short, long) = if na <= nb { (a, b) } else { (b, a) };
    let mut seen: Vec<char> = Vec::with_capacity(2);
    let mut common = 0usize;
    for c in short.chars() {
        if seen.contains(&c) {
            continue;
        }
        seen.push(c);
        let in_short = short.chars().filter(|&x| x == c).count();
        let in_long = long.chars().filter(|&x| x == c).count();
        common += in_short.min(in_long);
    }
    (2000 * common / (na + nb)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim(a: &str, b: &str) -> u16 {
        let (p, q) = (Profile::new(a), Profile::new(b));
        let s = similarity(&p, &q);
        assert!(upper_bound(&p, &q) >= s, "{a:?} {b:?}");
        s
    }

    #[test]
    fn short_and_whitespace_only_changes_pair() {
        // §2.13.3 row 15 and one-token lines.
        assert_eq!(sim("x  ", "x"), 999);
        assert_eq!(sim("\tx", "x"), 999);
        assert_eq!(sim("}", "} "), 999);
        assert_eq!(sim("}", "};"), 666);
        assert_eq!(sim("};", "}"), 666);
        assert_eq!(sim("ab", "aB"), 500);
        assert_eq!(sim("ab", "ba"), 999);
        assert_eq!(sim("x", "y"), 0);
        assert_eq!(sim("{", "} else {"), 222);
        // Longer texts: whitespace-only differences are 999, other changes the bigram Dice.
        assert_eq!(sim("if (x) {", "if(x){"), 999);
        assert_eq!(sim("abcd", "abce"), 666);
        assert_eq!(
            sim("hello world", "hello world!"),
            sim("hello world!", "hello world")
        );
        // 1000 is never returned for different keys.
        assert_eq!(sim("abca", "bcab"), 999);
    }

    #[test]
    fn samples_bound_the_work() {
        let long = "ab".repeat(100_000);
        let p = Profile::new(&long[..SAMPLE_CHARS]);
        assert_eq!(p.chars as usize, SAMPLE_CHARS);
        assert_eq!(p.grams.len(), SAMPLE_CHARS - 1);
    }

    /// The branch-free Dice equals hikasa-diff's bigram similarity.
    #[test]
    fn bigram_dice_equals_the_reference() {
        let texts = [
            "",
            "ab",
            "abc",
            "night",
            "nacht",
            "aaaa",
            "aaab",
            "abab",
            "baba",
            "é€😀é€",
            "hello world",
            "hello  world",
            "xyzzy",
            "the quick brown fox",
            "the quick brown fix",
        ];
        for a in texts {
            for b in texts {
                let reference = hikasa_diff::bigram_similarity(
                    &hikasa_diff::BigramProfile::new(a),
                    &hikasa_diff::BigramProfile::new(b),
                );
                if a.chars().count() >= 2 && b.chars().count() >= 2 {
                    assert_eq!(
                        bigram_dice(&Profile::new(a).grams, &Profile::new(b).grams),
                        reference,
                        "{a:?} {b:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn multisets() {
        assert_eq!(multiset_dice("aa", "a"), 666);
        assert_eq!(multiset_dice("a", "aab"), 500);
        assert_eq!(multiset_dice("", ""), 1000);
    }
}
