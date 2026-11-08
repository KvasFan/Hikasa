//! Host facts, clocks and the clipboard (T1): [`EnvFacts`], [`Platform`], [`Clock`], [`FixedClock`],
//! [`SystemClock`], [`capture_env`], and the clipboard providers [`SystemClipboard`], [`FileClipboard`] and
//! [`clipboard_provider`] (M2 W3, D-100).
//!
//! This module is the **only** place in the core crates that reads the process environment, the current and
//! home directories, the executable location and the system clock and time zone. Architecture check 2
//! (§6.1) names this file as the allow-point of `std::env`, the system clock and `directories`; nothing else
//! under `core/` may do so. [`capture_env`] and [`SystemClock`] are **adapter-only**: only `cli/` and `gui/`
//! (and this crate's own tests) reference them; so are [`SystemClipboard`] and [`clipboard_provider`], and this file is
//! the only one under `core/` that names the `arboard` crate (architecture check 2, contract M2 §1.3.9). Every other function receives the captured [`EnvFacts`] and an
//! injected [`Clock`] as values, so results never depend on ambient state.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use crate::api::ApiError;
use crate::clipboard::{
    ClipboardContent, ClipboardError, ClipboardPayload, ClipboardProvider, MAX_IMAGE_BYTES,
    MAX_TEXT_BYTES,
};

use crate::launcher::{AppLauncher, OpenCommand};
use hikasa_core::PathStyle;
use serde::{Deserialize, Serialize};

/// Platform family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// Linux.
    Linux,
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Any other Unix (the BSDs, illumos, …).
    OtherUnix,
}

impl Platform {
    /// The platform of the compilation target.
    pub const HOST: Platform = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(target_os = "linux") {
        Platform::Linux
    } else {
        Platform::OtherUnix
    };

    /// The local path syntax of the platform.
    pub const fn path_style(self) -> PathStyle {
        match self {
            Platform::Windows => PathStyle::Windows,
            Platform::Linux | Platform::MacOs | Platform::OtherUnix => PathStyle::Unix,
        }
    }

    /// True for every Unix-like platform.
    pub const fn is_unix(self) -> bool {
        match self {
            Platform::Windows => false,
            Platform::Linux | Platform::MacOs | Platform::OtherUnix => true,
        }
    }
}

/// Every ambient fact Hikasa uses, captured ONCE by the adapter (T1). Tests construct it literally (or with
/// [`EnvFacts::new`] and struct update syntax).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFacts {
    /// Platform.
    pub platform: Platform,
    /// Current directory.
    pub cwd: Option<PathBuf>,
    /// Home directory.
    pub home: Option<PathBuf>,
    /// Folder of the running executable (portable marker, GUI lookup).
    pub exe_dir: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `$XDG_CACHE_HOME`.
    pub xdg_cache_home: Option<PathBuf>,
    /// `$XDG_RUNTIME_DIR`.
    pub xdg_runtime_dir: Option<PathBuf>,
    /// `%APPDATA%`.
    pub appdata: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    pub local_appdata: Option<PathBuf>,
    /// Temp directory (`std::env::temp_dir()`; on macOS the per-user `$TMPDIR`).
    pub temp_dir: PathBuf,
    /// User name (`$USER`/`%USERNAME%`), for the Windows pipe name.
    pub user: Option<String>,
    /// Effective uid (Unix). std only: Linux reads `/proc/self/status`; other Unix systems read the owner of a
    /// file `capture_env` creates and removes in `temp_dir`. `None` on Windows or on failure.
    pub uid: Option<u32>,
    /// Codeset from `LC_ALL`/`LC_CTYPE`/`LANG` ("UTF-8", "KOI8-R" …); `None` → UTF-8.
    pub locale_encoding: Option<String>,
    /// `$HIKASA_IPC_ENDPOINT` (tests, unusual setups).
    pub ipc_endpoint_override: Option<OsString>,
    /// `$PATH` (GUI executable lookup).
    pub path_var: Option<OsString>,
    /// `$HIKASA_GUI_EXE`: the GUI executable to launch instead of the [`crate::api::gui::gui_exe_candidates`]
    /// (tests, unusual installs; [`crate::api::gui::find_gui_exe`]). Added in the wave-4 review (§1.8.7).
    pub gui_exe_override: Option<PathBuf>,
    /// `$NO_COLOR` is set to a non-empty value (no-color.org): adapters disable ANSI colours. Added in the
    /// wave-4 review (§1.8.7).
    pub no_color: bool,
    /// `$HIKASA_CLIPBOARD_FILE`: a file whose bytes stand in for the clipboard text ([`FileClipboard`]; tests,
    /// headless scripts). Added in M2 W3 (D-100).
    pub clipboard_file_override: Option<PathBuf>,
    /// `$SOURCE_DATE_EPOCH` as set (reproducible builds): the report time, a decimal count of seconds since the epoch
    /// shown in UTC ([`crate::Service::report_time`] parses it, so a value that does not parse gives a warning). Only
    /// reports read it (M2 W7, D-206).
    pub source_date_epoch: Option<String>,
}

impl EnvFacts {
    /// Facts with only the platform and the temp directory known (every optional fact `None`). Additive helper
    /// for tests and embedders: `EnvFacts { home: Some(..), ..EnvFacts::new(Platform::Linux, tmp) }`.
    pub fn new(platform: Platform, temp_dir: PathBuf) -> EnvFacts {
        EnvFacts {
            platform,
            cwd: None,
            home: None,
            exe_dir: None,
            xdg_config_home: None,
            xdg_cache_home: None,
            xdg_runtime_dir: None,
            appdata: None,
            local_appdata: None,
            temp_dir,
            user: None,
            uid: None,
            locale_encoding: None,
            ipc_endpoint_override: None,
            path_var: None,
            gui_exe_override: None,
            no_color: false,
            clipboard_file_override: None,
            source_date_epoch: None,
        }
    }
}

/// The environment a script references (§1.8.4, D-217): only the names `%NAME%` of its text, read once before the run, as
/// [`capture_env`] reads its facts; a name that is not set is absent (set and empty is kept). Never the whole environment.
pub fn capture_script_env<'n>(
    names: impl IntoIterator<Item = &'n String>,
) -> std::collections::BTreeMap<String, std::ffi::OsString> {
    names
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name.clone(), value)))
        .collect()
}

/// Reads the real process environment (`std::env`, `std::env::current_exe`, `directories::BaseDirs` for the
/// home folder). ADAPTER-ONLY: the architecture test fails if anything under `core/` calls it (except
/// hikasa-session's own tests).
pub fn capture_env() -> EnvFacts {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let path = |name: &str| var(name).map(PathBuf::from);
    let text = |name: &str| var(name).and_then(|v| v.into_string().ok());
    let home = directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .or_else(|| path("HOME"));
    let temp_dir = std::env::temp_dir();
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| text(name))
        .unwrap_or_default();
    EnvFacts {
        platform: Platform::HOST,
        cwd: std::env::current_dir().ok(),
        home,
        exe_dir: std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from)),
        xdg_config_home: path("XDG_CONFIG_HOME"),
        xdg_cache_home: path("XDG_CACHE_HOME"),
        xdg_runtime_dir: path("XDG_RUNTIME_DIR"),
        appdata: path("APPDATA"),
        local_appdata: path("LOCALAPPDATA"),
        user: text("USER")
            .or_else(|| text("USERNAME"))
            .or_else(|| text("LOGNAME")),
        uid: effective_uid(&temp_dir),
        locale_encoding: locale_codeset(&locale),
        ipc_endpoint_override: var("HIKASA_IPC_ENDPOINT"),
        path_var: var("PATH"),
        gui_exe_override: path("HIKASA_GUI_EXE"),
        no_color: var("NO_COLOR").is_some(),
        clipboard_file_override: path("HIKASA_CLIPBOARD_FILE"),
        // Set but empty is "set" here (D-206: an empty value is ignored with a warning), so not through `var`.
        source_date_epoch: std::env::var_os("SOURCE_DATE_EPOCH")
            .map(|v| v.to_string_lossy().into_owned()),
        temp_dir,
    }
}

/// The codeset of a locale value such as `en_US.UTF-8@euro` (`"UTF-8"`); `None` when the value names none
/// (`C`, `POSIX`, `en_US`), which means UTF-8.
pub(crate) fn locale_codeset(value: &str) -> Option<String> {
    let without_modifier = value.split('@').next().unwrap_or("");
    let (_, codeset) = without_modifier.split_once('.')?;
    let codeset = codeset.trim();
    (!codeset.is_empty()).then(|| codeset.to_owned())
}

/// The effective uid with std only (§1.8.2): Linux reads `/proc/self/status`; other Unix systems create a file
/// with a name of their own in `temp_dir` and read its owner.
#[cfg(unix)]
fn effective_uid(temp_dir: &std::path::Path) -> Option<u32> {
    if cfg!(target_os = "linux")
        && let Some(uid) = std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| status_euid(&status))
    {
        return Some(uid);
    }
    probe_uid(temp_dir)
}

/// The owner of a file created in `temp_dir` (the effective uid where `/proc` is missing).
#[cfg(unix)]
fn probe_uid(temp_dir: &std::path::Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    // One probe name per call: concurrent calls in one process (threads, tests) must not collide — a shared
    // name made every caller but one fail with "already exists" and report the uid as unknown (macOS CI).
    static PROBE: AtomicU64 = AtomicU64::new(0);
    for _ in 0..8 {
        let n = PROBE.fetch_add(1, Ordering::Relaxed);
        let probe = temp_dir.join(format!(".hikasa-uid-{}-{n}", std::process::id()));
        let file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => file,
            // A file left by a crashed process with a recycled pid: try the next name.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        };
        let uid = file.metadata().ok().map(|meta| meta.uid());
        drop(file);
        let _ = std::fs::remove_file(&probe);
        return uid;
    }
    None
}

/// No uids on Windows.
#[cfg(not(unix))]
fn effective_uid(_temp_dir: &std::path::Path) -> Option<u32> {
    None
}

/// The effective uid of a `/proc/<pid>/status` text: the second number of the `Uid:` line.
#[cfg(unix)]
fn status_euid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_whitespace().nth(1))
        .and_then(|euid| euid.parse().ok())
}

/// Source of "now" (D-24).
pub trait Clock: Send + Sync {
    /// Current time in the user's zone.
    fn now(&self) -> jiff::Zoned;
}

/// System clock and zone (adapter-only, like `capture_env`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    /// The system time in the system time zone.
    fn now(&self) -> jiff::Zoned {
        jiff::Zoned::now()
    }
}

/// Fixed time (tests, reproducible reports).
#[derive(Debug, Clone)]
pub struct FixedClock(pub jiff::Zoned);

impl Clock for FixedClock {
    /// The fixed time.
    fn now(&self) -> jiff::Zoned {
        self.0.clone()
    }
}

/// The operating-system clipboard through `arboard` (adapter-only, like [`SystemClock`]): non-empty text, else an
/// image, else `Empty`; text above [`MAX_TEXT_BYTES`] or an image above [`MAX_IMAGE_BYTES`] → `TooLarge`. One `arboard::Clipboard` per read; a slow selection owner is bounded by `arboard`'s own timeouts.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClipboard;

/// The clipboard error of an `arboard` failure (§1.3.6).
fn arboard_error(error: arboard::Error) -> ClipboardError {
    match error {
        arboard::Error::ContentNotAvailable => ClipboardError::Empty,
        arboard::Error::ClipboardNotSupported | arboard::Error::ClipboardOccupied => {
            ClipboardError::Unavailable(error.to_string())
        }
        other => ClipboardError::Failed(other.to_string()),
    }
}

impl ClipboardProvider for SystemClipboard {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
        match clipboard.get_text() {
            Ok(text) if text.len() as u64 > MAX_TEXT_BYTES => {
                return Err(ClipboardError::TooLarge(MAX_TEXT_BYTES));
            }
            // Empty text is no text: an image may still be there.
            Ok(text) if !text.is_empty() => return Ok(ClipboardContent::Text(text)),
            Ok(_) | Err(arboard::Error::ContentNotAvailable) => {}
            Err(e) => return Err(arboard_error(e)),
        }
        let image = clipboard.get_image().map_err(arboard_error)?;
        let too_large = || ClipboardError::TooLarge(MAX_IMAGE_BYTES);
        let pixels = (image.width as u64)
            .checked_mul(image.height as u64)
            .and_then(|n| n.checked_mul(4));
        if pixels.is_none_or(|n| n > MAX_IMAGE_BYTES) {
            return Err(too_large());
        }
        Ok(ClipboardContent::Image {
            width: u32::try_from(image.width).map_err(|_| too_large())?,
            height: u32::try_from(image.height).map_err(|_| too_large())?,
            rgba: image.bytes.into_owned(),
        })
    }

    /// Places a report (§1.7.10). On Linux (X11 and Wayland) the content must outlive `hikasa`: a helper process,
    /// `hikasa` itself through its hidden entry [`CLIPBOARD_HELPER_ARG`], takes the payload on its stdin and owns the
    /// selection until something else is copied; this returns once it reports the payload placed. Windows and macOS
    /// keep the content after the process ends: `arboard` directly.
    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> {
        if cfg!(target_os = "linux") {
            let exe = std::env::current_exe()
                .map_err(|e| ClipboardError::Failed(format!("cannot find hikasa: {e}")))?;
            let mut command = std::process::Command::new(exe);
            command.arg(CLIPBOARD_HELPER_ARG);
            return spawn_clipboard_helper(&mut command, payload).map(drop);
        }
        place_payload(payload)
    }

    /// Places a list of files (M2 W9c, D-252) with `arboard`'s `file_list`: `text/uri-list` on X11 and Wayland,
    /// `CF_HDROP` on Windows, file URLs on macOS. On Linux the list outlives the window through the helper, as a
    /// report does ([`ClipboardProvider::write_payload`]).
    fn write_files(&self, paths: &[PathBuf]) -> Result<(), ClipboardError> {
        if paths.is_empty() {
            return Err(ClipboardError::Failed("no file to place".into()));
        }
        if cfg!(target_os = "linux") {
            let exe = std::env::current_exe()
                .map_err(|e| ClipboardError::Failed(format!("cannot find hikasa: {e}")))?;
            let mut command = std::process::Command::new(exe);
            command.arg(CLIPBOARD_HELPER_ARG);
            return spawn_helper(&mut command, &encode_files(paths)).map(drop);
        }
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
        clipboard.set().file_list(paths).map_err(arboard_error)
    }

    /// The files of a file manager's copy (`arboard`'s `file_list`); `Empty` when the clipboard holds none.
    fn read_files(&self) -> Result<Vec<PathBuf>, ClipboardError> {
        let mut clipboard =
            arboard::Clipboard::new().map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
        match clipboard.get().file_list() {
            Ok(paths) if paths.is_empty() => Err(ClipboardError::Empty),
            Ok(paths) => Ok(paths),
            Err(e) => Err(arboard_error(e)),
        }
    }
}

/// The hidden internal entry of the Linux clipboard helper (§1.7.10): `hikasa __clipboard-helper`. Not in `--help`, the
/// reference or the help topics; the adapter's `main` runs [`clipboard_helper_main`] for it before parsing arguments.
pub const CLIPBOARD_HELPER_ARG: &str = "__clipboard-helper";

/// The status line of a helper that placed its payload.
const HELPER_PLACED: &str = "placed";

/// The prefix of a helper's status line when there is no clipboard (no display): `unavailable: <why>`, kind
/// `unsupported`. Any other failure is `failed: <why>`, kind `io`.
const HELPER_UNAVAILABLE: &str = "unavailable: ";

/// The prefix of a helper's status line when placing failed.
const HELPER_FAILED: &str = "failed: ";

/// Places a payload with `arboard` and returns (Windows, macOS; the helper's first, non-waiting placement).
fn place_payload(payload: &ClipboardPayload) -> Result<(), ClipboardError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| ClipboardError::Unavailable(e.to_string()))?;
    match &payload.html {
        Some(html) => clipboard.set_html(html.as_str(), Some(payload.text.as_str())),
        None => clipboard.set_text(payload.text.as_str()),
    }
    .map_err(arboard_error)
}

/// The second length of a helper input that carries a list of files (M2 W9c): the first length is then the list's
/// bytes, each path followed by a NUL byte (Unix: the path's bytes; elsewhere UTF-8).
const FILES_MARK: u64 = u64::MAX - 1;

/// What the helper places.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HelperContent {
    /// A report (§1.7.10).
    Payload(ClipboardPayload),
    /// A list of files (D-252).
    Files(Vec<PathBuf>),
}

/// The bytes of a path in the helper's list.
fn path_bytes(path: &std::path::Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

/// A path of the helper's list.
fn path_of_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// A list of files on the helper's stdin: its length and [`FILES_MARK`], then each path and a NUL byte.
fn encode_files(paths: &[PathBuf]) -> Vec<u8> {
    let mut list = Vec::new();
    for path in paths {
        list.extend_from_slice(&path_bytes(path));
        list.push(0);
    }
    let mut bytes = Vec::with_capacity(16 + list.len());
    bytes.extend_from_slice(&(list.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&FILES_MARK.to_le_bytes());
    bytes.extend_from_slice(&list);
    bytes
}

/// The payload on the helper's stdin: the text's and the HTML's lengths (8 bytes little-endian each; `u64::MAX`: no
/// HTML), then the text, then the HTML.
fn encode_payload(payload: &ClipboardPayload) -> Vec<u8> {
    let html_len = payload.html.as_ref().map_or(u64::MAX, |h| h.len() as u64);
    let mut bytes =
        Vec::with_capacity(16 + payload.text.len() + payload.html.as_ref().map_or(0, String::len));
    bytes.extend_from_slice(&(payload.text.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&html_len.to_le_bytes());
    bytes.extend_from_slice(payload.text.as_bytes());
    if let Some(html) = &payload.html {
        bytes.extend_from_slice(html.as_bytes());
    }
    bytes
}

/// Reads a payload of [`encode_payload`] or a list of [`encode_files`] (at most
/// [`crate::clipboard::CLIPBOARD_REPORT_MAX`] bytes).
fn decode_payload(input: &mut dyn std::io::Read) -> Result<HelperContent, String> {
    let mut head = [0u8; 16];
    input
        .read_exact(&mut head)
        .map_err(|e| format!("no payload: {e}"))?;
    let len = |b: &[u8]| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]));
    let (text_len, html_len) = (len(&head[..8]), len(&head[8..]));
    let max = crate::clipboard::CLIPBOARD_REPORT_MAX;
    if html_len == FILES_MARK {
        if text_len > max {
            return Err("the file list is larger than the clipboard limit".into());
        }
        let mut list = vec![0u8; usize::try_from(text_len).map_err(|e| e.to_string())?];
        input
            .read_exact(&mut list)
            .map_err(|e| format!("short file list: {e}"))?;
        let paths: Vec<PathBuf> = list
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(path_of_bytes)
            .collect();
        if paths.is_empty() {
            return Err("the file list is empty".into());
        }
        return Ok(HelperContent::Files(paths));
    }
    let html_size = if html_len == u64::MAX { 0 } else { html_len };
    if text_len.saturating_add(html_size) > max {
        return Err("the payload is larger than the clipboard limit".into());
    }
    let mut read = |n: u64| -> Result<String, String> {
        let mut bytes = vec![0u8; usize::try_from(n).map_err(|e| e.to_string())?];
        input
            .read_exact(&mut bytes)
            .map_err(|e| format!("short payload: {e}"))?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    };
    let text = read(text_len)?;
    let html = if html_len == u64::MAX {
        None
    } else {
        Some(read(html_len)?)
    };
    Ok(HelperContent::Payload(ClipboardPayload { text, html }))
}

/// Starts a clipboard helper `command`, writes the payload to its stdin, closes it and waits for its one status line
/// (§1.7.10): `placed` → `Ok` with the running child (the caller lets it run on); `unavailable: <why>` →
/// `Unavailable` (`unsupported`); another line, none, or a helper that cannot start → `Failed` (`io`), nothing placed.
/// On Unix the helper runs in the root folder (it keeps no folder of the caller's busy) and in a process group of its
/// own (a Ctrl-C of the caller's terminal does not end it). Adapter-only, like [`SystemClipboard`].
pub fn spawn_clipboard_helper(
    command: &mut std::process::Command,
    payload: &ClipboardPayload,
) -> Result<std::process::Child, ClipboardError> {
    spawn_helper(command, &encode_payload(payload))
}

/// [`spawn_clipboard_helper`] with a list of files (M2 W9c, D-252): the helper owns the list until something else
/// is copied. Adapter-only.
pub fn spawn_clipboard_files_helper(
    command: &mut std::process::Command,
    paths: &[PathBuf],
) -> Result<std::process::Child, ClipboardError> {
    spawn_helper(command, &encode_files(paths))
}

/// Starts a helper with `input` on its stdin and waits for its status line.
fn spawn_helper(
    command: &mut std::process::Command,
    input: &[u8],
) -> Result<std::process::Child, ClipboardError> {
    use std::io::{BufRead, Write};
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.current_dir("/").process_group(0);
    }
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ClipboardError::Failed(format!("cannot start the clipboard helper: {e}")))?;
    let fail = |child: &mut std::process::Child, message: String| {
        let _ = child.kill();
        let _ = child.wait();
        ClipboardError::Failed(message)
    };
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(input).and_then(|()| stdin.flush()));
    if let Some(Err(e)) = written {
        return Err(fail(
            &mut child,
            format!("the clipboard helper did not take the payload: {e}"),
        ));
    }
    let mut line = String::new();
    let read = child
        .stdout
        .take()
        .map(|stdout| std::io::BufReader::new(stdout).read_line(&mut line));
    match read {
        Some(Ok(_)) if line.trim_end() == HELPER_PLACED => Ok(child),
        Some(Ok(0)) | None => Err(fail(
            &mut child,
            "the clipboard helper ended without placing the report".into(),
        )),
        Some(Ok(_)) if line.starts_with(HELPER_UNAVAILABLE) => {
            let why = line.trim_end()[HELPER_UNAVAILABLE.len()..].to_owned();
            let _ = fail(&mut child, String::new());
            Err(ClipboardError::Unavailable(why))
        }
        Some(Ok(_)) => {
            let message = line
                .trim_end()
                .strip_prefix(HELPER_FAILED)
                .unwrap_or(line.trim_end())
                .to_owned();
            Err(fail(
                &mut child,
                format!("the clipboard helper failed: {message}"),
            ))
        }
        Some(Err(e)) => Err(fail(&mut child, format!("the clipboard helper: {e}"))),
    }
}

/// How the helper places a payload: the real clipboard, or a test's stand-in.
pub enum HelperPlacer {
    /// `arboard` with Linux's wait mode: placed, reported, then the selection is owned until something else is copied.
    System,
    /// Tests: the payload's HTML (else its text) is written to this file, then the helper reports and ends.
    File(PathBuf),
}

/// The Linux clipboard helper (§1.7.10): reads a payload from `input`, places it, writes `placed` (or
/// `unavailable: <why>` without a clipboard, `failed: <why>` otherwise)
/// as one line to `output`, and with [`HelperPlacer::System`] keeps the selection until another program takes it.
/// Returns the process exit code. Adapter-only.
pub fn clipboard_helper_main(
    input: &mut dyn std::io::Read,
    output: &mut dyn std::io::Write,
    placer: &HelperPlacer,
) -> i32 {
    let report = |output: &mut dyn std::io::Write, line: &str| {
        let _ = writeln!(output, "{line}");
        let _ = output.flush();
    };
    // A caller that went away before the whole payload came (stdin at its end) ends the helper here.
    let content = match decode_payload(input) {
        Ok(content) => content,
        Err(e) => {
            report(output, &format!("{HELPER_FAILED}{e}"));
            return 1;
        }
    };
    match placer {
        HelperPlacer::File(path) => {
            let bytes = match &content {
                HelperContent::Payload(payload) => payload
                    .html
                    .as_ref()
                    .unwrap_or(&payload.text)
                    .clone()
                    .into_bytes(),
                HelperContent::Files(paths) => file_lines(paths),
            };
            match crate::atomic::write_atomic(path, &bytes) {
                Ok(()) => {
                    report(output, HELPER_PLACED);
                    0
                }
                Err(e) => {
                    report(output, &format!("{HELPER_FAILED}{e}"));
                    1
                }
            }
        }
        HelperPlacer::System => {
            // No clipboard (no display, as `place_payload`'s `Unavailable`) is said apart from a failure.
            let mut clipboard = match arboard::Clipboard::new() {
                Ok(clipboard) => clipboard,
                Err(e) => {
                    report(output, &format!("{HELPER_UNAVAILABLE}{e}"));
                    return 1;
                }
            };
            let placed = match &content {
                HelperContent::Payload(payload) => match &payload.html {
                    Some(html) => clipboard.set_html(html.as_str(), Some(payload.text.as_str())),
                    None => clipboard.set_text(payload.text.as_str()),
                },
                HelperContent::Files(paths) => clipboard.set().file_list(paths),
            };
            if let Err(e) = placed {
                report(output, &helper_status(&arboard_error(e)));
                return 1;
            }
            report(output, HELPER_PLACED);
            wait_owning(&mut clipboard, &content);
            0
        }
    }
}

/// The status line of a helper whose placement failed with `error`.
fn helper_status(error: &ClipboardError) -> String {
    match error {
        ClipboardError::Unavailable(why) => format!("{HELPER_UNAVAILABLE}{why}"),
        ClipboardError::Failed(why) => format!("{HELPER_FAILED}{why}"),
        other => format!("{HELPER_FAILED}{other}"),
    }
}

/// The placer of the hidden entry `__clipboard-helper` (§1.7.10), for the CLI's and the GUI's `main` alike, which run
/// [`clipboard_helper_main`] on their stdin and stdout with it: the clipboard, or with
/// `HIKASA_CLIPBOARD_HELPER_FILE=<file>` (as `HIKASA_CLIPBOARD_FILE`: scripts, headless machines, tests) that file,
/// which gets the payload's HTML, else its text.
pub fn clipboard_helper_placer() -> HelperPlacer {
    match std::env::var_os("HIKASA_CLIPBOARD_HELPER_FILE") {
        Some(path) if !path.is_empty() => HelperPlacer::File(path.into()),
        _ => HelperPlacer::System,
    }
}

/// True when the process was started as the clipboard helper: its first argument is [`CLIPBOARD_HELPER_ARG`].
pub fn is_clipboard_helper(argv: &[std::ffi::OsString]) -> bool {
    argv.get(1).is_some_and(|a| a == CLIPBOARD_HELPER_ARG)
}

/// Places the payload once more in Linux's wait mode: returns when another program owns the selection.
#[cfg(target_os = "linux")]
fn wait_owning(clipboard: &mut arboard::Clipboard, content: &HelperContent) {
    use arboard::SetExtLinux;
    let set = clipboard.set().wait();
    let _ = match content {
        HelperContent::Payload(payload) => match &payload.html {
            Some(html) => set.html(html.as_str(), Some(payload.text.as_str())),
            None => set.text(payload.text.as_str()),
        },
        HelperContent::Files(paths) => set.file_list(paths),
    };
}

/// Elsewhere the content outlives the process: nothing to wait for.
#[cfg(not(target_os = "linux"))]
fn wait_owning(_clipboard: &mut arboard::Clipboard, _content: &HelperContent) {}

/// A list of files as [`FileClipboard`] keeps it: one path a line, `\n` after each (Unix: the path's bytes).
fn file_lines(paths: &[PathBuf]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(&path_bytes(path));
        bytes.push(b'\n');
    }
    bytes
}

/// A file standing in for the clipboard (`$HIKASA_CLIPBOARD_FILE`): its bytes as text (invalid UTF-8 replaced);
/// a missing file → `Unavailable`, an empty one → `Empty`, more than [`MAX_TEXT_BYTES`] → `TooLarge`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileClipboard(pub PathBuf);

impl ClipboardProvider for FileClipboard {
    fn read(&self) -> Result<ClipboardContent, ClipboardError> {
        // A regular file above the limit is refused from its length; otherwise at most one byte more than the
        // limit is read, so a huge file (or a device) never fills the memory.
        if std::fs::metadata(&self.0).is_ok_and(|m| m.is_file() && m.len() > MAX_TEXT_BYTES) {
            return Err(ClipboardError::TooLarge(MAX_TEXT_BYTES));
        }
        let read = std::fs::File::open(&self.0).and_then(|file| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(file, MAX_TEXT_BYTES + 1),
                &mut bytes,
            )
            .map(|_| bytes)
        });
        match read {
            Ok(bytes) if bytes.len() as u64 > MAX_TEXT_BYTES => {
                Err(ClipboardError::TooLarge(MAX_TEXT_BYTES))
            }
            Ok(bytes) if bytes.is_empty() => Err(ClipboardError::Empty),
            Ok(bytes) => Ok(ClipboardContent::Text(
                String::from_utf8_lossy(&bytes).into_owned(),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ClipboardError::Unavailable(
                format!("HIKASA_CLIPBOARD_FILE {} does not exist", self.0.display()),
            )),
            Err(e) => Err(ClipboardError::Failed(format!("{}: {e}", self.0.display()))),
        }
    }

    /// Writes the payload's HTML when it has one, else its text, atomically (§1.7.10), so a test reads back exactly
    /// a file target's bytes.
    fn write_payload(&self, payload: &ClipboardPayload) -> Result<(), ClipboardError> {
        let bytes = payload.html.as_ref().unwrap_or(&payload.text);
        crate::atomic::write_atomic(&self.0, bytes.as_bytes())
            .map_err(|e| ClipboardError::Failed(format!("{}: {e}", self.0.display())))
    }

    /// Writes the list one path a line (M2 W9c, D-252), atomically; a path holding a line break cannot be written.
    fn write_files(&self, paths: &[PathBuf]) -> Result<(), ClipboardError> {
        if let Some(bad) = paths
            .iter()
            .find(|p| path_bytes(p).iter().any(|b| matches!(b, b'\n' | b'\r')))
        {
            return Err(ClipboardError::Failed(format!(
                "{} holds a line break",
                bad.display()
            )));
        }
        crate::atomic::write_atomic(&self.0, &file_lines(paths))
            .map_err(|e| ClipboardError::Failed(format!("{}: {e}", self.0.display())))
    }

    /// The file's lines as paths (a `\r` before a line's end is dropped; empty lines are skipped); no line: `Empty`.
    fn read_files(&self) -> Result<Vec<PathBuf>, ClipboardError> {
        let bytes = match std::fs::read(&self.0) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ClipboardError::Unavailable(format!(
                    "HIKASA_CLIPBOARD_FILE {} does not exist",
                    self.0.display()
                )));
            }
            Err(e) => return Err(ClipboardError::Failed(format!("{}: {e}", self.0.display()))),
        };
        let paths: Vec<PathBuf> = bytes
            .split(|b| *b == b'\n')
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
            .filter(|line| !line.is_empty())
            .map(path_of_bytes)
            .collect();
        if paths.is_empty() {
            return Err(ClipboardError::Empty);
        }
        Ok(paths)
    }
}

/// The clipboard of an adapter (adapter-only): [`FileClipboard`] when `env.clipboard_file_override` is set, else
/// [`SystemClipboard`].
pub fn clipboard_provider(env: &EnvFacts) -> Arc<dyn ClipboardProvider> {
    match &env.clipboard_file_override {
        Some(path) => Arc::new(FileClipboard(path.clone())),
        None => Arc::new(SystemClipboard),
    }
}

/// What really starts programs (FC-062): `std::process::Command`, no shell, stdio null; the associated application by
/// `open`. Adapter-only, as [`SystemClipboard`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLauncher;

impl AppLauncher for SystemLauncher {
    fn run(&self, command: &OpenCommand) -> Result<i32, ApiError> {
        (self.start(command)?)()
    }

    fn start<'a>(
        &'a self,
        command: &'a OpenCommand,
    ) -> Result<crate::launcher::Started<'a>, ApiError> {
        let mut child = std::process::Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                ApiError::InvalidArgument(format!(
                    "cannot start {}: {e}",
                    command.program.to_string_lossy()
                ))
            })?;
        Ok(Box::new(move || {
            let status = child.wait().map_err(|e| {
                ApiError::InvalidArgument(format!("{}: {e}", command.program.to_string_lossy()))
            })?;
            Ok(status.code().unwrap_or(-1))
        }))
    }

    fn open_associated(&self, path: &std::path::Path) -> Result<(), ApiError> {
        open::that_detached(path)
            .map_err(|e| ApiError::InvalidArgument(format!("cannot open {}: {e}", path.display())))
    }

    /// *Show in File Manager* (D-254): [`reveal_command`] of this platform, run to its end; when it cannot start, or
    /// (outside Windows, whose `explorer.exe` exits 1 when it worked) it ends with an error, the folder is opened
    /// instead ([`reveal_fallback`]).
    fn reveal(&self, path: &std::path::Path) -> Result<(), ApiError> {
        let platform = Platform::HOST;
        let command = reveal_command(platform, path);
        let mut process = std::process::Command::new(&command.program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            // `/select,"<path>"` as one word: the standard quoting would quote the `/select,` too.
            for arg in &command.args {
                process.raw_arg(arg);
            }
        }
        #[cfg(not(windows))]
        process.args(&command.args);
        let status = process
            .current_dir(&command.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let shown = match status {
            Ok(status) => platform == Platform::Windows || status.success(),
            Err(_) => false,
        };
        if shown {
            return Ok(());
        }
        self.open_associated(&reveal_fallback(path))
    }
}

/// What *Show in File Manager* runs for `path` on `platform` (D-254), a pure builder: Windows
/// `explorer.exe /select,"<path>"`, macOS `open -R <path>`, Linux and the other Unix systems the D-Bus call
/// `org.freedesktop.FileManager1.ShowItems` of the session bus through `dbus-send` with the item's `file://` URI
/// ([`file_uri`]). The command starts in the item's folder.
pub fn reveal_command(platform: Platform, path: &std::path::Path) -> OpenCommand {
    let cwd = reveal_fallback(path);
    let words = |words: &[&str]| words.iter().map(OsString::from).collect::<Vec<_>>();
    match platform {
        Platform::Windows => {
            let mut arg = OsString::from("/select,\"");
            arg.push(path.as_os_str());
            arg.push("\"");
            OpenCommand {
                program: "explorer.exe".into(),
                args: vec![arg],
                cwd,
            }
        }
        Platform::MacOs => {
            let mut args = words(&["-R"]);
            args.push(path.as_os_str().to_owned());
            OpenCommand {
                program: "open".into(),
                args,
                cwd,
            }
        }
        Platform::Linux | Platform::OtherUnix => {
            let mut args = words(&[
                "--session",
                "--print-reply",
                "--reply-timeout=5000",
                "--type=method_call",
                "--dest=org.freedesktop.FileManager1",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
            ]);
            args.push(format!("array:string:{}", file_uri(path)).into());
            args.push("string:".into());
            OpenCommand {
                program: "dbus-send".into(),
                args,
                cwd,
            }
        }
    }
}

/// The folder *Show in File Manager* opens when the file manager cannot select the item: the item's folder (a root
/// is its own).
pub fn reveal_fallback(path: &std::path::Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(path)
        .to_path_buf()
}

/// The `file://` URI of an absolute path (RFC 8089): every byte but the unreserved ones (`A–Z a–z 0–9 - . _ ~`) and
/// `/` percent-encoded, so a comma (`dbus-send`'s array separator), a space or a byte that is not UTF-8 survives.
/// Windows paths become `file:///C:/…`.
pub fn file_uri(path: &std::path::Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt as _;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().replace('\\', "/").into_bytes();
    let mut uri = String::from("file://");
    if bytes.first() != Some(&b'/') {
        uri.push('/');
    }
    for byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(byte));
            }
            // A drive's colon stays readable.
            b':' if uri.len() <= 10 => uri.push(':'),
            other => uri.push_str(&format!("%{other:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_codesets() {
        assert_eq!(locale_codeset("en_US.UTF-8").as_deref(), Some("UTF-8"));
        assert_eq!(locale_codeset("ru_RU.KOI8-R@x").as_deref(), Some("KOI8-R"));
        assert_eq!(locale_codeset("C"), None);
        assert_eq!(locale_codeset("en_US"), None);
        assert_eq!(locale_codeset(""), None);
        assert_eq!(locale_codeset("de_DE."), None);
    }

    /// Regression (macOS CI): concurrent probes in one process all find the uid.
    #[cfg(unix)]
    #[test]
    fn concurrent_uid_probes_agree() {
        let dir = tempfile::tempdir().unwrap();
        let uids: Vec<Option<u32>> = std::thread::scope(|scope| {
            let probes: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| probe_uid(dir.path())))
                .collect();
            probes.into_iter().map(|p| p.join().unwrap()).collect()
        });
        assert!(
            uids.iter().all(|u| u.is_some() && *u == uids[0]),
            "{uids:?}"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "probes are removed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn proc_status_uid() {
        let status = "Name:\tx\nUid:\t1000\t1001\t1000\t1000\nGid:\t5\t5\t5\t5\n";
        assert_eq!(status_euid(status), Some(1001));
        assert_eq!(status_euid("Name: x\n"), None);
    }

    #[test]
    fn captured_facts_are_plausible() {
        let facts = capture_env();
        assert_eq!(facts.platform, Platform::HOST);
        assert!(!facts.temp_dir.as_os_str().is_empty());
        if cfg!(unix) {
            assert!(facts.uid.is_some(), "{facts:?}");
        }
    }

    /// A7.15: the adapter's spawn passes the payload and returns after the status line while the helper runs on; a
    /// helper that ends at once, reports a failure or cannot start is `Failed` (`io`).
    #[cfg(unix)]
    #[test]
    fn a7_15_the_helper_spawn() {
        use hikasa_core::{Classify, ErrorKind};
        let dir = tempfile::tempdir().unwrap();
        let got = dir.path().join("got");
        let payload = ClipboardPayload {
            text: "plain".into(),
            html: Some("<b>html</b>".into()),
        };
        let mut command = std::process::Command::new("sh");
        // `exec`: the shell (dash on Ubuntu) becomes the sleeping helper, so the kill below ends it.
        command
            .arg("-c")
            .arg("cat > \"$1\"; echo placed; exec sleep 30")
            .arg("sh")
            .arg(&got);
        let started = std::time::Instant::now();
        let mut child = spawn_clipboard_helper(&mut command, &payload).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
        assert!(child.try_wait().unwrap().is_none(), "the helper runs on");
        assert_eq!(std::fs::read(&got).unwrap(), encode_payload(&payload));
        let mut stdin = std::io::Cursor::new(encode_payload(&payload));
        assert_eq!(
            decode_payload(&mut stdin).unwrap(),
            HelperContent::Payload(payload.clone())
        );
        child.kill().unwrap();
        child.wait().unwrap();
        for script in ["exit 0", "cat >/dev/null; echo 'failed: no display'"] {
            let mut command = std::process::Command::new("sh");
            command.arg("-c").arg(script);
            let err = spawn_clipboard_helper(&mut command, &payload).unwrap_err();
            assert!(
                matches!(err, ClipboardError::Failed(_)),
                "{script}: {err:?}"
            );
            assert_eq!(err.to_vfs().kind(), ErrorKind::Io, "{script}");
            if script.contains("failed") {
                assert!(err.to_string().contains("no display"), "{err}");
            }
        }
        let mut command = std::process::Command::new(dir.path().join("no-such-helper"));
        let err = spawn_clipboard_helper(&mut command, &payload).unwrap_err();
        assert!(matches!(err, ClipboardError::Failed(_)), "{err:?}");
        // The test placer of the hidden entry.
        let file = dir.path().join("placed");
        let mut input = std::io::Cursor::new(encode_payload(&payload));
        let mut output = Vec::new();
        assert_eq!(
            clipboard_helper_main(&mut input, &mut output, &HelperPlacer::File(file.clone())),
            0
        );
        assert_eq!(output, b"placed\n");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "<b>html</b>");
        // A clipboard without a writer is `unsupported`.
        let err = crate::clipboard::NoClipboard
            .write_payload(&payload)
            .unwrap_err();
        assert_eq!(err.to_vfs().kind(), ErrorKind::Unsupported);
        assert_eq!(err.to_vfs_write().kind(), ErrorKind::Unsupported);
    }

    /// A7.15 (review): a helper without a clipboard (`unavailable: …`) is `Unavailable`, kind `unsupported`; a failed
    /// one is `Failed`, kind `io`, a write of the clipboard ("cannot write the clipboard"), never a read.
    #[cfg(unix)]
    #[test]
    fn a7_15_helper_failures_are_writes_and_unavailable_is_unsupported() {
        use hikasa_core::{Classify, ErrorKind};
        let payload = ClipboardPayload {
            text: "plain".into(),
            html: None,
        };
        let status = |script: &str| {
            let mut command = std::process::Command::new("sh");
            command.arg("-c").arg(script);
            spawn_clipboard_helper(&mut command, &payload).unwrap_err()
        };
        let err = status("cat >/dev/null; echo 'unavailable: no display'");
        assert!(
            matches!(&err, ClipboardError::Unavailable(why) if why == "no display"),
            "{err:?}"
        );
        assert_eq!(err.to_vfs_write().kind(), ErrorKind::Unsupported);
        let err = status("cat >/dev/null; echo 'failed: the selection owner went away'");
        assert!(matches!(err, ClipboardError::Failed(_)), "{err:?}");
        let vfs = err.to_vfs_write();
        assert_eq!(vfs.kind(), ErrorKind::Io);
        let message = vfs.to_string();
        assert!(message.contains("cannot write the clipboard"), "{message}");
        assert!(
            message.contains("the selection owner went away"),
            "{message}"
        );
        assert!(!message.contains("read"), "{message}");
        assert!(
            matches!(
                &vfs,
                hikasa_vfs::VfsError::Io {
                    op: hikasa_vfs::VfsOp::Write,
                    ..
                }
            ),
            "{vfs:?}"
        );
        // The helper's own status lines.
        let error = arboard_error(arboard::Error::ClipboardNotSupported);
        assert!(helper_status(&error).starts_with(HELPER_UNAVAILABLE));
        let error = ClipboardError::Failed("boom".into());
        assert_eq!(helper_status(&error), "failed: boom");
    }

    /// A7.15 (review): the helper runs in the root folder and in a process group of its own; a payload cut short
    /// (the caller went away, stdin at its end) ends it with one failure line.
    #[cfg(target_os = "linux")]
    #[test]
    fn a7_15_the_helper_runs_detached() {
        let dir = tempfile::tempdir().unwrap();
        let got = dir.path().join("where");
        let payload = ClipboardPayload {
            text: "plain".into(),
            html: None,
        };
        let mut command = std::process::Command::new("sh");
        command
            .arg("-c")
            .arg("cat >/dev/null; pwd > \"$1\"; cut -d' ' -f5 /proc/$$/stat >> \"$1\"; echo placed; exec sleep 30")
            .arg("sh")
            .arg(&got);
        let mut child = spawn_clipboard_helper(&mut command, &payload).unwrap();
        let pid = child.id();
        let text = std::fs::read_to_string(&got).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            ["/".to_owned(), pid.to_string()],
            "cwd, then the process group"
        );
        // Stdin at its end before the whole payload.
        let mut full = encode_payload(&payload);
        full.truncate(full.len() - 2);
        for input in [Vec::new(), full] {
            let mut output = Vec::new();
            let code = clipboard_helper_main(
                &mut std::io::Cursor::new(input),
                &mut output,
                &HelperPlacer::File(dir.path().join("never")),
            );
            assert_eq!(code, 1);
            assert!(String::from_utf8(output).unwrap().starts_with("failed: "));
            assert!(!dir.path().join("never").exists());
        }
    }

    /// A9.20 (D-252): a list of files goes through the helper's input and its test placer one path a line, in order;
    /// an empty or short list is refused.
    #[test]
    fn a9_20_the_helper_takes_a_file_list() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![PathBuf::from("/a/b c.txt"), PathBuf::from("/d/e,f")];
        let bytes = encode_files(&paths);
        assert_eq!(
            decode_payload(&mut std::io::Cursor::new(bytes.clone())).unwrap(),
            HelperContent::Files(paths.clone())
        );
        let file = dir.path().join("placed");
        let mut output = Vec::new();
        let code = clipboard_helper_main(
            &mut std::io::Cursor::new(bytes.clone()),
            &mut output,
            &HelperPlacer::File(file.clone()),
        );
        assert_eq!(code, 0);
        assert_eq!(output, b"placed\n");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "/a/b c.txt\n/d/e,f\n"
        );
        assert_eq!(FileClipboard(file).read_files().unwrap(), paths);
        let mut short = bytes;
        short.truncate(short.len() - 3);
        for input in [encode_files(&[]), short] {
            let mut output = Vec::new();
            let code = clipboard_helper_main(
                &mut std::io::Cursor::new(input),
                &mut output,
                &HelperPlacer::File(dir.path().join("never")),
            );
            assert_eq!(code, 1);
            assert!(!dir.path().join("never").exists());
        }
    }

    #[test]
    fn platform_helpers() {
        assert_eq!(Platform::Windows.path_style(), PathStyle::Windows);
        assert_eq!(Platform::Linux.path_style(), PathStyle::Unix);
        assert!(Platform::MacOs.is_unix() && !Platform::Windows.is_unix());
    }
}
