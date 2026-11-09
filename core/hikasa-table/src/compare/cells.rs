//! Cell interpretation and comparison (contract §5.3, T-6, T-7) and the order-preserving key encoding (§5.4).

use std::borrow::Cow;

use crate::cell::{Cell, CellKind, Value, datetime_to_ms};
use crate::compare::options::{ColumnRule, ColumnType};
use crate::compare::result::CellState;
use crate::regional::{Parsed, Regional};
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
    if rule.kind == ColumnType::Text {
        return Mode::Forced;
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

/// A cell as compared.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Interp<'a> {
    Empty,
    Num(f64),
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
            Interp::Num(_) => 2,
            Interp::Dt(_) => 3,
            Interp::Dur(_) => 4,
            Interp::Text(_) => 5,
            Interp::Err(_) => 6,
        }
    }
}

/// Interprets a value under a mode with a side's regional settings.
pub(crate) fn interpret<'a>(v: Value<'a>, mode: Mode, regional: &Regional) -> Interp<'a> {
    match v {
        Value::Empty => Interp::Empty,
        Value::Number(n) => Interp::Num(if n == 0.0 { 0.0 } else { n }),
        Value::Bool(b) => Interp::Bool(b),
        Value::DateTime(dt) => Interp::Dt(datetime_to_ms(dt)),
        Value::Duration(d) => Interp::Dur(d.as_millis() as i64),
        Value::Error(e) => Interp::Err(e as u8),
        Value::Text(t) => {
            if t.trim().is_empty() {
                return Interp::Empty;
            }
            let parsed = match mode {
                Mode::Forced | Mode::Text => None,
                Mode::Number => regional.parse_number(t).map(Parsed::Number),
                Mode::DateTime => regional.parse_datetime(t).map(Parsed::DateTime),
                Mode::Bool => regional.parse_bool(t).map(Parsed::Bool),
                Mode::Auto => regional.parse_any(t),
            };
            match parsed {
                Some(Parsed::Number(n)) => Interp::Num(if n == 0.0 { 0.0 } else { n }),
                Some(Parsed::DateTime(ms)) => Interp::Dt(ms),
                Some(Parsed::Bool(b)) => Interp::Bool(b),
                None => Interp::Text(t),
            }
        }
    }
}

/// Collapses whitespace runs to one space and trims.
pub(crate) fn collapse(s: &str) -> Cow<'_, str> {
    let needs = s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace)
        || s.as_bytes().windows(2).any(|w| w[0].is_ascii_whitespace() && w[1].is_ascii_whitespace())
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
    let s = if rule.ignore_whitespace { collapse(s) } else { Cow::Borrowed(s) };
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
    let (l, r) = (interpret(lv, mode, lreg), interpret(rv, mode, rreg));
    let state = match (l, r) {
        (Interp::Empty, Interp::Empty) => Some(CellState::Same),
        (Interp::Num(a), Interp::Num(b)) => {
            if a == b {
                Some(CellState::Same)
            } else if rule.numeric_tolerance.is_some_and(|t| t.within(a, b)) {
                Some(CellState::Unimportant)
            } else {
                None
            }
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
pub(crate) fn fast_same(l: Cell, lrow: &RowView<'_>, lc: u32, r: Cell, rrow: &RowView<'_>, rc: u32) -> bool {
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
        Interp::Num(v) => {
            let bits = v.to_bits();
            let ordered = if bits >> 63 == 1 { !bits } else { bits | (1 << 63) };
            out.extend_from_slice(&ordered.to_be_bytes());
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
        assert_eq!(cmp(Value::Text("1.0"), Value::Text("1"), Mode::Number, &plain), CellState::Same);
        assert_eq!(cmp(Value::Text("007"), Value::Text("7"), Mode::Text, &plain), CellState::Important);
        assert_eq!(cmp(Value::Text("007"), Value::Text("7"), Mode::Auto, &plain), CellState::Same);
        assert_eq!(cmp(Value::Number(5.0), Value::Text("5"), Mode::Auto, &plain), CellState::Same);
        assert_eq!(cmp(Value::Text(" "), Value::Empty, Mode::Text, &plain), CellState::Same);
        assert_eq!(cmp(Value::Text("x"), Value::Empty, Mode::Text, &plain), CellState::Important);
        let tol = ColumnRule {
            numeric_tolerance: Some(Tolerance::Absolute(0.01)),
            ..plain
        };
        assert_eq!(cmp(Value::Number(1.0), Value::Number(1.005), Mode::Number, &tol), CellState::Unimportant);
        assert_eq!(cmp(Value::Number(1.0), Value::Number(1.02), Mode::Number, &tol), CellState::Important);
        let case = ColumnRule {
            ignore_case: true,
            ignore_whitespace: true,
            ..plain
        };
        assert_eq!(cmp(Value::Text("Ab  c"), Value::Text("aB c "), Mode::Text, &case), CellState::Unimportant);
        let unimp = ColumnRule {
            unimportant: true,
            ..plain
        };
        assert_eq!(cmp(Value::Text("a"), Value::Text("b"), Mode::Text, &unimp), CellState::Unimportant);
        assert_eq!(cmp(Value::Number(5.0), Value::Text("5.0"), Mode::Forced, &plain), CellState::Important);
        assert_eq!(cmp(Value::Number(5.0), Value::Text("5"), Mode::Forced, &plain), CellState::Same);
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
    }
}
