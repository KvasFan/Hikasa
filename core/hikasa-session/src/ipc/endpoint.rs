//! The IPC endpoint per user and configuration (§2.23, ARCH-090/091, D-47): [`Endpoint`],
//! [`EndpointAddress`], [`InstanceLock`], [`fnv1a64`].
//!
//! ```text
//! Endpoint::for_config(facts, config):
//!   0. facts.ipc_endpoint_override = Some(o) → Unix: socket path o, lock o + ".lock";
//!                                              Windows: pipe name o, lock facts.temp_dir \ (o + ".lock")
//!   1. suffix = config.is_default ? "" : "-" + hex8(fnv1a64(config.dir bytes))     // first 8 lowercase hex digits
//!      (config.dir bytes: the OsStr bytes on Unix, UTF-16LE code units on Windows)
//!   2. Linux / OtherUnix:
//!        dir  = facts.xdg_runtime_dir when absolute, else facts.temp_dir / ("hikasa-" + id)   // Hikasa's own folder
//!               where id = facts.uid, else sanitize(facts.user), else "user"
//!        path = dir / ("hikasa" + suffix + ".sock")
//!      MacOs: dir = facts.temp_dir (the per-user $TMPDIR); path as above
//!      if len(path) > 100 bytes: path = dir / ("hk-" + hex16(fnv1a64(path)) + ".sock")
//!        and if still > 100 bytes: path = "/tmp/hikasa-" + id + "/hk-" + hex16(…) + ".sock" (Hikasa's own folder)
//!      lock = path + ".lock"
//!   3. Windows: name = "hikasa-" + sanitize(facts.user or "user") + suffix; address = \\.\pipe\<name>;
//!      lock = facts.temp_dir \ (name + ".lock")
//!   sanitize(s): every char outside [A-Za-z0-9_.-] becomes '_'
//! ```
//!
//! Two GUIs with different configuration folders never share a socket or a lock, while every launch with the
//! same configuration reaches the same single instance.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use super::error::IpcError;
use crate::host::{EnvFacts, Platform};
use crate::paths::ConfigLocation;

/// Longest Unix socket path used before the hashed fallback names (§2.23; `sun_path` is 104 bytes on macOS).
pub const MAX_SOCKET_PATH: usize = 100;

/// Transport address of an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EndpointAddress {
    /// Unix domain socket path.
    UnixSocket(PathBuf),
    /// Windows named pipe name (`hikasa-<user>[-<h8>]`).
    NamedPipe(String),
}

/// Per-user, per-configuration endpoint (§2.23, D-47).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Address.
    pub address: EndpointAddress,
    /// Single-instance lock file (Unix: `<socket>.lock`; Windows: `<temp>\<pipe name>.lock`).
    pub lock_path: PathBuf,
    /// Hikasa's own socket folder (`<temp>/hikasa-<id>` or `/tmp/hikasa-<id>`), which `IpcServer::bind` creates
    /// with mode 0700 and verifies; `None` for `$XDG_RUNTIME_DIR`, macOS `$TMPDIR`, an override and Windows.
    /// Additive field (wave 4).
    pub own_dir: Option<PathBuf>,
    /// The user id that must own the socket folder (Unix: `EnvFacts::uid`; `None` on Windows). Before a
    /// client connects and before a server binds, the folder is verified ([`Endpoint::verify_folder`]); on Unix
    /// an unknown uid fails closed. Field added in the wave-4 review (§1.8.7).
    pub uid: Option<u32>,
}

impl Endpoint {
    /// Computes the endpoint (§2.23). Pure: `ipc_endpoint_override` wins; nothing is created here (the server
    /// creates and verifies the socket folder in `IpcServer::bind`).
    pub fn for_config(facts: &EnvFacts, config: &ConfigLocation) -> Result<Endpoint, IpcError> {
        let windows = facts.platform == Platform::Windows;
        if let Some(o) = facts
            .ipc_endpoint_override
            .as_ref()
            .filter(|o| !o.is_empty())
        {
            return Ok(if windows {
                let text = o.to_string_lossy();
                let name = text.strip_prefix(r"\\.\pipe\").unwrap_or(&text).to_owned();
                Endpoint {
                    lock_path: facts.temp_dir.join(format!("{name}.lock")),
                    address: EndpointAddress::NamedPipe(name),
                    own_dir: None,
                    uid: None,
                }
            } else {
                let path = PathBuf::from(o);
                Endpoint {
                    lock_path: with_suffix(&path, ".lock"),
                    address: EndpointAddress::UnixSocket(path),
                    own_dir: None,
                    uid: facts.uid,
                }
            });
        }
        let suffix = if config.is_default {
            String::new()
        } else {
            let hash = fnv1a64(&dir_bytes(facts.platform, &config.dir));
            format!("-{}", &format!("{hash:016x}")[..8])
        };
        if windows {
            let user = facts.user.as_deref().unwrap_or("user");
            let name = format!("hikasa-{}{suffix}", sanitize(user));
            return Ok(Endpoint {
                lock_path: facts.temp_dir.join(format!("{name}.lock")),
                address: EndpointAddress::NamedPipe(name),
                own_dir: None,
                uid: None,
            });
        }
        let id = crate::paths::user_id(facts);
        let (dir, mut own_dir) = match facts.platform {
            Platform::MacOs => (facts.temp_dir.clone(), None),
            Platform::Linux | Platform::OtherUnix | Platform::Windows => {
                match facts
                    .xdg_runtime_dir
                    .as_ref()
                    .filter(|d| d.to_string_lossy().starts_with('/'))
                {
                    Some(runtime) => (runtime.clone(), None),
                    None => {
                        let own = facts.temp_dir.join(format!("hikasa-{id}"));
                        (own.clone(), Some(own))
                    }
                }
            }
        };
        let mut path = dir.join(format!("hikasa{suffix}.sock"));
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            let short = format!(
                "hk-{:016x}.sock",
                fnv1a64(path.as_os_str().as_encoded_bytes())
            );
            path = dir.join(&short);
            if path.as_os_str().len() > MAX_SOCKET_PATH {
                let own = PathBuf::from(format!("/tmp/hikasa-{id}"));
                path = own.join(&short);
                own_dir = Some(own);
            }
        }
        Ok(Endpoint {
            lock_path: with_suffix(&path, ".lock"),
            address: EndpointAddress::UnixSocket(path),
            own_dir,
            uid: facts.uid,
        })
    }

    /// Verifies the socket folder before anything talks through it (§2.23 "folder verification"; decision of the
    /// wave-4 review). Unix: the folder that holds the socket must be a folder owned by [`Endpoint::uid`] that no
    /// other user can write to (`mode & 0o022 == 0`); Hikasa's own folder ([`Endpoint::own_dir`]) must also be
    /// a real folder (not a symbolic link) with `mode & 0o077 == 0`. A symbolic link to the folder is accepted
    /// only when the link itself belongs to the same user. An unknown uid fails closed. A missing folder is
    /// [`IpcError::NotRunning`] (nobody can listen there); every other failure is
    /// [`IpcError::InsecureEndpoint`]. Windows (named pipes): nothing to verify.
    ///
    /// Only `std` metadata is used (no libc). The client calls it before connecting, so a socket planted by
    /// another user in a shared temp folder is never contacted; the server calls it before binding.
    pub fn verify_folder(&self) -> Result<(), IpcError> {
        check_folder(self)
    }

    /// Display text for messages (the socket path, or `\\.\pipe\<name>`).
    pub fn display(&self) -> String {
        match &self.address {
            EndpointAddress::UnixSocket(path) => path.display().to_string(),
            EndpointAddress::NamedPipe(name) => format!(r"\\.\pipe\{name}"),
        }
    }
}

/// FNV-1a 64-bit over bytes (stable everywhere; endpoint suffixes).
pub const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    hash
}

use crate::paths::sanitize;

/// `path` with `suffix` appended to its last component (`x.sock` → `x.sock.lock`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut text: OsString = path.as_os_str().to_owned();
    text.push(suffix);
    PathBuf::from(text)
}

/// The bytes hashed for the endpoint suffix: the OsStr bytes on Unix, UTF-16LE code units on Windows.
fn dir_bytes(platform: Platform, dir: &Path) -> Vec<u8> {
    match platform {
        Platform::Windows => utf16le(dir.as_os_str()),
        Platform::Linux | Platform::MacOs | Platform::OtherUnix => {
            dir.as_os_str().as_encoded_bytes().to_vec()
        }
    }
}

/// UTF-16LE code units of an OS string (exact on Windows; through the UTF-8 text elsewhere).
#[cfg(windows)]
fn utf16le(s: &OsStr) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().flat_map(u16::to_le_bytes).collect()
}

/// UTF-16LE code units of an OS string (exact on Windows; through the UTF-8 text elsewhere).
#[cfg(not(windows))]
fn utf16le(s: &OsStr) -> Vec<u8> {
    s.to_string_lossy()
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The folder that holds a socket path (`.` for a bare name).
#[cfg(unix)]
fn socket_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// [`Endpoint::verify_folder`] on Unix.
#[cfg(unix)]
fn check_folder(endpoint: &Endpoint) -> Result<(), IpcError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let EndpointAddress::UnixSocket(path) = &endpoint.address else {
        return Ok(());
    };
    let dir = socket_dir(path);
    let insecure = |reason| IpcError::InsecureEndpoint {
        path: dir.display().to_string(),
        reason,
    };
    let Some(uid) = endpoint.uid else {
        return Err(insecure(
            "the current user id is unknown, so the socket folder cannot be verified",
        ));
    };
    let not_found = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::NotFound {
            IpcError::NotRunning
        } else {
            IpcError::Io(e)
        }
    };
    let own = endpoint.own_dir.as_deref() == Some(dir.as_path());
    let link = std::fs::symlink_metadata(&dir).map_err(not_found)?;
    let meta = if link.file_type().is_symlink() {
        if own {
            return Err(insecure("the socket folder is a symbolic link"));
        }
        if link.uid() != uid {
            return Err(insecure(
                "the socket folder is a symbolic link owned by another user",
            ));
        }
        std::fs::metadata(&dir).map_err(not_found)?
    } else {
        link
    };
    if !meta.is_dir() {
        return Err(insecure("the socket folder is not a folder"));
    }
    if meta.uid() != uid {
        return Err(insecure("the socket folder belongs to another user"));
    }
    let mode = meta.permissions().mode();
    if own && mode & 0o077 != 0 {
        return Err(insecure("the socket folder is accessible by other users"));
    }
    if mode & 0o022 != 0 {
        return Err(insecure("the socket folder is writable by other users"));
    }
    Ok(())
}

/// Named pipes have no socket folder.
#[cfg(not(unix))]
fn check_folder(_endpoint: &Endpoint) -> Result<(), IpcError> {
    Ok(())
}

/// Creates Hikasa's own socket folder with mode 0700 (create-new: `mkdir` never follows or reuses a symbolic
/// link) and verifies it: a real folder (not a symbolic link) owned by [`Endpoint::uid`] that no other user can
/// access (`mode & 0o077 == 0`). A folder another user created first is refused, never used. No-op without an
/// own folder.
#[cfg(unix)]
pub(crate) fn prepare_own_dir(endpoint: &Endpoint) -> Result<(), IpcError> {
    use std::os::unix::fs::DirBuilderExt;
    let Some(dir) = &endpoint.own_dir else {
        return Ok(());
    };
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(IpcError::Io(e)),
    }
    let insecure = |reason| IpcError::InsecureEndpoint {
        path: dir.display().to_string(),
        reason,
    };
    let meta = std::fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() {
        return Err(insecure("the socket folder is a symbolic link"));
    }
    check_folder(&Endpoint {
        address: EndpointAddress::UnixSocket(dir.join("hikasa.sock")),
        lock_path: endpoint.lock_path.clone(),
        own_dir: Some(dir.clone()),
        uid: endpoint.uid,
    })
}

/// Windows uses named pipes: there is no socket folder.
#[cfg(not(unix))]
pub(crate) fn prepare_own_dir(_endpoint: &Endpoint) -> Result<(), IpcError> {
    Ok(())
}

/// Single-instance lock (ARCH-090): an exclusive `File::try_lock` on `Endpoint::lock_path`, held for the
/// server's lifetime. Race-free, unlike probing the socket.
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
}

impl InstanceLock {
    /// `Ok(None)` when another live instance holds the lock.
    ///
    /// Hikasa's own socket folder is created (mode 0700) and verified first, since the lock file lives next to
    /// the socket. The lock is released when the value is dropped (or the process ends).
    pub fn try_acquire(endpoint: &Endpoint) -> Result<Option<InstanceLock>, IpcError> {
        prepare_own_dir(endpoint)?;
        let file = open_lock_file(&endpoint.lock_path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(InstanceLock { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(IpcError::Io(e)),
        }
    }

    /// The lock file (diagnostics).
    pub fn file(&self) -> &File {
        &self.file
    }
}

/// Opens (creating it, mode 0600 on Unix) the lock file without truncating it.
fn open_lock_file(path: &Path) -> Result<File, IpcError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
