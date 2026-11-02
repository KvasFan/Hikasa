//! `hikasa folder sync` and `session run` of a folder-sync session (§1.6.17, D-185, D-190): compare with the sync's rules,
//! plan through `Service::folder_sync_plan`, ask, execute. A failed item or a row the comparison could not tell keeps
//! `ok: true` and exits 100; a bad word, nothing selected, unanswered questions and an empty source are whole-command
//! failures, with nothing written. `--gui` sends the sync to the GUI instead, after its words were checked here.

use hikasa_core::{JobKind, Warning};
use hikasa_folder::DisplayOptions;
use hikasa_folder::NullObserver;
use hikasa_folder::ops::{ConfirmMode, ReadOnlySides};
use hikasa_folder::select::SideSet;
use hikasa_folder::sync::SyncOptions;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{FOLDER_SYNC_SCHEMA, GUI_OPEN_SCHEMA};
use std::io::Write as _;

use hikasa_session::api::folder_ops::emptied_text;
use hikasa_session::api::{
    ApiOp, EmptiedDto, FolderCompareRequest, FolderSyncRequest, GuiOpenRequest, OpOverrides, SessionRunRequest,
    ViewWords, human, parse_confirm_mode, parse_sides, resolve_folder_display, sync_options,
    sync_words_of,
};
use hikasa_session::ipc::SyncWords;
use hikasa_session::sessions::SessionRef;

use super::{Ctx, folder_op, gui, tri};
use crate::args::{FolderSyncArgs, SessionRunArgs};

/// The operation of the exit codes: a base that cannot be loaded is 107.
const OP: ApiOp = ApiOp::FolderSync;

/// A failure, with the warnings seen so far.
type Failure = Box<(ApiError, Vec<Warning>)>;

fn failure(error: ApiError, warnings: &[Warning]) -> Failure {
    Box::new((error, warnings.to_vec()))
}

fn bare<T>(result: Result<T, ApiError>) -> Result<T, Failure> {
    result.map_err(|e| failure(e, &[]))
}

/// What one sync runs: the comparison, the sync, and how it is answered and shown.
struct SyncRun {
    compare: FolderCompareRequest,
    request: FolderSyncRequest,
    mode: ConfirmMode,
    dry_run: bool,
    all_rows: bool,
}

/// The words of the flags.
fn words(args: &FolderSyncArgs) -> SyncWords {
    let p = &args.preset;
    let preset = [
        (p.update_left, "update-left"),
        (p.update_right, "update-right"),
        (p.update_both, "update-both"),
        (p.mirror_left, "mirror-left"),
        (p.mirror_right, "mirror-right"),
    ]
    .into_iter()
    .find_map(|(on, word)| on.then(|| word.to_owned()));
    SyncWords {
        preset,
        custom: p.custom.clone(),
        orphans: args.orphans.clone(),
        create_empty: args.create_empty,
        within_archives: args.within_archives,
        recycle: args.recycle,
        recycle_fallback: args.recycle_fallback,
        overrides: args.overrides.clone(),
        allow_empty_source: args.allow_empty_source,
    }
}

/// Runs `folder sync`.
pub fn run(ctx: &mut Ctx<'_>, args: &FolderSyncArgs) -> u8 {
    if args.gui {
        return match gui_request(ctx, args) {
            Ok(open) => gui::open_view(ctx, &open, args.no_launch, &[]),
            Err(e) => ctx.fail(GUI_OPEN_SCHEMA, &e, ApiOp::GuiOpen { wait: args.wait }, &[]),
        };
    }
    let result = prepare(ctx, args).and_then(|run| execute(ctx, run));
    match result {
        Ok(code) => code,
        Err(failure) => ctx.fail(FOLDER_SYNC_SCHEMA, &failure.0, OP, &failure.1),
    }
}

/// The session's sync and comparison, or the two sides. The sync is `None` when nothing holds one: no session, or a
/// folder-compare session; then a preset flag is required.
fn base_of(
    ctx: &Ctx<'_>,
    args: &FolderSyncArgs,
) -> Result<(FolderCompareRequest, Option<SyncOptions>, ReadOnlySides), ApiError> {
    let common = &args.common;
    let mut request = super::folder::rules_request(ctx, &common.rule)?;
    request.ignore_unimportant = common.select.ignore_unimportant;
    let (options, read_only) = match (&common.session, &common.left, &common.right) {
        (Some(reference), _, _) => {
            let reference = SessionRef::parse(reference)?;
            let (session, options) = match ctx.service.session_run_request(&reference)? {
                SessionRunRequest::Sync { request, options } => (request, Some(options)),
                SessionRunRequest::Folder(request) => (request, None),
                SessionRunRequest::Text(_) => {
                    return Err(ApiError::InvalidArgument(format!(
                        "`{reference}` is no folder session"
                    )));
                }
            };
            request.left = session.left;
            request.right = session.right;
            request.base_rules = session.base_rules;
            request.base_text_rules = session.base_text_rules;
            (options, ctx.service.session_read_only(&reference)?)
        }
        (None, Some(left), Some(right)) => {
            request.left = left.clone();
            request.right = right.clone();
            (None, ReadOnlySides::default())
        }
        _ => {
            return Err(ApiError::InvalidArgument(
                "name the two sides, or a --session".to_owned(),
            ));
        }
    };
    Ok((request, options, read_only))
}

/// The sync of the flags, checked before anything is compared: the words, a preset (or a folder-sync session's),
/// `--select` only with `--just-selection`. A folder-compare session holds no sync: its preset is not Update Right by
/// default, it is a usage error (§1.6.17).
fn sync_of(
    ctx: &Ctx<'_>,
    args: &FolderSyncArgs,
    base: Option<&SyncOptions>,
) -> Result<SyncOptions, ApiError> {
    let words = words(args);
    let named = words.preset.is_some() || words.custom.is_some();
    if !named && base.is_none() {
        let held = if args.common.session.is_some() {
            " (this session is no folder-sync session and holds none)"
        } else {
            ""
        };
        return Err(ApiError::InvalidArgument(format!(
            "a sync needs its preset{held}: --update-left, --update-right, --update-both, --mirror-left, \
             --mirror-right or --custom"
        )));
    }
    let select = &args.common.select;
    if !args.just_selection && (!select.select.is_empty() || !select.path.is_empty()) {
        return Err(ApiError::InvalidArgument(
            "a sync ignores the selection: --select and --path need --just-selection".to_owned(),
        ));
    }
    let windows = ctx.service.env().platform == hikasa_session::host::Platform::Windows;
    let defaults = SyncOptions::default();
    sync_options(&words, base.unwrap_or(&defaults), windows)
}

/// The sync of `folder sync`'s flags.
fn prepare(ctx: &Ctx<'_>, args: &FolderSyncArgs) -> Result<SyncRun, Failure> {
    let (compare, base, mut read_only) = bare(base_of(ctx, args))?;
    let options = bare(sync_of(ctx, args, base.as_ref()))?;
    if let Some(word) = &args.confirm.read_only {
        let named = bare(parse_sides(word))?;
        read_only.left |= named.contains(SideSet::LEFT);
        read_only.right |= named.contains(SideSet::RIGHT);
    }
    let mode = bare(mode_of(args.confirm.yes, args.confirm.confirm.as_deref()))?;
    let select = &args.common.select;
    let shown = bare(display_of(
        select.display.as_deref(),
        select.ignore_unimportant,
    ))?;
    let just = args
        .just_selection
        .then(|| folder_op::select_request(select, select.include_hidden))
        .transpose();
    let request = FolderSyncRequest {
        options,
        select: bare(just)?,
        visible: args.visible.then_some(shown),
        include_hidden: select.include_hidden.then_some(true),
        iu: select.ignore_unimportant,
        read_only,
        allow_empty_source: args.allow_empty_source,
        overrides: OpOverrides {
            backup: tri(args.backup, args.no_backup),
            preserve: None,
            links: None,
        },
    };
    Ok(SyncRun {
        compare,
        request,
        mode,
        dry_run: args.confirm.dry_run,
        all_rows: args.all_rows,
    })
}

/// `--display` of `--visible` (default `all`).
fn display_of(word: Option<&str>, iu: bool) -> Result<DisplayOptions, ApiError> {
    resolve_folder_display(Some(word.unwrap_or("all")), None, false, iu)
}

/// `--yes`, `--confirm`.
fn mode_of(yes: bool, confirm: Option<&str>) -> Result<ConfirmMode, ApiError> {
    if yes {
        return Ok(ConfirmMode::YesToAll);
    }
    Ok(confirm
        .map(parse_confirm_mode)
        .transpose()?
        .unwrap_or_default())
}

/// `folder sync --gui` (D-190): the words are checked here, as `folder compare --gui` checks its own (D-50), then sent.
fn gui_request(ctx: &Ctx<'_>, args: &FolderSyncArgs) -> Result<GuiOpenRequest, ApiError> {
    let (compare, base, mut read_only) = base_of(ctx, args)?;
    // The session's sync with the flags on top travels whole, in words the GUI reads back with the same parser.
    let options = sync_of(ctx, args, base.as_ref())?;
    let words = sync_words_of(&options, args.allow_empty_source);
    let view = ViewWords {
        display: args.common.select.display.clone(),
        ..ViewWords::default()
    };
    if let Some(word) = &args.confirm.read_only {
        let named = parse_sides(word)?;
        read_only.left |= named.contains(SideSet::LEFT);
        read_only.right |= named.contains(SideSet::RIGHT);
    }
    let mut open = GuiOpenRequest::for_sync(&compare, &words, &view, args.wait);
    open.options.read_only.left = read_only.left;
    open.options.read_only.right = read_only.right;
    open.options.no_backups = args.no_backup;
    Ok(open)
}

/// `session run` of a folder-sync session: its specs, rules, `[rules.sync]` and read-only sides, with `--dry-run`, `--yes`,
/// `--confirm`, `--allow-empty-source` and `--all-rows`.
pub fn run_session(
    ctx: &mut Ctx<'_>,
    args: &SessionRunArgs,
    compare: FolderCompareRequest,
    options: SyncOptions,
) -> u8 {
    let prepared = || -> Result<SyncRun, ApiError> {
        let reference = SessionRef::parse(&args.reference)?;
        let mode = mode_of(args.yes, args.confirm.as_deref())?;
        let mut compare = compare;
        compare.ignore_unimportant = args.ignore_unimportant;
        Ok(SyncRun {
            compare,
            request: FolderSyncRequest {
                options,
                select: None,
                visible: None,
                include_hidden: None,
                iu: args.ignore_unimportant,
                read_only: ctx.service.session_read_only(&reference)?,
                allow_empty_source: args.allow_empty_source,
                overrides: OpOverrides::default(),
            },
            mode,
            dry_run: args.dry_run,
            all_rows: args.all_rows,
        })
    };
    match prepared()
        .map_err(|e| failure(e, &[]))
        .and_then(|run| execute(ctx, run))
    {
        Ok(code) => code,
        Err(failure) => ctx.fail(FOLDER_SYNC_SCHEMA, &failure.0, OP, &failure.1),
    }
}

/// Compares with the sync's rules, plans, asks, runs, prints.
fn execute(ctx: &mut Ctx<'_>, run: SyncRun) -> Result<u8, Failure> {
    let job = ctx.job(JobKind::FolderCompare);
    let prepared = bare(
        ctx.service
            .folder_sync_prepare(&run.compare, &run.request.options, &job),
    )?;
    let response = ctx
        .service
        .folder_run(&prepared, &job, &NullObserver)
        .map_err(|e| failure(e, prepared.warnings()))?;
    let mut warnings = response.warnings.clone();
    ctx.progress.clear();
    if job.cancel.is_cancelled() {
        return Err(failure(ApiError::Cancelled, &warnings));
    }
    let job = ctx.job(JobKind::FolderSync);
    let mut plan = ctx
        .service
        .folder_sync_plan(prepared.model(), &run.request, &job)
        .map_err(|e| failure(e, &warnings))?;
    ctx.progress.clear();
    let outcome = if run.dry_run {
        ctx.service.folder_sync_dry_run(plan)
    } else {
        // The question comes after what a failed nested mount would cost: the folders emptied entirely.
        if run.mode == ConfirmMode::Prompt && !plan.emptied.is_empty() {
            let read = prepared.model().read();
            let mut stderr = std::io::stderr();
            for &(node, empty) in &plan.emptied {
                let folder = EmptiedDto {
                    path: read.path(node).as_str().to_owned(),
                    empty: empty.as_str(),
                    emptied: empty.opposite().as_str(),
                };
                let _ = writeln!(stderr, "# {}", emptied_text(&folder));
            }
        }
        let stand_in = empty_op(&plan.op);
        let op = std::mem::replace(&mut plan.op, stand_in);
        plan.op = folder_op::answered(ctx, op, run.mode, &job.cancel)
            .map_err(|e| failure(e, &warnings))?;
        ctx.service
            .folder_sync_execute(plan, &job)
            .map_err(|e| failure(e, &warnings))?
    };
    ctx.progress.clear();
    let code = outcome.exit_code().code();
    let dto = outcome.dto(&prepared.model().read(), run.all_rows);
    warnings.extend(dto.warnings.iter().cloned());
    if ctx.out.json() {
        return Ok(ctx.json_success(FOLDER_SYNC_SCHEMA, code, &dto, &warnings, OP));
    }
    let written = ctx.out.lines(human::folder_sync_lines(&dto));
    Ok(ctx.human_done(written, code, FOLDER_SYNC_SCHEMA, &warnings, OP))
}

/// A stand-in while the plan is answered.
fn empty_op(op: &hikasa_folder::ops::OpPlan) -> hikasa_folder::ops::OpPlan {
    hikasa_folder::ops::OpPlan {
        verb: op.verb,
        bases: op.bases.clone(),
        plan: hikasa_vfs::ops::Plan {
            op: op.plan.op,
            options: op.plan.options,
            items: Vec::new(),
            warnings: Vec::new(),
        },
        meta: Vec::new(),
        exec: hikasa_vfs::ops::ExecOptions::default(),
        warnings: Vec::new(),
        refresh: Vec::new(),
    }
}
