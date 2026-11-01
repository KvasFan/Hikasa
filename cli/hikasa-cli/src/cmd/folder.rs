//! `hikasa folder compare <left> <right>` (§3.3): `Service::folder_prepare` + `Service::folder_run`, then the
//! human report (`Service::render_folder_with`; the status list by default) or `hikasa.folder_compare.v1`
//! (`FolderCompareResponse::json`, display `differences` with the folder structure by default). Rules precedence
//! (§3.5): defaults ← `--rules-file` ← `--criteria` ← the individual flags (the service applies them in that
//! order). `--gui` sends the request to the GUI instead (D-50).

use hikasa_core::{JobKind, Warning};
use hikasa_folder::{
    CaseMode, Change, DateFilter, DisplayOptions, DosAttrs, FolderRulesPatch, NullObserver,
    SizeFilter,
};
use hikasa_report::{FolderColumns, FolderLayout, FolderReportOptions, OutputFormat};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{FOLDER_COMPARE_SCHEMA, GUI_OPEN_SCHEMA};
use hikasa_session::api::{
    ApiOp, FolderCompareRequest, GuiOpenRequest, ReportSettings, ViewWords, parse_archive_handling,
    parse_content_method, resolve_folder_display,
};

use super::{Ctx, gui, importance_patch, tri};
use crate::args::FolderCompareArgs;
use crate::output::WriteError;

/// Runs `folder compare`.
pub fn run(ctx: &mut Ctx<'_>, args: &FolderCompareArgs) -> u8 {
    let op = ApiOp::FolderCompare;
    let request = match request(ctx, args) {
        Ok(request) => request,
        Err(e) => {
            let (schema, op) = if args.gui {
                (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: args.wait })
            } else {
                (FOLDER_COMPARE_SCHEMA, op)
            };
            return ctx.fail(schema, &e, op, &[]);
        }
    };
    if args.gui {
        let words = ViewWords {
            display: args.display.clone(),
            context_lines: None,
            folders: args.folders.clone(),
            suppress_filters: args.suppress_filters,
            expand_all: args.expand_all,
        };
        let open = GuiOpenRequest::for_folder(&request, &words, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    execute(ctx, args, &request)
}

/// Compares and prints (`folder compare` without `--gui`, and `session run` of a Folder Compare session, T11):
/// `args` gives the presentation words, `request` the comparison.
pub fn execute(ctx: &mut Ctx<'_>, args: &FolderCompareArgs, request: &FolderCompareRequest) -> u8 {
    let op = ApiOp::FolderCompare;
    // The words are checked before the scan; an unsupported layout/format combination (`--report list --html`)
    // is refused by the renderer afterwards (§1.9.3 item 10a).
    let plan = match Plan::new(args) {
        Ok(plan) => plan,
        Err(e) => return ctx.fail(FOLDER_COMPARE_SCHEMA, &e, op, &[]),
    };
    let job = ctx.job(JobKind::FolderCompare);
    let prepared = match ctx.service.folder_prepare_in(request, &job) {
        Ok(prepared) => prepared,
        Err(e) => return ctx.fail(FOLDER_COMPARE_SCHEMA, &e, op, &[]),
    };
    let resp = match ctx.service.folder_run(&prepared, &job, &NullObserver) {
        Ok(resp) => resp,
        Err(e) => {
            let warnings: Vec<Warning> = prepared.warnings().to_vec();
            return ctx.fail(FOLDER_COMPARE_SCHEMA, &e, op, &warnings);
        }
    };
    ctx.progress.clear();
    let code = resp.exit_code.code();
    let warnings = resp.warnings.clone();
    // A scan that was cancelled ends `Ok` with a part of the tree, for the GUI: here it is a failed command. Nothing
    // is rendered and `--output` keeps the report it had (D-159).
    if job.cancel.is_cancelled() {
        return ctx.fail(FOLDER_COMPARE_SCHEMA, &ApiError::Cancelled, op, &warnings);
    }
    if let Some(target) = &args.output {
        let mut report = Vec::new();
        let written = ctx
            .service
            .render_folder_with(&resp, &plan.report(false), &plan.settings, &mut report)
            .and_then(|()| ctx.service.write_output(target, &report).map(|_| ()));
        if let Err(e) = written {
            return ctx.fail(FOLDER_COMPARE_SCHEMA, &e, op, &warnings);
        }
        if !ctx.out.json() {
            return ctx.human_done(Ok(()), code, FOLDER_COMPARE_SCHEMA, &warnings, op);
        }
    }
    if ctx.out.json() {
        let result = resp.json(&plan.display);
        return ctx.json_success(FOLDER_COMPARE_SCHEMA, code, result, &warnings, op);
    }
    let colour = ctx.out.colour();
    let written = ctx
        .service
        .render_folder_with(
            &resp,
            &plan.report(colour),
            &plan.settings,
            ctx.out.stdout(),
        )
        .map_err(WriteError::from_api);
    ctx.human_done(written, code, FOLDER_COMPARE_SCHEMA, &warnings, op)
}

/// The request of the flags (§1.9.2 mapping): every individual flag becomes one `FolderRulesPatch` field (the
/// unix flags one field each, §1.8.7 item 19); filter expressions are parsed by hikasa-folder (`--cutoff` with
/// the clock's zone), attribute letters by hikasa-vfs, the content method by the session.
fn request(ctx: &Ctx<'_>, args: &FolderCompareArgs) -> Result<FolderCompareRequest, ApiError> {
    let zone = ctx.service.clock().now().time_zone().clone();
    let patch = FolderRulesPatch {
        size: tri(args.size, args.no_size),
        timestamps_enabled: tri(args.timestamps, args.no_timestamps),
        tolerance_secs: args.tolerance,
        ignore_dst: args.ignore_dst.then_some(true),
        ignore_timezone: args.ignore_timezone.then_some(true),
        filename_case: args.filename_case.then_some(true),
        dos_attributes: args
            .attributes
            .as_deref()
            .map(DosAttrs::parse_letters)
            .transpose()?,
        unix_permissions: args.permissions.then_some(true),
        unix_owner: args.owner.then_some(true),
        unix_group: args.group.then_some(true),
        content_method: args
            .content
            .as_deref()
            .map(parse_content_method)
            .transpose()?,
        skip_if_quick_same: args.skip_if_quick_same.then_some(true),
        override_quick: args.no_override_quick.then_some(false),
        case: if args.case_sensitive {
            Some(CaseMode::Sensitive)
        } else if args.case_insensitive {
            Some(CaseMode::Insensitive)
        } else {
            None
        },
        align_extensions: args.align_extensions.then_some(true),
        align_normalization: args.align_normalization.then_some(true),
        name_filter: args.filters.clone(),
        exclude_protected: args.include_protected.then_some(false),
        size_filter: args
            .size_filter
            .as_deref()
            .map(SizeFilter::parse)
            .transpose()?
            .map(Change::Set),
        date_filter: args
            .cutoff
            .as_deref()
            .map(|expr| DateFilter::parse(expr, &zone))
            .transpose()?
            .map(Change::Set),
        follow_symlinks: args.follow_symlinks.then_some(true),
        flatten: args.flatten.then_some(true),
        scan_orphan_folders: args.no_scan_orphans.then_some(false),
        archives: args
            .archives
            .as_deref()
            .map(parse_archive_handling)
            .transpose()?,
        open_archives: (!args.open_archive.is_empty()).then(|| args.open_archive.clone()),
        ..FolderRulesPatch::default()
    };
    Ok(FolderCompareRequest {
        left: args.left.clone(),
        right: args.right.clone(),
        rules_file: args.rules_file.clone(),
        criteria: args.criteria.clone(),
        patch,
        text_rules_file: args.text_rules_file.clone(),
        text_patch: importance_patch(&args.importance, ctx.matches)?,
        ignore_unimportant: args.ignore_unimportant,
        base_rules: None,
        base_text_rules: None,
    })
}

/// The parsed presentation words of one invocation.
struct Plan {
    /// Display filter, folder mode and suppress-filters (the effective ignore-unimportant replaces the flag).
    display: DisplayOptions,
    layout: FolderLayout,
    columns: FolderColumns,
    html: Option<bool>,
    settings: ReportSettings,
}

impl Plan {
    fn new(args: &FolderCompareArgs) -> Result<Plan, ApiError> {
        let display = resolve_folder_display(
            args.display.as_deref(),
            args.folders.as_deref(),
            args.suppress_filters,
            args.ignore_unimportant,
        )?;
        let layout = FolderLayout::parse(&args.report)?;
        let columns = match args.columns.as_deref() {
            Some(list) => FolderColumns::parse_list(list)?,
            None => FolderColumns::default(),
        };
        let html = if args.html_mono {
            Some(true)
        } else if args.html {
            Some(false)
        } else {
            None
        };
        Ok(Plan {
            display,
            layout,
            columns,
            html,
            settings: ReportSettings {
                title: args.title.clone(),
                ..ReportSettings::default()
            },
        })
    }

    /// The report options; `ansi` only for a terminal (never for `--output`).
    fn report(&self, ansi: bool) -> FolderReportOptions {
        FolderReportOptions {
            layout: self.layout,
            display: self.display,
            columns: self.columns,
            format: match self.html {
                Some(mono) => OutputFormat::Html { mono },
                None => OutputFormat::Text { ansi },
            },
        }
    }
}
