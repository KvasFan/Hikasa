//! Change detection by polling (D-74, Q4 = a): [`ChangeStamp`], [`ConfigWatch`], [`ConfigChange`],
//! [`ChangeKind`].
//!
//! A stamp is the length, the modification time and the CRC-32 of a file's bytes; a missing file is
//! [`ChangeStamp::Absent`]. [`ConfigWatch::poll`] re-reads the stamps of the files it tracks; it holds no timer
//! and no thread (the adapters call it on window focus and every 2 s, D-62).
//!
//! *W1 note:* the contract computes the CRC on an unchanged length and time only when the file was modified less
//! than 2 s before the previous poll. Core code may not read the clock (T1), so `poll` computes the CRC whenever
//! length and time are unchanged; settings files are a few kilobytes, so this costs little and catches every
//! same-size, same-time edit.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use hikasa_core::Ctl;
use hikasa_vfs::{StreamOptions, crc32_reader};

use crate::error::SessionError;

/// The identity of a file's content at one moment (D-74).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeStamp {
    /// The file does not exist.
    Absent,
    /// The file exists.
    Present {
        /// Length in bytes.
        len: u64,
        /// Modification time in nanoseconds since the Unix epoch (none when the platform has none).
        modified_ns: Option<i128>,
        /// CRC-32 of the bytes.
        crc32: u32,
    },
}

impl ChangeStamp {
    /// The stamp of `path` now (reads the whole file). A missing file is `Absent`; other errors are
    /// `SessionError::Io`.
    pub fn of(path: &Path) -> Result<ChangeStamp, SessionError> {
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ChangeStamp::Absent),
            Err(e) => return Err(SessionError::io(path, e)),
        };
        let modified_ns = modified_ns(&metadata);
        match std::fs::read(path) {
            Ok(bytes) => Ok(ChangeStamp::of_bytes(&bytes, modified_ns)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ChangeStamp::Absent),
            Err(e) => Err(SessionError::io(path, e)),
        }
    }

    /// The stamp of `bytes` with the modification time `modified_ns`.
    pub fn of_bytes(bytes: &[u8], modified_ns: Option<i128>) -> ChangeStamp {
        let mut reader = bytes;
        let crc32 = crc32_reader(&mut reader, &StreamOptions::default(), &Ctl::detached())
            .unwrap_or_else(|_| crc32_fallback(bytes));
        ChangeStamp::Present {
            len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            modified_ns,
            crc32,
        }
    }
}

/// CRC-32 (IEEE) bit by bit; only used if the stream helper fails, which it cannot for an in-memory reader.
fn crc32_fallback(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// The modification time of `metadata` in nanoseconds since the Unix epoch (negative before 1970). The one
/// spelling of the time of every stamp (a load and a later [`ChangeStamp::of`] must agree).
pub(crate) fn modified_ns(metadata: &std::fs::Metadata) -> Option<i128> {
    let modified = metadata.modified().ok()?;
    Some(match modified.duration_since(UNIX_EPOCH) {
        Ok(after) => i128::try_from(after.as_nanos()).unwrap_or(i128::MAX),
        Err(before) => -i128::try_from(before.duration().as_nanos()).unwrap_or(i128::MAX),
    })
}

/// What happened to a tracked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// It exists now and did not before.
    Created,
    /// Its content changed (or it could not be read).
    Changed,
    /// It existed and is gone.
    Removed,
}

/// One change found by [`ConfigWatch::poll`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigChange {
    /// The file.
    pub path: PathBuf,
    /// What happened.
    pub kind: ChangeKind,
}

/// One tracked file.
#[derive(Debug, Clone)]
struct Tracked {
    path: PathBuf,
    stamp: ChangeStamp,
    /// The file could not be read at the last poll (reported once).
    unreadable: bool,
}

/// Polls the stamps of a set of files and folders of format files (D-74). No timer, no thread.
#[derive(Debug, Clone, Default)]
pub struct ConfigWatch {
    files: Vec<Tracked>,
    /// Folders whose `*.toml` files are tracked; listed again at each poll.
    folders: Vec<PathBuf>,
}

impl ConfigWatch {
    /// Tracks `paths`, stamped now.
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> ConfigWatch {
        let mut watch = ConfigWatch::default();
        for path in paths {
            watch.track(path);
        }
        watch
    }

    /// Tracks one more file, stamped now (an unreadable file starts as `Absent`).
    pub fn track(&mut self, path: PathBuf) {
        if self.files.iter().any(|t| t.path == path) {
            return;
        }
        let stamp = ChangeStamp::of(&path).unwrap_or(ChangeStamp::Absent);
        self.files.push(Tracked {
            path,
            stamp,
            unreadable: false,
        });
    }

    /// Tracks every `*.toml` file of `folder`, now and at each poll (new files are `Created`, removed ones
    /// `Removed`).
    pub fn track_folder(&mut self, folder: PathBuf) {
        for path in toml_files(&folder) {
            self.track(path);
        }
        if !self.folders.contains(&folder) {
            self.folders.push(folder);
        }
    }

    /// The tracked files, in tracking order.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|t| t.path.as_path())
    }

    /// Re-reads every stamp and reports the differences, sorted by path. An unreadable file is reported once as
    /// `Changed` and keeps its old stamp.
    pub fn poll(&mut self) -> Vec<ConfigChange> {
        for folder in self.folders.clone() {
            for path in toml_files(&folder) {
                if !self.files.iter().any(|t| t.path == path) {
                    self.files.push(Tracked {
                        path,
                        stamp: ChangeStamp::Absent,
                        unreadable: false,
                    });
                }
            }
        }
        let mut changes = Vec::new();
        for tracked in &mut self.files {
            match ChangeStamp::of(&tracked.path) {
                Ok(now) => {
                    tracked.unreadable = false;
                    let kind = match (&tracked.stamp, &now) {
                        (a, b) if a == b => None,
                        (ChangeStamp::Absent, _) => Some(ChangeKind::Created),
                        (_, ChangeStamp::Absent) => Some(ChangeKind::Removed),
                        _ => Some(ChangeKind::Changed),
                    };
                    tracked.stamp = now;
                    if let Some(kind) = kind {
                        changes.push(ConfigChange {
                            path: tracked.path.clone(),
                            kind,
                        });
                    }
                }
                Err(_) => {
                    if !tracked.unreadable {
                        tracked.unreadable = true;
                        changes.push(ConfigChange {
                            path: tracked.path.clone(),
                            kind: ChangeKind::Changed,
                        });
                    }
                }
            }
        }
        // Files of a tracked folder that are gone and were reported stop being tracked.
        let folders = &self.folders;
        self.files.retain(|t| {
            t.stamp != ChangeStamp::Absent
                || !t
                    .path
                    .parent()
                    .is_some_and(|p| folders.iter().any(|f| f == p))
        });
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        changes
    }

    /// Takes the current stamp of `path` without reporting it (after our own save).
    pub fn acknowledge(&mut self, path: &Path) {
        if let Some(tracked) = self.files.iter_mut().find(|t| t.path == path) {
            if let Ok(stamp) = ChangeStamp::of(path) {
                tracked.stamp = stamp;
                tracked.unreadable = false;
            }
        } else {
            self.track(path.to_path_buf());
        }
    }
}

/// The `*.toml` files of `folder` (none when it cannot be listed), sorted.
fn toml_files(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
                && p.is_file()
        })
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_of_bytes() {
        let a = ChangeStamp::of_bytes(b"123456789", Some(5));
        assert_eq!(
            a,
            ChangeStamp::Present {
                len: 9,
                modified_ns: Some(5),
                crc32: 0xCBF4_3926
            }
        );
        assert_eq!(crc32_fallback(b"123456789"), 0xCBF4_3926);
        assert_ne!(a, ChangeStamp::of_bytes(b"123456780", Some(5)));
    }
}
