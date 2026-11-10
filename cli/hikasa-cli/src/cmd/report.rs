//! `hikasa report text|folder` (M2 W7, §1.7.15), and the report half of `text compare`, `folder compare` and
//! `session run`: REPORT becomes one `ReportRequest` through `Service::report_request` before anything is compared,
//! then the comparison, `Service::report_plan` and `Service::report_execute` (or `report_dry_run` with
//! `--dry-run`). Every word is parsed by the core (T11); the clipboard payload is placed here, on the provider this
//! process gave the service (§1.7.10).

use std::io::Write;

use hikasa_core::{JobKind, Warning};
use hikasa_folder::NullObserver;
use hikasa_folder::select::SideSet;
use hikasa_report::{ReportKind, ReportWords};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    FOLDER_COMPARE_SCHEMA, REPORT_SCHEMA, TABLE_COMPARE_SCHEMA, TEXT_COMPARE_SCHEMA,
};
use hikasa_session::api::{
    ApiOp, ExpandWords, FolderCompareRequest, ReportFlags, ReportOutcome, ReportSource,
    ReportTarget, SelectRequest, SessionRunRequest, TableCompareRequest, TextCompareRequest,
    parse_select_path, resolve_folder_display, resolve_text_display,
};
use hikasa_session::sessions::SessionRef;
use hikasa_text::ViewOptions;

use super::{Ctx, folder, text, tri};
use crate::args::{ReportArgs, ReportCmd, ReportFolderArgs, ReportTextArgs};

/// Which document a report command writes with `--json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `text compare`, `folder compare`, `session run`: the comparison's document; the report goes to its target.
    Compare,
    /// `report text|folder`: `hikasa.report.v1`.
    Report,
}

impl Shape {
    /// The schema and operation of a command of `kind`.
    fn schema_op(self, kind: ReportKind) -> (&'static str, ApiOp) {
        match (self, kind) {
            (Shape::Report, kind) => (REPORT_SCHEMA, ApiOp::Report { kind }),
            (Shape::Compare, ReportKind::Text) => (TEXT_COMPARE_SCHEMA, ApiOp::TextCompare),
            (Shape::Compare, ReportKind::Folder) => (FOLDER_COMPARE_SCHEMA, ApiOp::FolderCompare),
            (Shape::Compare, ReportKind::Table) => (TABLE_COMPARE_SCHEMA, ApiOp::TableCompare),
        }
    }
}

/// Runs `report text` and `report folder`.
pub fn run(ctx: &mut Ctx<'_>, cmd: &ReportCmd) -> u8 {
    match cmd {
        ReportCmd::Text(args) => match text_request(ctx, args) {
            Ok(request) => run_text(ctx, &args.report, request, "side-by-side", Shape::Report),
            Err(e) => fail_early(ctx, ReportKind::Text, &e),
        },
        ReportCmd::Folder(args) => match folder_request(ctx, args) {
            Ok(request) => run_folder(ctx, &args.report, request, "side-by-side", Shape::Report),
            Err(e) => fail_early(ctx, ReportKind::Folder, &e),
        },
    }
}

/// A failure before anything ran.
fn fail_early(ctx: &mut Ctx<'_>, kind: ReportKind, err: &ApiError) -> u8 {
    ctx.fail(REPORT_SCHEMA, err, ApiOp::Report { kind }, &[])
}

/// The comparison of `report text`: the two files, or a Text Compare session with the flags on top.
fn text_request(ctx: &Ctx<'_>, args: &ReportTextArgs) -> Result<TextCompareRequest, ApiError> {
    let left = args.left.clone().unwrap_or_default();
    let right = args.right.clone().unwrap_or_default();
    let flags = text::request_of(ctx, &args.rule, left, right)?;
    let Some(reference) = &args.session else {
        return Ok(flags);
    };
    match ctx
        .service
        .session_run_request(&SessionRef::parse(reference)?)?
    {
        SessionRunRequest::Text(mut request) => {
            if flags.format.is_some() {
                request.format = flags.format;
                request.formats = Default::default();
            }
            // A pane's own format on top of the session's (D-281).
            if flags.formats.left.is_some() || flags.formats.right.is_some() {
                if let Some(both) = request.format.take() {
                    request.formats.left.get_or_insert_with(|| both.clone());
                    request.formats.right.get_or_insert(both);
                }
                if flags.formats.left.is_some() {
                    request.formats.left = flags.formats.left;
                }
                if flags.formats.right.is_some() {
                    request.formats.right = flags.formats.right;
                }
            }
            if flags.encodings.left.is_some() {
                request.encodings.left = flags.encodings.left;
            }
            if flags.encodings.right.is_some() {
                request.encodings.right = flags.encodings.right;
            }
            if flags.rules_file.is_some() {
                request.rules_file = flags.rules_file;
            }
            request.rules_patch = flags.rules_patch;
            request.ignore_unimportant |= flags.ignore_unimportant;
            request.anchors = flags.anchors;
            Ok(request.with_ignored(flags.ignored))
        }
        SessionRunRequest::Folder(_) | SessionRunRequest::Sync { .. } => {
            Err(ApiError::InvalidArgument(format!(
                "session `{reference}` is not a Text Compare session: use `hikasa report folder`"
            )))
        }
        SessionRunRequest::Table(_) => Err(ApiError::InvalidArgument(format!(
            "session `{reference}` is a Table Compare session: use `hikasa table compare --report`"
        ))),
    }
}

/// The comparison of `report folder`: the two folders, or a Folder Compare or Folder Sync session with the flags on
/// top.
fn folder_request(
    ctx: &Ctx<'_>,
    args: &ReportFolderArgs,
) -> Result<FolderCompareRequest, ApiError> {
    let mut flags = folder::rules_request(ctx, &args.rule)?;
    flags.ignore_unimportant = args.ignore_unimportant;
    let Some(reference) = &args.session else {
        flags.left = args.left.clone().unwrap_or_default();
        flags.right = args.right.clone().unwrap_or_default();
        return Ok(flags);
    };
    let mut request = match ctx
        .service
        .session_run_request(&SessionRef::parse(reference)?)?
    {
        SessionRunRequest::Folder(request) | SessionRunRequest::Sync { request, .. } => request,
        SessionRunRequest::Text(_) => {
            return Err(ApiError::InvalidArgument(format!(
                "session `{reference}` is a Text Compare session: use `hikasa report text`"
            )));
        }
        SessionRunRequest::Table(_) => {
            return Err(ApiError::InvalidArgument(format!(
                "session `{reference}` is a Table Compare session: use `hikasa session run`"
            )));
        }
    };
    if flags.rules_file.is_some() {
        request.rules_file = flags.rules_file;
    }
    if flags.criteria.is_some() {
        request.criteria = flags.criteria;
    }
    request.patch = flags.patch;
    if flags.text_rules_file.is_some() {
        request.text_rules_file = flags.text_rules_file;
    }
    request.text_patch = flags.text_patch;
    request.ignore_unimportant |= flags.ignore_unimportant;
    Ok(request)
}

/// The target of REPORT: `--output-to`, `--output <FILE>` (always a file), else standard output.
pub fn target(args: &ReportArgs) -> ReportTarget {
    match (&args.output_to, &args.output) {
        (Some(to), _) => ReportTarget::parse(to),
        (None, Some(file)) => ReportTarget::File(file.clone()),
        (None, None) => ReportTarget::Stdout,
    }
}

/// The raw flags of REPORT for the service (T11: the words are parsed there).
pub fn flags(args: &ReportArgs, ansi: bool) -> ReportFlags {
    let select = (!args.select.is_empty() || !args.paths.is_empty()).then(|| SelectRequest {
        masks: args.select.clone(),
        paths: args
            .paths
            .iter()
            .map(|p| parse_select_path(p))
            .map(|(path, sides)| {
                (
                    path,
                    if sides.is_empty() {
                        SideSet::all()
                    } else {
                        sides
                    },
                )
            })
            .collect(),
        selection: None,
        include_hidden: Some(true),
        display: None,
        expand: ExpandWords::All,
        iu: false,
    });
    ReportFlags {
        words: ReportWords {
            layout: args.layout.clone(),
            options: args.options.clone(),
            output_options: args.output_options.clone(),
            title: args.title.clone(),
        },
        html: if args.html_mono {
            Some(true)
        } else if args.html {
            Some(false)
        } else {
            None
        },
        wrap: args.wrap.clone(),
        patch_style: args.patch_style.clone(),
        columns: args.columns.clone(),
        line_numbers: tri(args.line_numbers, args.no_line_numbers),
        display: args.display.clone(),
        folders: args.folders.clone(),
        suppress_filters: args.suppress_filters,
        context: args.context,
        width: args.width,
        lines: args.lines.clone(),
        select,
        ansi,
    }
}

/// The refusal of `--json` with the stdout target in `report` (§1.7.15).
fn shared_stdout() -> ApiError {
    ApiError::InvalidArgument(
        "the report and the JSON document cannot share stdout: pass --output-to".to_owned(),
    )
}

/// What a report run ended with.
enum Reported {
    /// The outcome.
    Done(Box<ReportOutcome>),
    /// Standard output went away (a broken pipe): the exit code stays.
    BrokenPipe,
}

/// Plans, then executes or dry-runs the report of `source`; places a clipboard payload.
fn report(
    ctx: &mut Ctx<'_>,
    dry_run: bool,
    source: ReportSource<'_>,
    req: &mut hikasa_report::ReportRequest,
    target: &ReportTarget,
    select: Option<&SelectRequest>,
) -> Result<Reported, ApiError> {
    let job = ctx.job(JobKind::Report);
    let plan = ctx.service.report_plan(source, req, target, select, &job)?;
    if dry_run {
        return Ok(Reported::Done(Box::new(ctx.service.report_dry_run(plan))));
    }
    let executed = if *target == ReportTarget::Stdout {
        let stdout: &mut dyn Write = ctx.out.stdout();
        ctx.service
            .report_execute(source, req, plan, Some(stdout), &job)
    } else {
        ctx.service.report_execute(source, req, plan, None, &job)
    };
    let outcome = match executed {
        Ok(outcome) => outcome,
        Err(e) if e.is_broken_pipe() => return Ok(Reported::BrokenPipe),
        Err(e) => return Err(e),
    };
    if let Some(payload) = &outcome.clipboard {
        ctx.clipboard
            .write_payload(payload)
            .map_err(|e| ApiError::Vfs(e.to_vfs_write()))?;
    }
    Ok(Reported::Done(Box::new(outcome)))
}

/// The end of a report command: the dry run's lines, the report's JSON document, or the comparison's.
fn finish(
    ctx: &mut Ctx<'_>,
    shape: Shape,
    kind: ReportKind,
    code: u8,
    reported: Option<Reported>,
    mut warnings: Vec<Warning>,
    compare_json: &mut dyn FnMut(&mut Ctx<'_>, &[Warning]) -> u8,
) -> u8 {
    let (schema, op) = shape.schema_op(kind);
    let mut lines = Vec::new();
    let mut dto = None;
    match reported {
        Some(Reported::Done(outcome)) => {
            warnings.extend(outcome.warnings.iter().cloned());
            if outcome.dry_run {
                lines = outcome.dry_run_lines();
            }
            dto = Some(outcome.dto());
        }
        Some(Reported::BrokenPipe) | None => {}
    }
    if ctx.out.json() {
        return match (shape, dto) {
            (Shape::Report, Some(dto)) => ctx.json_success(schema, code, dto, &warnings, op),
            _ => compare_json(ctx, &warnings),
        };
    }
    let written = if lines.is_empty() {
        Ok(())
    } else {
        ctx.out.lines(lines)
    };
    ctx.human_done(written, code, schema, &warnings, op)
}

/// A text report: `text compare` (default layout `unified`), `session run` of a Text Compare session, `report text`
/// (`side-by-side`).
pub fn run_text(
    ctx: &mut Ctx<'_>,
    args: &ReportArgs,
    mut request: TextCompareRequest,
    default_layout: &'static str,
    shape: Shape,
) -> u8 {
    let kind = ReportKind::Text;
    let (schema, op) = shape.schema_op(kind);
    let target = target(args);
    if shape == Shape::Report && ctx.out.json() && target == ReportTarget::Stdout {
        return ctx.fail(schema, &shared_stdout(), op, &[]);
    }
    let ansi = target == ReportTarget::Stdout && ctx.out.colour();
    let flags = flags(args, ansi);
    let mut req = match ctx
        .service
        .report_request(kind, default_layout, &flags, &target)
    {
        Ok(req) => req,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    // The word sets the comparison's ignore-unimportant too (§1.7.4).
    request.ignore_unimportant |= req.ignore_unimportant();
    let json_display = match resolve_text_display(
        args.display.as_deref(),
        args.context,
        ctx.service.options().context_lines,
        true,
    ) {
        Ok(display) => display,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    let job = ctx.job(JobKind::TextCompare);
    let resp = match ctx.service.text_compare(&request, &job, None) {
        Ok(resp) => resp,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    ctx.progress.clear();
    let code = resp.exit_code.code();
    let warnings = resp.warnings.clone();
    // A Ctrl-C that came as the comparison ended stops the command the same way: the target stays (D-159).
    if job.cancel.is_cancelled() {
        return ctx.fail(schema, &ApiError::Cancelled, op, &warnings);
    }
    // `--json` of a compare subcommand without a target: the JSON document only (M1).
    let json_only = shape == Shape::Compare && ctx.out.json() && target == ReportTarget::Stdout;
    let reported = if json_only {
        None
    } else {
        match report(
            ctx,
            args.dry_run,
            ReportSource::Text(&resp),
            &mut req,
            &target,
            None,
        ) {
            Ok(reported) => Some(reported),
            Err(e) => return ctx.fail(schema, &e, op, &warnings),
        }
    };
    let view = ViewOptions {
        filter: json_display.text_filter(),
        ignore_unimportant: resp.ignore_unimportant,
        expanded: Vec::new(),
    };
    let mut compare_json = |ctx: &mut Ctx<'_>, warnings: &[Warning]| {
        ctx.json_success(schema, code, resp.json(&view), warnings, op)
    };
    finish(
        ctx,
        shape,
        kind,
        code,
        reported,
        warnings,
        &mut compare_json,
    )
}

/// A folder report: `folder compare` (default layout `list`), `session run` of a Folder Compare session,
/// `report folder` (`side-by-side`).
pub fn run_folder(
    ctx: &mut Ctx<'_>,
    args: &ReportArgs,
    mut request: FolderCompareRequest,
    default_layout: &'static str,
    shape: Shape,
) -> u8 {
    let kind = ReportKind::Folder;
    let (schema, op) = shape.schema_op(kind);
    let target = target(args);
    if shape == Shape::Report && ctx.out.json() && target == ReportTarget::Stdout {
        return ctx.fail(schema, &shared_stdout(), op, &[]);
    }
    let ansi = target == ReportTarget::Stdout && ctx.out.colour();
    let flags = flags(args, ansi);
    let mut req = match ctx
        .service
        .report_request(kind, default_layout, &flags, &target)
    {
        Ok(req) => req,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    request.ignore_unimportant |= req.ignore_unimportant();
    let json_display = match resolve_folder_display(
        args.display.as_deref(),
        args.folders.as_deref(),
        args.suppress_filters,
        request.ignore_unimportant,
    ) {
        Ok(display) => display,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    let job = ctx.job(JobKind::FolderCompare);
    let prepared = match ctx.service.folder_prepare_in(&request, &job) {
        Ok(prepared) => prepared,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    let resp = match ctx.service.folder_run(&prepared, &job, &NullObserver) {
        Ok(resp) => resp,
        Err(e) => {
            let warnings: Vec<Warning> = prepared.warnings().to_vec();
            return ctx.fail(schema, &e, op, &warnings);
        }
    };
    ctx.progress.clear();
    let code = resp.exit_code.code();
    let warnings = resp.warnings.clone();
    // A scan that was cancelled ends `Ok` with a part of the tree, for the GUI: here it is a failed command. Nothing
    // is rendered and the target keeps the report it had (D-159).
    if job.cancel.is_cancelled() {
        return ctx.fail(schema, &ApiError::Cancelled, op, &warnings);
    }
    let json_only = shape == Shape::Compare && ctx.out.json() && target == ReportTarget::Stdout;
    let reported = if json_only {
        None
    } else {
        let source = ReportSource::Folder {
            request: &request,
            prepared: &prepared,
            response: &resp,
        };
        match report(
            ctx,
            args.dry_run,
            source,
            &mut req,
            &target,
            flags.select.as_ref(),
        ) {
            Ok(reported) => Some(reported),
            Err(e) => return ctx.fail(schema, &e, op, &warnings),
        }
    };
    let mut compare_json = |ctx: &mut Ctx<'_>, warnings: &[Warning]| {
        ctx.json_success(schema, code, resp.json(&json_display), warnings, op)
    };
    finish(
        ctx,
        shape,
        kind,
        code,
        reported,
        warnings,
        &mut compare_json,
    )
}

/// True when a REPORT flag asks for a report (else `table compare` prints its human output).
pub(crate) fn asks_report(args: &ReportArgs) -> bool {
    args.layout.is_some()
        || !args.options.is_empty()
        || !args.output_options.is_empty()
        || args.output_to.is_some()
        || args.output.is_some()
        || args.title.is_some()
        || args.html
        || args.html_mono
        || args.line_numbers
        || args.no_line_numbers
        || args.width.is_some()
        || args.dry_run
        || args.folders.is_some()
        || args.suppress_filters
        || args.context.is_some()
        || args.patch_style.is_some()
        || args.columns.is_some()
        || args.wrap.is_some()
        || args.lines.is_some()
        || !args.select.is_empty()
        || !args.paths.is_empty()
}

/// A table comparison (Table Compare §6.5): `table compare` and `session run` of a Table Compare session. Without a
/// report flag the human output (a line per sheet pair, at most 20 rows per sheet unless `--display all`, the summary
/// line) or, with `--json`, `hikasa.table_compare.v1`; with one, REPORT (default layout side-by-side, display all)
/// through the same plan and execution as text reports.
pub fn run_table(
    ctx: &mut Ctx<'_>,
    args: &ReportArgs,
    mut request: TableCompareRequest,
    shape: Shape,
) -> u8 {
    let kind = ReportKind::Table;
    let (schema, op) = shape.schema_op(kind);
    let target = target(args);
    if shape == Shape::Report && ctx.out.json() && target == ReportTarget::Stdout {
        return ctx.fail(schema, &shared_stdout(), op, &[]);
    }
    let wants_report = shape == Shape::Report || asks_report(args);
    let ansi = target == ReportTarget::Stdout && ctx.out.colour();
    let flags = flags(args, ansi);
    let mut req = if wants_report {
        match ctx
            .service
            .report_request(kind, "side-by-side", &flags, &target)
        {
            Ok(req) => Some(req),
            Err(e) => return ctx.fail(schema, &e, op, &[]),
        }
    } else {
        None
    };
    if let Some(req) = &req {
        request.ignore_unimportant |= req.ignore_unimportant();
    }
    let job = ctx.job(JobKind::TableCompare);
    let resp = match ctx.service.table_compare(&request, &job) {
        Ok(resp) => resp,
        Err(e) => return ctx.fail(schema, &e, op, &[]),
    };
    ctx.progress.clear();
    let code = resp.exit_code.code();
    let warnings = resp.warnings.clone();
    if job.cancel.is_cancelled() {
        return ctx.fail(schema, &ApiError::Cancelled, op, &warnings);
    }
    let json_only = shape == Shape::Compare && ctx.out.json() && target == ReportTarget::Stdout;
    let Some(req) = req.as_mut().filter(|_| !json_only) else {
        if ctx.out.json() {
            return ctx.json_success(schema, code, resp.json(&resp.view), &warnings, op);
        }
        let all = resp.shows_all_rows();
        let written = ctx.out.lines(resp.human_lines((!all).then_some(20)));
        return ctx.human_done(written, code, schema, &warnings, op);
    };
    let reported = match report(
        ctx,
        args.dry_run,
        ReportSource::Table(&resp),
        req,
        &target,
        None,
    ) {
        Ok(reported) => Some(reported),
        Err(e) => return ctx.fail(schema, &e, op, &warnings),
    };
    let mut compare_json = |ctx: &mut Ctx<'_>, warnings: &[Warning]| {
        ctx.json_success(schema, code, resp.json(&resp.view), warnings, op)
    };
    finish(
        ctx,
        shape,
        kind,
        code,
        reported,
        warnings,
        &mut compare_json,
    )
}
