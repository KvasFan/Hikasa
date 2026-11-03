//! Path completion, browse places and archive names (M2 W9a, D-239, D-240): the typed forms of `vfs ls` the window's
//! path edits and browse dialog use, so every back-end (archives too) completes and browses the same way. The
//! splitting of the typed text and the matching rule are pure ([`split_typed`], [`completion_matches`]); the Service
//! calls list in a job.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::path::Path;

use hikasa_core::case::{fold, nfc};
use hikasa_core::spec::PathStyle;
use hikasa_vfs::{ArchiveFormat, Capabilities, EntryKind, ListOptions, Vfs as _};
use serde::{Deserialize, Serialize};

use super::{ApiError, JobControl, Service, run_guarded};
use crate::host::Platform;
use crate::settings::state::{MruList, State};
use hikasa_vfs::SpecUse;

/// One completion of a typed path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    /// The entry's name.
    pub name: String,
    /// The whole text to put in the field: the typed folder part, the name, and a separator after a folder so that
    /// typing goes on.
    pub text: String,
    /// A folder.
    pub folder: bool,
    /// A file whose name the archive masks know (it opens as a folder, D-240).
    pub archive: bool,
}

/// The completions of a typed path.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CompletionDto {
    /// The folder part as typed (`""`: the current folder).
    pub base: String,
    /// At most `limit` items: folders and archives first, then by the D-20 key.
    pub items: Vec<Completion>,
    /// More entries matched than `limit`.
    pub truncated: bool,
}

/// What a place of the browse dialog is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaceKind {
    /// The home folder.
    Home,
    /// The current folder.
    Current,
    /// `/` (Unix).
    Root,
    /// A drive letter that exists (Windows).
    Drive,
    /// A base of the view (the window adds them).
    Base,
    /// `state.mru.folders`.
    Recent,
}

/// A place of the browse dialog (D-240).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceDto {
    /// What the list shows.
    pub label: String,
    /// The spec it opens.
    pub spec: String,
    /// Its kind.
    pub kind: PlaceKind,
}

/// The separators of a path style: `/`; Windows also `\`.
fn is_separator(c: char, style: PathStyle) -> bool {
    c == '/' || (style == PathStyle::Windows && c == '\\')
}

/// Splits typed text into its folder part (up to and including the last separator; on Windows a bare drive `C:` too)
/// and the beginning of a name. `""` as the folder part is the current folder. Pure, so the Windows forms are tested on
/// every host: `C:\a\b` → (`C:\a\`, `b`); `C:x` → (`C:`, `x`); `\\srv\share\d` → (`\\srv\share\`, `d`).
pub fn split_typed(typed: &str, style: PathStyle) -> (&str, &str) {
    if let Some((at, c)) = typed
        .char_indices()
        .rev()
        .find(|&(_, c)| is_separator(c, style))
    {
        let cut = at + c.len_utf8();
        return (
            typed.get(..cut).unwrap_or(""),
            typed.get(cut..).unwrap_or(""),
        );
    }
    if style == PathStyle::Windows {
        let bytes = typed.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return (typed.get(..2).unwrap_or(""), typed.get(2..).unwrap_or(""));
        }
    }
    ("", typed)
}

/// True when `name` starts with `prefix` by the back-end's rule: exact NFC when case-sensitive, else case-folded NFC.
pub fn completion_matches(name: &str, prefix: &str, case_sensitive: bool) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let name = nfc(name);
    let prefix = nfc(prefix);
    if case_sensitive {
        name.starts_with(prefix.as_ref())
    } else {
        fold(&name).starts_with(fold(&prefix).as_ref())
    }
}

/// The order of completions: folders and archives first, then by the case-folded NFC name, then the exact name.
fn completion_order(a: &Completion, b: &Completion) -> Ordering {
    let rank = |c: &Completion| u8::from(!(c.folder || c.archive));
    rank(a)
        .cmp(&rank(b))
        .then_with(|| fold(&nfc(&a.name)).cmp(&fold(&nfc(&b.name))))
        .then_with(|| a.name.cmp(&b.name))
}

/// The browse dialog's places without the view's bases (pure; the drives come through `probe`, so a test gives a
/// table): Home, the current folder, `/` on Unix or every drive `A:`…`Z:` the probe finds on Windows, then the recent
/// folders.
pub fn places(
    platform: Platform,
    home: Option<&Path>,
    cwd: Option<&Path>,
    probe: &dyn Fn(&str) -> bool,
    recent: &[String],
) -> Vec<PlaceDto> {
    let mut out = Vec::new();
    if let Some(home) = home {
        out.push(PlaceDto {
            label: "Home".into(),
            spec: home.to_string_lossy().into_owned(),
            kind: PlaceKind::Home,
        });
    }
    if let Some(cwd) = cwd {
        out.push(PlaceDto {
            label: "Current folder".into(),
            spec: cwd.to_string_lossy().into_owned(),
            kind: PlaceKind::Current,
        });
    }
    if platform == Platform::Windows {
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:\\", char::from(letter));
            if probe(&drive) {
                out.push(PlaceDto {
                    label: drive.trim_end_matches('\\').to_owned(),
                    spec: drive,
                    kind: PlaceKind::Drive,
                });
            }
        }
    } else {
        out.push(PlaceDto {
            label: "/".into(),
            spec: "/".into(),
            kind: PlaceKind::Root,
        });
    }
    for folder in recent {
        out.push(PlaceDto {
            label: folder.clone(),
            spec: folder.clone(),
            kind: PlaceKind::Recent,
        });
    }
    out
}

impl Service {
    /// The entries of the typed text's folder part whose names start with its last part (case-insensitive where the
    /// back-end is), folders and archives first, then by the D-20 key; at most `limit` (D-239). The folder part is
    /// parsed as a folder spec (`~`, archives and their members included, as `vfs ls` does); a folder that does not
    /// exist is `not_found`. Names that are not valid Unicode are left out (they cannot be typed back). Entries whose
    /// kind the folder record gives are not stat'ed.
    pub fn path_completions(
        &self,
        typed: &OsStr,
        limit: usize,
        job: &JobControl,
    ) -> Result<CompletionDto, ApiError> {
        run_guarded(|| {
            let style = self.env().platform.path_style();
            let typed = typed.to_string_lossy();
            let (base, prefix) = split_typed(&typed, style);
            let folder_text = if base.is_empty() { "." } else { base };
            let spec = self.parse_spec_in(OsStr::new(folder_text), SpecUse::Folder, &job.cancel)?;
            let vfs = self.vfs();
            let case_sensitive = vfs
                .capabilities(&spec)
                .map_err(ApiError::from_vfs)?
                .contains(Capabilities::CASE_SENSITIVE);
            // Every entry the record types is declined: its name and kind are all a completion needs.
            let decline = |_: &str, _: EntryKind| false;
            let options = ListOptions {
                follow: hikasa_vfs::FollowLinks::Yes,
                prefilter: Some(&decline),
            };
            let listing = vfs
                .list(&spec, &options, &job.cancel)
                .map_err(ApiError::from_vfs)?;
            let named = listing
                .entries
                .iter()
                .map(|e| (&e.name, e.kind))
                .chain(listing.name_only.iter().map(|n| (&n.name, n.kind)));
            let separator = if style == PathStyle::Windows {
                '\\'
            } else {
                '/'
            };
            let mut items = Vec::new();
            for (name, kind) in named {
                if job.cancel.is_cancelled() {
                    return Err(ApiError::Cancelled);
                }
                if name.is_lossy() || !completion_matches(name.as_str(), prefix, case_sensitive) {
                    continue;
                }
                let folder = kind == EntryKind::Dir;
                let archive = !folder && vfs.archive_format(name.as_str()).is_some();
                let mut text = format!("{base}{}", name.as_str());
                if folder {
                    text.push(separator);
                }
                items.push(Completion {
                    name: name.as_str().to_owned(),
                    text,
                    folder,
                    archive,
                });
            }
            items.sort_by(completion_order);
            let truncated = items.len() > limit;
            items.truncate(limit);
            Ok(CompletionDto {
                base: base.to_owned(),
                items,
                truncated,
            })
        })
    }

    /// The archive format the `archives.toml` masks give a file name (D-240: the browse dialog's zipper), in memory.
    pub fn archive_format_of(&self, name: &str) -> Option<ArchiveFormat> {
        self.vfs().archive_format(name)
    }

    /// The browse dialog's places (D-240): Home, the current folder, `/` or the drives found (probed in the job on
    /// Windows), then `state.mru.folders`. The window adds the view's bases.
    pub fn browse_places(&self, job: &JobControl) -> Vec<PlaceDto> {
        let env = self.env();
        let probe = |drive: &str| {
            !job.cancel.is_cancelled()
                && self
                    .vfs_stat_in(OsStr::new(drive), hikasa_vfs::FollowLinks::Yes, job)
                    .is_ok()
        };
        let recent = self
            .state_snapshot()
            .map(|state| state.mru(MruList::Folders).to_vec())
            .unwrap_or_default();
        places(
            env.platform,
            env.home.as_deref(),
            env.cwd.as_deref(),
            &probe,
            &recent,
        )
    }

    /// `state.toml` as it is on disk, leniently loaded (defaults when it cannot be read).
    fn state_snapshot(&self) -> Option<State> {
        self.settings_store()
            .file::<State>(crate::settings::LoadMode::Lenient)
            .ok()
            .map(|file| file.value().clone())
    }
}
