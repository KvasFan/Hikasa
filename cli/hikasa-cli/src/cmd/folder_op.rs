//! `hikasa folder op <VERB>` (§1.6.17, D-185): compare, then plan through `Service::folder_op_plan`, ask, execute. A failed
//! item keeps `ok: true` and exits 100, as `vfs`; nothing selected, a bad word, unanswered questions and `ReadOnlySide` are
//! whole-command failures, with nothing written.

use std::io::{self, IsTerminal, Write};

use hikasa_core::{ExitCode, JobKind, Warning};
use hikasa_folder::ops::{
    ConfirmMode, FolderOp, OpPlan, PathStructure, ReadOnlySides, Recycle, RenameMask, TouchFrom,
};
use hikasa_folder::select::{ExcludeWhat, SideSet};
use hikasa_folder::NullObserver;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{FOLDER_CONTENTS_SCHEMA, FOLDER_OP_SCHEMA, SESSION_WRITE_SCHEMA};
use hikasa_session::api::folder_ops::{ContentsDto, FolderOpDto, confirm_words, op_word};
use hikasa_session::api::sessions::ExcludeMasksDto;
use hikasa_session::api::{
    ApiOp, ContentsKind, ContentsRequest, ExpandWords, FolderCompareRequest, FolderOpRequest,
    OpOverrides, OpRequest, OpenTarget, OpenWithRequest, PreparedFolderCompare, SelectRequest,
    SessionRunRequest, SessionWriteDto, human, parse_content_method, parse_link_policy,
    parse_overwrite, parse_select_path, parse_side, parse_sides, parse_time,
    resolve_folder_display,
};
use hikasa_session::launcher::OpenCommandDto;
use hikasa_session::sessions::SessionRef;
use hikasa_vfs::ops::{OverwritePolicy, Preserve};

use super::{Ctx, folder};
use crate::args::{
    FolderConfirmArgs, FolderCopyFlags, FolderCopyToFlags, FolderOpCommon, FolderOpVerb,
    FolderSelectArgs,
};
use crate::confirm::ask;
use crate::output::WriteError;

/// The operation of the exit codes: a base that cannot be loaded is 107.
const OP: ApiOp = ApiOp::FolderOp;
/// Exit code of a successful command.
const OK: u8 = ExitCode::Success.code();

/// The schema of a verb.
pub fn schema_of(verb: &FolderOpVerb) -> &'static str {
    match verb {
        FolderOpVerb::Compare { .. } | FolderOpVerb::Crc { .. } => FOLDER_CONTENTS_SCHEMA,
        FolderOpVerb::Exclude { .. } | FolderOpVerb::Ignored { .. } => SESSION_WRITE_SCHEMA,
        _ => FOLDER_OP_SCHEMA,
    }
}

/// Runs a `folder op` verb.
pub fn run(ctx: &mut Ctx<'_>, verb: &FolderOpVerb) -> u8 {
    match run_verb(ctx, verb) {
        Ok(code) => code,
        Err((e, warnings)) => ctx.fail(schema_of(verb), &e, OP, &warnings),
    }
}

/// A failure, with the warnings seen so far.
type Failure = (ApiError, Vec<Warning>);

/// An error with no warnings.
fn bare<T>(result: Result<T, ApiError>) -> Result<T, Failure> {
    result.map_err(|e| (e, Vec::new()))
}

fn run_verb(ctx: &mut Ctx<'_>, verb: &FolderOpVerb) -> Result<u8, Failure> {
    match verb {
        FolderOpVerb::Copy {
            common,
            confirm,
            flags,
        } => {
            let op = bare(copy_op(flags, false))?;
            plan_verb(ctx, common, confirm, op, bare(overrides(flags))?)
        }
        FolderOpVerb::Move {
            common,
            confirm,
            flags,
        } => {
            let op = bare(copy_op(flags, true))?;
            plan_verb(ctx, common, confirm, op, bare(overrides(flags))?)
        }
        FolderOpVerb::Exchange { common, confirm } => {
            let op = OpRequest::Op(FolderOp::Exchange);
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Delete {
            common,
            confirm,
            side,
            recycle,
            recycle_fallback,
        } => {
            let op = OpRequest::Op(FolderOp::Delete {
                sides: bare(parse_sides(side))?,
                recycle: Recycle {
                    use_bin: *recycle,
                    fallback: *recycle_fallback,
                },
            });
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Rename {
            common,
            confirm,
            side,
            name,
            mask,
            regex,
        } => {
            let word = match (name, mask, regex.as_slice()) {
                (Some(name), _, _) => RenameMask::Name(name.clone()),
                (_, Some(mask), _) => RenameMask::Dos(mask.clone()),
                (_, _, [old, new]) => RenameMask::Regex {
                    old: old.clone(),
                    new: new.clone(),
                },
                _ => {
                    let e = "rename needs one of --name, --mask, --regex";
                    return Err((ApiError::InvalidArgument(e.into()), Vec::new()));
                }
            };
            let op = OpRequest::Op(FolderOp::Rename {
                sides: bare(parse_sides(side))?,
                mask: word,
            });
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Touch {
            common,
            confirm,
            side,
            time,
            now: _,
            from_other,
        } => {
            let sides = bare(parse_sides(side))?;
            let op = match (time, from_other) {
                (Some(text), _) => OpRequest::Op(FolderOp::Touch {
                    sides,
                    from: TouchFrom::At(bare(parse_time(text))?),
                }),
                (None, true) => OpRequest::Op(FolderOp::Touch {
                    sides,
                    from: TouchFrom::OtherSide {
                        to: bare(parse_side(side))?,
                    },
                }),
                (None, false) => OpRequest::TouchNow { sides },
            };
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Mkdir {
            common,
            confirm,
            side,
            into,
            name,
        } => {
            let folder = name.ends_with('/') || (cfg!(windows) && name.ends_with('\\'));
            let op = OpRequest::MkdirIn {
                sides: bare(parse_sides(side))?,
                parent: into.clone().unwrap_or_default(),
                name: name.trim_end_matches(['/', '\\']).to_owned(),
                folder,
            };
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Copyto {
            common,
            confirm,
            flags,
        } => {
            let op = bare(copy_to_op(flags, false))?;
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Moveto {
            common,
            confirm,
            flags,
        } => {
            let op = bare(copy_to_op(flags, true))?;
            plan_verb(ctx, common, confirm, op, OpOverrides::default())
        }
        FolderOpVerb::Compare { common, method } => {
            let kind = ContentsKind::Compare(bare(parse_content_method(method))?);
            contents(ctx, common, kind)
        }
        FolderOpVerb::Crc { common, side } => {
            contents(ctx, common, ContentsKind::Crc(bare(parse_sides(side))?))
        }
        FolderOpVerb::Exclude {
            common,
            what,
            dry_run,
            force,
        } => exclude(ctx, common, what.as_deref(), *dry_run, *force),
        FolderOpVerb::Ignored {
            common,
            off,
            dry_run,
            force,
        } => ignored(ctx, common, *off, *dry_run, *force),
        FolderOpVerb::OpenWith {
            common,
            app,
            associated,
            side,
            dry_run,
        } => open_with(ctx, common, app.as_deref(), *associated, side.as_deref(), *dry_run),
    }
}

/// `--to`, `--overwrite` of `copy`, `move`: the operation names the replacement.
fn copy_op(flags: &FolderCopyFlags, moving: bool) -> Result<OpRequest, ApiError> {
    let to = parse_side(&flags.to)?;
    let overwrite = match &flags.overwrite {
        Some(word) => parse_overwrite(word)?,
        None => OverwritePolicy::Always,
    };
    Ok(OpRequest::Op(if moving {
        FolderOp::Move { to, overwrite }
    } else {
        FolderOp::Copy { to, overwrite }
    }))
}

/// `--no-preserve`, `--preserve-created`, `--symlinks`, `--backup`, `--no-backup`.
fn overrides(flags: &FolderCopyFlags) -> Result<OpOverrides, ApiError> {
    let preserve = if flags.no_preserve {
        Some(Preserve::NONE)
    } else if flags.preserve_created {
        Some(Preserve {
            created: true,
            ..Preserve::default()
        })
    } else {
        None
    };
    Ok(OpOverrides {
        backup: super::tri(flags.backup, flags.no_backup),
        preserve,
        links: flags.symlinks.as_deref().map(parse_link_policy).transpose()?,
    })
}

/// `--side`, `--target`, `--structure`, `--overwrite`.
fn copy_to_op(flags: &FolderCopyToFlags, moving: bool) -> Result<OpRequest, ApiError> {
    let sides = parse_sides(&flags.side)?;
    let structure = match flags.structure.as_deref() {
        None | Some("relative") => PathStructure::Relative,
        Some("base") => PathStructure::Base,
        Some("none") => PathStructure::None,
        Some(word) => {
            return Err(ApiError::InvalidArgument(format!(
                "unknown structure {word:?} (valid: relative, base, none)"
            )));
        }
    };
    let overwrite = match &flags.overwrite {
        Some(word) => parse_overwrite(word)?,
        None => OverwritePolicy::Fail,
    };
    let target = flags.target.clone();
    Ok(if moving {
        OpRequest::MoveTo {
            sides,
            target,
            structure,
            overwrite,
        }
    } else {
        OpRequest::CopyTo {
            sides,
            target,
            structure,
            overwrite,
        }
    })
}

/// The selection of the flags: `--display` defaults to `all`, and every folder is open: a headless command sees the whole
/// tree, as a script after `expand all`.
fn select_request(args: &FolderSelectArgs, include_hidden: bool) -> Result<SelectRequest, ApiError> {
    let display = resolve_folder_display(
        Some(args.display.as_deref().unwrap_or("all")),
        None,
        false,
        args.ignore_unimportant,
    )?;
    let expand = if args.no_expand {
        ExpandWords::None
    } else if args.expand.is_empty() {
        ExpandWords::All
    } else {
        ExpandWords::Paths(args.expand.clone())
    };
    Ok(SelectRequest {
        masks: args.select.clone(),
        paths: args.path.iter().map(|word| parse_select_path(word)).collect(),
        selection: None,
        include_hidden: include_hidden.then_some(true),
        display: Some(display),
        expand,
        iu: args.ignore_unimportant,
    })
}

/// The comparison of the arguments or of `--session`, flags on top.
fn compare_request(ctx: &Ctx<'_>, common: &FolderOpCommon) -> Result<FolderCompareRequest, ApiError> {
    let mut request = folder::rules_request(ctx, &common.rule)?;
    request.ignore_unimportant = common.select.ignore_unimportant;
    match (&common.session, &common.left, &common.right) {
        (Some(reference), _, _) => {
            let reference = SessionRef::parse(reference)?;
            let SessionRunRequest::Folder(session) = ctx.service.session_run_request(&reference)?
            else {
                return Err(ApiError::InvalidArgument(format!(
                    "`{reference}` is no folder session"
                )));
            };
            request.left = session.left;
            request.right = session.right;
            request.base_rules = session.base_rules;
            request.base_text_rules = session.base_text_rules;
        }
        (None, Some(left), Some(right)) => {
            request.left = left.clone();
            request.right = right.clone();
        }
        _ => {
            return Err(ApiError::InvalidArgument(
                "name the two sides, or a --session".to_owned(),
            ));
        }
    }
    Ok(request)
}

/// Compares, as `folder compare` does: a base that cannot be loaded is 107, and a stop ends here.
fn compared(
    ctx: &mut Ctx<'_>,
    common: &FolderOpCommon,
) -> Result<(PreparedFolderCompare, Vec<Warning>), Failure> {
    let request = bare(compare_request(ctx, common))?;
    let job = ctx.job(JobKind::FolderCompare);
    let prepared = bare(ctx.service.folder_prepare_in(&request, &job))?;
    let response = ctx
        .service
        .folder_run(&prepared, &job, &NullObserver)
        .map_err(|e| (e, prepared.warnings().to_vec()))?;
    let warnings = response.warnings.clone();
    ctx.progress.clear();
    if job.cancel.is_cancelled() {
        return Err((ApiError::Cancelled, warnings));
    }
    Ok((prepared, warnings))
}

/// `--read-only` over what a session says.
fn read_only(
    ctx: &Ctx<'_>,
    common: &FolderOpCommon,
    confirm: &FolderConfirmArgs,
) -> Result<ReadOnlySides, ApiError> {
    let mut sides = match &common.session {
        Some(reference) => ctx.service.session_read_only(&SessionRef::parse(reference)?)?,
        None => ReadOnlySides::default(),
    };
    if let Some(word) = &confirm.read_only {
        let named = parse_sides(word)?;
        sides.left |= named.contains(SideSet::LEFT);
        sides.right |= named.contains(SideSet::RIGHT);
    }
    Ok(sides)
}

/// `--yes`, `--confirm`.
fn mode_of(confirm: &FolderConfirmArgs) -> Result<ConfirmMode, ApiError> {
    if confirm.yes {
        return Ok(ConfirmMode::YesToAll);
    }
    match confirm.confirm.as_deref() {
        None | Some("prompt") => Ok(ConfirmMode::Prompt),
        Some("yes-to-all") => Ok(ConfirmMode::YesToAll),
        Some("no-to-all") => Ok(ConfirmMode::NoToAll),
        Some(word) => Err(ApiError::InvalidArgument(format!(
            "unknown confirmation {word:?} (valid: prompt, yes-to-all, no-to-all)"
        ))),
    }
}

/// The answers of a plan: a mode needs no person; `Prompt` asks on a terminal, and without one nothing is touched.
fn answered(ctx: &Ctx<'_>, plan: OpPlan, mode: ConfirmMode) -> Result<OpPlan, ApiError> {
    let mut machine = ctx.service.folder_confirmation_strict(&plan, Some(mode));
    if machine.next().is_some() {
        if !io::stdin().is_terminal() {
            let open = machine.open_questions();
            let total: u64 = open.values().sum();
            return Err(ApiError::InvalidArgument(format!(
                "{total} questions need an answer: pass --yes or --confirm no-to-all, or --dry-run to see the plan ({})",
                confirm_words(&open)
            )));
        }
        ctx.progress.clear();
        let mut stderr = io::stderr();
        ask(&mut machine, &mut io::stdin().lock(), &mut stderr);
        let _ = stderr.flush();
    }
    machine.finish(plan).map_err(|_| ApiError::Cancelled)
}

/// Plans `op`, shows it or runs it.
fn plan_verb(
    ctx: &mut Ctx<'_>,
    common: &FolderOpCommon,
    confirm: &FolderConfirmArgs,
    op: OpRequest,
    overrides: OpOverrides,
) -> Result<u8, Failure> {
    let mode = bare(mode_of(confirm))?;
    let request = FolderOpRequest {
        op,
        select: bare(select_request(&common.select, common.select.include_hidden))?,
        read_only: bare(read_only(ctx, common, confirm))?,
        overrides,
    };
    // A refused side costs no comparison: nothing here reads a base.
    let (prepared, mut warnings) = compared(ctx, common)?;
    let job = ctx.job(JobKind::FolderOp);
    let plan = ctx
        .service
        .folder_op_plan(prepared.model(), &request, &job)
        .map_err(|e| (e, warnings.clone()))?;
    ctx.progress.clear();
    let outcome = if confirm.dry_run {
        ctx.service.folder_op_dry_run(plan)
    } else {
        let plan = answered(ctx, plan, mode).map_err(|e| (e, warnings.clone()))?;
        ctx.service
            .folder_op_execute(plan, &job)
            .map_err(|e| (e, warnings.clone()))?
    };
    ctx.progress.clear();
    let code = outcome.exit_code().code();
    let dto = outcome.dto();
    warnings.extend(dto.warnings.iter().cloned());
    Ok(print(ctx, &dto, outcome.verb, code, &warnings))
}

/// `hikasa.folder_op.v1`, or one line per item and the summary.
fn print(
    ctx: &mut Ctx<'_>,
    dto: &FolderOpDto,
    verb: hikasa_folder::ops::OpVerb,
    code: u8,
    warnings: &[Warning],
) -> u8 {
    if ctx.out.json() {
        return ctx.json_success(FOLDER_OP_SCHEMA, code, dto, warnings, OP);
    }
    let written = ctx.out.lines(human::folder_op_lines(dto, verb));
    ctx.human_done(written, code, FOLDER_OP_SCHEMA, warnings, OP)
}

/// `compare`, `crc`: `hikasa.folder_contents.v1`.
fn contents(
    ctx: &mut Ctx<'_>,
    common: &FolderOpCommon,
    kind: ContentsKind,
) -> Result<u8, Failure> {
    let select = bare(select_request(&common.select, common.select.include_hidden))?;
    let (prepared, warnings) = compared(ctx, common)?;
    let request = ContentsRequest {
        select,
        kind,
        text_rules: prepared.text_rules().cloned(),
    };
    let job = ctx.job(JobKind::FolderOp);
    let dto: ContentsDto = ctx
        .service
        .folder_contents(prepared.model(), &request, &job)
        .map_err(|e| (e, warnings.clone()))?;
    ctx.progress.clear();
    let code = if dto.ok() {
        OK
    } else {
        ExitCode::UnknownError.code()
    };
    if ctx.out.json() {
        return Ok(ctx.json_success(FOLDER_CONTENTS_SCHEMA, code, &dto, &warnings, OP));
    }
    let written = ctx.out.lines(human::folder_contents_lines(&dto));
    Ok(ctx.human_done(written, code, FOLDER_CONTENTS_SCHEMA, &warnings, OP))
}

/// A session write, with what it says of masks or marks.
fn session_write(ctx: &mut Ctx<'_>, dto: &SessionWriteDto, warnings: &[Warning]) -> u8 {
    if ctx.out.json() {
        return ctx.json_success(SESSION_WRITE_SCHEMA, OK, dto, warnings, OP);
    }
    let mut lines = human::session_write_lines(dto);
    if let Some(masks) = &dto.masks {
        lines.extend(masks.files.iter().map(|mask| format!("exclude file {mask}")));
        lines.extend(masks.folders.iter().map(|mask| format!("exclude folder {mask}")));
    }
    if let Some(list) = &dto.ignored {
        lines.extend(list.iter().map(|path| format!("ignored {path}")));
    }
    let written: Result<(), WriteError> = ctx.out.lines(lines);
    ctx.human_done(written, OK, SESSION_WRITE_SCHEMA, warnings, OP)
}

/// The session a write goes to: `--session`, needed unless `--dry-run`.
fn session_of(common: &FolderOpCommon, dry_run: bool) -> Result<Option<SessionRef>, ApiError> {
    match (&common.session, dry_run) {
        (Some(reference), _) => SessionRef::parse(reference).map(Some),
        (None, true) => Ok(None),
        (None, false) => Err(ApiError::InvalidArgument(
            "this verb changes a session: name it with --session, or pass --dry-run".to_owned(),
        )),
    }
}

/// A dry run of a session write.
fn dry_write(reference: Option<SessionRef>) -> SessionWriteDto {
    SessionWriteDto {
        action: "edit",
        reference: reference.map_or_else(String::new, |r| r.to_string()),
        written: Vec::new(),
        removed: Vec::new(),
        warnings: Vec::new(),
        masks: None,
        ignored: None,
    }
}

/// FC-052: `exclude` rewrites `[rules]`, and a re-comparison hides the items.
fn exclude(
    ctx: &mut Ctx<'_>,
    common: &FolderOpCommon,
    what: Option<&str>,
    dry_run: bool,
    force: bool,
) -> Result<u8, Failure> {
    let what = match what {
        None | Some("items") => ExcludeWhat::Items,
        Some("type") => ExcludeWhat::Type,
        Some(word) => {
            let e = format!("unknown word {word:?} (valid: items, type)");
            return Err((ApiError::InvalidArgument(e), Vec::new()));
        }
    };
    let session = bare(session_of(common, dry_run))?;
    let select = bare(select_request(&common.select, common.select.include_hidden))?;
    let (prepared, warnings) = compared(ctx, common)?;
    let fail = |e: ApiError| (e, warnings.clone());
    let (files, folders) = ctx
        .service
        .folder_exclude_masks(prepared.model(), &select, what)
        .map_err(fail)?;
    let masks = ExcludeMasksDto { files, folders };
    let dto = match (dry_run, session) {
        (false, Some(reference)) => ctx
            .service
            .session_add_excludes(&reference, &masks, force)
            .map_err(fail)?,
        (_, reference) => SessionWriteDto {
            masks: Some(masks),
            ..dry_write(reference)
        },
    };
    Ok(session_write(ctx, &dto, &warnings))
}

/// FC-033: `ignored` sets the marks, and the session keeps the new list.
fn ignored(
    ctx: &mut Ctx<'_>,
    common: &FolderOpCommon,
    off: bool,
    dry_run: bool,
    force: bool,
) -> Result<u8, Failure> {
    let session = bare(session_of(common, dry_run))?;
    let select = bare(select_request(&common.select, common.select.include_hidden))?;
    let (prepared, warnings) = compared(ctx, common)?;
    let fail = |e: ApiError| (e, warnings.clone());
    let rows = ctx
        .service
        .folder_selected_rows(prepared.model(), &select)
        .map_err(fail)?;
    if rows.is_empty() {
        let e = ApiError::InvalidArgument("nothing selected: pass --select all, or a mask".into());
        return Err(fail(e));
    }
    let list = ctx.service.folder_set_ignored(prepared.model(), &rows, !off);
    let dto = match (dry_run, session) {
        (false, Some(reference)) => ctx
            .service
            .session_set_ignored(&reference, &list, force)
            .map_err(fail)?,
        (_, reference) => SessionWriteDto {
            ignored: Some(list),
            ..dry_write(reference)
        },
    };
    Ok(session_write(ctx, &dto, &warnings))
}

/// `open-with`: prints the commands with `--dry-run`, else runs them; a program's own exit code is not Hikasa's.
fn open_with(
    ctx: &mut Ctx<'_>,
    common: &FolderOpCommon,
    app: Option<&str>,
    associated: bool,
    side: Option<&str>,
    dry_run: bool,
) -> Result<u8, Failure> {
    let target = match (associated, app) {
        (true, _) | (_, None) => OpenTarget::Associated,
        (false, Some(word)) => match word.parse::<usize>() {
            Ok(index) => OpenTarget::App(index),
            Err(_) => OpenTarget::Named(word.to_owned()),
        },
    };
    let sides = bare(side.map_or(Ok(SideSet::LEFT), parse_sides))?;
    let paths: Vec<_> = common.select.path.iter().map(|word| parse_select_path(word)).collect();
    if paths.is_empty() {
        let e = ApiError::InvalidArgument("open-with needs --path".to_owned());
        return Err((e, Vec::new()));
    }
    let (prepared, warnings) = compared(ctx, common)?;
    let fail = |e: ApiError| (e, warnings.clone());
    let items = ctx
        .service
        .folder_open_items(prepared.model(), &paths, sides)
        .map_err(fail)?;
    let job = ctx.job(JobKind::FolderOp);
    if target == OpenTarget::Associated {
        if !dry_run {
            for (item, _) in &items {
                ctx.service.open_associated(item, &job).map_err(fail)?;
            }
        }
        let commands = items
            .iter()
            .map(|(item, _)| OpenCommandDto {
                program: "associated".to_owned(),
                args: vec![item.canonical()],
                cwd: String::new(),
            })
            .collect();
        return Ok(print_commands(ctx, &prepared, commands, dry_run, &warnings));
    }
    let request = OpenWithRequest {
        target: target.clone(),
        items,
        line: None,
    };
    let commands = ctx
        .service
        .open_with_commands(&request, &job)
        .map_err(fail)?;
    if !dry_run {
        let wait = ctx.service.open_with_waits(&target).map_err(fail)?;
        ctx.service
            .open_with_run(&commands, wait, &job)
            .map_err(fail)?;
    }
    let shown = commands.iter().map(OpenCommandDto::from).collect();
    Ok(print_commands(ctx, &prepared, shown, dry_run, &warnings))
}

/// `commands` of `hikasa.folder_op.v1`, or one line per command.
fn print_commands(
    ctx: &mut Ctx<'_>,
    prepared: &PreparedFolderCompare,
    commands: Vec<OpenCommandDto>,
    dry_run: bool,
    warnings: &[Warning],
) -> u8 {
    if ctx.out.json() {
        let specs = prepared.specs();
        let dto = FolderOpDto {
            op: op_word(),
            dry_run,
            cancelled: false,
            left: specs.left.canonical(),
            right: specs.right.canonical(),
            items: Vec::new(),
            backups: Vec::new(),
            summary: hikasa_vfs::ops::Summary::default(),
            commands: Some(commands),
            warnings: Vec::new(),
        };
        return ctx.json_success(FOLDER_OP_SCHEMA, OK, &dto, warnings, OP);
    }
    let lines = commands.iter().map(|command| {
        let mut line = command.program.clone();
        for arg in &command.args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    });
    let written = ctx.out.lines(lines.collect::<Vec<_>>());
    ctx.human_done(written, OK, FOLDER_OP_SCHEMA, warnings, OP)
}
