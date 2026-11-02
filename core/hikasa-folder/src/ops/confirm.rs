//! Confirmation as data (§1.6.11, D-178): the CLI, the dialogs and W8's `option confirm:` are three drivers of one
//! machine. A reply `YesToAll` is per class, a mode per plan.

use std::collections::{BTreeMap, BTreeSet};

use hikasa_core::Spec;
use hikasa_vfs::EntryKind;
use hikasa_vfs::ops::{Action, SkipReason};

use super::conflict::Conflicts;
use super::{OpPlan, OpVerb};

/// The class of a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QuestionKind {
    /// The operation as a whole.
    Operation,
    /// A read-only target.
    ReadOnly,
    /// A system file.
    System,
    /// A target newer than its source.
    OverwriteNewer,
    /// A move that replaces.
    ReplaceDuringMove,
    /// No recycle bin for the item.
    NoRecycleBin,
}

impl QuestionKind {
    /// The classes of the items' questions, in the order they are asked.
    pub const ITEMS: [QuestionKind; 5] = [
        QuestionKind::ReadOnly,
        QuestionKind::System,
        QuestionKind::OverwriteNewer,
        QuestionKind::ReplaceDuringMove,
        QuestionKind::NoRecycleBin,
    ];

    /// The stable word of the class.
    pub const fn as_str(self) -> &'static str {
        match self {
            QuestionKind::Operation => "operation",
            QuestionKind::ReadOnly => "read_only",
            QuestionKind::System => "system",
            QuestionKind::OverwriteNewer => "overwrite_newer",
            QuestionKind::ReplaceDuringMove => "replace_during_move",
            QuestionKind::NoRecycleBin => "no_recycle_bin",
        }
    }

    /// The conflict of the class.
    const fn conflict(self) -> Conflicts {
        match self {
            QuestionKind::Operation => Conflicts::empty(),
            QuestionKind::ReadOnly => Conflicts::READ_ONLY,
            QuestionKind::System => Conflicts::SYSTEM,
            QuestionKind::OverwriteNewer => Conflicts::OVERWRITE_NEWER,
            QuestionKind::ReplaceDuringMove => Conflicts::REPLACE_DURING_MOVE,
            QuestionKind::NoRecycleBin => Conflicts::NO_RECYCLE_BIN,
        }
    }

    /// The text of the question about `path`.
    fn text(self, path: &str) -> String {
        match self {
            QuestionKind::Operation => String::new(),
            QuestionKind::ReadOnly => format!("Replace or delete read-only {path}?"),
            QuestionKind::System => format!("Replace or delete system file {path}?"),
            QuestionKind::OverwriteNewer => format!("Overwrite newer {path}?"),
            QuestionKind::ReplaceDuringMove => format!("Replace {path} with the moved file?"),
            QuestionKind::NoRecycleBin => {
                format!("{path} cannot go to the recycle bin. Delete it permanently?")
            }
        }
    }
}

/// Cancel, or No to the operation: nothing runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the operation was declined")]
pub struct Declined;

/// One question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// Its class.
    pub kind: QuestionKind,
    /// The plan item it is about (`None` for the operation).
    pub item: Option<u32>,
    /// The questions left, this one included.
    pub remaining: u64,
    /// The text.
    pub text: String,
}

/// An answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Yes to this question.
    Yes,
    /// No to this question.
    No,
    /// Yes to the remaining questions of this class.
    YesToAll,
    /// No to the remaining questions of this class.
    NoToAll,
    /// Nothing runs.
    Cancel,
}

impl Reply {
    /// The reply of a word on a line, what a prompt offers as `[y]es [n]o [a]ll [s]kip all [c]ancel` (ASCII
    /// case-insensitive); `None` for anything else.
    pub fn parse(word: &str) -> Option<Reply> {
        match word.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Some(Reply::Yes),
            "n" | "no" => Some(Reply::No),
            "a" | "all" => Some(Reply::YesToAll),
            "s" | "skip" | "skip all" => Some(Reply::NoToAll),
            "c" | "cancel" => Some(Reply::Cancel),
            _ => None,
        }
    }
}

/// How a driver answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmMode {
    /// Questions are asked.
    #[default]
    Prompt,
    /// Everything is yes.
    YesToAll,
    /// The operation is yes, and each item question no: the items without a conflict proceed.
    NoToAll,
}

/// `[file_operations]` as data; `hikasa_session` fills it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmPolicy {
    /// `confirm_copy`.
    pub copy: bool,
    /// `confirm_move`.
    pub move_: bool,
    /// `confirm_delete`.
    pub delete: bool,
    /// `confirm_read_only`.
    pub read_only: bool,
    /// `confirm_system`.
    pub system: bool,
    /// `confirm_overwrite_newer`.
    pub overwrite_newer: bool,
    /// `confirm_replace_during_move`.
    pub replace_during_move: bool,
}

impl ConfirmMode {
    /// `prompt`, `yes-to-all`, `no-to-all`.
    pub fn parse(word: &str) -> Option<ConfirmMode> {
        match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "prompt" => Some(ConfirmMode::Prompt),
            "yes-to-all" => Some(ConfirmMode::YesToAll),
            "no-to-all" => Some(ConfirmMode::NoToAll),
            _ => None,
        }
    }
}

impl ConfirmPolicy {
    /// Every question on: the defaults of `[file_operations]`.
    pub const ALL: ConfirmPolicy = ConfirmPolicy {
        copy: true,
        move_: true,
        delete: true,
        read_only: true,
        system: true,
        overwrite_newer: true,
        replace_during_move: true,
    };

    /// True when a key asks the question of `kind`. The bin is always asked: nothing but a mode may answer it.
    const fn asks(&self, kind: QuestionKind) -> bool {
        match kind {
            QuestionKind::Operation | QuestionKind::NoRecycleBin => true,
            QuestionKind::ReadOnly => self.read_only,
            QuestionKind::System => self.system,
            QuestionKind::OverwriteNewer => self.overwrite_newer,
            QuestionKind::ReplaceDuringMove => self.replace_during_move,
        }
    }

    /// True when `verb` asks its operation question (§1.6.11).
    fn asks_operation(&self, verb: OpVerb) -> bool {
        match verb {
            OpVerb::Copy | OpVerb::CopyTo => self.copy,
            OpVerb::Move | OpVerb::Exchange | OpVerb::MoveTo => self.move_,
            OpVerb::Delete => self.delete,
            OpVerb::Rename | OpVerb::Touch | OpVerb::Mkdir => false,
            OpVerb::Sync => true,
        }
    }
}

/// The text of the operation question: items and bytes per action.
fn operation_text(plan: &OpPlan) -> String {
    let mut per_action: BTreeMap<&'static str, (u64, u64)> = BTreeMap::new();
    for item in &plan.plan.items {
        if item.error.is_none() && item.skip.is_none() {
            let row = per_action.entry(item.action.as_str()).or_default();
            row.0 += 1;
            row.1 += item.size.unwrap_or(0);
        }
    }
    let rows: Vec<String> = per_action
        .iter()
        .map(|(action, (items, bytes))| format!("{action} {items} items, {bytes} bytes"))
        .collect();
    format!("{}: {}?", plan.verb.as_str(), rows.join("; "))
}

/// The state of an item's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// Every question of the item so far had a yes.
    Yes,
    /// Some question had a no.
    No,
}

/// The question machine: a cursor over the questions of a plan.
#[derive(Debug, Clone)]
pub struct Confirmation {
    /// The questions still to come, the next one last.
    queue: Vec<Question>,
    /// The question to answer now.
    current: Option<Question>,
    /// What a `YesToAll` or a `NoToAll` said of a class.
    all: BTreeMap<QuestionKind, bool>,
    /// The items with a conflict, by what was answered.
    answers: BTreeMap<u32, Answer>,
    /// The items whose `R` goes.
    clear: BTreeSet<u32>,
    /// A `Cancel`, or a no to the operation.
    declined: bool,
    /// The operation question is still open.
    operation_open: bool,
}

impl Confirmation {
    /// The machine of `plan`. A key that is `false` drops its question: the item proceeds, which for a read-only target
    /// is what a yes does. With a mode other than `Prompt` no question is left.
    pub fn new(plan: &OpPlan, policy: &ConfirmPolicy, mode: ConfirmMode) -> Confirmation {
        let mut questions = Vec::new();
        let mut answers = BTreeMap::new();
        let mut clear = BTreeSet::new();
        let runs = |index: usize| {
            plan.plan
                .items
                .get(index)
                .is_some_and(|item| item.error.is_none() && item.skip.is_none())
        };
        let any_runs = (0..plan.plan.items.len()).any(runs);
        let ask_operation =
            any_runs && mode == ConfirmMode::Prompt && policy.asks_operation(plan.verb);
        if ask_operation {
            questions.push(Question {
                kind: QuestionKind::Operation,
                item: None,
                remaining: 0,
                text: operation_text(plan),
            });
        }
        for (index, meta) in plan.meta.iter().enumerate() {
            if meta.conflicts.is_empty() || !runs(index) {
                continue;
            }
            let number = u32::try_from(index).unwrap_or(u32::MAX);
            let mut refused = false;
            for kind in QuestionKind::ITEMS {
                if !meta.conflicts.contains(kind.conflict()) {
                    continue;
                }
                match mode {
                    ConfirmMode::YesToAll => {}
                    ConfirmMode::NoToAll if policy.asks(kind) => refused = true,
                    ConfirmMode::NoToAll => {}
                    ConfirmMode::Prompt if policy.asks(kind) => questions.push(Question {
                        kind,
                        item: Some(number),
                        remaining: 0,
                        text: kind.text(&meta.path),
                    }),
                    ConfirmMode::Prompt => {}
                }
            }
            if refused {
                answers.insert(number, Answer::No);
            } else {
                answers.insert(number, Answer::Yes);
                if meta.conflicts.contains(Conflicts::READ_ONLY) {
                    clear.insert(number);
                }
            }
        }
        let total = questions.len() as u64;
        for (index, question) in questions.iter_mut().enumerate() {
            question.remaining = total - index as u64;
        }
        questions.reverse();
        let mut machine = Confirmation {
            queue: questions,
            current: None,
            all: BTreeMap::new(),
            answers,
            clear,
            declined: false,
            operation_open: ask_operation,
        };
        machine.advance();
        machine
    }

    /// The question to answer; `None`: nothing left to ask.
    pub fn next(&self) -> Option<&Question> {
        self.current.as_ref()
    }

    /// The kinds of the questions left, with their counts.
    pub fn open_questions(&self) -> BTreeMap<QuestionKind, u64> {
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
        if question.kind == QuestionKind::Operation {
            self.operation_open = false;
            if !yes {
                self.declined = true;
                self.queue.clear();
                return;
            }
        } else {
            self.answer(&question, yes);
        }
        self.advance();
    }

    /// Records an answer to an item's question.
    fn answer(&mut self, question: &Question, yes: bool) {
        let Some(item) = question.item else {
            return;
        };
        if !yes {
            self.answers.insert(item, Answer::No);
            self.clear.remove(&item);
        }
    }

    /// Moves to the next question that still needs a person: a question of a declined item is not asked, and one of a class
    /// answered "to all" answers itself.
    fn advance(&mut self) {
        while let Some(mut question) = self.queue.pop() {
            let declined = question
                .item
                .is_some_and(|item| self.answers.get(&item) == Some(&Answer::No));
            if declined {
                continue;
            }
            if let Some(&yes) = self.all.get(&question.kind) {
                self.answer(&question, yes);
                continue;
            }
            question.remaining = self.queue.len() as u64 + 1;
            self.current = Some(question);
            return;
        }
    }

    /// The plan with its answers: declined items become `Action::Skip`, reason `declined`; `clear_read_only` as answered.
    /// What lies below a declined folder goes with it, under the same reason: the answer is why it stays, and the report
    /// says so. `Err(Declined)` after `Cancel` or a `No` to `Operation`, and with the operation question still open: what was
    /// not answered does not run. An item question left open is a declined item.
    pub fn finish(self, mut plan: OpPlan) -> Result<OpPlan, Declined> {
        if self.declined || self.operation_open {
            return Err(Declined);
        }
        let mut answers = self.answers;
        for question in self.current.iter().chain(&self.queue) {
            if let Some(item) = question.item {
                answers.insert(item, Answer::No);
            }
        }
        for (index, answer) in answers {
            let at = index as usize;
            let (Some(item), Some(meta)) = (plan.plan.items.get_mut(at), plan.meta.get_mut(at))
            else {
                continue;
            };
            meta.answer = Some(answer == Answer::Yes);
            if answer == Answer::No {
                item.action = Action::Skip;
                item.skip = Some(SkipReason::Declined);
                item.replaces = false;
                item.backup = None;
            } else if self.clear.contains(&index) {
                item.clear_read_only = true;
            }
        }
        decline_below(&mut plan);
        Ok(plan)
    }
}

/// The folders a declined `create_dir` stood for: what lies below them cannot run.
struct Refused {
    depth: u32,
    source: Option<Spec>,
    target: Spec,
}

/// True when `spec` lies below one of the folders.
fn below(spec: &Spec, refused: &Refused) -> bool {
    std::iter::successors(spec.parent(), Spec::parent)
        .any(|dir| dir == refused.target || Some(&dir) == refused.source.as_ref())
}

/// Declines every item below a declined folder. Skips hold their subtree in one stretch of the plan, so a stack is enough.
fn decline_below(plan: &mut OpPlan) {
    let mut stack: Vec<Refused> = Vec::new();
    for (item, meta) in plan.plan.items.iter_mut().zip(&mut plan.meta) {
        while stack.last().is_some_and(|folder| {
            item.depth <= folder.depth
                || !(below(&item.target, folder)
                    || item.source.as_ref().is_some_and(|spec| below(spec, folder)))
        }) {
            stack.pop();
        }
        if !stack.is_empty() && item.error.is_none() && item.skip.is_none() {
            item.action = Action::Skip;
            item.skip = Some(SkipReason::Declined);
            item.replaces = false;
            item.backup = None;
            meta.answer = Some(false);
        }
        if item.skip == Some(SkipReason::Declined) && item.kind == EntryKind::Dir {
            stack.push(Refused {
                depth: item.depth,
                source: item.source.clone(),
                target: item.target.clone(),
            });
        }
    }
}
