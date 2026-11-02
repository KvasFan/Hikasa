//! The CLI's driver of the question machine (§1.6.11): one line per question on stderr, `[y]es [n]o [a]ll [s]kip all [c]ancel`.
//! A person's lines come from a thread that reads the terminal, so a Ctrl-C at a prompt ends it at once: a blocked read would
//! wait for a line. The questions themselves are read from a [`BufRead`] in the tests, with no terminal.

use std::io::{self, BufRead, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use hikasa_core::CancelToken;
use hikasa_folder::ops::{Confirmation, Reply};
use hikasa_report::escape_controls;

/// The prompt after each question.
const CHOICES: &str = "[y]es [n]o [a]ll [s]kip all [c]ancel";
/// How often a prompt looks at the cancel token.
const LOOK: Duration = Duration::from_millis(20);

/// Asks every question of `machine` on `output`. `line` gives the next line; `None`, the end of the input or a stop, is
/// `Cancel`: nothing is assumed.
fn drive(
    machine: &mut Confirmation,
    line: &mut dyn FnMut() -> Option<String>,
    output: &mut dyn Write,
) {
    while let Some(question) = machine.next() {
        let text = escape_controls(&question.text).into_owned();
        let reply = loop {
            let _ = writeln!(output, "{text} {CHOICES}");
            let _ = output.flush();
            match line() {
                None => break Reply::Cancel,
                Some(line) => {
                    if let Some(reply) = Reply::parse(&line) {
                        break reply;
                    }
                }
            }
        };
        machine.reply(reply);
    }
}

/// The lines of standard input, from a thread of their own: a read cannot be stopped, a wait for its result can. The thread
/// ends with the process, or at the end of the input.
fn stdin_lines() -> Receiver<String> {
    let (send, lines) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        loop {
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if send.send(line).is_err() => break,
                Ok(_) => {}
            }
        }
    });
    lines
}

/// Asks every question of `machine` on `output`, reading the terminal. The cancel token ends the prompt at once, as a `Cancel`.
pub fn ask(machine: &mut Confirmation, cancel: &CancelToken, output: &mut dyn Write) {
    let lines = stdin_lines();
    let mut next = || loop {
        if cancel.is_cancelled() {
            return None;
        }
        match lines.recv_timeout(LOOK) {
            Ok(line) => return Some(line),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    };
    drive(machine, &mut next, output);
}

/// [`drive`] over a reader.
#[cfg(test)]
fn ask_from(machine: &mut Confirmation, input: &mut dyn BufRead, output: &mut dyn Write) {
    let mut next = || {
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    };
    drive(machine, &mut next, output);
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
        ask_from(
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
