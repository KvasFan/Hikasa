//! The **only** case and Unicode normalisation rules used anywhere in Hikasa (masks, name alignment, line keys).
//!
//! * Case-insensitive comparison uses the *simple* 1:1 lower-case mapping ([`simple_lower`]): a char maps to
//!   its lower case only when that lower case is exactly one char, otherwise to itself. There is no locale
//!   tailoring and no expansion (`"STRASSE"` and `"straße"` are different; `'İ'` maps to itself because its
//!   full lower case has two chars). D-07.
//! * Names and masks are compared in Unicode NFC ([`nfc`]).
//!
//! Every function here is pure, total and allocation-free when nothing has to change.

use std::borrow::Cow;

use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};

/// Simple 1:1 lower-case mapping: `c.to_lowercase()` when that yields exactly one char, otherwise `c`.
/// No locale tailoring. (Example: `'İ'` maps to itself because its full lower case is two chars.)
#[inline]
pub fn simple_lower(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(lower), None) => lower,
        _ => c,
    }
}

/// Simple 1:1 upper-case mapping (same rule as [`simple_lower`]); used for set ranges in masks (§2.2.2).
#[inline]
pub fn simple_upper(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_uppercase();
    }
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(upper), None) => upper,
        _ => c,
    }
}

/// [`simple_lower`] applied to every char; borrows when nothing changes.
pub fn fold(s: &str) -> Cow<'_, str> {
    let mut iter = s.char_indices();
    while let Some((at, c)) = iter.next() {
        let lower = simple_lower(c);
        if lower != c {
            let mut out = String::with_capacity(s.len());
            out.push_str(s.get(..at).unwrap_or(""));
            out.push(lower);
            for (_, rest) in iter {
                out.push(simple_lower(rest));
            }
            return Cow::Owned(out);
        }
    }
    Cow::Borrowed(s)
}

/// `fold(a) == fold(b)` without allocating.
pub fn eq_folded(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        return a.eq_ignore_ascii_case(b);
    }
    a.chars().map(simple_lower).eq(b.chars().map(simple_lower))
}

/// Unicode NFC (`unicode-normalization`); borrows when `s` is already NFC (ASCII fast path, then the
/// `is_nfc_quick` check).
pub fn nfc(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        return Cow::Borrowed(s);
    }
    if is_nfc_quick(s.chars()) == IsNormalized::Yes {
        return Cow::Borrowed(s);
    }
    let normalized: String = s.nfc().collect();
    if normalized == s {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_mappings() {
        assert_eq!(simple_lower('A'), 'a');
        assert_eq!(simple_lower('a'), 'a');
        assert_eq!(simple_lower('Ä'), 'ä');
        assert_eq!(
            simple_lower('İ'),
            'İ',
            "full lower case of U+0130 has two chars"
        );
        assert_eq!(simple_lower('\u{212A}'), 'k', "KELVIN SIGN");
        assert_eq!(simple_upper('ß'), 'ß', "full upper case of ß is SS");
        assert_eq!(simple_upper('z'), 'Z');
        assert_eq!(simple_upper('ǆ'), 'Ǆ');
    }

    #[test]
    fn fold_borrows_when_unchanged() {
        assert!(matches!(fold("abc"), Cow::Borrowed("abc")));
        assert!(matches!(fold("straße"), Cow::Borrowed(_)));
        assert_eq!(fold("STRASSE"), "strasse");
        assert_eq!(fold("ÀbÇ"), "àbç");
        assert_ne!(fold("STRASSE"), fold("straße"), "1:1 folding only");
    }

    #[test]
    fn eq_folded_matches_fold() {
        for (a, b) in [
            ("ABC", "abc"),
            ("Straße", "STRAßE"),
            ("k", "\u{212A}"),
            ("a", "b"),
            ("ab", "a"),
        ] {
            assert_eq!(eq_folded(a, b), fold(a) == fold(b), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn nfc_borrows_and_composes() {
        assert!(matches!(nfc("plain"), Cow::Borrowed(_)));
        assert!(matches!(nfc("café"), Cow::Borrowed(_)));
        assert_eq!(nfc("cafe\u{301}"), "café");
        assert_eq!(
            nfc("\u{212B}"),
            "\u{C5}",
            "ANGSTROM SIGN is a singleton decomposition"
        );
    }

    #[test]
    fn simple_lower_is_idempotent_for_every_char() {
        for code in 0..=0x10FFFFu32 {
            if let Some(c) = char::from_u32(code) {
                let once = simple_lower(c);
                assert_eq!(simple_lower(once), once, "U+{code:04X}");
            }
        }
    }
}
