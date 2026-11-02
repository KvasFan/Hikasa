//! The CLI's driver of the question machine (§1.6.11): one line per question on stderr, `[y]es [n]o [a]ll [s]kip all [c]ancel`.
//! The lines come from a [`BufRead`], so a test scripts them: no terminal, no PTY.

use std::io::{BufRead, Write};

use hikasa_folder::ops::{Confirmation, Reply};

/// The prompt after each question.
const CHOICES: &str = "[y]es [n]o [a]ll [s]kip all [c]ancel";

/// Asks every question of `machine` on `output`, reading `input`. The end of the input is `Cancel`: nothing is assumed.
pub fn ask(machine: &mut Confirmation, input: &mut dyn BufRead, output: &mut dyn Write) {
    while let Some(question) = machine.next() {
        let text = question.text.clone();
        let reply = loop {
            let _ = writeln!(output, "{text} {CHOICES}");
            let _ = output.flush();
            let mut line = String::new();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => break Reply::Cancel,
                Ok(_) => {
                    if let Some(reply) = Reply::parse(&line) {
                        break reply;
                    }
                }
            }
        };
        machine.reply(reply);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use hikasa_core::{CaseSensitivity, Ctl, Side};
    use hikasa_folder::ops::{
        ConfirmMode, ConfirmPolicy, FolderOp, OpPlan, OpScope, OpSettings, ReadOnlySides, plan_op,
    };
    use hikasa_folder::select::{Include, Selection, SideSet};
    use hikasa_folder::{
        ContentEngines, FolderEnv, FolderModel, FolderRules, NullObserver, compare_into,
    };
    use hikasa_vfs::ops::{OverwritePolicy, SkipReason};
    use hikasa_vfs::{MemMeta, MemoryVfs, StreamOptions};

    use super::*;

    fn path(p: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", p.replace('/', "\\"))
        } else {
            p.to_owned()
        }
    }

    /// A copy with one newer target, so an operation question and one item question.
    fn plan() -> OpPlan {
        let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
        let at = |secs| MemMeta {
            mtime: hikasa_core::FileTime::from_unix(secs, 0),
            ..MemMeta::default()
        };
        vfs.insert_file(&path("/l/x"), b"a".to_vec(), at(10))
            .unwrap();
        vfs.insert_file(&path("/r/x"), b"b".to_vec(), at(20))
            .unwrap();
        let spec = |p: &str| {
            hikasa_core::Spec::Local(
                hikasa_core::LocalPath::new(std::path::PathBuf::from(path(p))).unwrap(),
            )
        };
        let model = FolderModel::new(spec("/l"), spec("/r"), FolderRules::default()).unwrap();
        let engines = ContentEngines::with_stream_options(StreamOptions::default(), None);
        let now = jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC);
        let env = FolderEnv {
            vfs: &vfs,
            content: &engines,
            pool: None,
            ctl: Ctl::detached(),
            observer: &NullObserver,
            now: &now,
            content_budget: 1 << 20,
            archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
        };
        compare_into(&model, &env).unwrap();
        let read = model.read();
        let mut selection = Selection::default();
        selection.insert(read.children(read.root()).next().unwrap(), SideSet::LEFT);
        let scope = OpScope {
            selection,
            include: Include::visible(),
            iu: false,
            read_only: ReadOnlySides::default(),
        };
        let op = FolderOp::Copy {
            to: Side::Right,
            overwrite: OverwritePolicy::Always,
        };
        plan_op(
            &read,
            &vfs,
            &scope,
            &op,
            &OpSettings::default(),
            &Ctl::detached(),
        )
        .unwrap()
    }

    fn after(lines: &str) -> (Result<OpPlan, hikasa_folder::ops::Declined>, String) {
        let plan = plan();
        let mut machine = Confirmation::new(&plan, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
        let mut output = Vec::new();
        ask(
            &mut machine,
            &mut Cursor::new(lines.to_owned()),
            &mut output,
        );
        (machine.finish(plan), String::from_utf8(output).unwrap())
    }

    #[test]
    fn a6_16_yes_and_no() {
        let (done, text) = after("y\ny\n");
        assert!(done.unwrap().plan.items[0].skip.is_none());
        assert_eq!(text.lines().count(), 2);
        assert!(
            text.contains("[y]es [n]o [a]ll [s]kip all [c]ancel"),
            "{text}"
        );
        let (done, _) = after("y\nn\n");
        assert_eq!(done.unwrap().plan.items[0].skip, Some(SkipReason::Declined));
        let (done, _) = after("n\n");
        assert!(done.is_err());
    }

    #[test]
    fn a6_16_all_skip_all_and_cancel() {
        let (done, _) = after("a\na\n");
        assert!(done.unwrap().plan.items[0].skip.is_none());
        let (done, _) = after("y\ns\n");
        assert_eq!(done.unwrap().plan.items[0].skip, Some(SkipReason::Declined));
        let (done, _) = after("y\nc\n");
        assert!(done.is_err());
    }

    #[test]
    fn a6_16_the_end_of_the_input_is_cancel() {
        let (done, _) = after("");
        assert!(done.is_err());
        let (done, _) = after("y\n");
        assert!(done.is_err());
    }

    #[test]
    fn a6_16_junk_asks_again() {
        let (done, text) = after("what\ny\nmaybe\n\ny\n");
        assert!(done.unwrap().plan.items[0].skip.is_none());
        assert_eq!(text.lines().count(), 5);
    }
}
