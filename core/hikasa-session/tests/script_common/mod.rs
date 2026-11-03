//! Helpers of the script tests (W8a): a recording `ScriptUi`, a sandbox under `work/` (the captured cwd), and one call
//! that checks and runs a script's text.
#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use hikasa_core::{CancelToken, JobKind};
use hikasa_folder::ops::{Confirmation, Reply};
use hikasa_session::Service;
use hikasa_session::api::script::{
    HostUi, ScriptEvent, ScriptOutcome, ScriptRunRequest, ScriptSource, ScriptText, ScriptUi,
};
use hikasa_session::clipboard::ClipboardPayload;
use hikasa_session::jobs::JobControl;

/// A test's step at each log line.
pub type LineHook = Box<dyn FnMut(&str)>;

/// What a run told its interface.
#[derive(Default)]
pub struct Ui {
    /// `<kind> <where> <text>` of each log line.
    pub lines: Vec<String>,
    /// Every event.
    pub events: Vec<ScriptEvent>,
    /// The interface can ask (a terminal).
    pub can_ask: bool,
    /// The replies to W6's questions, in order (then `Cancel`).
    pub replies: VecDeque<Reply>,
    /// The questions asked.
    pub questions: Vec<String>,
    /// The answers to the stop-on-error question (then No).
    pub go_on: VecDeque<bool>,
    /// Stop-on-error questions asked.
    pub asked_continue: u32,
    /// Beeps.
    pub beeps: u32,
    /// Clipboard payloads placed.
    pub clipboard: Vec<ClipboardPayload>,
    /// Cancelled at the first question (a cancel while a question waits).
    pub cancel_at_question: Option<CancelToken>,
    /// Called with each log line as it comes (a test changes the disk between two commands).
    pub on_line: Option<LineHook>,
}

impl HostUi for Ui {
    fn event(&mut self, event: ScriptEvent) {
        if let ScriptEvent::Log { kind, line, text } = &event {
            let shown = hikasa_session::api::script_log::line(*kind, *line, text);
            if let Some(hook) = &mut self.on_line {
                hook(&shown);
            }
            self.lines.push(shown);
        }
        self.events.push(event);
    }
    fn can_ask(&self) -> bool {
        self.can_ask
    }
    fn ask_continue(&mut self, _line: u32, _message: &str) -> bool {
        self.asked_continue += 1;
        self.go_on.pop_front().unwrap_or(false)
    }
    fn beep(&mut self) {
        self.beeps += 1;
    }
}

impl ScriptUi for Ui {
    fn answer(&mut self, machine: &mut Confirmation, _cancel: &CancelToken) {
        while let Some(question) = machine.next() {
            self.questions.push(question.text.clone());
            if let Some(token) = &self.cancel_at_question {
                token.cancel();
                machine.reply(Reply::Cancel);
                continue;
            }
            let reply = self.replies.pop_front().unwrap_or(Reply::Cancel);
            machine.reply(reply);
        }
    }
    fn clipboard(&mut self, payload: &ClipboardPayload) -> Result<(), hikasa_session::ApiError> {
        self.clipboard.push(payload.clone());
        Ok(())
    }
}

/// A sandbox under the test's `TMPDIR`: the service's cwd is `root/work`. Dropping it gives every folder and file its
/// write bits back first (an `attrib +r` leaves read-only rows, which the temporary folder could not remove).
pub struct World {
    pub dir: tempfile::TempDir,
    pub service: Service,
}

impl World {
    pub fn new() -> World {
        let dir = tempfile::Builder::new().prefix("hks").tempdir().unwrap();
        let service = crate::common::service(dir.path());
        World { dir, service }
    }

    /// `work/<rel>`.
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join("work").join(rel)
    }

    /// Writes `work/<rel>` with a fixed time (seconds).
    pub fn file(&self, rel: &str, content: &str, secs: i64) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        set_mtime(&path, secs);
    }

    /// Makes `work/<rel>` a folder.
    pub fn dir(&self, rel: &str) {
        std::fs::create_dir_all(self.path(rel)).unwrap();
    }

    /// Every file below `work/<rel>` with its bytes, sorted (the dump of a tree).
    pub fn dump(&self, rel: &str) -> Vec<(String, Option<Vec<u8>>)> {
        let root = self.path(rel);
        let mut out = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(dir) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .strip_prefix(&root)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/");
                if path.is_dir() {
                    out.push((name, None));
                    pending.push(path);
                } else {
                    out.push((name, Some(std::fs::read(&path).unwrap())));
                }
            }
        }
        out.sort();
        out
    }

    /// The script's text as `script_load` reads it from `work/s.txt`.
    pub fn text(&self, source: &str) -> ScriptText {
        let path = self.path("s.txt");
        std::fs::write(&path, source).unwrap();
        let located = self.service.script_locate(path.as_os_str()).unwrap();
        assert!(matches!(located, ScriptSource::Spec(_)));
        self.service.script_load(located, None).unwrap()
    }

    /// Checks and runs `source` with `req` (its env from the script's references when empty) and `ui`.
    pub fn run_with(&self, source: &str, req: ScriptRunRequest, ui: &mut Ui) -> ScriptOutcome {
        let job = crate::common::job(&self.service, JobKind::Script);
        self.run_job(source, req, ui, &job, None)
    }

    pub fn run_job(
        &self,
        source: &str,
        mut req: ScriptRunRequest,
        ui: &mut Ui,
        job: &JobControl,
        stdout: Option<&mut dyn std::io::Write>,
    ) -> ScriptOutcome {
        let text = self.text(source);
        if req.env.is_empty() {
            let references = self.service.script_references(&text);
            req.env = hikasa_session::capture_script_env(&references.env);
        }
        let check = self.service.script_check(&text, Some(&req.args), &req.env);
        let script = check
            .script
            .unwrap_or_else(|| panic!("{:?}", check.errors.lines("s.txt")));
        match stdout {
            Some(out) => {
                let out: &mut dyn std::io::Write = &mut *out;
                self.service
                    .script_run(&script, &text, &req, ui, Some(out), job)
            }
            None => self.service.script_run(&script, &text, &req, ui, None, job),
        }
    }

    /// Runs with `option confirm:yes-to-all` semantics from the request and a fresh interface.
    pub fn run(&self, source: &str) -> (ScriptOutcome, Ui) {
        let mut ui = Ui::default();
        let outcome = self.run_with(source, yes(), &mut ui);
        (outcome, ui)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        writable_below(self.dir.path());
    }
}

/// Makes `root` and everything below it writable again, folders before their contents (so they can be listed and
/// emptied); links are not followed.
pub fn writable_below(root: &Path) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        set_read_only(&path, false);
        if meta.is_dir()
            && let Ok(entries) = std::fs::read_dir(&path)
        {
            pending.extend(entries.flatten().map(|e| e.path()));
        }
    }
}

/// Sets or clears the read-only state of a file or folder: the write bits on Unix (`0o222` off, `0o200` on, as W6's
/// `READ_ONLY` conflict reads them), the DOS attribute on Windows.
pub fn set_read_only(path: &Path, read_only: bool) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let mut permissions = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = permissions.mode();
        permissions.set_mode(if read_only {
            mode & !0o222
        } else {
            mode | 0o200
        });
    }
    #[cfg(not(unix))]
    {
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(read_only);
    }
    let _ = std::fs::set_permissions(path, permissions);
}

/// Sets the modification time of a file or folder (std only: no time crate in this crate's tests).
pub fn set_mtime(path: &Path, secs: i64) {
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(u64::try_from(secs).unwrap());
    let file = if path.is_dir() {
        std::fs::File::open(path)
    } else {
        std::fs::File::options().write(true).open(path)
    };
    file.unwrap().set_modified(time).unwrap();
}

/// A request with `--confirm yes-to-all`, JSON kept.
pub fn yes() -> ScriptRunRequest {
    ScriptRunRequest {
        confirm: Some(hikasa_folder::ops::ConfirmMode::YesToAll),
        json: true,
        ..ScriptRunRequest::default()
    }
}

/// Arguments as `OsString`s.
pub fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// The empty environment.
pub fn no_env() -> BTreeMap<String, OsString> {
    BTreeMap::new()
}

/// The statuses of a run's commands, as words.
pub fn statuses(outcome: &ScriptOutcome) -> Vec<String> {
    outcome
        .commands
        .iter()
        .map(|c| {
            serde_json::to_value(c.status)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// `work/<rel>` exists.
pub fn exists(path: &Path) -> bool {
    path.exists()
}
