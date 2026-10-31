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

use crate::clipboard::{
    ClipboardContent, ClipboardError, ClipboardProvider, MAX_IMAGE_BYTES, MAX_TEXT_BYTES,
};

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
        }
    }
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
}

/// The clipboard of an adapter (adapter-only): [`FileClipboard`] when `env.clipboard_file_override` is set, else
/// [`SystemClipboard`].
pub fn clipboard_provider(env: &EnvFacts) -> Arc<dyn ClipboardProvider> {
    match &env.clipboard_file_override {
        Some(path) => Arc::new(FileClipboard(path.clone())),
        None => Arc::new(SystemClipboard),
    }
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

    #[test]
    fn platform_helpers() {
        assert_eq!(Platform::Windows.path_style(), PathStyle::Windows);
        assert_eq!(Platform::Linux.path_style(), PathStyle::Unix);
        assert!(Platform::MacOs.is_unix() && !Platform::Windows.is_unix());
    }
}
