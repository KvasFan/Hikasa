//! The words every question machine shares (§1.6.11, moved here by M2 W10b, §1.10.14): a [`Reply`], a [`ConfirmMode`],
//! [`Declined`], and [`Ask`], what a driver (the CLI's prompt, W11's dialogs) needs of a machine to ask its questions one
//! by one. W6's folder-operation `Confirmation` (re-exported by `hikasa_folder::ops`) and W10b's `SaveConfirmation`
//! (`hikasa_text::save`) are two such machines.

/// Cancel, or No to the operation: nothing runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the operation was declined")]
pub struct Declined;

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

/// A question machine as a driver sees it: the text of the question to answer now, and the answer. Object safe, so one
/// driver asks every kind of machine (`&mut dyn Ask`). A machine's own `next()` gives the whole question (its class, its
/// item, how many are left); this is the part every driver needs.
pub trait Ask {
    /// The text of the question to answer now; `None` when nothing is left to ask.
    fn question(&self) -> Option<&str>;
    /// Answers the question [`Ask::question`] shows (nothing happens when there is none).
    fn answer(&mut self, reply: Reply);
    /// How many questions are left, the current one included (0 when nothing is left).
    fn remaining(&self) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_and_modes_parse() {
        assert_eq!(Reply::parse(" Y \n"), Some(Reply::Yes));
        assert_eq!(Reply::parse("skip all"), Some(Reply::NoToAll));
        assert_eq!(Reply::parse("a"), Some(Reply::YesToAll));
        assert_eq!(Reply::parse("c"), Some(Reply::Cancel));
        assert_eq!(Reply::parse("no"), Some(Reply::No));
        assert_eq!(Reply::parse("maybe"), None);
        assert_eq!(
            ConfirmMode::parse("yes_to_all"),
            Some(ConfirmMode::YesToAll)
        );
        assert_eq!(ConfirmMode::parse("No-To-All"), Some(ConfirmMode::NoToAll));
        assert_eq!(ConfirmMode::parse("prompt"), Some(ConfirmMode::Prompt));
        assert_eq!(ConfirmMode::parse("never"), None);
        assert_eq!(Declined.to_string(), "the operation was declined");
    }
}
