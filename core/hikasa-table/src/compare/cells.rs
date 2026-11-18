//! Cell interpretation and comparison (contract §5.3, T-6, T-7) and the order-preserving key encoding (§5.4).

use std::borrow::Cow;

use crate::cell::{Cell, CellKind, Value, datetime_to_ms};
use crate::compare::options::{ColumnRule, ColumnType};
use crate::compare::result::CellState;
use crate::regional::{Decimal, EXACT_DIGITS, NumberText, Regional};
use crate::sheet::{ColumnKind, RowView};

/// How text cells of a comparison column are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Display texts compare (forced Text).
    Forced,
    /// Text cells stay text.
    Text,
    /// Text cells parse as numbers.
    Number,
    /// … as date-times.
    DateTime,
    /// … as booleans.
    Bool,
    /// … opportunistically (number, date, bool).
    Auto,
}

/// The mode of a comparison column (T-6).
pub(crate) fn mode(rule: &ColumnRule, l: ColumnKind, r: ColumnKind) -> Mode {
    match rule.kind {
        ColumnType::Text => return Mode::Forced,
        ColumnType::Number => return Mode::Number,
        ColumnType::General => {}
    }
    use ColumnKind as K;
    let one = |k: ColumnKind| match k {
        K::Number => Some(Mode::Number),
        K::DateTime => Some(Mode::DateTime),
        K::Bool => Some(Mode::Bool),
        K::Text => Some(Mode::Text),
        K::Empty | K::Duration | K::Mixed => None,
    };
    match (l, r) {
        (K::Empty, x) | (x, K::Empty) => one(x).unwrap_or(Mode::Auto),
        (a, b) if a == b => one(a).unwrap_or(Mode::Auto),
        _ => Mode::Auto,
    }
}

/// A number with more than 15 significant digits (F1): compared by its exact decimal, never through `f64`; the
/// approximation serves tolerances and the key order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Big {
    pub dec: Decimal,
    pub approx: f64,
}

/// A cell as compared. `Num` holds a number of at most 15 significant digits (its `f64` is exact enough: two such
/// decimals are equal exactly when their `f64`s are), `Big` one of more; the two never equal each other.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Interp<'a> {
    Empty,
    Num(f64),
    Big(Big),
    Dt(i64),
    Dur(i64),
    Bool(bool),
    Err(u8),
    Text(&'a str),
}

impl Interp<'_> {
    fn rank(&self) -> u8 {
        match self {
            Interp::Empty => 0,
            Interp::Bool(_) => 1,
            Interp::Num(_) | Interp::Big(_) => 2,
            Interp::Dt(_) => 3,
            Interp::Dur(_) => 4,
            Interp::Text(_) => 5,
            Interp::Err(_) => 6,
        }
    }
}

/// Powers of ten an `f64` holds exactly.
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

/// `v` rounded to 15 significant digits (the precision a workbook shows, F6). A deterministic function of `v`:
/// equality and the key encoding use the same one.
pub(crate) fn round15(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let m = v.abs();
    if m < 1e15 && m.fract() == 0.0 {
        return v;
    }
    let mut e = m.log10().floor() as i32;
    for _ in 0..3 {
        let k = 14 - e;
        if !(-22..=22).contains(&k) {
            break;
        }
        let p = POW10[k.unsigned_abs() as usize];
        let t = if k >= 0 { m * p } else { m / p };
        if t >= 1e15 {
            e += 1;
            continue;
        }
        if t < 1e14 {
            e -= 1;
            continue;
        }
        let n = t.round();
        let r = if k >= 0 { n / p } else { n * p };
        return r.copysign(v);
    }
    format!("{v:.14e}").parse().unwrap_or(v)
}

/// The decimal of a clean ASCII number (`1.25e3`), for numbers past the exact range.
fn decimal_of(clean: &str) -> Option<Decimal> {
    Regional::default()
        .scan_number(clean)
        .and_then(|n| n.decimal)
}

/// A workbook number (F6): compared at 15 significant digits (`0.1 + 0.2` equals `0.3`), except integers below
/// 2^53, which are exact whatever their length (long ids); with `exact` the value's shortest round-trip decimal.
pub(crate) fn workbook_number(v: f64, exact: bool) -> Interp<'static> {
    if v == 0.0 {
        return Interp::Num(0.0);
    }
    let r = round15(v);
    if r == v {
        return Interp::Num(v);
    }
    if exact {
        if let Some(dec) = decimal_of(&format!("{v:e}")) {
            return Interp::Big(Big { dec, approx: v });
        }
        return Interp::Num(v);
    }
    if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 {
        let mut mantissa = v.abs() as u128;
        let mut exponent = 0;
        while mantissa.is_multiple_of(10) {
            mantissa /= 10;
            exponent += 1;
        }
        return Interp::Big(Big {
            dec: Decimal {
                negative: v < 0.0,
                mantissa,
                exponent,
            },
            approx: v,
        });
    }
    Interp::Num(r)
}

/// A number read from text: `Num` up to 15 significant digits, `Big` above (text past 38 digits stays text).
fn text_number<'a>(t: &'a str, n: NumberText) -> Interp<'a> {
    if n.significant <= EXACT_DIGITS {
        return Interp::Num(n.value);
    }
    match n.decimal {
        Some(dec) => Interp::Big(Big {
            dec,
            approx: n.value,
        }),
        None => Interp::Text(t),
    }
}

/// Interprets a value under a mode with a side's regional settings; `exact`: workbook numbers compare by their
/// exact value (F6).
pub(crate) fn interpret<'a>(
    v: Value<'a>,
    mode: Mode,
    regional: &Regional,
    exact: bool,
) -> Interp<'a> {
    match v {
        Value::Empty => Interp::Empty,
        Value::Number(n) => workbook_number(n, exact),
        Value::Bool(b) => Interp::Bool(b),
        Value::DateTime(dt) => Interp::Dt(datetime_to_ms(dt)),
        Value::Duration(d) => Interp::Dur(d.as_millis() as i64),
        Value::Error(e) => Interp::Err(e as u8),
        Value::Text(t) => {
            if t.trim().is_empty() {
                return Interp::Empty;
            }
            match mode {
                Mode::Forced | Mode::Text => Interp::Text(t),
                Mode::Number => regional
                    .scan_number(t)
                    .map_or(Interp::Text(t), |n| text_number(t, n)),
                Mode::DateTime => regional
                    .parse_datetime(t)
                    .map_or(Interp::Text(t), Interp::Dt),
                Mode::Bool => regional.parse_bool(t).map_or(Interp::Text(t), Interp::Bool),
                Mode::Auto => match regional.scan_number(t) {
                    // A leading zero is part of an identifier (`007`, `01234`): text, never the number (F2).
                    Some(n) if n.leading_zero => Interp::Text(t),
                    Some(n) => text_number(t, n),
                    None => regional
                        .parse_datetime(t)
                        .map(Interp::Dt)
                        .or_else(|| regional.parse_bool(t).map(Interp::Bool))
                        .unwrap_or(Interp::Text(t)),
                },
            }
        }
    }
}

/// Collapses whitespace runs to one space and trims.
pub(crate) fn collapse(s: &str) -> Cow<'_, str> {
    let needs = s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace)
        || s.as_bytes()
            .windows(2)
            .any(|w| w[0].is_ascii_whitespace() && w[1].is_ascii_whitespace())
        || s.chars().any(|c| c.is_whitespace() && c != ' ');
    if !needs {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    for part in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    Cow::Owned(out)
}

/// A text folded per the rule's exceptions (case, whitespace).
pub(crate) fn fold_text<'a>(s: &'a str, rule: &ColumnRule) -> Cow<'a, str> {
    let s = if rule.ignore_whitespace {
        collapse(s)
    } else {
        Cow::Borrowed(s)
    };
    if rule.ignore_case {
        match s {
            Cow::Borrowed(b) => hikasa_core::case::fold(b),
            Cow::Owned(o) => Cow::Owned(hikasa_core::case::fold(&o).into_owned()),
        }
    } else {
        s
    }
}

fn texts_state(a: &str, b: &str, rule: &ColumnRule) -> Option<CellState> {
    if a == b {
        return Some(CellState::Same);
    }
    if (rule.ignore_case || rule.ignore_whitespace) && fold_text(a, rule) == fold_text(b, rule) {
        return Some(CellState::Unimportant);
    }
    None
}

/// Display text of an interpretation (mixed kinds compare these).
fn interp_display<'a>(i: &Interp<'a>, raw: Value<'a>) -> Cow<'a, str> {
    match i {
        Interp::Text(t) => Cow::Borrowed(t),
        Interp::Empty => Cow::Borrowed(""),
        _ => Cow::Owned(raw.display().into_owned()),
    }
}

/// Compares two values of one comparison column (§5.3 rules 2–6; rule 1 is [`fast_same`]).
pub(crate) fn compare_values(
    lv: Value<'_>,
    rv: Value<'_>,
    mode: Mode,
    rule: &ColumnRule,
    lreg: &Regional,
    rreg: &Regional,
) -> CellState {
    let differs = || {
        if rule.unimportant {
            CellState::Unimportant
        } else {
            CellState::Important
        }
    };
    if mode == Mode::Forced {
        if lv.is_blank() && rv.is_blank() {
            return CellState::Same;
        }
        return texts_state(&lv.display(), &rv.display(), rule).unwrap_or_else(differs);
    }
    let exact = rule.exact_numbers;
    let (l, r) = (
        interpret(lv, mode, lreg, exact),
        interpret(rv, mode, rreg, exact),
    );
    let tolerated = |a: f64, b: f64| {
        rule.numeric_tolerance
            .is_some_and(|t| t.within(a, b))
            .then_some(CellState::Unimportant)
    };
    let state = match (l, r) {
        (Interp::Empty, Interp::Empty) => Some(CellState::Same),
        (Interp::Num(a), Interp::Num(b)) => {
            if a == b {
                Some(CellState::Same)
            } else {
                tolerated(a, b)
            }
        }
        (Interp::Big(a), Interp::Big(b)) => {
            if a.dec == b.dec {
                Some(CellState::Same)
            } else {
                tolerated(a.approx, b.approx)
            }
        }
        (Interp::Num(a), Interp::Big(b)) | (Interp::Big(b), Interp::Num(a)) => {
            tolerated(a, b.approx)
        }
        (Interp::Dt(a), Interp::Dt(b)) | (Interp::Dur(a), Interp::Dur(b)) => {
            if a == b {
                Some(CellState::Same)
            } else if rule
                .date_tolerance_ms
                .is_some_and(|t| a.abs_diff(b) <= t.unsigned_abs())
            {
                Some(CellState::Unimportant)
            } else {
                None
            }
        }
        (Interp::Bool(a), Interp::Bool(b)) => (a == b).then_some(CellState::Same),
        (Interp::Err(a), Interp::Err(b)) => (a == b).then_some(CellState::Same),
        (Interp::Text(a), Interp::Text(b)) => texts_state(a, b, rule),
        (Interp::Empty, _) | (_, Interp::Empty) => None,
        (a, b) => texts_state(&interp_display(&a, lv), &interp_display(&b, rv), rule),
    };
    state.unwrap_or_else(differs)
}

/// §5.3 rule 1: identical text spans, or identical typed cells.
#[inline]
pub(crate) fn fast_same(
    l: Cell,
    lrow: &RowView<'_>,
    lc: u32,
    r: Cell,
    rrow: &RowView<'_>,
    rc: u32,
) -> bool {
    match (l.kind(), r.kind()) {
        (CellKind::Text, CellKind::Text) => match (lrow.get(lc), rrow.get(rc)) {
            (Value::Text(a), Value::Text(b)) => a == b,
            _ => false,
        },
        (CellKind::Empty, CellKind::Empty) => true,
        (a, b) if a == b && a != CellKind::Text => l.payload_eq(&r),
        _ => false,
    }
}

/// Appends the order-preserving encoding of one interpreted value (§5.4) to `out`.
pub(crate) fn encode(i: &Interp<'_>, rule: &ColumnRule, out: &mut Vec<u8>) {
    out.push(i.rank());
    match *i {
        Interp::Empty => {}
        Interp::Bool(b) => out.push(u8::from(b)),
        // Numbers: the ordered bits of the value (of the approximation for `Big`), then `0` for an exact `Num` or
        // `1` and the exact decimal for a `Big`: equal encodings exactly for equal numbers, in numeric order.
        Interp::Num(v) => {
            out.extend_from_slice(&ordered_bits(v).to_be_bytes());
            out.push(0);
        }
        Interp::Big(b) => {
            out.extend_from_slice(&ordered_bits(b.approx).to_be_bytes());
            out.push(1);
            encode_decimal(&b.dec, out);
        }
        Interp::Dt(v) | Interp::Dur(v) => {
            out.extend_from_slice(&((v as u64) ^ (1 << 63)).to_be_bytes());
        }
        Interp::Err(e) => out.push(e),
        Interp::Text(t) => {
            let folded = fold_text(t, rule);
            for &b in folded.as_bytes() {
                out.push(b);
                if b == 0 {
                    out.push(0xFF);
                }
            }
            out.extend_from_slice(&[0, 0]);
        }
    }
}

/// `f64` bits whose unsigned order is the numeric order.
fn ordered_bits(v: f64) -> u64 {
    let bits = v.to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

/// Order-preserving bytes of a decimal: sign, magnitude (power of ten of the first digit), digits, terminator;
/// inverted below zero.
fn encode_decimal(d: &Decimal, out: &mut Vec<u8>) {
    let digits = d.mantissa.to_string();
    let magnitude = d.exponent.saturating_add(digits.len() as i32);
    let flip = |b: u8| if d.negative { !b } else { b };
    out.push(if d.negative { 0 } else { 1 });
    for b in ((magnitude as u32) ^ (1 << 31)).to_be_bytes() {
        out.push(flip(b));
    }
    for b in digits.bytes() {
        out.push(flip(b));
    }
    out.push(flip(0));
}

/// Header names: equal → same; equal folded (case, whitespace) → unimportant; else `None`.
pub(crate) fn texts_header(a: &str, b: &str) -> Option<CellState> {
    let rule = ColumnRule {
        ignore_case: true,
        ignore_whitespace: true,
        ..ColumnRule::default()
    };
    texts_state(a, b, &rule)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::options::Tolerance;

    fn cmp(a: Value<'_>, b: Value<'_>, mode: Mode, rule: &ColumnRule) -> CellState {
        let r = Regional::default();
        compare_values(a, b, mode, rule, &r, &r)
    }

    #[test]
    fn typed_equality_and_tolerances() {
        let plain = ColumnRule::default();
        assert_eq!(
            cmp(Value::Text("1.0"), Value::Text("1"), Mode::Number, &plain),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Text("007"), Value::Text("7"), Mode::Text, &plain),
            CellState::Important
        );
        // A leading zero keeps an identifier text, even opportunistically (F2); forced Number equates them.
        assert_eq!(
            cmp(Value::Text("007"), Value::Text("7"), Mode::Auto, &plain),
            CellState::Important
        );
        assert_eq!(
            cmp(
                Value::Text("01234"),
                Value::Number(1234.0),
                Mode::Auto,
                &plain
            ),
            CellState::Important
        );
        assert_eq!(
            cmp(Value::Text("007"), Value::Text("7"), Mode::Number, &plain),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Text("0.50"), Value::Text(".5"), Mode::Auto, &plain),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Number(5.0), Value::Text("5"), Mode::Auto, &plain),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Text(" "), Value::Empty, Mode::Text, &plain),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Text("x"), Value::Empty, Mode::Text, &plain),
            CellState::Important
        );
        let tol = ColumnRule {
            numeric_tolerance: Some(Tolerance::Absolute(0.01)),
            ..plain
        };
        assert_eq!(
            cmp(Value::Number(1.0), Value::Number(1.005), Mode::Number, &tol),
            CellState::Unimportant
        );
        assert_eq!(
            cmp(Value::Number(1.0), Value::Number(1.02), Mode::Number, &tol),
            CellState::Important
        );
        let case = ColumnRule {
            ignore_case: true,
            ignore_whitespace: true,
            ..plain
        };
        assert_eq!(
            cmp(
                Value::Text("Ab  c"),
                Value::Text("aB c "),
                Mode::Text,
                &case
            ),
            CellState::Unimportant
        );
        let unimp = ColumnRule {
            unimportant: true,
            ..plain
        };
        assert_eq!(
            cmp(Value::Text("a"), Value::Text("b"), Mode::Text, &unimp),
            CellState::Unimportant
        );
        assert_eq!(
            cmp(Value::Number(5.0), Value::Text("5.0"), Mode::Forced, &plain),
            CellState::Important
        );
        assert_eq!(
            cmp(Value::Number(5.0), Value::Text("5"), Mode::Forced, &plain),
            CellState::Same
        );
    }

    /// Review mutations R2, R3, R4: lone tabs and no-break spaces are whitespace; a date difference equal to the
    /// tolerance is within it; error codes compare by code.
    #[test]
    fn whitespace_tolerance_boundaries_and_error_codes() {
        let ws = ColumnRule {
            ignore_whitespace: true,
            ..ColumnRule::default()
        };
        for spaced in ["a\tb", "a\u{a0}b", "a\nb"] {
            assert_eq!(
                cmp(Value::Text(spaced), Value::Text("a b"), Mode::Text, &ws),
                CellState::Unimportant,
                "{spaced:?}"
            );
        }
        let tol = ColumnRule {
            date_tolerance_ms: Some(1000),
            ..ColumnRule::default()
        };
        let at = |s: u32| {
            Value::DateTime(jiff::civil::DateTime::constant(
                2024, 1, 2, 3, 4, s as i8, 0,
            ))
        };
        assert_eq!(
            cmp(at(1), at(2), Mode::DateTime, &tol),
            CellState::Unimportant
        );
        assert_eq!(
            cmp(at(1), at(3), Mode::DateTime, &tol),
            CellState::Important
        );
        let plain = ColumnRule::default();
        let err = |e| Value::Error(e);
        assert_eq!(
            cmp(
                err(crate::CellError::Div0),
                err(crate::CellError::NA),
                Mode::Auto,
                &plain
            ),
            CellState::Important
        );
        assert_eq!(
            cmp(
                err(crate::CellError::NA),
                err(crate::CellError::NA),
                Mode::Auto,
                &plain
            ),
            CellState::Same
        );
    }

    /// Review mutation R6: a NUL inside a text is escaped, so the encoding of a row is injective (a text ending in
    /// NUL never reads as a shorter text and an empty value).
    #[test]
    fn text_encoding_escapes_nul() {
        let rule = ColumnRule::default();
        let enc = |vals: &[Interp<'_>]| {
            let mut out = Vec::new();
            for v in vals {
                encode(v, &rule, &mut out);
            }
            out
        };
        assert_ne!(
            enc(&[Interp::Text("a\0")]),
            enc(&[Interp::Text("a"), Interp::Empty])
        );
        assert_ne!(
            enc(&[Interp::Text("x"), Interp::Text("\0yy\0")]),
            enc(&[Interp::Text("x\0\0\u{5}\0yy"), Interp::Empty])
        );
    }

    #[test]
    fn long_numbers_compare_by_their_decimal() {
        let plain = ColumnRule::default();
        for mode in [Mode::Auto, Mode::Number] {
            // 17 digits: one f64, two numbers (F1).
            assert_eq!(
                cmp(
                    Value::Text("12345678901234567"),
                    Value::Text("12345678901234568"),
                    mode,
                    &plain
                ),
                CellState::Important,
                "{mode:?}"
            );
            assert_eq!(
                cmp(
                    Value::Text("12345678901234567"),
                    Value::Text("1.2345678901234567e16"),
                    mode,
                    &plain
                ),
                CellState::Same,
                "{mode:?}"
            );
            assert_eq!(
                cmp(
                    Value::Text("0.12345678901234567"),
                    Value::Text("0.1234567890123456700"),
                    mode,
                    &plain
                ),
                CellState::Same
            );
        }
        // A tolerance sees the approximations: within it is an unimportant difference, never same.
        let tol = ColumnRule {
            numeric_tolerance: Some(Tolerance::Absolute(0.0)),
            ..plain
        };
        assert_eq!(
            cmp(
                Value::Text("12345678901234567"),
                Value::Text("12345678901234568"),
                Mode::Number,
                &tol
            ),
            CellState::Unimportant
        );
        // A workbook integer below 2^53 is exact, so a 16-digit id equals its text.
        assert_eq!(
            cmp(
                Value::Number(1_234_567_890_123_456.0),
                Value::Text("1234567890123456"),
                Mode::Auto,
                &plain
            ),
            CellState::Same
        );
        assert_eq!(
            cmp(
                Value::Number(1_234_567_890_123_456.0),
                Value::Number(1_234_567_890_123_457.0),
                Mode::Number,
                &plain
            ),
            CellState::Important
        );
    }

    #[test]
    fn workbook_numbers_compare_at_15_digits_unless_exact() {
        let plain = ColumnRule::default();
        let sum = 0.1 + 0.2;
        assert_ne!(sum, 0.3);
        assert_eq!(
            cmp(Value::Number(sum), Value::Text("0.3"), Mode::Auto, &plain),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Number(sum), Value::Number(0.3), Mode::Number, &plain),
            CellState::Same
        );
        let exact = ColumnRule {
            exact_numbers: true,
            ..plain
        };
        assert_eq!(
            cmp(Value::Number(sum), Value::Text("0.3"), Mode::Auto, &exact),
            CellState::Important
        );
        assert_eq!(
            cmp(
                Value::Number(sum),
                Value::Text("0.30000000000000004"),
                Mode::Auto,
                &exact
            ),
            CellState::Same
        );
        assert_eq!(
            cmp(Value::Number(sum), Value::Number(0.3), Mode::Number, &exact),
            CellState::Important
        );
        // Rounding keeps what has 15 digits or fewer.
        for v in [
            0.3,
            1.0 / 3.0,
            999.999_999_999_999,
            123_456_789_012_345.0,
            1e-5,
            -2.5e300,
            0.1,
        ] {
            let r = round15(v);
            assert_eq!(
                format!("{r:.14e}").parse::<f64>().unwrap(),
                r,
                "{v} rounds to a 15-digit decimal"
            );
            assert_eq!(round15(r), r, "{v}: idempotent");
        }
        assert_eq!(round15(999.999_999_999_999), 999.999_999_999_999);
        assert_eq!(round15(0.1 + 0.7), 0.8);
        assert_eq!(round15(-(0.1 + 0.2)), -0.3);
    }

    #[test]
    fn encoding_preserves_order() {
        let rule = ColumnRule::default();
        let vals = [
            Interp::Empty,
            Interp::Bool(false),
            Interp::Num(f64::MIN),
            Interp::Num(-2.5),
            Interp::Num(0.0),
            Interp::Num(3.0),
            Interp::Num(1e300),
            Interp::Dt(-5),
            Interp::Dt(7),
            Interp::Text(""),
            Interp::Text("a"),
            Interp::Text("a\0"),
            Interp::Text("ab"),
            Interp::Text("b"),
        ];
        let enc: Vec<Vec<u8>> = vals
            .iter()
            .map(|v| {
                let mut out = Vec::new();
                encode(v, &rule, &mut out);
                out
            })
            .collect();
        for w in enc.windows(2) {
            assert!(w[0] < w[1], "{w:?}");
        }
        // Long numbers sit among the others in numeric order and are distinct from their neighbours.
        let r = Regional::default();
        fn num<'a>(t: &'a str, r: &Regional) -> Interp<'a> {
            interpret(Value::Text(t), Mode::Number, r, false)
        }
        let nums = [
            "-12345678901234568",
            "-12345678901234567",
            "-5",
            "0",
            "12345678901234560",
            "12345678901234567",
            "12345678901234568",
            "123456789012345678",
            "1e300",
        ];
        let enc: Vec<Vec<u8>> = nums
            .iter()
            .map(|t| {
                let mut out = Vec::new();
                encode(&num(t, &r), &rule, &mut out);
                out
            })
            .collect();
        for (w, t) in enc.windows(2).zip(nums.windows(2)) {
            assert!(w[0] < w[1], "{t:?}");
        }
        // Equal values, one encoding.
        let one = |t: &str| {
            let mut out = Vec::new();
            encode(&num(t, &r), &rule, &mut out);
            out
        };
        assert_eq!(one("12345678901234567"), one("1.2345678901234567e16"));
        assert_eq!(one("1.0"), one("1"));
        let mut wb = Vec::new();
        encode(
            &workbook_number(1_234_567_890_123_456.0, false),
            &rule,
            &mut wb,
        );
        assert_eq!(wb, one("1234567890123456"));
    }
}
