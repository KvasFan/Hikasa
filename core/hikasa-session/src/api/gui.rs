//! GUI client helpers and the shared `gui open` parser (`hikasa_session::api::gui`; §1.8.5, §1.8.7, §3.6;
//! CLI-001…003, D-22, D-23, D-49, D-50, D-51).
//!
//! `hikasa gui open` and `hikasa-gui` accept the same arguments (D-51): [`normalize_legacy_args`] rewrites the
//! legacy spellings (`-name`, `/name` on Windows, aliases) to `--name[=value]`, and [`parse_open_args`] turns the
//! result into [`OpenArgs`]. [`Service::resolve_open`] resolves the specs (absolute canonical strings; `-` spooled
//! to the cache folder; `clipboard://` rejected), infers the view kind and makes the rules files absolute;
//! [`Service::gui_open`] then validates the request ([`Service::check_open`]), connects to (or launches,
//! [`Service::gui_launch`]) the GUI and sends `OpenView`. The GUI turns an `OpenView` back into service requests
//! with [`Service::open_view_request`] and reports a closing view's exit code with [`ViewOutcome::close`] (D-23).
//! `--gui` builds its request with [`GuiOpenRequest::for_text`] / [`GuiOpenRequest::for_folder`] (D-50), and a
//! second GUI instance forwards its arguments with [`Service::forward_open`].
//!
//! The switch table (canonical name ← aliases; `=v` takes a value, `[=v]` an optional one); [`open_usage`] prints
//! it:
//!
//! | Switch | Meaning |
//! |---|---|
//! | `--wait` | wait for the view to close (CLI-G01) |
//! | `--no-launch` ← `--nolaunch` | do not start the GUI |
//! | `--fv=v` ← `--fileviewer` | force the view type (`ViewKind::parse`) |
//! | `--title1=v` … `--title4=v` ← `--lefttitle`, `--righttitle`, `--centertitle`, `--outputtitle` | pane titles |
//! | `--ro` ← `--readonly`; `--ro1` ← `--lro`, `--leftreadonly`; `--ro2` ← `--rro`, `--rightreadonly` | read-only panes |
//! | `--filters=v` | name filter of the initial folder comparison |
//! | `--expandall` ← `--expand-all` | expand all subfolders |
//! | `--format=v` | forced format |
//! | `--iu` ← `--ignoreunimportant`, `--ignore-unimportant` | ignore unimportant differences |
//! | `--stdin-name=v` | name hint of `-` (D-15, D-49) |
//! | `--solo` | force a new GUI instance (`hikasa-gui` only) |
//! | `--json`, `--quiet` ← `-q`, `--no-color` | output flags |
//! | `--config-dir=v`, `--log=v`, `--verbose` | meaningful for `hikasa-gui` |
//! | `--help` ← `-h`, `/?` | usage |
//! | `--automerge`, `--center=v`, `--mergeoutput=v`, `--force`, `--reviewconflicts`, `--favorleft`, `--favorright`, `--closescript`, `--silent`, `--edit`, `--sync`, `--nobackups`, `--qc[=v]` ← `--quickcompare`, `--savetarget=v`, `--vcs1=v` … `--vcs4=v` ← `--vcsleft`, `--vcsright`, `--vcscenter`, `--vcsoutput` | recognised CLI-003 switches that M1 does not implement: forwarded in `OpenOptions::extra` with warning `switch_ignored` |

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hikasa_core::{
    CompareVerdict, ContentResult, ExitCode, Pair, Spec, SpecContext, Warning, WarningCode,
};
use hikasa_folder::{DisplayOptions, FolderRulesPatch};
use hikasa_formats::{EnabledOverrides, FormatKind};
use hikasa_text::{TextRulesPatch, ViewOptions};
use hikasa_vfs::{EntryKind, FollowLinks, Vfs};

use super::dto::GuiPingDto;
use super::requests::{FolderCompareRequest, TextCompareRequest};
use super::vocab::{resolve_folder_display, resolve_text_display};
use super::{ApiError, Service};
use crate::host::{EnvFacts, Platform};
use crate::ipc::{
    CloseReason, IpcClient, OpenOptions, REQUEST_TIMEOUT, Request, Response, RetryPolicy,
    ServerInfo, ViewId, ViewInfo, ViewKind, ViewResult, connect_or_launch,
};
use crate::jobs::run_guarded;

/// What a launched GUI needs to serve the CLI's endpoint (§1.8.5 amendment, decision 6 of the wave-4 review):
/// the endpoint depends on the configuration folder (D-47), so the launch carries it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuiLaunch {
    /// Arguments for `hikasa-gui`: `--config-dir <absolute dir>` for an explicit (`--config-dir`) or portable
    /// configuration, nothing for the per-user default (the GUI resolves the same folder from the inherited
    /// environment).
    pub args: Vec<OsString>,
    /// Environment variables to set for the child: `HIKASA_IPC_ENDPOINT` when the endpoint is overridden.
    pub env: Vec<(OsString, OsString)>,
}

/// Starts the GUI process and sleeps between connection attempts (implemented by hikasa-cli with
/// `std::process::Command`, detached, stdio null, and `std::thread::sleep`; tests record the calls).
pub trait GuiLauncher {
    /// Starts the GUI executable ([`find_gui_exe`]; `LaunchError::NotFound` when there is none) with
    /// `launch.args` and `launch.env` (requests then arrive over IPC).
    fn launch(&self, launch: &GuiLaunch) -> Result<(), LaunchError>;
    /// Sleeps `d` (the retry budget is the sum of these durations; core never reads a clock, T1).
    fn sleep(&self, d: Duration);
}

/// Launch failures.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LaunchError {
    /// No candidate executable exists (exit 103).
    #[error("hikasa-gui executable not found")]
    NotFound,
    /// Spawning failed: the executable exists but cannot be started (`ApiError::GuiNotStartable`, exit 103).
    /// The launcher names the executable in the error's message.
    #[error("cannot start hikasa-gui: {0}")]
    Spawn(#[from] std::io::Error),
}

impl hikasa_core::Classify for LaunchError {
    /// `NotFound` → `not_found`; `Spawn` → the OS error's kind.
    fn kind(&self) -> hikasa_core::ErrorKind {
        match self {
            LaunchError::NotFound => hikasa_core::ErrorKind::NotFound,
            LaunchError::Spawn(e) => hikasa_core::io_error_kind(e),
        }
    }
}

/// The name of the GUI executable on `platform`.
fn gui_exe_name(platform: Platform) -> &'static str {
    if platform == Platform::Windows {
        "hikasa-gui.exe"
    } else {
        "hikasa-gui"
    }
}

/// Candidate GUI executables, in order: sibling of the CLI executable, then each `PATH` entry (pure).
///
/// The name is `hikasa-gui` (`hikasa-gui.exe` on Windows); `PATH` is split at `:` (`;` on Windows, where
/// surrounding quotes are removed); empty entries are skipped and duplicates dropped.
pub fn gui_exe_candidates(
    exe_dir: Option<&Path>,
    path_var: Option<&OsStr>,
    platform: Platform,
) -> Vec<PathBuf> {
    let exe = gui_exe_name(platform);
    let separator = if platform == Platform::Windows {
        ';'
    } else {
        ':'
    };
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |dir: PathBuf| {
        let candidate = dir.join(exe);
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    };
    if let Some(dir) = exe_dir {
        push(dir.to_path_buf());
    }
    if let Some(path_var) = path_var {
        for part in split_os(path_var, separator) {
            let part = if platform == Platform::Windows {
                strip_quotes(part)
            } else {
                part
            };
            if !part.is_empty() {
                push(PathBuf::from(part));
            }
        }
    }
    out
}

/// The GUI executable to launch (additive, §1.8.7): `EnvFacts::gui_exe_override` (`$HIKASA_GUI_EXE`) alone when
/// it is set and names a file, else the first of [`gui_exe_candidates`] that is an executable file (on Unix a
/// candidate without an execute bit is skipped, like `which` does, so it never shadows a later one). `None` → the
/// launcher reports `LaunchError::NotFound` (exit 103). The override lets tests launch a fake GUI whatever is
/// installed next to the CLI (§6.3 `cli_gui.rs`).
///
/// A relative path is resolved against `EnvFacts::cwd` (`./<path>` when it is unknown), the way it was checked,
/// so the launcher never searches `PATH` for a bare name (wave-5 review, §1.9.3). An override that exists but
/// cannot be started is returned as is: the launch fails with `GuiNotStartable` (exit 103), whose message names
/// it.
pub fn find_gui_exe(facts: &EnvFacts) -> Option<PathBuf> {
    let absolute = |path: &Path| -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            facts
                .cwd
                .as_deref()
                .unwrap_or_else(|| Path::new("."))
                .join(path)
        }
    };
    if let Some(exe) = &facts.gui_exe_override {
        let exe = absolute(exe);
        return exe.is_file().then_some(exe);
    }
    gui_exe_candidates(
        facts.exe_dir.as_deref(),
        facts.path_var.as_deref(),
        facts.platform,
    )
    .into_iter()
    .map(|candidate| absolute(&candidate))
    .find(|candidate| is_executable_file(candidate))
}

/// A regular file (links followed) that may be executed: on Unix one of the execute bits is set.
fn is_executable_file(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        Ok(_) | Err(_) => false,
    }
}

/// Splits an OS string at an ASCII separator, keeping non-UTF-8 parts exact.
#[cfg(unix)]
fn split_os(s: &OsStr, separator: char) -> Vec<OsString> {
    use std::os::unix::ffi::OsStrExt;
    let sep = u8::try_from(separator).unwrap_or(b':');
    s.as_bytes()
        .split(|&b| b == sep)
        .map(|part| OsStr::from_bytes(part).to_os_string())
        .collect()
}

/// Splits an OS string at an ASCII separator, keeping non-UTF-8 parts exact.
#[cfg(windows)]
fn split_os(s: &OsStr, separator: char) -> Vec<OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let sep = separator as u16;
    let units: Vec<u16> = s.encode_wide().collect();
    units
        .split(|&u| u == sep)
        .map(OsString::from_wide)
        .collect()
}

/// Splits an OS string at an ASCII separator (lossy on other targets).
#[cfg(not(any(unix, windows)))]
fn split_os(s: &OsStr, separator: char) -> Vec<OsString> {
    s.to_string_lossy()
        .split(separator)
        .map(OsString::from)
        .collect()
}

/// Splits an OS string at its first `=`: (before, after). Exact for non-UTF-8 values.
fn split_eq(s: &OsStr) -> (OsString, Option<OsString>) {
    let mut parts = split_os(s, '=').into_iter();
    let head = parts.next().unwrap_or_default();
    let rest: Vec<OsString> = parts.collect();
    if rest.is_empty() {
        return (head, None);
    }
    let mut value = OsString::new();
    for (k, part) in rest.iter().enumerate() {
        if k > 0 {
            value.push("=");
        }
        value.push(part);
    }
    (head, Some(value))
}

/// Removes one pair of surrounding double quotes (Windows `PATH` entries may be quoted).
fn strip_quotes(part: OsString) -> OsString {
    match part.to_str() {
        Some(text) if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') => {
            OsString::from(&text[1..text.len() - 1])
        }
        _ => part,
    }
}

/// `gui open` request (also built by `--gui`, D-50: [`GuiOpenRequest::for_text`], [`GuiOpenRequest::for_folder`]).
#[derive(Debug, Clone, Default)]
pub struct GuiOpenRequest {
    /// Raw specs (resolved to absolute canonical specs before sending: the GUI has another cwd; "-" is spooled,
    /// `clipboard://` is rejected, D-49).
    pub specs: Vec<OsString>,
    /// Forced view kind (`--fv`); None → inferred (CLI-002).
    pub kind: Option<ViewKind>,
    /// View options.
    pub options: OpenOptions,
    /// Wait for the view to close (CLI-G01).
    pub wait: bool,
    /// The text rules file as typed (`--rules-file` of `text compare --gui`, `--text-rules-file` of
    /// `folder compare --gui`); made absolute into `options.text_rules_file` (wins over a value already there).
    /// `-`, `clipboard://` and paths that are not valid UTF-8 are `InvalidArgument`. Added in the wave-4 review.
    pub text_rules_file: Option<OsString>,
    /// The folder rules file as typed (`--rules-file` of `folder compare --gui`); made absolute into
    /// `options.folder_rules_file`, with the checks of `text_rules_file`. Added in the wave-4 review.
    pub folder_rules_file: Option<OsString>,
}

/// The view words of `--gui` that are not part of a compare request (D-50): they travel as words and the GUI
/// parses them with the same core parsers as the CLI ([`Service::open_view_request`]). Additive (wave-4 review).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewWords {
    /// `--display <word>` (text: `DisplayMode` words; folders: `FolderDisplayFilter` words).
    pub display: Option<String>,
    /// `--context <n>` (text).
    pub context_lines: Option<u32>,
    /// `--folders <word>` (folders).
    pub folders: Option<String>,
    /// `--suppress-filters` (folders).
    pub suppress_filters: bool,
    /// `--expand-all` (folders).
    pub expand_all: bool,
}

impl GuiOpenRequest {
    /// `text compare … --gui` (D-50): the request's two specs, forced format, per-side encodings, rules file,
    /// rules patch and ignore-unimportant, plus the view words; kind `text_compare`. The inverse is
    /// [`Service::open_view_request`].
    pub fn for_text(req: &TextCompareRequest, view: &ViewWords, wait: bool) -> GuiOpenRequest {
        let mut options = OpenOptions {
            format: req.format.clone(),
            encodings: [req.encodings.left.clone(), req.encodings.right.clone()],
            ignore_unimportant: req.ignore_unimportant,
            text_rules: (req.rules_patch != TextRulesPatch::default())
                .then(|| req.rules_patch.clone()),
            ..OpenOptions::default()
        };
        apply_words(&mut options, view);
        GuiOpenRequest {
            specs: vec![req.left.clone(), req.right.clone()],
            kind: Some(ViewKind::TextCompare),
            options,
            wait,
            text_rules_file: req.rules_file.clone(),
            folder_rules_file: None,
        }
    }

    /// `folder compare … --gui` (D-50): the request's two specs, rules file, `--criteria`, folder rules patch
    /// (its `name_filter` carries `--filters`), text rules file and patch and ignore-unimportant, plus the view
    /// words; kind `folder_compare`. The inverse is [`Service::open_view_request`].
    pub fn for_folder(req: &FolderCompareRequest, view: &ViewWords, wait: bool) -> GuiOpenRequest {
        let mut options = OpenOptions {
            ignore_unimportant: req.ignore_unimportant,
            criteria: req.criteria.clone(),
            folder_rules: (req.patch != FolderRulesPatch::default()).then(|| req.patch.clone()),
            text_rules: (req.text_patch != TextRulesPatch::default())
                .then(|| req.text_patch.clone()),
            ..OpenOptions::default()
        };
        apply_words(&mut options, view);
        GuiOpenRequest {
            specs: vec![req.left.clone(), req.right.clone()],
            kind: Some(ViewKind::FolderCompare),
            options,
            wait,
            text_rules_file: req.text_rules_file.clone(),
            folder_rules_file: req.rules_file.clone(),
        }
    }
}

/// Copies the view words into the options.
fn apply_words(options: &mut OpenOptions, view: &ViewWords) {
    options.display.clone_from(&view.display);
    options.context_lines = view.context_lines;
    options.folders.clone_from(&view.folders);
    options.suppress_filters = view.suppress_filters;
    options.expand_all = view.expand_all;
}

/// `gui open` outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiOpenOutcome {
    /// Opened, not waiting.
    Opened {
        /// The view.
        view: ViewId,
    },
    /// Waited until the view closed; `exit_code` is the raw code sent by the GUI (D-23).
    Closed {
        /// The view.
        view: ViewId,
        /// Its result.
        result: ViewResult,
        /// The view's exit code.
        exit_code: u8,
    },
}

impl GuiOpenOutcome {
    /// The process exit code of `gui open` / `--gui` (§2.21, D-23): 0 when the view opened without `--wait`,
    /// else the raw code the GUI sent (a newer GUI may send a code this build does not know). Additive.
    pub fn exit_code(&self) -> u8 {
        match self {
            GuiOpenOutcome::Opened { .. } => ExitCode::Success.code(),
            GuiOpenOutcome::Closed { exit_code, .. } => *exit_code,
        }
    }
}

/// Everything `gui_open` did (additive): the outcome plus what was sent, for `hikasa.gui_open.v1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiOpenReport {
    /// The outcome.
    pub outcome: GuiOpenOutcome,
    /// The view kind sent.
    pub kind: ViewKind,
    /// The canonical absolute specs sent.
    pub specs: Vec<String>,
    /// The file stdin was spooled to (`gui open -`, D-49); deleted again after `--wait`.
    pub spooled_stdin: Option<PathBuf>,
}

impl GuiOpenReport {
    /// The `hikasa.gui_open.v1` result.
    pub fn dto(&self) -> super::dto::GuiOpenDto {
        let spooled_stdin = self.spooled_stdin.as_ref().map(|p| p.display().to_string());
        match &self.outcome {
            GuiOpenOutcome::Opened { view } => super::dto::GuiOpenDto {
                view: view.0,
                waited: false,
                result: None,
                view_exit_code: None,
                spooled_stdin,
            },
            GuiOpenOutcome::Closed {
                view,
                result,
                exit_code,
            } => super::dto::GuiOpenDto {
                view: view.0,
                waited: true,
                result: Some(result.clone()),
                view_exit_code: Some(*exit_code),
                spooled_stdin,
            },
        }
    }

    /// The process exit code ([`GuiOpenOutcome::exit_code`]). Additive.
    pub fn exit_code(&self) -> u8 {
        self.outcome.exit_code()
    }
}

/// A `gui open` request resolved for sending (or for the GUI's own command line): what `OpenView` carries.
/// Additive (wave-4 review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOpen {
    /// The view kind (`--fv`, or inferred, CLI-002).
    pub kind: ViewKind,
    /// Canonical absolute specs (§4.4.3 rule 4).
    pub specs: Vec<String>,
    /// The options, with absolute rules file paths.
    pub options: OpenOptions,
    /// The file stdin was spooled to (`-`, D-49), if any; the caller owns it.
    pub spooled_stdin: Option<PathBuf>,
}

/// The service requests of an `OpenView` ([`Service::open_view_request`]): what the GUI runs for a view.
/// Additive (wave-4 review).
#[derive(Debug, Clone)]
pub enum ViewRequest {
    /// A Text Compare view.
    Text {
        /// The comparison to run (`Service::text_compare`).
        request: TextCompareRequest,
        /// The initial view options (`--display`, `--context`, ignore-unimportant).
        view: ViewOptions,
    },
    /// A Folder Compare view.
    Folder {
        /// The comparison to run (`Service::folder_prepare` + `folder_run`).
        request: FolderCompareRequest,
        /// The initial display options (`--display`, `--folders`, `--suppress-filters`, ignore-unimportant;
        /// the effective ignore-unimportant also counts the criteria word, `PreparedFolderCompare::ignore_unimportant`).
        display: DisplayOptions,
        /// `--expand-all` / `--expandall`.
        expand_all: bool,
    },
}

/// What a view knows when it closes: the input of the D-23 exit-code rules ([`ViewOutcome::close`]). The GUI
/// calls it for every waiter instead of choosing codes itself. Additive (wave-4 review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewOutcome {
    /// No verdict: a file view whose comparison has not finished or was cancelled (exit 0, D-23).
    NoVerdict,
    /// The view could not load its inputs (exit 100, reason `load_failed`).
    LoadFailed,
    /// A file view with its content result (exit as for `quick`: 1/2/11/12/13, 100 for `error`).
    Text {
        /// The verdict at close time.
        result: ContentResult,
        /// The view's ignore-unimportant at close time.
        ignore_unimportant: bool,
    },
    /// A folder view with its verdict at close time; a view still scanning (or cancelled) reports
    /// `CompareVerdict::Incomplete` and exits 100 (decision of the wave-4 review).
    Folder {
        /// The verdict at close time (`Incomplete` while scanning).
        verdict: CompareVerdict,
    },
}

impl ViewOutcome {
    /// The `ViewClosed` result and exit code (D-23): `NoVerdict` → 0; `LoadFailed` → 100 with reason
    /// `load_failed` (whatever `reason` says); `Text` → `ExitCode::for_content`; `Folder` →
    /// `ExitCode::for_verdict` (Incomplete → 100).
    pub fn close(&self, reason: CloseReason, saved: bool) -> (ViewResult, ExitCode) {
        match self {
            ViewOutcome::NoVerdict => (
                ViewResult::new(None, None, saved, reason),
                ExitCode::Success,
            ),
            ViewOutcome::LoadFailed => (
                ViewResult::new(None, None, saved, CloseReason::LoadFailed),
                ExitCode::UnknownError,
            ),
            ViewOutcome::Text {
                result,
                ignore_unimportant,
            } => (
                ViewResult::new(Some(result.kind()), None, saved, reason),
                ExitCode::for_content(result, *ignore_unimportant),
            ),
            ViewOutcome::Folder { verdict } => (
                ViewResult::new(None, Some(*verdict), saved, reason),
                ExitCode::for_verdict(*verdict),
            ),
        }
    }
}

/// Parsed `hikasa gui open` / `hikasa-gui` arguments (CLI-001…CLI-003, D-51).
#[derive(Debug, Clone, Default)]
pub struct OpenArgs {
    /// The request.
    pub request: GuiOpenRequest,
    /// `--no-launch` (CLI only).
    pub no_launch: bool,
    /// `--solo` (GUI only: force a new instance).
    pub solo: bool,
    /// `--json` given after `open` (output flags only).
    pub json: bool,
    /// `--quiet` given after `open`.
    pub quiet: bool,
    /// `--no-color` given after `open`.
    pub no_color: bool,
    /// `--config-dir` (the GUI's configuration; the CLI merges it with its global flag before building the
    /// service, §1.9.2 amendment).
    pub config_dir: Option<PathBuf>,
    /// `--log` (as `config_dir`).
    pub log: Option<PathBuf>,
    /// `--verbose` (as `config_dir`).
    pub verbose: bool,
    /// `switch_ignored` warnings for recognised CLI-003 switches that M1 does not implement (they are
    /// forwarded in `OpenOptions::extra`).
    pub warnings: Vec<Warning>,
    /// `--stdin-name <name>`: the name hint of a `-` spec (D-15, D-49); the adapter passes it to
    /// `StdinSource`. Additive.
    pub stdin_name: Option<String>,
    /// `--help` (`-h`, `/?` on Windows) was given ([`open_usage`]). Additive.
    pub help: bool,
}

/// How a switch takes a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arity {
    /// No value.
    Flag,
    /// A value (`--name=value` or `--name value`).
    Value,
    /// A value only with `=` (`--qc`, `--qc=crc`).
    Optional,
}

/// What a switch does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Wait,
    NoLaunch,
    Fv,
    Title(usize),
    ReadOnlyAll,
    ReadOnlyLeft,
    ReadOnlyRight,
    Filters,
    ExpandAll,
    Format,
    IgnoreUnimportant,
    StdinName,
    Solo,
    Json,
    Quiet,
    NoColor,
    ConfigDir,
    Log,
    Verbose,
    Help,
    /// Recognised but not implemented in M1: forwarded with `switch_ignored`.
    Forward,
}

/// One switch: canonical name, aliases, arity, action and its usage line.
struct Switch {
    name: &'static str,
    aliases: &'static [&'static str],
    arity: Arity,
    action: Action,
    help: &'static str,
}

const fn sw(
    name: &'static str,
    aliases: &'static [&'static str],
    arity: Arity,
    action: Action,
    help: &'static str,
) -> Switch {
    Switch {
        name,
        aliases,
        arity,
        action,
        help,
    }
}

/// The switch table of `gui open` (§1.9.2 row, CLI-003).
const SWITCHES: &[Switch] = &[
    sw(
        "wait",
        &[],
        Arity::Flag,
        Action::Wait,
        "wait until the view closes; exit with its code",
    ),
    sw(
        "no-launch",
        &["nolaunch"],
        Arity::Flag,
        Action::NoLaunch,
        "do not start the GUI",
    ),
    sw(
        "fv",
        &["fileviewer"],
        Arity::Value,
        Action::Fv,
        "view type (\"Text Compare\", folder-compare, …)",
    ),
    sw(
        "title1",
        &["lefttitle"],
        Arity::Value,
        Action::Title(0),
        "left pane title",
    ),
    sw(
        "title2",
        &["righttitle"],
        Arity::Value,
        Action::Title(1),
        "right pane title",
    ),
    sw(
        "title3",
        &["centertitle"],
        Arity::Value,
        Action::Title(2),
        "center pane title",
    ),
    sw(
        "title4",
        &["outputtitle"],
        Arity::Value,
        Action::Title(3),
        "output pane title",
    ),
    sw(
        "ro",
        &["readonly"],
        Arity::Flag,
        Action::ReadOnlyAll,
        "every pane read-only",
    ),
    sw(
        "ro1",
        &["lro", "leftreadonly"],
        Arity::Flag,
        Action::ReadOnlyLeft,
        "left pane read-only",
    ),
    sw(
        "ro2",
        &["rro", "rightreadonly"],
        Arity::Flag,
        Action::ReadOnlyRight,
        "right pane read-only",
    ),
    sw(
        "filters",
        &[],
        Arity::Value,
        Action::Filters,
        "name filter of a folder comparison",
    ),
    sw(
        "expandall",
        &["expand-all"],
        Arity::Flag,
        Action::ExpandAll,
        "expand all subfolders",
    ),
    sw(
        "format",
        &[],
        Arity::Value,
        Action::Format,
        "force a file format (id or name)",
    ),
    sw(
        "iu",
        &["ignoreunimportant", "ignore-unimportant"],
        Arity::Flag,
        Action::IgnoreUnimportant,
        "ignore unimportant differences",
    ),
    sw(
        "stdin-name",
        &[],
        Arity::Value,
        Action::StdinName,
        "file name hint of `-` (standard input)",
    ),
    sw(
        "solo",
        &[],
        Arity::Flag,
        Action::Solo,
        "start a new GUI instance (hikasa-gui)",
    ),
    sw(
        "json",
        &[],
        Arity::Flag,
        Action::Json,
        "JSON output (hikasa)",
    ),
    sw(
        "quiet",
        &["q"],
        Arity::Flag,
        Action::Quiet,
        "no progress and no non-error messages",
    ),
    sw(
        "no-color",
        &[],
        Arity::Flag,
        Action::NoColor,
        "no ANSI colours",
    ),
    sw(
        "config-dir",
        &[],
        Arity::Value,
        Action::ConfigDir,
        "settings folder",
    ),
    sw(
        "log",
        &[],
        Arity::Value,
        Action::Log,
        "write diagnostic logs to a file",
    ),
    sw(
        "verbose",
        &[],
        Arity::Flag,
        Action::Verbose,
        "debug-level logs",
    ),
    sw(
        "help",
        &["h", "?"],
        Arity::Flag,
        Action::Help,
        "show this help",
    ),
    sw("automerge", &[], Arity::Flag, Action::Forward, ""),
    sw("center", &[], Arity::Value, Action::Forward, ""),
    sw("mergeoutput", &[], Arity::Value, Action::Forward, ""),
    sw("force", &[], Arity::Flag, Action::Forward, ""),
    sw("reviewconflicts", &[], Arity::Flag, Action::Forward, ""),
    sw("favorleft", &[], Arity::Flag, Action::Forward, ""),
    sw("favorright", &[], Arity::Flag, Action::Forward, ""),
    sw("closescript", &[], Arity::Flag, Action::Forward, ""),
    sw("silent", &[], Arity::Flag, Action::Forward, ""),
    sw("edit", &[], Arity::Flag, Action::Forward, ""),
    sw("sync", &[], Arity::Flag, Action::Forward, ""),
    sw("nobackups", &[], Arity::Flag, Action::Forward, ""),
    sw(
        "qc",
        &["quickcompare"],
        Arity::Optional,
        Action::Forward,
        "",
    ),
    sw("savetarget", &[], Arity::Value, Action::Forward, ""),
    sw("vcs1", &["vcsleft"], Arity::Value, Action::Forward, ""),
    sw("vcs2", &["vcsright"], Arity::Value, Action::Forward, ""),
    sw("vcs3", &["vcscenter"], Arity::Value, Action::Forward, ""),
    sw("vcs4", &["vcsoutput"], Arity::Value, Action::Forward, ""),
];

/// The switch named `name` (canonical or alias, ASCII-case-insensitive).
fn lookup(name: &str) -> Option<&'static Switch> {
    let lower = name.to_ascii_lowercase();
    SWITCHES
        .iter()
        .find(|s| s.name == lower || s.aliases.contains(&lower.as_str()))
}

/// The switch spelling of an argument: (body after the prefix, prefix was `/`), for `--name[=v]`, `-name[=v]`,
/// and `/name[=v]` on Windows only. `-` alone, `--` and plain words are not switches. Exact for non-UTF-8
/// arguments (only the name must be UTF-8).
fn switch_body(arg: &OsStr, platform: Platform) -> Option<(OsString, bool)> {
    let (head, rest) = split_prefix(arg);
    let body = match head {
        "--" | "-" => rest,
        "/" if platform == Platform::Windows => rest,
        _ => return None,
    };
    if body.is_empty() {
        return None;
    }
    Some((body, head == "/"))
}

/// Splits the switch prefix (`--`, `-`, `/` or nothing) from an argument, exactly.
fn split_prefix(arg: &OsStr) -> (&'static str, OsString) {
    let text = arg.to_string_lossy();
    let prefix = if text.starts_with("--") {
        "--"
    } else if text.starts_with('-') {
        "-"
    } else if text.starts_with('/') {
        "/"
    } else {
        ""
    };
    (prefix, os_skip(arg, prefix.len()))
}

/// `s` without its first `n` bytes (`n` covers an ASCII prefix), exact for non-UTF-8 strings.
fn os_skip(s: &OsStr, n: usize) -> OsString {
    if n == 0 {
        return s.to_os_string();
    }
    let parts = split_os(s, '\u{0}');
    // `split_os` keeps the string whole (a NUL never occurs in an argument); drop the prefix from the first part.
    let mut out = OsString::new();
    for (k, part) in parts.iter().enumerate() {
        if k == 0 {
            let lossy = part.to_string_lossy();
            match part.to_str() {
                Some(text) => out.push(text.get(n..).unwrap_or("")),
                None => out.push(os_tail(part, n, &lossy)),
            }
        } else {
            out.push("\u{0}");
            out.push(part);
        }
    }
    out
}

/// The tail of a non-UTF-8 OS string after an ASCII prefix of `n` bytes.
#[cfg(unix)]
fn os_tail(s: &OsStr, n: usize, _lossy: &str) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    OsStr::from_bytes(s.as_bytes().get(n..).unwrap_or_default()).to_os_string()
}

/// The tail of a non-UTF-8 OS string after an ASCII prefix of `n` code units.
#[cfg(windows)]
fn os_tail(s: &OsStr, n: usize, _lossy: &str) -> OsString {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units: Vec<u16> = s.encode_wide().skip(n).collect();
    OsString::from_wide(&units)
}

/// The tail of an OS string (lossy on other targets).
#[cfg(not(any(unix, windows)))]
fn os_tail(_s: &OsStr, n: usize, lossy: &str) -> OsString {
    OsString::from(lossy.get(n..).unwrap_or(""))
}

/// A known switch in an argument: (switch, inline value).
fn known_switch(arg: &OsStr, platform: Platform) -> Option<(&'static Switch, Option<OsString>)> {
    let (body, _) = switch_body(arg, platform)?;
    let (name, value) = split_eq(&body);
    let switch = lookup(name.to_str()?)?;
    Some((switch, value))
}

/// `--name` or `--name=value` (exact value).
fn canonical_spelling(switch: &Switch, value: Option<&OsStr>) -> OsString {
    let mut out = OsString::from(format!("--{}", switch.name));
    if let Some(value) = value {
        out.push("=");
        out.push(value);
    }
    out
}

/// Rewrites CLI-001 spellings to `--name[=value]`: `-name[=v]` for every known switch, `/name[=v]` on Windows
/// only (on Unix a leading `/` is a path), and aliases (`--readonly` → `--ro`, `--lefttitle` → `--title1`,
/// `--fileviewer` → `--fv`, `--ignoreunimportant` → `--iu`, `--quickcompare` → `--qc`, …). Pure, table-driven.
///
/// Switch names match ASCII-case-insensitively. Inline values are kept exactly, also when they are not valid
/// UTF-8 (`--config-dir=<bytes>`). The separate value of a value switch (`--title1 x`) and every argument after
/// `--` are kept verbatim; unknown switches are left unchanged (`parse_open_args` rejects them), as are `/…`
/// arguments on Unix.
pub fn normalize_legacy_args(args: &[OsString], platform: Platform) -> Vec<OsString> {
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            out.push(arg.clone());
            out.extend(iter.by_ref().cloned());
            break;
        }
        match known_switch(arg, platform) {
            Some((switch, value)) => {
                out.push(canonical_spelling(switch, value.as_deref()));
                if switch.arity == Arity::Value
                    && value.is_none()
                    && let Some(next) = iter.next()
                {
                    out.push(next.clone());
                }
            }
            None => out.push(arg.clone()),
        }
    }
    out
}

/// True for a Windows `/name[=v]` argument whose name looks like a switch (ASCII letters, digits, `-`, `?`):
/// an unknown one is an error rather than a root-relative path (`/readonlyy`); `/dir/file`, `/a.txt` stay paths.
fn looks_like_windows_switch(arg: &OsStr) -> bool {
    let Some(text) = arg.to_str() else {
        return false;
    };
    let Some(body) = text.strip_prefix('/') else {
        return false;
    };
    let name = body.split_once('=').map_or(body, |(name, _)| name);
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '?'))
}

/// Parses normalised arguments: positional specs (CLI-002) and the switches of §1.9.2's `gui open` row. A value
/// is given as `--name=value` or `--name value`. Unknown switch names are `InvalidArgument` (exit 100).
///
/// The arguments are normalised first ([`normalize_legacy_args`] is idempotent), so legacy spellings are
/// accepted here too. `-` is a spec (stdin); everything after `--` is a spec. A flag given a value
/// (`--ro=1`), a value switch without a value, an unknown `--fv` view type and a non-UTF-8 value of a text
/// switch are `InvalidArgument`; `--config-dir` and `--log` take any path, also one that is not valid UTF-8, in
/// both spellings. On Windows an unknown `/name` whose name looks like a switch is `InvalidArgument` too.
/// Recognised CLI-003 switches that M1 does not implement are forwarded in `OpenOptions::extra` (name → value,
/// `""` for flags) with one `switch_ignored` warning each.
pub fn parse_open_args(args: &[OsString], platform: Platform) -> Result<OpenArgs, ApiError> {
    let args = normalize_legacy_args(args, platform);
    let mut parsed = OpenArgs::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            parsed.request.specs.extend(iter.by_ref());
            break;
        }
        let (prefix, _) = split_prefix(&arg);
        if arg == "-" || prefix != "-" && prefix != "--" {
            if platform == Platform::Windows && looks_like_windows_switch(&arg) {
                return Err(ApiError::InvalidArgument(format!(
                    "unknown switch `{}`",
                    arg.to_string_lossy()
                )));
            }
            parsed.request.specs.push(arg);
            continue;
        }
        let Some((switch, inline)) = known_switch(&arg, Platform::Linux) else {
            return Err(ApiError::InvalidArgument(format!(
                "unknown switch `{}`",
                arg.to_string_lossy()
            )));
        };
        let value: Option<OsString> = match (switch.arity, inline) {
            (Arity::Flag, Some(_)) => {
                return Err(ApiError::InvalidArgument(format!(
                    "switch `--{}` takes no value",
                    switch.name
                )));
            }
            (Arity::Flag | Arity::Optional, None) => None,
            (Arity::Value | Arity::Optional, Some(v)) => Some(v),
            (Arity::Value, None) => Some(iter.next().ok_or_else(|| {
                ApiError::InvalidArgument(format!("switch `--{}` needs a value", switch.name))
            })?),
        };
        apply(&mut parsed, switch, value)?;
    }
    Ok(parsed)
}

/// Applies one parsed switch.
fn apply(parsed: &mut OpenArgs, switch: &Switch, value: Option<OsString>) -> Result<(), ApiError> {
    let text = |value: Option<OsString>| -> Result<String, ApiError> {
        value.unwrap_or_default().into_string().map_err(|_| {
            ApiError::InvalidArgument(format!(
                "the value of `--{}` is not valid UTF-8",
                switch.name
            ))
        })
    };
    let options = &mut parsed.request.options;
    match switch.action {
        Action::Wait => parsed.request.wait = true,
        Action::NoLaunch => parsed.no_launch = true,
        Action::Fv => {
            let word = text(value)?;
            let kind = ViewKind::parse(&word).ok_or_else(|| {
                ApiError::InvalidArgument(format!("unknown view type {word:?} for `--fv`"))
            })?;
            parsed.request.kind = Some(kind);
        }
        Action::Title(k) => {
            let title = text(value)?;
            if let Some(slot) = options.titles.get_mut(k) {
                *slot = Some(title);
            }
        }
        Action::ReadOnlyAll => options.read_only.all = true,
        Action::ReadOnlyLeft => options.read_only.left = true,
        Action::ReadOnlyRight => options.read_only.right = true,
        Action::Filters => options.filters = Some(text(value)?),
        Action::ExpandAll => options.expand_all = true,
        Action::Format => options.format = Some(text(value)?),
        Action::IgnoreUnimportant => options.ignore_unimportant = true,
        Action::StdinName => parsed.stdin_name = Some(text(value)?),
        Action::Solo => parsed.solo = true,
        Action::Json => parsed.json = true,
        Action::Quiet => parsed.quiet = true,
        Action::NoColor => parsed.no_color = true,
        Action::ConfigDir => parsed.config_dir = value.map(PathBuf::from),
        Action::Log => parsed.log = value.map(PathBuf::from),
        Action::Verbose => parsed.verbose = true,
        Action::Help => parsed.help = true,
        Action::Forward => {
            let value = text(value)?;
            options.extra.insert(switch.name.to_owned(), value);
            parsed.warnings.push(Warning {
                code: WarningCode::SwitchIgnored,
                message: format!(
                    "switch `--{}` is not supported in this version; it is forwarded to the GUI",
                    switch.name
                ),
                spec: None,
            });
        }
    }
    Ok(())
}

/// The usage text of `hikasa gui open` and `hikasa-gui`, generated from the switch table (one parser, one help
/// text, D-51). Additive (wave-4 review).
pub fn open_usage() -> String {
    let mut out = String::from(
        "Usage: hikasa gui open [switches] <left> <right>\n       hikasa-gui [switches] [<left> <right>]\n\n\
         Two folders open a Folder Compare view, two files a Text Compare view; `-` is standard input.\n\
         Switches may also be written -name, -name=value and, on Windows, /name.\n\n",
    );
    for switch in SWITCHES.iter().filter(|s| s.action != Action::Forward) {
        let mut spelling = format!("--{}", switch.name);
        if switch.arity == Arity::Value {
            spelling.push_str(" <value>");
        }
        out.push_str(&format!("  {spelling:<24} {}", switch.help));
        if !switch.aliases.is_empty() {
            let aliases: Vec<String> = switch.aliases.iter().map(|a| format!("--{a}")).collect();
            out.push_str(&format!(" (also {})", aliases.join(", ")));
        }
        out.push('\n');
    }
    let forwarded: Vec<String> = SWITCHES
        .iter()
        .filter(|s| s.action == Action::Forward)
        .map(|s| format!("--{}", s.name))
        .collect();
    out.push_str(&format!(
        "\nRecognised but not implemented in this version (forwarded with a warning): {}\n",
        forwarded.join(", ")
    ));
    out
}

/// CLI-002 positional rules for M1: two folders → FolderCompare; two files → by the effective format kind
/// (Text/Table → TextCompare; other kinds → TextCompare with warning until their views exist); one spec or more
/// than two specs → `InvalidArgument` (M2 adds sessions, workspaces, merge, patch, snapshot).
///
/// A forced kind is returned as it is (the GUI answers `unsupported_view` for kinds it does not serve). The
/// kinds come from `stat` (links followed); a spec that cannot be stat'ed takes the kind of the other one (the
/// GUI shows the load error); when neither can be stat'ed, the left one's error is returned. A folder paired
/// with a file is `InvalidArgument`. The "other kinds" warning is logged (tracing).
pub fn infer_view_kind(
    service: &Service,
    specs: &[Spec],
    forced: Option<ViewKind>,
) -> Result<ViewKind, ApiError> {
    if let Some(kind) = forced {
        return Ok(kind);
    }
    let [left, right] = specs else {
        return Err(ApiError::InvalidArgument(format!(
            "gui open needs two specs to infer the view (got {}); sessions, workspaces, merges, patches and \
             snapshots come in a later version",
            specs.len()
        )));
    };
    let stat = |spec: &Spec| service.vfs().stat(spec, FollowLinks::Yes);
    let kinds = match (stat(left), stat(right)) {
        (Ok(l), Ok(r)) => (l.kind, r.kind),
        (Ok(l), Err(_)) => (l.kind, l.kind),
        (Err(_), Ok(r)) => (r.kind, r.kind),
        (Err(source), Err(_)) => {
            return Err(ApiError::LoadFailed {
                side: hikasa_core::Side::Left,
                spec: left.canonical(),
                source,
            });
        }
    };
    match kinds {
        (EntryKind::Dir, EntryKind::Dir) => Ok(ViewKind::FolderCompare),
        (EntryKind::Dir, _) | (_, EntryKind::Dir) => Err(ApiError::InvalidArgument(
            "cannot compare a folder with a file".into(),
        )),
        _ => {
            let name = service
                .detection_name(left)
                .or_else(|| service.detection_name(right));
            let kind = match name {
                Some(name) => service
                    .registry()
                    .detect(&name, &EnabledOverrides::default())?
                    .format
                    .kind(),
                None => FormatKind::Text,
            };
            if !matches!(kind, FormatKind::Text | FormatKind::Table) {
                tracing::warn!(
                    target: "hikasa::gui",
                    "the {} view does not exist yet; the files open in Text Compare",
                    kind.as_str()
                );
            }
            Ok(ViewKind::TextCompare)
        }
    }
}

impl Service {
    /// The launch arguments and environment for `hikasa-gui` (§1.8.5 amendment): `--config-dir <dir>` when the
    /// configuration is explicit or portable (the GUI then computes the same endpoint, D-47), and
    /// `HIKASA_IPC_ENDPOINT` when the endpoint is overridden. Additive.
    pub fn gui_launch(&self) -> GuiLaunch {
        let mut launch = GuiLaunch::default();
        match self.config().mode {
            crate::ConfigMode::Explicit | crate::ConfigMode::Portable => {
                launch.args.push(OsString::from("--config-dir"));
                launch.args.push(self.config().dir.clone().into_os_string());
            }
            crate::ConfigMode::User => {}
        }
        if let Some(endpoint) = &self.env().ipc_endpoint_override {
            launch
                .env
                .push((OsString::from("HIKASA_IPC_ENDPOINT"), endpoint.clone()));
        }
        launch
    }

    /// `gui open` and `--gui` (§3.6). `launcher = None` means `--no-launch`.
    pub fn gui_open(
        &self,
        req: &GuiOpenRequest,
        launcher: Option<&dyn GuiLauncher>,
    ) -> Result<GuiOpenOutcome, ApiError> {
        self.gui_open_with(req, launcher, &RetryPolicy::default())
            .map(|report| report.outcome)
    }

    /// [`Service::gui_open`] with an explicit retry policy, returning what was sent and the spool file
    /// (additive; `hikasa.gui_open.v1` needs `spooled_stdin`).
    ///
    /// Steps (§3.6): [`Service::resolve_open`] (specs → canonical absolute strings; `-` → the stdin spool file;
    /// `clipboard://` → `InvalidArgument`, D-49; rules files absolute; kind ← `req.kind` or
    /// [`infer_view_kind`]); [`Service::check_open`] (vocabulary, formats, encodings and rules are validated
    /// locally, so a typo exits 100 before the GUI is contacted); connect (or launch with
    /// [`Service::gui_launch`] and retry, [`connect_or_launch`]); `OpenView`; with `wait`, block until
    /// `ViewClosed` and delete the spool file. The spool file is also deleted when the view could not be opened.
    pub fn gui_open_with(
        &self,
        req: &GuiOpenRequest,
        launcher: Option<&dyn GuiLauncher>,
        policy: &RetryPolicy,
    ) -> Result<GuiOpenReport, ApiError> {
        run_guarded(|| {
            let mut spool: Option<PathBuf> = None;
            let result = self.gui_open_steps(req, launcher, policy, &mut spool);
            let keep_spool = matches!(
                &result,
                Ok(GuiOpenReport {
                    outcome: GuiOpenOutcome::Opened { .. },
                    ..
                })
            );
            if !keep_spool && let Some(file) = &spool {
                remove_spool(file);
            }
            result
        })
    }

    /// Second-instance forwarding (§1.10 amendment, ARCH-090): `hikasa-gui <args>` that finds a running GUI
    /// (`IpcServer::bind` → `AlreadyRunning`) sends its request through this: the specs are resolved against the
    /// second process's cwd (absolute, §4.4.3 rule 4), `-` is spooled, rules files are made absolute, nothing is
    /// launched and nothing is awaited. Exit 0 on success, else `err.exit_code(ApiOp::GuiOpen { wait: false })`.
    /// Additive.
    pub fn forward_open(&self, req: &GuiOpenRequest) -> Result<GuiOpenReport, ApiError> {
        let req = GuiOpenRequest {
            wait: false,
            ..req.clone()
        };
        self.gui_open_with(&req, None, &RetryPolicy::default())
    }

    /// Resolves a `gui open` request for sending, or for the GUI's own command line (additive): specs →
    /// canonical absolute strings with the captured cwd (`-` → the stdin spool file, D-49; `clipboard://` and
    /// paths that are not valid UTF-8 → `InvalidArgument`); the rules files ([`GuiOpenRequest::text_rules_file`]
    /// and `folder_rules_file`, else the strings already in the options) made absolute (`-`, `clipboard://` and
    /// non-UTF-8 paths → `InvalidArgument`); kind ← `req.kind` or [`infer_view_kind`]. On error nothing is left
    /// behind (a spool file is removed).
    pub fn resolve_open(&self, req: &GuiOpenRequest) -> Result<ResolvedOpen, ApiError> {
        run_guarded(|| {
            let mut spool = None;
            let result = self.resolve_open_into(req, &mut spool);
            if result.is_err()
                && let Some(file) = &spool
            {
                remove_spool(file);
            }
            result
        })
    }

    fn resolve_open_into(
        &self,
        req: &GuiOpenRequest,
        spool: &mut Option<PathBuf>,
    ) -> Result<ResolvedOpen, ApiError> {
        let mut specs = Vec::with_capacity(req.specs.len());
        for raw in &req.specs {
            let spec = match self.parse_spec(raw)? {
                Spec::Stdin => {
                    if spool.is_some() {
                        return Err(ApiError::InvalidArgument(
                            "standard input can be opened only once".into(),
                        ));
                    }
                    let file = self.spool_stdin()?;
                    *spool = Some(file.clone());
                    self.parse_spec(file.as_os_str())?
                }
                Spec::Clipboard => {
                    return Err(ApiError::InvalidArgument(
                        "clipboard:// cannot be opened in the GUI in this version (D-49)".into(),
                    ));
                }
                spec => spec,
            };
            if spec.is_lossy() {
                return Err(ApiError::InvalidArgument(format!(
                    "{}: paths that are not valid UTF-8 cannot be sent to the GUI",
                    spec.canonical()
                )));
            }
            specs.push(spec);
        }
        let kind = infer_view_kind(self, &specs, req.kind)?;
        let mut options = req.options.clone();
        options.text_rules_file = self.absolute_rules_file(
            req.text_rules_file.as_deref(),
            options.text_rules_file.take(),
        )?;
        options.folder_rules_file = self.absolute_rules_file(
            req.folder_rules_file.as_deref(),
            options.folder_rules_file.take(),
        )?;
        Ok(ResolvedOpen {
            kind,
            specs: specs.iter().map(Spec::canonical).collect(),
            options,
            spooled_stdin: spool.clone(),
        })
    }

    /// Validates a resolved request locally (additive; `gui_open` calls it before contacting the GUI): the
    /// `OpenView` → request mapping ([`Service::open_view_request`]: two absolute specs, the display words), the
    /// forced format, the encoding labels, the text rules (file + patch, validated) and, for folders, the rules
    /// file, `--criteria`, the patch and the text rules of `--content rules`. Kinds other than Text and Folder
    /// Compare are left to the GUI (`unsupported_view`).
    pub fn check_open(&self, resolved: &ResolvedOpen) -> Result<(), ApiError> {
        if !resolved.kind.is_served_in_m1() {
            return Ok(());
        }
        let mut warnings = Vec::new();
        match self.open_view_request(resolved.kind, &resolved.specs, &resolved.options)? {
            ViewRequest::Text { request, .. } => {
                self.text_rules(
                    request.rules_file.as_deref(),
                    &request.rules_patch,
                    &mut warnings,
                )?;
                if let Some(format) = request.format.as_deref() {
                    self.registry().force(format)?;
                }
                self.load_options(request.encodings.left.as_deref())?;
                self.load_options(request.encodings.right.as_deref())?;
            }
            ViewRequest::Folder { request, .. } => {
                self.folder_rules(&request, &mut warnings)?;
            }
        }
        Ok(())
    }

    /// Turns an `OpenView` into the service request of its view (additive; the GUI's inverse of
    /// [`GuiOpenRequest::for_text`] / [`GuiOpenRequest::for_folder`], D-50). The specs must be exactly two
    /// canonical absolute specs (§4.4.3 rule 4: relative specs, `-` and `clipboard://` are `InvalidArgument`;
    /// they are never resolved against the GUI's cwd). Text: format, encodings, rules file and patch,
    /// ignore-unimportant; the view options from `display`/`context_lines` (default: all). Folders: rules file,
    /// criteria, patch (the legacy `filters` switch, when given, replaces the patch's `name_filter`), text rules
    /// file and patch, ignore-unimportant; the display options from `display`/`folders`/`suppress_filters`
    /// (default: differences with the folder structure). Other kinds are `InvalidArgument`.
    pub fn open_view_request(
        &self,
        kind: ViewKind,
        specs: &[String],
        options: &OpenOptions,
    ) -> Result<ViewRequest, ApiError> {
        let [left, right] = specs else {
            return Err(ApiError::InvalidArgument(format!(
                "a {} view needs exactly two specs (got {})",
                kind.as_str(),
                specs.len()
            )));
        };
        let absolute = SpecContext::absolute_only(self.env().platform.path_style());
        for spec in [left, right] {
            match Spec::parse(spec, &absolute)? {
                Spec::Stdin | Spec::Clipboard => {
                    return Err(ApiError::InvalidArgument(format!(
                        "`{spec}` cannot be opened over IPC (D-49)"
                    )));
                }
                _ => {}
            }
        }
        let pair = Pair::new(OsString::from(left), OsString::from(right));
        match kind {
            ViewKind::TextCompare => {
                let mode = resolve_text_display(
                    options.display.as_deref(),
                    options.context_lines,
                    self.options().context_lines,
                    false,
                )?;
                Ok(ViewRequest::Text {
                    request: TextCompareRequest {
                        left: pair.left,
                        right: pair.right,
                        format: options.format.clone(),
                        encodings: Pair::new(
                            options.encodings[0].clone(),
                            options.encodings[1].clone(),
                        ),
                        rules_file: options.text_rules_file.clone().map(OsString::from),
                        rules_patch: options.text_rules.clone().unwrap_or_default(),
                        ignore_unimportant: options.ignore_unimportant,
                    },
                    view: ViewOptions {
                        filter: mode.text_filter(),
                        ignore_unimportant: options.ignore_unimportant,
                        expanded: Vec::new(),
                    },
                })
            }
            ViewKind::FolderCompare => {
                let mut patch = options.folder_rules.clone().unwrap_or_default();
                if let Some(filters) = &options.filters {
                    patch.name_filter = Some(filters.clone());
                }
                let display = resolve_folder_display(
                    options.display.as_deref(),
                    options.folders.as_deref(),
                    options.suppress_filters,
                    options.ignore_unimportant,
                )?;
                Ok(ViewRequest::Folder {
                    request: FolderCompareRequest {
                        left: pair.left,
                        right: pair.right,
                        rules_file: options.folder_rules_file.clone().map(OsString::from),
                        criteria: options.criteria.clone(),
                        patch,
                        text_rules_file: options.text_rules_file.clone().map(OsString::from),
                        text_patch: options.text_rules.clone().unwrap_or_default(),
                        ignore_unimportant: options.ignore_unimportant,
                    },
                    display,
                    expand_all: options.expand_all,
                })
            }
            other => Err(ApiError::InvalidArgument(format!(
                "view kind `{}` has no service request in this version",
                other.as_str()
            ))),
        }
    }

    fn gui_open_steps(
        &self,
        req: &GuiOpenRequest,
        launcher: Option<&dyn GuiLauncher>,
        policy: &RetryPolicy,
        spool: &mut Option<PathBuf>,
    ) -> Result<GuiOpenReport, ApiError> {
        let resolved = self.resolve_open_into(req, spool)?;
        self.check_open(&resolved)?;
        let endpoint = self.endpoint()?;
        let mut client = connect_or_launch(
            &endpoint,
            &self.generator(),
            launcher,
            &self.gui_launch(),
            policy,
            REQUEST_TIMEOUT,
        )?;
        let request = Request::OpenView {
            kind: resolved.kind,
            specs: resolved.specs.clone(),
            options: resolved.options,
            wait: req.wait,
        };
        let (id, response) = client.request(request)?;
        let view = match response {
            Response::ViewOpened { view } => view,
            other => return Err(ApiError::Ipc(crate::ipc::client::unexpected(&other))),
        };
        let outcome = if req.wait {
            let (view, result, exit_code) = client.wait_view_closed(id)?;
            if let Some(file) = spool.as_ref() {
                remove_spool(file);
            }
            GuiOpenOutcome::Closed {
                view,
                result,
                exit_code,
            }
        } else {
            GuiOpenOutcome::Opened { view }
        };
        Ok(GuiOpenReport {
            outcome,
            kind: resolved.kind,
            specs: resolved.specs,
            spooled_stdin: spool.clone(),
        })
    }

    /// `gui ping` (never launches, D-22).
    pub fn gui_ping(&self) -> Result<ServerInfo, ApiError> {
        run_guarded(|| {
            let mut client = self.gui_client()?;
            match client.request(Request::Ping)?.1 {
                Response::Pong => Ok(client.server().clone()),
                other => Err(ApiError::Ipc(crate::ipc::client::unexpected(&other))),
            }
        })
    }

    /// `gui ping` as the `hikasa.gui_ping.v1` result (with the endpoint text). Additive.
    pub fn gui_ping_dto(&self) -> Result<GuiPingDto, ApiError> {
        let info = self.gui_ping()?;
        Ok(GuiPingDto::new(self.endpoint()?.display(), &info))
    }

    /// `gui list`.
    pub fn gui_list(&self) -> Result<Vec<ViewInfo>, ApiError> {
        run_guarded(|| match self.gui_client()?.request(Request::ListViews)?.1 {
            Response::Views { views } => Ok(views),
            other => Err(ApiError::Ipc(crate::ipc::client::unexpected(&other))),
        })
    }

    /// `gui close`.
    pub fn gui_close(&self, view: ViewId) -> Result<(), ApiError> {
        run_guarded(
            || match self.gui_client()?.request(Request::CloseView { view })?.1 {
                Response::Ok => Ok(()),
                other => Err(ApiError::Ipc(crate::ipc::client::unexpected(&other))),
            },
        )
    }

    /// `gui quit`.
    pub fn gui_quit(&self) -> Result<(), ApiError> {
        run_guarded(|| match self.gui_client()?.request(Request::Shutdown)?.1 {
            Response::Ok => Ok(()),
            other => Err(ApiError::Ipc(crate::ipc::client::unexpected(&other))),
        })
    }

    /// A connected client without launching (D-22): no GUI → `Ipc(NotRunning)` (exit 100 under `GuiControl`).
    fn gui_client(&self) -> Result<IpcClient, ApiError> {
        let endpoint = self.endpoint()?;
        Ok(IpcClient::connect(
            &endpoint,
            &self.generator(),
            REQUEST_TIMEOUT,
        )?)
    }

    /// Writes the stdin content to `<cache>/stdin/<n>/<name>` and returns the file (D-49, §1.8.7).
    ///
    /// Unix: the cache folder must belong to this user and not be writable by others (the temp fallback must
    /// also be a real folder, not a symbolic link); `stdin/` is a real 0700 folder of this user; `<n>` is created
    /// fresh with mode 0700 (`mkdir` never follows a link; the first free number wins, so concurrent invocations
    /// never share a file); the file is created with `create_new` and mode 0600. A failure removes everything
    /// this call created.
    fn spool_stdin(&self) -> Result<PathBuf, ApiError> {
        let blob = self.stdin().ok_or_else(|| {
            ApiError::InvalidArgument("`-` needs standard input, which was not provided".into())
        })?;
        let name = blob
            .name
            .as_deref()
            .and_then(|hint| Path::new(hint).file_name())
            .and_then(OsStr::to_str)
            .filter(|n| !n.is_empty() && *n != "." && *n != "..")
            .unwrap_or("stdin.txt")
            .to_owned();
        let io_error = |path: &Path, e| ApiError::Session(crate::SessionError::io(path, e));
        let cache = &self.config().cache_dir;
        let fallback = cache.starts_with(&self.env().temp_dir);
        private::ensure_dir(cache, true, false).map_err(|e| io_error(cache, e))?;
        private::verify_dir(cache, self.env().uid, fallback, false)
            .map_err(|e| io_error(cache, e))?;
        let root = cache.join("stdin");
        private::ensure_dir(&root, false, true).map_err(|e| io_error(&root, e))?;
        private::verify_dir(&root, self.env().uid, true, true).map_err(|e| io_error(&root, e))?;
        let mut n: u32 = 1;
        let dir = loop {
            let candidate = root.join(n.to_string());
            match private::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100_000 => n += 1,
                Err(e) => return Err(io_error(&candidate, e)),
            }
        };
        let file = dir.join(name);
        if let Err(e) = private::write_new(&file, &blob.bytes) {
            remove_spool(&file);
            return Err(io_error(&file, e));
        }
        Ok(file)
    }

    /// A rules file for the GUI as an absolute canonical spec (the GUI has another cwd, D-50): `raw` (as typed)
    /// wins over `text` (already in the options).
    fn absolute_rules_file(
        &self,
        raw: Option<&OsStr>,
        text: Option<String>,
    ) -> Result<Option<String>, ApiError> {
        let input: OsString = match (raw, text) {
            (Some(raw), _) => raw.to_os_string(),
            (None, Some(text)) => OsString::from(text),
            (None, None) => return Ok(None),
        };
        let spec = match Spec::parse_os(&input, &self.spec_context())? {
            Spec::Stdin | Spec::Clipboard => {
                return Err(ApiError::InvalidArgument(format!(
                    "rules file `{}` cannot be sent to the GUI (D-49)",
                    input.to_string_lossy()
                )));
            }
            spec => spec,
        };
        if spec.is_lossy() {
            return Err(ApiError::InvalidArgument(format!(
                "{}: paths that are not valid UTF-8 cannot be sent to the GUI",
                spec.canonical()
            )));
        }
        Ok(Some(spec.canonical()))
    }
}

/// Deletes a spooled stdin file and its numbered folder (best effort).
fn remove_spool(file: &Path) {
    let _ = std::fs::remove_file(file);
    if let Some(dir) = file.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Private folders and files of the stdin spool (decision 5 of the wave-4 review).
mod private {
    use std::io::{self, Write};
    use std::path::Path;

    /// Creates `dir` when it is missing (mode 0700 for every created component on Unix); `parents` creates
    /// missing parents too. An existing entry is left to [`verify_dir`].
    pub(super) fn ensure_dir(dir: &Path, parents: bool, _strict: bool) -> io::Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(parents);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Creates a fresh folder (create-new; mode 0700 on Unix).
    pub(super) fn create_dir(dir: &Path) -> io::Result<()> {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)
    }

    /// Unix: `dir` must belong to `uid` and not be writable by others; with `no_link` it must be a real folder
    /// (not a symbolic link); with `private` also `mode & 0o077 == 0`. An unknown uid fails closed.
    #[cfg(unix)]
    pub(super) fn verify_dir(
        dir: &Path,
        uid: Option<u32>,
        no_link: bool,
        private: bool,
    ) -> io::Result<()> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let denied = |what: &str| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("refusing to spool standard input: {what}"),
            )
        };
        let uid = uid.ok_or_else(|| denied("the current user id is unknown"))?;
        let link = std::fs::symlink_metadata(dir)?;
        let meta = if link.file_type().is_symlink() {
            if no_link {
                return Err(denied("the folder is a symbolic link"));
            }
            std::fs::metadata(dir)?
        } else {
            link
        };
        if !meta.is_dir() {
            return Err(denied("not a folder"));
        }
        if meta.uid() != uid {
            return Err(denied("the folder belongs to another user"));
        }
        let mode = meta.permissions().mode();
        if mode & 0o022 != 0 {
            return Err(denied("the folder is writable by other users"));
        }
        if private && mode & 0o077 != 0 {
            return Err(denied("the folder is accessible by other users"));
        }
        Ok(())
    }

    /// Windows: per-user profile folders; nothing to verify with std.
    #[cfg(not(unix))]
    pub(super) fn verify_dir(
        _dir: &Path,
        _uid: Option<u32>,
        _no_link: bool,
        _private: bool,
    ) -> io::Result<()> {
        Ok(())
    }

    /// Writes a new file (create-new, mode 0600 on Unix; never follows a link) and syncs it.
    pub(super) fn write_new(file: &Path, bytes: &[u8]) -> io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut handle = options.open(file)?;
        handle.write_all(bytes)?;
        handle.sync_all()
    }
}
