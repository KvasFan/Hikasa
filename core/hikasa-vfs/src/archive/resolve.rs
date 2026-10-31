//! Spec resolution (normative, D-138; §1.5.4): maps the local path of a typed spec to the spec that names the item.
//!
//! [`resolve_with`] is the pure core: it sees the world only through [`Probe`], so every row runs on every platform
//! and in both path styles. [`resolve_spec`] plugs a [`Vfs`] into it. Deterministic: the same tree gives the same
//! answer on any thread. It never fails; a problem is reported when the spec is used.

use std::sync::atomic::{AtomicBool, Ordering};

use hikasa_core::{CancelToken, Cancelled, Ctl, LocalPath, NullProgress, PathStyle, Spec};

use super::format::ArchiveFormat;
use crate::entry::EntryKind;
use crate::error::VfsError;
use crate::vfs::{FollowLinks, Vfs};

/// What the caller wants of the named item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecUse {
    /// The named item, whatever it is: an archive file stays a file.
    Any,
    /// A folder is wanted: an archive file is its root (`folder compare`, `vfs ls`, an input ending in a separator).
    Folder,
}

/// What a probe found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probed {
    /// A folder.
    Dir,
    /// A regular file.
    File,
    /// Anything else (a link member, a device, a broken link).
    Other,
    /// Nothing there; `NotFound` and `NotADirectory` alike, so a file used as a folder behaves the same on every
    /// platform.
    Missing,
    /// It could not be determined (permission, I/O).
    Failed,
}

/// The world as resolution sees it.
pub trait Probe {
    /// The kind of `spec` (links on disk followed; link members are [`Probed::Other`]).
    fn kind(&self, spec: &Spec) -> Probed;
    /// The archive format of a file name per the masks.
    fn format(&self, name: &str) -> Option<ArchiveFormat>;
    /// Opens the archive `archive` (`prepare`); `false` when it cannot be read.
    fn open(&self, archive: &Spec) -> bool;
}

/// The name of `path` as text; `None` for a root or a name that is not UTF-8, which no member can be.
fn text_name(path: &LocalPath, style: PathStyle) -> Option<String> {
    path.file_name_in(style)?.into_string().ok()
}

/// Resolves `spec` (the algorithm of §1.5.4, in `style`).
pub fn resolve_with(probe: &dyn Probe, spec: Spec, use_: SpecUse, style: PathStyle) -> Spec {
    let Spec::Local(path) = &spec else {
        return spec;
    };
    let name = text_name(path, style);
    match probe.kind(&spec) {
        Probed::Dir | Probed::Other | Probed::Failed => return spec,
        Probed::File => {
            let archive = use_ == SpecUse::Folder
                && name.as_deref().is_some_and(|n| probe.format(n).is_some());
            return if archive {
                Spec::archive_member(spec, "")
            } else {
                spec
            };
        }
        Probed::Missing => {}
    }
    let Some(name) = name else {
        return spec;
    };
    let mut rest = vec![name];
    let mut parent = path.parent_in(style);
    while let Some(at) = parent {
        if rest.first().is_some_and(|n| n == "..") {
            // `..` stays in a Unix path (D-06), so nothing below it can name a member: a spec never walks out of
            // an archive. This also ends the walk, because the parent of `/a/..` is `/a/../..`.
            return spec;
        }
        let holder = Spec::Local(at.clone());
        match probe.kind(&holder) {
            Probed::Missing => {
                let Some(name) = text_name(&at, style) else {
                    return spec;
                };
                rest.insert(0, name);
                parent = at.parent_in(style);
            }
            Probed::Dir | Probed::Other | Probed::Failed => return spec,
            Probed::File => {
                return match text_name(&at, style).and_then(|n| probe.format(&n)) {
                    Some(_) => descend(probe, holder, &rest, use_, &spec),
                    None => spec,
                };
            }
        }
    }
    spec
}

/// Step 4: `rest` is what follows the archive file or member `archive`.
fn descend(
    probe: &dyn Probe,
    archive: Spec,
    rest: &[String],
    use_: SpecUse,
    original: &Spec,
) -> Spec {
    if rest
        .iter()
        .any(|name| name.is_empty() || name == "." || name == "..")
    {
        return original.clone();
    }
    let member = |archive: &Spec| Spec::archive_member(archive.clone(), rest.join("/"));
    let Some((last, before)) = rest.split_last() else {
        return original.clone();
    };
    let plain = before.iter().all(|name| probe.format(name).is_none());
    if plain && (use_ == SpecUse::Any || probe.format(last).is_none()) {
        return member(&archive);
    }
    if !probe.open(&archive) {
        return member(&archive);
    }
    for (at, name) in rest.iter().enumerate() {
        let prefix = rest.get(..=at).unwrap_or(rest).join("/");
        let here = Spec::archive_member(archive.clone(), prefix);
        match probe.kind(&here) {
            Probed::Dir => {}
            Probed::File if probe.format(name).is_some() => {
                let left = rest.get(at + 1..).unwrap_or(&[]);
                return if !left.is_empty() {
                    descend(probe, here, left, use_, original)
                } else if use_ == SpecUse::Folder {
                    Spec::archive_member(here, "")
                } else {
                    member(&archive)
                };
            }
            Probed::File | Probed::Other | Probed::Missing | Probed::Failed => {
                return member(&archive);
            }
        }
    }
    member(&archive)
}

/// A [`Probe`] over a [`Vfs`].
struct VfsProbe<'a> {
    vfs: &'a dyn Vfs,
    ctl: Ctl<'a>,
    cancelled: AtomicBool,
}

impl Probe for VfsProbe<'_> {
    fn kind(&self, spec: &Spec) -> Probed {
        match self.vfs.stat(spec, FollowLinks::Yes) {
            Ok(entry) => match entry.kind {
                EntryKind::Dir => Probed::Dir,
                EntryKind::File => Probed::File,
                EntryKind::Symlink | EntryKind::Other => Probed::Other,
            },
            Err(VfsError::NotFound { .. } | VfsError::NotADirectory { .. }) => Probed::Missing,
            Err(_) => Probed::Failed,
        }
    }

    fn format(&self, name: &str) -> Option<ArchiveFormat> {
        self.vfs.archive_format(name)
    }

    fn open(&self, archive: &Spec) -> bool {
        let root = Spec::archive_member(archive.clone(), "");
        match self.vfs.prepare(&root, &self.ctl) {
            Ok(()) => true,
            Err(error) => {
                if error.is_cancelled() {
                    self.cancelled.store(true, Ordering::Relaxed);
                }
                false
            }
        }
    }
}

/// Resolves `spec` through `vfs` (`stat`, `archive_format`, `prepare` only; no listing, no current folder, no
/// environment). At most one `stat` per path component plus one member lookup per component inside archives; in
/// the common case (`a.tar.gz/x.txt`) the archive is not opened. Only `cancel` stops it.
pub fn resolve_spec(
    vfs: &dyn Vfs,
    spec: Spec,
    use_: SpecUse,
    cancel: &CancelToken,
) -> Result<Spec, Cancelled> {
    let progress = NullProgress;
    let probe = VfsProbe {
        vfs,
        ctl: Ctl {
            cancel,
            progress: &progress,
        },
        cancelled: AtomicBool::new(false),
    };
    let resolved = resolve_with(&probe, spec, use_, PathStyle::HOST);
    if probe.cancelled.load(Ordering::Relaxed) {
        return Err(Cancelled);
    }
    Ok(resolved)
}
