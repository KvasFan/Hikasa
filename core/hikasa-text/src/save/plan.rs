//! Save plans and their execution (T12; D-273, D-275…D-277; §1.10.13–§1.10.14).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::progress::Ctl;
use hikasa_core::{ErrorKind, ErrorReport, LocalPath, Side, Spec, Warning, WarningCode};
use hikasa_formats::{ConvertError, Encoder, ExternalConversion, ProcessRunner, run_external};
use hikasa_vfs::ops::{
    self, Action, BackupNames, BatchOptions, ExecOptions, Expect, ItemStamp, ItemStatus, OpKind,
    Plan, PlanItem, PlanOptions, Preserve, SkipReason, spec_key,
};
use hikasa_vfs::{DosAttrs, Entry, EntryKind, FollowLinks, Vfs, VfsError};

use super::content::{ContentIssue, ContentOptions, Piece, ReencodeReason, SaveContent, SourceMap};
use super::{
    ItemContent, SaveContentSummary, SaveIssue, SaveItem, SaveMode, SavePlan, SaveReport,
    SaveRequest, SaveSource, UnmappablePolicy, save_content,
};
use crate::document::{LineIdx, LoadStamp};
use crate::edit::{EditableDocument, SavedFile, SpliceSource};
use crate::error::TextError;

/// Most symbolic links followed when a save resolves its target (the Linux limit).
const MAX_LINK_HOPS: usize = 40;

/// The reason of a target that is not a regular file (§1.10.14).
const NOT_REGULAR: &str = "is not a regular file";

/// What [`plan_save_with`] needs beyond the VFS: where an External save conversion runs (FF-030, D-39), injected
/// (hikasa-text starts no process, §1.10.25).
#[derive(Clone, Copy, Default)]
pub struct SaveEnv<'a> {
    /// A private temporary folder and the process runner of [`hikasa_formats::run_external`].
    pub external: Option<(&'a Path, &'a dyn ProcessRunner)>,
}

impl std::fmt::Debug for SaveEnv<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveEnv")
            .field("external", &self.external.map(|(dir, _)| dir))
            .finish()
    }
}

/// [`plan_save_with`] without an External conversion runner: an item whose request has one fails.
pub fn plan_save(
    vfs: &dyn Vfs,
    docs: &[(Option<Side>, &EditableDocument, &SaveRequest)],
    ctl: &Ctl<'_>,
) -> Result<SavePlan, TextError> {
    plan_save_with(vfs, docs, &SaveEnv::default(), ctl)
}

/// Plans the save of `docs`, one item each, as one plan (§1.10.13; run as a job, `JobKind::TextSave`): the target
/// (Save As, the view's `--savetarget`, in place; a symbolic link followed, `via_link` naming it; what is not a
/// regular file is an item error), its `stat`, the splice source read once (its physical lines and CRC-32: a source
/// whose CRC or size is not its stamp's changed), the [`SaveContent`], the backup (`BackupNames::assign` with
/// `rel = None`, one generation; an archive's one backup), the questions of §1.10.14 and the warnings. The item's
/// `expect.target` is the stamp seen now, so a change between plan and execution fails it. Nothing is written.
/// Item failures are item errors; the call fails only when it is cancelled.
pub fn plan_save_with(
    vfs: &dyn Vfs,
    docs: &[(Option<Side>, &EditableDocument, &SaveRequest)],
    env: &SaveEnv<'_>,
    ctl: &Ctl<'_>,
) -> Result<SavePlan, TextError> {
    ctl.check()?;
    // Targets first, so a backup never takes a path of another item.
    let targets: Vec<Result<Resolved, ErrorReport>> = docs
        .iter()
        .map(|(_, doc, req)| resolve(vfs, doc, req))
        .collect();
    let mut names = BackupNames::default();
    for resolved in targets.iter().flatten() {
        names.reserve(vfs, &resolved.target);
    }
    let mut out = Out::default();
    let mut seen = BTreeSet::new();
    let mut archive_backups: BTreeMap<String, (Spec, bool)> = BTreeMap::new();
    for ((side, doc, req), resolved) in docs.iter().zip(targets) {
        ctl.check()?;
        let resolved = match resolved {
            Ok(resolved) => resolved,
            Err(error) => {
                out.failed(*side, doc, fallback_target(doc, req), error);
                continue;
            }
        };
        if !seen.insert(spec_key(vfs, &resolved.target)) {
            out.skipped(*side, doc, resolved, SkipReason::DuplicateTarget);
            continue;
        }
        plan_one(
            vfs,
            *side,
            doc,
            req,
            resolved,
            env,
            &mut names,
            &mut archive_backups,
            &mut out,
            ctl,
        )?;
    }
    let mut exec = ExecOptions::default();
    for (outer, (backup, stands)) in archive_backups {
        if stands {
            // What stands at the archive's backup is lost by its rewrite: each item that rewrites it asks.
            for (item, save) in out.plan.iter().zip(&mut out.items) {
                if item.error.is_none()
                    && item.skip.is_none()
                    && item
                        .target
                        .outermost_file()
                        .is_some_and(|o| o.canonical() == outer)
                {
                    save.issues.push(SaveIssue::OverwriteBackup {
                        backup: backup.clone(),
                    });
                }
            }
        }
        exec.archive_backups.insert(outer, backup);
    }
    let mut plan = SavePlan {
        plan: Plan {
            op: OpKind::Batch,
            options: PlanOptions::Batch(BatchOptions {
                preserve: Preserve::NONE,
                ..BatchOptions::default()
            }),
            items: out.plan,
            warnings: Vec::new(),
        },
        items: out.items,
        exec,
        warnings: out.warnings,
        confirmed: false,
        contents: out.contents,
    };
    let confirmed = plan.runs().all(|(_, item)| item.questions().is_empty());
    plan.confirmed = confirmed;
    Ok(plan)
}

/// The items being built.
#[derive(Default)]
struct Out {
    plan: Vec<PlanItem>,
    items: Vec<SaveItem>,
    contents: Vec<Option<ItemContent>>,
    warnings: Vec<Warning>,
}

impl Out {
    fn summary_of(doc: &EditableDocument) -> SaveContentSummary {
        let (encoding, bom) = doc.encoding();
        SaveContentSummary {
            bytes: 0,
            encoding,
            bom,
            eols: *doc.document().eol(),
            spliced_bytes: 0,
            encoded_lines: 0,
            pre_save: doc.pre_save_lines(),
        }
    }

    fn failed(
        &mut self,
        side: Option<Side>,
        doc: &EditableDocument,
        target: Spec,
        error: ErrorReport,
    ) {
        let mut item = PlanItem::planned(
            Action::Write,
            EntryKind::File,
            Some(target.clone()),
            target.clone(),
            0,
        );
        item.error = Some(error);
        self.plan.push(item);
        self.items.push(SaveItem {
            side,
            target,
            via_link: None,
            creates: false,
            mode: SaveMode::InPlace,
            content: Out::summary_of(doc),
            issues: Vec::new(),
            external: None,
            generation: doc.generation(),
        });
        self.contents.push(None);
    }

    fn skipped(
        &mut self,
        side: Option<Side>,
        doc: &EditableDocument,
        resolved: Resolved,
        reason: SkipReason,
    ) {
        let mut item = PlanItem::planned(
            Action::Skip,
            EntryKind::File,
            Some(resolved.target.clone()),
            resolved.target.clone(),
            0,
        );
        item.skip = Some(reason);
        self.plan.push(item);
        self.items.push(SaveItem {
            side,
            target: resolved.target,
            via_link: resolved.via_link,
            creates: resolved.entry.is_none(),
            mode: resolved.mode,
            content: Out::summary_of(doc),
            issues: Vec::new(),
            external: None,
            generation: doc.generation(),
        });
        self.contents.push(None);
    }
}

/// A target as planning resolved it.
struct Resolved {
    mode: SaveMode,
    /// The file written.
    target: Spec,
    /// The link it was asked through.
    via_link: Option<Spec>,
    /// What stands at the target (`None`: absent).
    entry: Option<Entry>,
}

/// The spec an item without a target names: the document's own, else its origin, else standard input.
fn fallback_target(doc: &EditableDocument, req: &SaveRequest) -> Spec {
    req.target
        .clone()
        .or_else(|| doc.save_target().cloned())
        .or_else(|| doc.location().cloned())
        .or_else(|| doc.document().origin().spec.clone())
        .unwrap_or(Spec::Stdin)
}

/// The report of a VFS error naming its spec.
fn report(error: &VfsError) -> ErrorReport {
    let mut report = ErrorReport::capture(error, None);
    report.spec = error.spec().map(str::to_owned);
    report
}

/// A report of its own.
fn report_with(kind: ErrorKind, spec: &Spec, message: String) -> ErrorReport {
    ErrorReport {
        kind,
        message,
        spec: Some(spec.canonical()),
        causes: Vec::new(),
    }
}

/// The target of a request (§1.10.14, D-275): Save As, the view's save target, the location; links followed.
fn resolve(
    vfs: &dyn Vfs,
    doc: &EditableDocument,
    req: &SaveRequest,
) -> Result<Resolved, ErrorReport> {
    let (mode, asked) = if let Some(target) = &req.target {
        (SaveMode::SaveAs, target.clone())
    } else if let Some(target) = doc.save_target() {
        (SaveMode::SaveTarget, target.clone())
    } else if let (true, Some(location)) = (doc.can_save_in_place(), doc.location()) {
        (SaveMode::InPlace, location.clone())
    } else {
        let spec = fallback_target(doc, req);
        return Err(report_with(
            ErrorKind::InvalidInput,
            &spec,
            "the document has no file to save in place: Save As needs a target".to_owned(),
        ));
    };
    let (target, entry) = follow_links(vfs, &asked).map_err(|e| report(&e))?;
    if let Some(found) = &entry
        && found.kind != EntryKind::File
    {
        return Err(report(&VfsError::InvalidTarget {
            spec: target.canonical(),
            reason: NOT_REGULAR,
        }));
    }
    let via_link = (target != asked).then_some(asked);
    Ok(Resolved {
        mode,
        target,
        via_link,
        entry,
    })
}

/// The file a chain of symbolic links ends at (as `open_write` with `replace_link = false` writes it, docs/11 "Local
/// writes"), with what stands there (`None`: absent; a dangling link names the file to create). A relative link
/// target is taken relative to the link's folder. Only local paths hold links.
fn follow_links(vfs: &dyn Vfs, spec: &Spec) -> Result<(Spec, Option<Entry>), VfsError> {
    let mut current = spec.clone();
    for _ in 0..MAX_LINK_HOPS {
        let entry = match vfs.stat(&current, FollowLinks::No) {
            Ok(entry) => entry,
            Err(VfsError::NotFound { .. }) => return Ok((current, None)),
            Err(e) => return Err(e),
        };
        if entry.kind != EntryKind::Symlink {
            return Ok((current, Some(entry)));
        }
        let Spec::Local(path) = &current else {
            return Ok((current, Some(entry)));
        };
        let text = PathBuf::from(entry.link_target_os().unwrap_or_default());
        let joined = match path.as_path().parent() {
            Some(dir) if text.is_relative() => dir.join(&text),
            _ => text,
        };
        let normalized = joined
            .to_str()
            .and_then(|s| {
                hikasa_core::normalize_local(s, hikasa_core::PathStyle::HOST, None, None).ok()
            })
            .map_or(joined.clone(), PathBuf::from);
        current = match LocalPath::new(normalized) {
            Ok(local) => Spec::Local(local),
            Err(_) => return Ok((current, Some(entry))),
        };
    }
    Err(VfsError::InvalidTarget {
        spec: spec.canonical(),
        reason: "is a chain of too many symbolic links",
    })
}

/// True when `a` and `b` name one file on `vfs` (their links followed).
fn same_file(vfs: &dyn Vfs, a: &Spec, b: &Spec) -> bool {
    let resolved = |spec: &Spec| follow_links(vfs, spec).map_or_else(|_| spec.clone(), |(s, _)| s);
    spec_key(vfs, &resolved(a)) == spec_key(vfs, &resolved(b))
}

/// The stamp of an entry as a plan item's `expect` has it.
fn item_stamp(entry: &Entry) -> ItemStamp {
    ItemStamp {
        kind: entry.kind,
        size: Some(entry.size),
        mtime: entry.mtime,
    }
}

/// How the splice source compares with its stamp.
enum Verified {
    /// Read, and its size and CRC are the stamp's; its lines (`None` when too large to map).
    Same(Option<SourceMap>),
    /// The size and time of a `stat` are the stamp's (not read).
    Stat,
    /// Different, gone or unreadable.
    Changed,
}

/// Reads the splice source for its lines and CRC.
fn read_source(
    vfs: &dyn Vfs,
    doc: &EditableDocument,
    source: &SpliceSource,
    ctl: &Ctl<'_>,
) -> Result<Verified, TextError> {
    let (encoding, bom_len) = doc.splice_encoding().unwrap_or((doc.encoding().0, 0));
    let mut reader = match vfs.open_read_with(&source.spec, ctl.cancel) {
        Ok(reader) => reader,
        Err(e) if e.is_cancelled() => return Err(TextError::Cancelled),
        Err(_) => return Ok(Verified::Changed),
    };
    let ctrl_z = doc.document().ctrl_z_eof();
    match SourceMap::scan(&mut *reader, encoding, bom_len, ctrl_z, ctl) {
        Ok(map) => {
            let same = map
                .as_ref()
                .is_some_and(|m| m.len() == source.stamp.size && m.crc32() == source.stamp.crc32);
            Ok(if same {
                Verified::Same(map)
            } else {
                Verified::Changed
            })
        }
        Err(super::ScanError::Cancelled) => Err(TextError::Cancelled),
        Err(super::ScanError::Io(_)) => Ok(Verified::Changed),
    }
}

/// Plans one document.
#[allow(clippy::too_many_arguments)]
fn plan_one(
    vfs: &dyn Vfs,
    side: Option<Side>,
    doc: &EditableDocument,
    req: &SaveRequest,
    resolved: Resolved,
    env: &SaveEnv<'_>,
    names: &mut BackupNames,
    archive_backups: &mut BTreeMap<String, (Spec, bool)>,
    out: &mut Out,
    ctl: &Ctl<'_>,
) -> Result<(), TextError> {
    let Resolved {
        mode,
        target,
        via_link,
        entry,
    } = resolved;
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let warn = |warnings: &mut Vec<Warning>, code: WarningCode, message: String| {
        warnings.push(Warning {
            code,
            message,
            spec: Some(target.canonical()),
        });
    };
    let fail = |out: &mut Out, error: ErrorReport| {
        out.failed(side, doc, target.clone(), error);
    };
    let splice = doc.splice_source().cloned();
    let is_splice = splice
        .as_ref()
        .is_some_and(|source| same_file(vfs, &target, &source.spec));
    let own = is_splice
        || doc
            .location()
            .is_some_and(|location| same_file(vfs, &target, location));
    // The source is read for its lines (a splice), and on an encoding change too: its Ctrl-Z tail is written back,
    // and a source that changed is found.
    let want_splice = splice.is_some() && doc.spliceable() && req.conversion.is_none();
    // The splice source against its stamp (§1.10.13).
    let verified = match (&splice, &entry) {
        (Some(source), _) if want_splice => read_source(vfs, doc, source, ctl)?,
        (Some(source), Some(found)) if is_splice => {
            if found.size == source.stamp.size && found.mtime == source.stamp.mtime {
                Verified::Stat
            } else {
                read_source(vfs, doc, source, ctl)?
            }
        }
        (Some(_), None) if is_splice => Verified::Changed,
        _ => Verified::Stat,
    };
    let changed = matches!(verified, Verified::Changed) && splice.is_some();
    if changed && is_splice {
        issues.push(SaveIssue::ChangedOnDisk);
    } else if !changed
        && is_splice
        && let (Some(source), Some(found)) = (&splice, &entry)
        && found.mtime != source.stamp.mtime
    {
        issues.push(SaveIssue::SourceTouched);
        warn(
            &mut warnings,
            WarningCode::SourceTouched,
            "the file has a new time but the content it was loaded from: it is saved without a question".to_owned(),
        );
    }
    // The content.
    let map = match verified {
        Verified::Same(map) => map,
        Verified::Stat | Verified::Changed => None,
    };
    let opts = ContentOptions {
        reencode: changed.then_some(ReencodeReason::SourceChanged),
    };
    let map = map.filter(|_| want_splice);
    let mut content = match &req.transform {
        Some(spec) => super::converted_content(doc, map, spec, opts.reencode, ctl)?,
        None => save_content(doc, map.as_ref(), &opts),
    };
    let mut external = None;
    if let Some(conv) = &req.conversion {
        let Some(save) = &conv.save else {
            fail(
                out,
                report_with(
                    ErrorKind::Unsupported,
                    &target,
                    "the format's conversion has no save command line".to_owned(),
                ),
            );
            return Ok(());
        };
        external = Some(save.clone());
        match convert(doc, req.transform.as_ref(), conv, save, &target, env, ctl) {
            Ok(converted) => content = converted,
            Err(ConvertError::Cancelled) => return Err(TextError::Cancelled),
            Err(e) => {
                let error = TextError::Convert(e);
                let mut report = ErrorReport::capture(&error, Some(&target));
                report.spec = Some(target.canonical());
                fail(out, report);
                return Ok(());
            }
        }
    }
    for issue in &content.issues {
        match issue {
            ContentIssue::Unmappable { count, first } => match req.unmappable {
                UnmappablePolicy::Ask => issues.push(SaveIssue::Unmappable {
                    count: *count,
                    first: first.clone(),
                }),
                UnmappablePolicy::Refuse => {
                    let at = first.first().map_or(String::new(), |c| {
                        format!(
                            " (the first: U+{:04X} at line {}, column {})",
                            u32::from(c.char),
                            c.line.0 + 1,
                            c.column
                        )
                    });
                    fail(
                        out,
                        report_with(
                            ErrorKind::InvalidInput,
                            &target,
                            format!(
                                "{count} characters cannot be written in {}{at}",
                                content.encoding.name()
                            ),
                        ),
                    );
                    return Ok(());
                }
                UnmappablePolicy::Replace => warn(
                    &mut warnings,
                    WarningCode::UnmappableReplaced,
                    format!(
                        "{count} characters that {} cannot write are written as '?'",
                        content.encoding.name()
                    ),
                ),
            },
            ContentIssue::LossyRewritten { lines } => {
                issues.push(SaveIssue::LossyRewritten { lines: *lines });
            }
            ContentIssue::Reencoded { reason } => {
                issues.push(SaveIssue::Reencoded { reason: *reason });
                if splice.is_some() {
                    warn(
                        &mut warnings,
                        WarningCode::SaveReencoded,
                        format!("the whole text is encoded ({})", reason.as_str()),
                    );
                }
            }
        }
    }
    // The target.
    let member = matches!(target, Spec::ArchiveMember { .. });
    if let Some(found) = &entry {
        let dos_r = found
            .attrs
            .dos()
            .is_some_and(|attrs| attrs.contains(DosAttrs::READONLY));
        let writable = vfs.writable(&target).ok().flatten();
        if member && writable == Some(false) {
            fail(
                out,
                report_with(
                    ErrorKind::PermissionDenied,
                    &target,
                    "the archive it is in cannot be written".to_owned(),
                ),
            );
            return Ok(());
        }
        if dos_r || writable == Some(false) {
            issues.push(SaveIssue::ReadOnlyTarget);
        }
        if !own {
            issues.push(SaveIssue::ReplaceExisting);
        }
    }
    let mut item = PlanItem::planned(
        Action::Write,
        EntryKind::File,
        Some(target.clone()),
        target.clone(),
        0,
    );
    item.size = Some(content.bytes);
    item.replaces = entry.is_some();
    item.expect = entry.as_ref().map(|found| Expect {
        source: None,
        target: Some(item_stamp(found)),
    });
    // A file is backed up when it is replaced; an archive whenever a member is written (its rebuild replaces it).
    if let Some(policy) = &req.backup
        && (member || entry.is_some())
    {
        if member {
            if let Some(outer) = target.outermost_file()
                && let std::collections::btree_map::Entry::Vacant(slot) =
                    archive_backups.entry(outer.canonical())
            {
                match names.assign(policy, vfs, outer, None) {
                    Ok(backup) => {
                        let stands = stands_unmade(vfs, doc, &backup);
                        slot.insert((backup, stands));
                    }
                    Err(e) => {
                        fail(out, report(&e));
                        return Ok(());
                    }
                }
            }
        } else {
            match names.assign(policy, vfs, &target, None) {
                Ok(backup) => {
                    if stands_unmade(vfs, doc, &backup) {
                        issues.push(SaveIssue::OverwriteBackup {
                            backup: backup.clone(),
                        });
                    }
                    item.backup = Some(backup);
                }
                Err(e) => {
                    fail(out, report(&e));
                    return Ok(());
                }
            }
        }
    }
    out.plan.push(item);
    out.items.push(SaveItem {
        side,
        target: target.clone(),
        via_link,
        creates: entry.is_none(),
        mode,
        content: SaveContentSummary::of(&content, doc.pre_save_lines()),
        issues,
        external,
        generation: doc.generation(),
    });
    out.contents.push(Some(ItemContent {
        source: splice.filter(|_| content.reads_source()),
        content: Arc::new(content),
    }));
    out.warnings.extend(warnings);
    Ok(())
}

/// True when a file stands at `backup` that this document's last save did not make there (§1.10.14,
/// `OverwriteBackup`): a backup holding the file the last save replaced (its stamp) is the one generation kept.
fn stands_unmade(vfs: &dyn Vfs, doc: &EditableDocument, backup: &Spec) -> bool {
    let Ok(found) = vfs.stat(backup, FollowLinks::No) else {
        return false;
    };
    match doc.last_backup() {
        Some((made, stamp)) if spec_key(vfs, made) == spec_key(vfs, backup) => {
            !(stamp.kind == found.kind
                && stamp.size.is_none_or(|size| size == found.size)
                && stamp.mtime.is_none_or(|time| found.mtime == Some(time)))
        }
        _ => true,
    }
}

/// The External save conversion (FF-030, D-39): the whole text encoded is `%s`, the program's `%t` the content.
fn convert(
    doc: &EditableDocument,
    transform: Option<&super::ConvertSpec>,
    conv: &ExternalConversion,
    save: &str,
    target: &Spec,
    env: &SaveEnv<'_>,
    ctl: &Ctl<'_>,
) -> Result<SaveContent, ConvertError> {
    let Some((temp_dir, runner)) = env.external else {
        return Err(ConvertError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!(
                "{}: the save needs an external conversion program, which is not enabled here",
                target.canonical()
            ),
        )));
    };
    let (encoding, bom) = doc.encoding();
    let mut input = Vec::new();
    if bom {
        input.extend_from_slice(encoding.bom());
    }
    let mut encoder = Encoder::new(encoding);
    let n = doc.line_count();
    let mut p = 0u32;
    let mut text = String::new();
    if let Some(spec) = transform {
        super::convert::for_each_line(doc, spec, |line, eol| {
            text.push_str(line);
            text.push_str(eol);
        });
        p = n;
    }
    while p < n {
        let end = doc.phys_end(p);
        for l in p..end {
            text.push_str(&doc.line(LineIdx(l)));
        }
        text.push_str(doc.line_ending(LineIdx(end - 1)).as_text());
        p = end;
    }
    encoder.encode(&text, true, &mut input);
    drop(text);
    let run = ExternalConversion {
        load: save.to_owned(),
        save: None,
        ..conv.clone()
    };
    let bytes = run_external(
        &run,
        &input,
        &target.canonical(),
        temp_dir,
        runner,
        ctl.cancel,
    )?;
    let len = bytes.len() as u64;
    Ok(SaveContent {
        pieces: vec![Piece::Encoded(Arc::from(bytes))],
        bytes: len,
        encoding,
        bom,
        eols: *doc.document().eol(),
        spliced_bytes: 0,
        encoded_lines: n,
        issues: vec![ContentIssue::Reencoded {
            reason: ReencodeReason::NoSource,
        }],
    })
}

/// Executes a confirmed save plan (§1.10.13): `hikasa_vfs::ops` streams each item's pieces from `contents` onto its
/// target (`Action::Write`: no metadata of the source, the replaced file's mode kept, `expect` checked, backups and
/// archive batches; `Phase::Writing` progress in bytes; cancellation per step and before the commit). A splice source
/// whose CRC is not its stamp's at the end of its read fails the item before the commit. The items of a plan that
/// asks questions and was not confirmed fail without a call. Each done item's new stamp (its size, its time after the
/// write, the CRC of the bytes written) is in [`SaveReport::written`] and [`SaveReport::saved`]; a failed item leaves
/// the file untouched (INV-VFS-3), and its document modified.
pub fn execute_save(
    vfs: &dyn Vfs,
    mut plan: SavePlan,
    contents: SaveSource<'_>,
    ctl: &Ctl<'_>,
) -> SaveReport {
    if !plan.confirmed {
        let asking: Vec<usize> = plan
            .runs()
            .filter(|(_, item)| !item.questions().is_empty())
            .map(|(i, _)| i)
            .collect();
        for i in asking {
            if let Some(planned) = plan.plan.items.get_mut(i) {
                planned.error = Some(report_with(
                    ErrorKind::InvalidInput,
                    &planned.target,
                    "the save asks questions: answer them (SaveConfirmation) first".to_owned(),
                ));
            }
        }
    }
    // What an archive's backup will hold: the archive file as it stands now.
    let outers: BTreeMap<String, ItemStamp> = plan
        .exec
        .archive_backups
        .keys()
        .filter_map(|outer| {
            let spec = plan.plan.items.iter().find_map(|item| {
                item.target
                    .outermost_file()
                    .filter(|o| o.canonical() == *outer)
            })?;
            let entry = vfs.stat(spec, FollowLinks::Yes).ok()?;
            Some((outer.clone(), item_stamp(&entry)))
        })
        .collect();
    let op = ops::execute_with(&contents, vfs, plan.plan.clone(), &plan.exec, ctl);
    let mut written = Vec::with_capacity(plan.items.len());
    let mut saved = Vec::with_capacity(plan.items.len());
    for (i, item) in plan.items.iter().enumerate() {
        let done = op
            .results
            .get(i)
            .is_some_and(|r| r.status == ItemStatus::Done);
        let planned = plan.plan.items.get(i);
        let crc = contents.written_crc(&item.target);
        let (Some(planned), true, Some(crc32)) = (planned, done, crc) else {
            written.push(None);
            saved.push(None);
            continue;
        };
        let mtime = vfs
            .stat(&item.target, FollowLinks::No)
            .ok()
            .and_then(|entry| entry.mtime);
        let stamp = LoadStamp {
            size: item.content.bytes,
            mtime,
            crc32,
        };
        let backup = match (&planned.backup, item.target.outermost_file()) {
            (Some(backup), _) => planned
                .expect
                .and_then(|expect| expect.target)
                .map(|stamp| (backup.clone(), stamp)),
            (None, Some(outer)) => {
                let key = outer.canonical();
                plan.exec
                    .archive_backups
                    .get(&key)
                    .cloned()
                    .zip(outers.get(&key).copied())
            }
            (None, None) => None,
        };
        written.push(Some(stamp));
        saved.push(Some(SavedFile {
            target: item.target.clone(),
            stamp,
            generation: item.generation,
            encoding: item.content.encoding,
            bom: item.content.bom,
            moves_location: item.mode == SaveMode::SaveAs,
            backup,
        }));
    }
    SaveReport { op, written, saved }
}
