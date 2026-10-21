//! GUI client helpers and the shared `gui open` parser (`hikasa_session::api::gui`; §1.8.5, §3.6; CLI-001…003,
//! D-22, D-23, D-49, D-50, D-51).
//!
//! `hikasa gui open` and `hikasa-gui` accept the same arguments (D-51): [`normalize_legacy_args`] rewrites the
//! legacy spellings (`-name`, `/name` on Windows, aliases) to `--name[=value]`, and [`parse_open_args`] turns the
//! result into [`OpenArgs`]. [`Service::gui_open`] resolves the specs (absolute canonical strings; `-` spooled
//! to the cache folder; `clipboard://` rejected), infers the view kind, connects to (or launches) the GUI and
//! sends `OpenView`.
//!
//! The switch table (canonical name ← aliases; `=v` takes a value, `[=v]` an optional one):
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

use hikasa_core::{Spec, Warning, WarningCode};
use hikasa_formats::{EnabledOverrides, FormatKind};
use hikasa_vfs::{EntryKind, FollowLinks, Vfs};

use super::{ApiError, Service};
use crate::atomic::write_atomic;
use crate::host::Platform;
use crate::ipc::{
    IpcClient, OpenOptions, REQUEST_TIMEOUT, Request, Response, RetryPolicy, ServerInfo, ViewId,
    ViewInfo, ViewKind, ViewResult, connect_or_launch,
};
use crate::jobs::run_guarded;

/// Starts the GUI process and sleeps between connection attempts (implemented by hikasa-cli with
/// `std::process::Command`, detached, stdio null, and `std::thread::sleep`; tests record the calls).
pub trait GuiLauncher {
    /// Starts `hikasa-gui` without arguments (requests then arrive over IPC).
    fn launch(&self) -> Result<(), LaunchError>;
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
    /// Spawning failed.
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

/// Candidate GUI executables, in order: sibling of the CLI executable, then each `PATH` entry (pure).
///
/// The name is `hikasa-gui` (`hikasa-gui.exe` on Windows); `PATH` is split at `:` (`;` on Windows, where
/// surrounding quotes are removed); empty entries are skipped and duplicates dropped.
pub fn gui_exe_candidates(
    exe_dir: Option<&Path>,
    path_var: Option<&OsStr>,
    platform: Platform,
) -> Vec<PathBuf> {
    let exe = if platform == Platform::Windows {
        "hikasa-gui.exe"
    } else {
        "hikasa-gui"
    };
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

/// Removes one pair of surrounding double quotes (Windows `PATH` entries may be quoted).
fn strip_quotes(part: OsString) -> OsString {
    match part.to_str() {
        Some(text) if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') => {
            OsString::from(&text[1..text.len() - 1])
        }
        _ => part,
    }
}

/// `gui open` request (also built by `--gui`, D-50).
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
    /// `--config-dir` (meaningful for `hikasa-gui`; the CLI rejects it after `open` because its service already
    /// exists).
    pub config_dir: Option<PathBuf>,
    /// `--log` (as `config_dir`).
    pub log: Option<PathBuf>,
    /// `--verbose` (as `config_dir`).
    pub verbose: bool,
    /// `switch_ignored` warnings for recognised CLI-003 switches that M1 does not implement (they are
    /// forwarded in `OpenOptions::extra`).
    pub warnings: Vec<Warning>,
    /// `--stdin-name <name>`: the name hint of a `-` spec (D-15, D-49); the adapter puts it into the stdin
    /// `Blob`. Additive.
    pub stdin_name: Option<String>,
    /// `--help` (`-h`, `/?` on Windows) was given. Additive.
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

/// One switch: canonical name, aliases, arity and action.
struct Switch {
    name: &'static str,
    aliases: &'static [&'static str],
    arity: Arity,
    action: Action,
}

const fn sw(
    name: &'static str,
    aliases: &'static [&'static str],
    arity: Arity,
    action: Action,
) -> Switch {
    Switch {
        name,
        aliases,
        arity,
        action,
    }
}

/// The switch table of `gui open` (§1.9.2 row, CLI-003).
const SWITCHES: &[Switch] = &[
    sw("wait", &[], Arity::Flag, Action::Wait),
    sw("no-launch", &["nolaunch"], Arity::Flag, Action::NoLaunch),
    sw("fv", &["fileviewer"], Arity::Value, Action::Fv),
    sw("title1", &["lefttitle"], Arity::Value, Action::Title(0)),
    sw("title2", &["righttitle"], Arity::Value, Action::Title(1)),
    sw("title3", &["centertitle"], Arity::Value, Action::Title(2)),
    sw("title4", &["outputtitle"], Arity::Value, Action::Title(3)),
    sw("ro", &["readonly"], Arity::Flag, Action::ReadOnlyAll),
    sw(
        "ro1",
        &["lro", "leftreadonly"],
        Arity::Flag,
        Action::ReadOnlyLeft,
    ),
    sw(
        "ro2",
        &["rro", "rightreadonly"],
        Arity::Flag,
        Action::ReadOnlyRight,
    ),
    sw("filters", &[], Arity::Value, Action::Filters),
    sw("expandall", &["expand-all"], Arity::Flag, Action::ExpandAll),
    sw("format", &[], Arity::Value, Action::Format),
    sw(
        "iu",
        &["ignoreunimportant", "ignore-unimportant"],
        Arity::Flag,
        Action::IgnoreUnimportant,
    ),
    sw("stdin-name", &[], Arity::Value, Action::StdinName),
    sw("solo", &[], Arity::Flag, Action::Solo),
    sw("json", &[], Arity::Flag, Action::Json),
    sw("quiet", &["q"], Arity::Flag, Action::Quiet),
    sw("no-color", &[], Arity::Flag, Action::NoColor),
    sw("config-dir", &[], Arity::Value, Action::ConfigDir),
    sw("log", &[], Arity::Value, Action::Log),
    sw("verbose", &[], Arity::Flag, Action::Verbose),
    sw("help", &["h", "?"], Arity::Flag, Action::Help),
    sw("automerge", &[], Arity::Flag, Action::Forward),
    sw("center", &[], Arity::Value, Action::Forward),
    sw("mergeoutput", &[], Arity::Value, Action::Forward),
    sw("force", &[], Arity::Flag, Action::Forward),
    sw("reviewconflicts", &[], Arity::Flag, Action::Forward),
    sw("favorleft", &[], Arity::Flag, Action::Forward),
    sw("favorright", &[], Arity::Flag, Action::Forward),
    sw("closescript", &[], Arity::Flag, Action::Forward),
    sw("silent", &[], Arity::Flag, Action::Forward),
    sw("edit", &[], Arity::Flag, Action::Forward),
    sw("sync", &[], Arity::Flag, Action::Forward),
    sw("nobackups", &[], Arity::Flag, Action::Forward),
    sw("qc", &["quickcompare"], Arity::Optional, Action::Forward),
    sw("savetarget", &[], Arity::Value, Action::Forward),
    sw("vcs1", &["vcsleft"], Arity::Value, Action::Forward),
    sw("vcs2", &["vcsright"], Arity::Value, Action::Forward),
    sw("vcs3", &["vcscenter"], Arity::Value, Action::Forward),
    sw("vcs4", &["vcsoutput"], Arity::Value, Action::Forward),
];

/// The switch named `name` (canonical or alias, ASCII-case-insensitive).
fn lookup(name: &str) -> Option<&'static Switch> {
    let lower = name.to_ascii_lowercase();
    SWITCHES
        .iter()
        .find(|s| s.name == lower || s.aliases.contains(&lower.as_str()))
}

/// Splits a switch spelling into (name, value): `--name[=v]`, `-name[=v]`, and `/name[=v]` on Windows only.
/// `-` alone, `--` and plain words are not switches.
fn split_switch(arg: &str, platform: Platform) -> Option<(&str, Option<&str>)> {
    let body = if let Some(rest) = arg.strip_prefix("--") {
        rest
    } else if let Some(rest) = arg.strip_prefix('-') {
        rest
    } else if platform == Platform::Windows {
        arg.strip_prefix('/')?
    } else {
        return None;
    };
    if body.is_empty() {
        return None;
    }
    Some(match body.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (body, None),
    })
}

/// Rewrites CLI-001 spellings to `--name[=value]`: `-name[=v]` for every known switch, `/name[=v]` on Windows
/// only (on Unix a leading `/` is a path), and aliases (`--readonly` → `--ro`, `--lefttitle` → `--title1`,
/// `--fileviewer` → `--fv`, `--ignoreunimportant` → `--iu`, `--quickcompare` → `--qc`, …). Pure, table-driven.
///
/// Switch names match ASCII-case-insensitively. The separate value of a value switch (`--title1 x`) and every
/// argument after `--` are kept verbatim; unknown switches are left unchanged (`parse_open_args` rejects
/// them), as are `/…` arguments on Unix and non-UTF-8 arguments (paths).
pub fn normalize_legacy_args(args: &[OsString], platform: Platform) -> Vec<OsString> {
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            out.push(arg.clone());
            continue;
        };
        if text == "--" {
            out.push(arg.clone());
            out.extend(iter.by_ref().cloned());
            break;
        }
        let known = split_switch(text, platform)
            .and_then(|(name, value)| lookup(name).map(|switch| (switch, value)));
        match known {
            Some((switch, value)) => {
                let canonical = match value {
                    Some(value) => format!("--{}={value}", switch.name),
                    None => format!("--{}", switch.name),
                };
                out.push(OsString::from(canonical));
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

/// Parses normalised arguments: positional specs (CLI-002) and the switches of §1.9.2's `gui open` row. A value
/// is given as `--name=value` or `--name value`. Unknown switch names are `InvalidArgument` (exit 100).
///
/// The arguments are normalised first ([`normalize_legacy_args`] is idempotent), so legacy spellings are
/// accepted here too. `-` is a spec (stdin); everything after `--` is a spec. A flag given a value
/// (`--ro=1`), a value switch without a value, an unknown `--fv` view type and a non-UTF-8 value of a text
/// switch are `InvalidArgument`. Recognised CLI-003 switches that M1 does not implement are forwarded in
/// `OpenOptions::extra` (name → value, `""` for flags) with one `switch_ignored` warning each.
pub fn parse_open_args(args: &[OsString], platform: Platform) -> Result<OpenArgs, ApiError> {
    let args = normalize_legacy_args(args, platform);
    let mut parsed = OpenArgs::default();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let Some(text) = arg.to_str() else {
            parsed.request.specs.push(arg);
            continue;
        };
        if text == "--" {
            parsed.request.specs.extend(iter.by_ref());
            break;
        }
        if text == "-" || !text.starts_with('-') {
            parsed.request.specs.push(arg);
            continue;
        }
        let Some((switch, inline)) = split_switch(text, Platform::Linux)
            .and_then(|(name, value)| lookup(name).map(|switch| (switch, value)))
        else {
            return Err(ApiError::InvalidArgument(format!(
                "unknown switch `{text}`"
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
            (Arity::Value | Arity::Optional, Some(v)) => Some(OsString::from(v)),
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
    /// Steps (§3.6): specs → canonical absolute strings (`parse_spec` with the captured cwd); `-` → the stdin
    /// blob spooled to `<cache>/stdin/<n>/<name>` (`--stdin-name`, else `stdin.txt`; `n` is the first free
    /// number, so concurrent invocations never share a file); `clipboard://` → `InvalidArgument` (D-49); the
    /// rules file paths of the options are made absolute; kind ← `req.kind` or [`infer_view_kind`]; connect
    /// (or launch and retry, [`connect_or_launch`]); `OpenView`; with `wait`, block until `ViewClosed` and
    /// delete the spool file. The spool file is also deleted when the view could not be opened.
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

    fn gui_open_steps(
        &self,
        req: &GuiOpenRequest,
        launcher: Option<&dyn GuiLauncher>,
        policy: &RetryPolicy,
        spool: &mut Option<PathBuf>,
    ) -> Result<GuiOpenReport, ApiError> {
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
                    let spec = self.parse_spec(file.as_os_str())?;
                    *spool = Some(file);
                    spec
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
        options.text_rules_file = self.absolute_option(options.text_rules_file.take())?;
        options.folder_rules_file = self.absolute_option(options.folder_rules_file.take())?;
        let spec_strings: Vec<String> = specs.iter().map(Spec::canonical).collect();
        let endpoint = self.endpoint()?;
        let mut client = connect_or_launch(
            &endpoint,
            &self.generator(),
            launcher,
            policy,
            REQUEST_TIMEOUT,
        )?;
        let request = Request::OpenView {
            kind,
            specs: spec_strings.clone(),
            options,
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
            kind,
            specs: spec_strings,
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

    /// Writes the stdin blob to `<cache>/stdin/<n>/<name>` and returns the file (D-49).
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
        let root = self.config().cache_dir.join("stdin");
        let io_error = |path: &Path, e| ApiError::Session(crate::SessionError::io(path, e));
        std::fs::create_dir_all(&root).map_err(|e| io_error(&root, e))?;
        let mut n: u32 = 1;
        let dir = loop {
            let candidate = root.join(n.to_string());
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 100_000 => n += 1,
                Err(e) => return Err(io_error(&candidate, e)),
            }
        };
        let file = dir.join(name);
        write_atomic(&file, &blob.bytes).map_err(|e| io_error(&file, e))?;
        Ok(file)
    }

    /// A rules file path made absolute against the captured cwd (the GUI has another cwd, D-50).
    fn absolute_option(&self, path: Option<String>) -> Result<Option<String>, ApiError> {
        match path {
            Some(path) => match self.parse_spec(OsStr::new(&path))? {
                spec @ Spec::Local(_) => Ok(Some(spec.canonical())),
                _ => Ok(Some(path)),
            },
            None => Ok(None),
        }
    }
}

/// Deletes a spooled stdin file and its numbered folder (best effort).
fn remove_spool(file: &Path) {
    let _ = std::fs::remove_file(file);
    if let Some(dir) = file.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}
