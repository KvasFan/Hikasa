//! The save questions (§1.10.14, D-274): W6's machine (§1.6.11) over a [`SavePlan`]. A reply `YesToAll` is per class, a
//! mode per plan; a save has no operation question ("a save without a question executes at once after planning").

use std::collections::{BTreeMap, BTreeSet};

use hikasa_core::confirm::{Ask, ConfirmMode, Declined, Reply};
use hikasa_core::{Warning, WarningCode};
use hikasa_vfs::ops::{Action, SkipReason};

use super::{SaveIssue, SaveItem, SavePlan, SaveQuestion};

/// Which questions are asked (`[file_operations]` as W6 has them): a key that is `false` drops its question and the
/// item proceeds, as a yes would.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavePolicy {
    /// `confirm_read_only`: `ReadOnlyTarget`.
    pub read_only: bool,
    /// `confirm_overwrite_newer`: `OverwriteBackup` (as W6 asks it).
    pub overwrite_backup: bool,
}

impl SavePolicy {
    /// Every question on (the defaults).
    pub const ALL: SavePolicy = SavePolicy {
        read_only: true,
        overwrite_backup: true,
    };

    /// True when the policy asks `kind`.
    const fn asks(&self, kind: SaveQuestion) -> bool {
        match kind {
            SaveQuestion::ReadOnlyTarget => self.read_only,
            SaveQuestion::OverwriteBackup => self.overwrite_backup,
            SaveQuestion::ChangedOnDisk
            | SaveQuestion::Unmappable
            | SaveQuestion::LossyRewritten
            | SaveQuestion::ReplaceExisting => true,
        }
    }
}

impl Default for SavePolicy {
    fn default() -> SavePolicy {
        SavePolicy::ALL
    }
}

/// One question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAsk {
    /// Its class.
    pub kind: SaveQuestion,
    /// The plan item it is about.
    pub item: u32,
    /// The questions left, this one included.
    pub remaining: u64,
    /// The text.
    pub text: String,
}

/// The text of a question about `item`.
fn text(kind: SaveQuestion, item: &SaveItem) -> String {
    let path = item.target.canonical();
    match kind {
        SaveQuestion::ChangedOnDisk => {
            format!("{path} changed on disk since it was loaded. Overwrite it?")
        }
        SaveQuestion::ReadOnlyTarget => format!("{path} is read-only. Save over it?"),
        SaveQuestion::Unmappable => {
            let (count, first) = item
                .issues
                .iter()
                .find_map(|issue| match issue {
                    SaveIssue::Unmappable { count, first } => Some((*count, first.first())),
                    _ => None,
                })
                .unwrap_or((0, None));
            let at = first.map_or(String::new(), |c| {
                format!(
                    " (the first: U+{:04X} at line {}, column {})",
                    u32::from(c.char),
                    c.line.0 + 1,
                    c.column
                )
            });
            format!(
                "{count} characters of {path} cannot be written in {}{at}. Write them as '?'?",
                item.content.encoding.name()
            )
        }
        SaveQuestion::LossyRewritten => {
            let lines = item
                .issues
                .iter()
                .find_map(|issue| match issue {
                    SaveIssue::LossyRewritten { lines } => Some(*lines),
                    _ => None,
                })
                .unwrap_or(0);
            format!(
                "{lines} lines of {path} hold bytes that could not be decoded; saving writes them as U+FFFD. Save?"
            )
        }
        SaveQuestion::ReplaceExisting => format!("{path} exists. Replace it?"),
        SaveQuestion::OverwriteBackup => {
            format!("The backup of {path} would replace a file that stands there. Replace it?")
        }
    }
}

/// The state of an item's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
}

/// The question machine of a save: a cursor over the questions of a plan (§1.6.11's shape).
#[derive(Debug, Clone)]
pub struct SaveConfirmation {
    /// The questions still to come, the next one last.
    queue: Vec<SaveAsk>,
    current: Option<SaveAsk>,
    all: BTreeMap<SaveQuestion, bool>,
    answers: BTreeMap<u32, Answer>,
    /// The items whose `R` goes (`ReadOnlyTarget` answered yes, or not asked).
    clear: BTreeSet<u32>,
    declined: bool,
}

impl SaveConfirmation {
    /// The machine of `plan`. With a mode other than `Prompt` no question is left: `YesToAll` answers every question
    /// yes, `NoToAll` every question no (the items without one proceed).
    pub fn new(plan: &SavePlan, policy: &SavePolicy, mode: ConfirmMode) -> SaveConfirmation {
        let mut questions = Vec::new();
        let mut answers = BTreeMap::new();
        let mut clear = BTreeSet::new();
        for (index, item) in plan.runs() {
            let number = u32::try_from(index).unwrap_or(u32::MAX);
            let kinds = item.questions();
            if kinds.is_empty() {
                continue;
            }
            let mut refused = false;
            for kind in kinds {
                match mode {
                    ConfirmMode::YesToAll => {}
                    ConfirmMode::NoToAll if policy.asks(kind) => refused = true,
                    ConfirmMode::NoToAll => {}
                    ConfirmMode::Prompt if policy.asks(kind) => questions.push(SaveAsk {
                        kind,
                        item: number,
                        remaining: 0,
                        text: text(kind, item),
                    }),
                    ConfirmMode::Prompt => {}
                }
            }
            if refused {
                answers.insert(number, Answer::No);
            } else {
                answers.insert(number, Answer::Yes);
                if item.issues.contains(&SaveIssue::ReadOnlyTarget) {
                    clear.insert(number);
                }
            }
        }
        questions.reverse();
        let mut machine = SaveConfirmation {
            queue: questions,
            current: None,
            all: BTreeMap::new(),
            answers,
            clear,
            declined: false,
        };
        machine.advance();
        machine
    }

    /// The question to answer; `None`: nothing left to ask.
    pub fn next(&self) -> Option<&SaveAsk> {
        self.current.as_ref()
    }

    /// The classes of the questions left, with their counts (a dry run lists them; the CLI without a terminal names
    /// them).
    pub fn open_questions(&self) -> BTreeMap<SaveQuestion, u64> {
        let mut counts = BTreeMap::new();
        for question in self.current.iter().chain(&self.queue) {
            *counts.entry(question.kind).or_insert(0) += 1;
        }
        counts
    }

    /// Answers the current question.
    pub fn reply(&mut self, reply: Reply) {
        let Some(question) = self.current.take() else {
            return;
        };
        if reply == Reply::Cancel {
            self.declined = true;
            self.queue.clear();
            return;
        }
        let yes = matches!(reply, Reply::Yes | Reply::YesToAll);
        if matches!(reply, Reply::YesToAll | Reply::NoToAll) {
            self.all.insert(question.kind, yes);
        }
        self.record(&question, yes);
        self.advance();
    }

    fn record(&mut self, question: &SaveAsk, yes: bool) {
        if !yes {
            self.answers.insert(question.item, Answer::No);
            self.clear.remove(&question.item);
        }
    }

    /// The next question that needs a person: those of a declined item are not asked, a class answered "to all"
    /// answers itself.
    fn advance(&mut self) {
        while let Some(mut question) = self.queue.pop() {
            if self.answers.get(&question.item) == Some(&Answer::No) {
                continue;
            }
            if let Some(&yes) = self.all.get(&question.kind) {
                self.record(&question, yes);
                continue;
            }
            question.remaining = self.queue.len() as u64 + 1;
            self.current = Some(question);
            return;
        }
    }

    /// The plan with its answers: declined items (and those whose question is still open) become `Action::Skip`,
    /// reason `declined`; `clear_read_only` as answered; the warnings of the items that write (`unmappable_replaced`,
    /// `lossy_line_rewritten`) added and those of declined items dropped. `Err(Declined)` after `Cancel`.
    pub fn finish(self, mut plan: SavePlan) -> Result<SavePlan, Declined> {
        if self.declined {
            return Err(Declined);
        }
        let mut answers = self.answers;
        for question in self.current.iter().chain(&self.queue) {
            answers.insert(question.item, Answer::No);
        }
        let mut dropped = BTreeSet::new();
        for (index, answer) in answers {
            let at = index as usize;
            let (Some(planned), Some(item)) = (plan.plan.items.get_mut(at), plan.items.get(at))
            else {
                continue;
            };
            if answer == Answer::No {
                planned.action = Action::Skip;
                planned.skip = Some(SkipReason::Declined);
                planned.replaces = false;
                planned.backup = None;
                dropped.insert(item.target.canonical());
                continue;
            }
            if self.clear.contains(&index) {
                planned.clear_read_only = true;
            }
            for issue in &item.issues {
                let (code, message) = match issue {
                    SaveIssue::Unmappable { count, .. } => (
                        WarningCode::UnmappableReplaced,
                        format!(
                            "{count} characters that {} cannot write were written as '?'",
                            item.content.encoding.name()
                        ),
                    ),
                    SaveIssue::LossyRewritten { lines } => (
                        WarningCode::LossyLineRewritten,
                        format!("{lines} lines with undecodable bytes were written as U+FFFD"),
                    ),
                    _ => continue,
                };
                plan.warnings.push(Warning {
                    code,
                    message,
                    spec: Some(item.target.canonical()),
                });
            }
        }
        // Archive backups of archives no item writes any more.
        let writes: BTreeSet<String> = plan
            .plan
            .items
            .iter()
            .filter(|item| item.action == Action::Write)
            .filter_map(|item| item.target.outermost_file().map(|outer| outer.canonical()))
            .collect();
        plan.exec
            .archive_backups
            .retain(|outer, _| writes.contains(outer));
        plan.warnings
            .retain(|w| w.spec.as_ref().is_none_or(|spec| !dropped.contains(spec)));
        plan.confirmed = true;
        Ok(plan)
    }
}

impl Ask for SaveConfirmation {
    fn question(&self) -> Option<&str> {
        self.current.as_ref().map(|question| question.text.as_str())
    }

    fn answer(&mut self, reply: Reply) {
        self.reply(reply);
    }

    fn remaining(&self) -> u64 {
        self.current
            .as_ref()
            .map_or(0, |question| question.remaining)
    }
}
