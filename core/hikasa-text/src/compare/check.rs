//! `TextComparison::check_invariants` (T4; INV-TC-1..8, INV-ROW-1; Appendix B).

use hikasa_core::cancel::CancelToken;
use hikasa_core::content::ContentResult;
use hikasa_core::error::InvariantViolation;
use hikasa_core::side::Side;
use hikasa_diff::RunKind;
use hikasa_formats::{LineState, Tokenizer};

use crate::classify::KeyShape;
use crate::compare::TextComparison;
use crate::compare::rows::{Importance, RowState, raw_equal};
use crate::compare::sections;
use crate::document::LineIdx;

fn fail<T>(id: &'static str, detail: String) -> Result<T, InvariantViolation> {
    Err(InvariantViolation { id, detail })
}

impl TextComparison {
    /// Re-checks INV-TC-1..8:
    ///
    /// 1. the runs cover both documents contiguously and the rows correspond 1:1 to the rows of the runs;
    /// 2. the rows' left (right) lines are exactly `0..left_lines` (`0..right_lines`) in order;
    /// 3. the sections are maximal, disjoint, ordered and cover exactly the non-Same rows (one per row for
    ///    `lines_independent` formats), with correct kinds, importance and line ranges;
    /// 4. the statistics equal a recount and their identities hold;
    /// 5. the verdicts follow §2.17.5 (and ignore-unimportant only makes them "more same");
    /// 6. (quick consistency is a property of `quick_compare_bytes`, checked by P-TC-2);
    /// 7. (parallel = sequential is checked by P-TC-3);
    /// 8. importance follows the rules: `Same` ⇔ `None` (INV-ROW-1); Equal pairs are Same exactly when the
    ///    lines are raw-equal (text, artificial break, and endings when compared), else Changed/Unimportant;
    ///    Changed pairs are Changed/Important, except raw-equal ones, which are Changed/Unimportant
    ///    (byte-identical lines never show as important); orphans are Important when orphans are always
    ///    important or the line is not trivial.
    pub fn check_invariants(&self) -> Result<(), InvariantViolation> {
        let (nl, nr) = (self.left.line_count(), self.right.line_count());
        // 1. Runs and rows.
        let (mut li, mut ri, mut row) = (0u32, 0u32, 0usize);
        if self.run_rows.len() != self.runs.len() {
            return fail("INV-TC-1", "run row offsets out of step".into());
        }
        for (k, run) in self.runs.iter().enumerate() {
            if run.left.start != li || run.right.start != ri {
                return fail(
                    "INV-TC-1",
                    format!("run {k} {run:?} does not continue at ({li}, {ri})"),
                );
            }
            if self.run_rows[k] as usize != row {
                return fail(
                    "INV-TC-1",
                    format!("run {k} starts at row {}, expected {row}", self.run_rows[k]),
                );
            }
            for j in 0..run.rows() {
                let Some(r) = self.rows.get(row) else {
                    return fail("INV-TC-1", format!("row {row} of run {k} missing"));
                };
                let (l, rr) = (r.left(), r.right());
                let (state, importance) = (r.state(), r.importance());
                let ok = match run.kind {
                    RunKind::Equal => {
                        l == Some(run.left.start + j)
                            && rr == Some(run.right.start + j)
                            && matches!(
                                (state, importance),
                                (RowState::Same, Importance::None)
                                    | (RowState::Changed, Importance::Unimportant)
                            )
                    }
                    RunKind::Changed => {
                        l == Some(run.left.start + j)
                            && rr == Some(run.right.start + j)
                            && state == RowState::Changed
                            && importance != Importance::None
                    }
                    RunKind::Delete => {
                        l == Some(run.left.start + j)
                            && rr.is_none()
                            && state == RowState::LeftOnly
                            && importance != Importance::None
                    }
                    RunKind::Insert => {
                        l.is_none()
                            && rr == Some(run.right.start + j)
                            && state == RowState::RightOnly
                            && importance != Importance::None
                    }
                };
                if !ok {
                    return fail(
                        "INV-TC-1",
                        format!("row {row} {:?} does not match run {k} {run:?}", r.unpack()),
                    );
                }
                row += 1;
            }
            li = run.left.end;
            ri = run.right.end;
        }
        if row != self.rows.len() {
            return fail(
                "INV-TC-1",
                format!("{} rows for {row} run rows", self.rows.len()),
            );
        }
        // 2. Coverage and order.
        if li != nl || ri != nr {
            return fail(
                "INV-TC-2",
                format!("runs cover ({li}, {ri}) of ({nl}, {nr}) lines"),
            );
        }
        let (mut el, mut er) = (0u32, 0u32);
        for (k, r) in self.rows.iter().enumerate() {
            if let Some(l) = r.left() {
                if l != el {
                    return fail("INV-TC-2", format!("row {k}: left line {l}, expected {el}"));
                }
                el += 1;
            }
            if let Some(x) = r.right() {
                if x != er {
                    return fail(
                        "INV-TC-2",
                        format!("row {k}: right line {x}, expected {er}"),
                    );
                }
                er += 1;
            }
            // INV-ROW-1.
            if (r.state() == RowState::Same) != (r.importance() == Importance::None) {
                return fail("INV-ROW-1", format!("row {k}: {:?}", r.unpack()));
            }
        }
        if el != nl || er != nr {
            return fail(
                "INV-TC-2",
                format!("rows cover ({el}, {er}) of ({nl}, {nr}) lines"),
            );
        }
        // 3 and 4. Sections and statistics equal a recount.
        let recount = sections::derive(&self.rows, nl, nr, self.format.text().lines_independent);
        if recount.sections != self.sections {
            return fail("INV-TC-3", "sections differ from a recount".into());
        }
        if recount.stats != self.stats || !self.stats.identities_hold() {
            return fail(
                "INV-TC-4",
                format!("statistics {:?} vs recount {:?}", self.stats, recount.stats),
            );
        }
        if recount.class_runs != self.class_runs {
            return fail("INV-TC-3", "class runs differ from a recount".into());
        }
        // 5. Verdicts.
        let (strict, lenient) = (self.verdict(false), self.verdict(true));
        let expected = if self.bytes_identical {
            ContentResult::BinarySame
        } else if self.left.binary().is_binary || self.right.binary().is_binary {
            ContentResult::BinaryDifferent
        } else if self.stats.important_rows > 0 {
            ContentResult::ImportantDifferences
        } else if self.stats.unimportant_rows > 0 {
            ContentResult::UnimportantOnly
        } else {
            ContentResult::RulesSame
        };
        if strict != expected
            || (strict == ContentResult::UnimportantOnly && lenient != ContentResult::RulesSame)
            || (strict != ContentResult::UnimportantOnly && lenient != strict)
        {
            return fail(
                "INV-TC-5",
                format!("verdicts {strict:?} / {lenient:?}, expected {expected:?}"),
            );
        }
        // 8. Importance follows the rules.
        self.check_importance()
    }

    /// INV-TC-8 (see `check_invariants`).
    fn check_importance(&self) -> Result<(), InvariantViolation> {
        let compare_eol = self.rules.importance.compare_line_endings;
        let orphans = self.rules.importance.orphans_always_important;
        let shape = KeyShape {
            identity: self.identity_path,
            compare_eol,
        };
        let (mut lc, mut rc) = (self.left.cursor(), self.right.cursor());
        let mut tokens = Vec::new();
        let mut key = Vec::new();
        for (run, &first) in self.runs.iter().zip(&self.run_rows) {
            for j in 0..run.rows() {
                let k = (first + j) as usize;
                let Some(r) = self.rows.get(k) else {
                    return fail("INV-TC-8", format!("row {k} missing"));
                };
                match (r.left(), r.right()) {
                    (Some(l), Some(x)) => {
                        let equal = raw_equal(
                            lc.line(l),
                            rc.line(x),
                            self.left.line_ending(LineIdx(l)),
                            self.right.line_ending(LineIdx(x)),
                            compare_eol,
                        );
                        let expected = match (run.kind, equal) {
                            (RunKind::Equal, true) => (RowState::Same, Importance::None),
                            (RunKind::Changed, false) => (RowState::Changed, Importance::Important),
                            _ => (RowState::Changed, Importance::Unimportant),
                        };
                        if (r.state(), r.importance()) != expected {
                            return fail(
                                "INV-TC-8",
                                format!(
                                    "row {k} of a {:?} run: raw equality {equal} but {:?}",
                                    run.kind,
                                    r.unpack()
                                ),
                            );
                        }
                    }
                    (l, x) => {
                        let (side, line) = match (l, x) {
                            (Some(l), _) => (Side::Left, l),
                            (_, Some(x)) => (Side::Right, x),
                            _ => {
                                return fail("INV-TC-8", format!("row {k}: orphan without a line"));
                            }
                        };
                        let doc = self.document(side);
                        let text = doc.line(LineIdx(line));
                        key.clear();
                        if shape.identity {
                            self.set.write_identity_key(
                                &text,
                                doc.line_ending(LineIdx(line)),
                                &mut key,
                            );
                        } else {
                            let state = if self.format.grammar().has_multiline() {
                                self.start_state_for_check(side, line)
                            } else {
                                LineState::Normal
                            };
                            Tokenizer::new(self.format.grammar()).tokenize_line(
                                &text,
                                line,
                                state,
                                &mut tokens,
                            );
                            self.set.write_key(
                                &text,
                                &tokens,
                                doc.line_ending(LineIdx(line)),
                                &mut key,
                                &CancelToken::never(),
                            );
                        }
                        let expected = if orphans || !shape.is_trivial(&key) {
                            Importance::Important
                        } else {
                            Importance::Unimportant
                        };
                        if r.importance() != expected {
                            return fail(
                                "INV-TC-8",
                                format!(
                                    "row {k}: orphan importance {:?}, expected {expected:?}",
                                    r.importance()
                                ),
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The start state of `line` for the invariant check (prepared on demand, never cancelled).
    fn start_state_for_check(&self, side: Side, line: u32) -> LineState {
        let cell = match side {
            Side::Right => &self.start_states[1],
            _ => &self.start_states[0],
        };
        if cell.get().is_none() {
            let _ = self.prepare_spans(side, &CancelToken::never());
        }
        cell.get()
            .and_then(|s| s.get(line as usize).copied())
            .unwrap_or(LineState::Normal)
    }
}
