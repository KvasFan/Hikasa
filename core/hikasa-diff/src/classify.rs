//! Generic importance splitting of inline ranges ([`split_ranges_by_class`], TC-014).

use std::ops::Range;

/// Splits byte ranges at class boundaries: `class_of(byte_offset)` returns a class id; output ranges carry the
/// class of all their bytes. Used by hikasa-text to colour unimportant parts blue (TC-014).
///
/// Pieces are returned in input order; empty input ranges are skipped. `class_of` is called once per byte, so a
/// caller that classifies by `char` should give every byte of a char the same class.
///
/// ```
/// use hikasa_diff::split_ranges_by_class;
///
/// // Bytes 0..4 are class 1, the rest class 0.
/// let pieces = split_ranges_by_class(&[2..6, 8..9], &|at| u8::from(at < 4));
/// assert_eq!(pieces, [(2..4, 1), (4..6, 0), (8..9, 0)]);
/// ```
pub fn split_ranges_by_class(
    ranges: &[Range<u32>],
    class_of: &dyn Fn(u32) -> u8,
) -> Vec<(Range<u32>, u8)> {
    let mut out = Vec::with_capacity(ranges.len());
    for range in ranges {
        if range.start >= range.end {
            continue;
        }
        let mut start = range.start;
        let mut class = class_of(start);
        for at in range.start + 1..range.end {
            let next = class_of(at);
            if next != class {
                out.push((start..at, class));
                start = at;
                class = next;
            }
        }
        out.push((start..range.end, class));
    }
    out
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // one-range expectations are intended
mod tests {
    use super::*;

    #[test]
    fn splits_at_every_class_change() {
        let class = |at: u32| u8::from(at.is_multiple_of(3));
        assert_eq!(
            split_ranges_by_class(&[0..4], &class),
            [(0..1, 1), (1..3, 0), (3..4, 1)]
        );
        assert!(split_ranges_by_class(&[], &class).is_empty());
        assert!(split_ranges_by_class(&[5..5], &class).is_empty());
        assert_eq!(split_ranges_by_class(&[1..3], &|_| 7), [(1..3, 7)]);
    }
}
