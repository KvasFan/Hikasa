//! `hikasa text compare <left> <right>` (§3.2): `Service::text_compare`, then the human report
//! (`Service::render_text_with`; the annotated unified diff by default) or `hikasa.text_compare.v1`
//! (`TextCompareResponse::json`, display `mismatches` by default). `--output` writes the report atomically through
//! `Service::write_output` and prints nothing (with `--json` the envelope still goes to stdout). `--gui` sends the
//! request to the GUI instead (D-50).

use hikasa_core::{JobKind, Pair};
use hikasa_report::{OutputFormat, PatchStyle, TextLayout, TextReportOptions, Wrap};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{GUI_OPEN_SCHEMA, TEXT_COMPARE_SCHEMA};
use hikasa_session::api::{
    ApiOp, GuiOpenRequest, ReportSettings, TextCompareRequest, TextCompareResponse, ViewWords,
    parse_algorithm, resolve_text_display,
};
use hikasa_text::ViewOptions;

use super::{Ctx, gui, importance_patch, tri};
use crate::args::TextCompareArgs;
use crate::output::WriteError;

/// Runs `text compare`.
pub fn run(ctx: &mut Ctx<'_>, args: &TextCompareArgs) -> u8 {
    let request = match request(ctx, args) {
        Ok(request) => request,
        Err(e) => return fail(ctx, args, &e),
    };
    if args.gui {
        let words = ViewWords {
            display: args.display.clone(),
            context_lines: args.context,
            ..ViewWords::default()
        };
        let open = GuiOpenRequest::for_text(&request, &words, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    execute(ctx, args, &request)
}

/// Compares and prints (`text compare` without `--gui`, and `session run` of a Text Compare session, T11):
/// `args` gives the presentation words, `request` the comparison.
pub fn execute(ctx: &mut Ctx<'_>, args: &TextCompareArgs, request: &TextCompareRequest) -> u8 {
    let op = ApiOp::TextCompare;
    // Every word is checked before the comparison runs, so a typo fails at once (exit 100). A layout/format
    // combination the renderer does not support (`--report patch --html`) is refused by the renderer after the
    // comparison (exit 100, `--output` untouched): checking it first needs a hikasa-report addition (§1.9.3 item
    // 10a); a table here would be a spec rule of the adapter (T11).
    let plan = match Plan::new(ctx, args) {
        Ok(plan) => plan,
        Err(e) => return ctx.fail(TEXT_COMPARE_SCHEMA, &e, op, &[]),
    };
    let job = ctx.job(JobKind::TextCompare);
    let resp = match ctx.service.text_compare(request, &job, None) {
        Ok(resp) => resp,
        Err(e) => return ctx.fail(TEXT_COMPARE_SCHEMA, &e, op, &[]),
    };
    ctx.progress.clear();
    let code = resp.exit_code.code();
    let mut warnings = resp.warnings.clone();
    // A Ctrl-C that came as the comparison ended stops the command the same way: `--output` stays (D-159).
    if job.cancel.is_cancelled() {
        return ctx.fail(TEXT_COMPARE_SCHEMA, &ApiError::Cancelled, op, &warnings);
    }
    if let Some(target) = &args.output {
        let mut report = Vec::new();
        let rendered = ctx
            .service
            .render_text_with(
                &resp,
                &plan.report(&resp, false),
                &plan.settings,
                &mut report,
            )
            .and_then(|w| ctx.service.write_output(target, &report).map(|_| w));
        match rendered {
            Ok(w) => warnings.extend(w),
            Err(e) => return ctx.fail(TEXT_COMPARE_SCHEMA, &e, op, &warnings),
        }
        if !ctx.out.json() {
            return ctx.human_done(Ok(()), code, TEXT_COMPARE_SCHEMA, &warnings, op);
        }
    }
    if ctx.out.json() {
        let view = ViewOptions {
            filter: plan.json_display.text_filter(),
            ignore_unimportant: resp.ignore_unimportant,
            expanded: Vec::new(),
        };
        return ctx.json_success(TEXT_COMPARE_SCHEMA, code, resp.json(&view), &warnings, op);
    }
    let colour = ctx.out.colour();
    let written = ctx
        .service
        .render_text_with(
            &resp,
            &plan.report(&resp, colour),
            &plan.settings,
            ctx.out.stdout(),
        )
        .map(|w| warnings.extend(w))
        .map_err(WriteError::from_api);
    ctx.human_done(written, code, TEXT_COMPARE_SCHEMA, &warnings, op)
}

/// A failure before the comparison: the GUI's document and operation with `--gui`.
fn fail(ctx: &mut Ctx<'_>, args: &TextCompareArgs, err: &ApiError) -> u8 {
    let (schema, op) = if args.gui {
        (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: args.wait })
    } else {
        (TEXT_COMPARE_SCHEMA, ApiOp::TextCompare)
    };
    ctx.fail(schema, err, op, &[])
}

/// The request of the flags (§1.9.2 mapping): `--encoding` fills both sides, `--left-encoding` and
/// `--right-encoding` replace one; the rules patch carries the alignment and importance flags.
fn request(ctx: &Ctx<'_>, args: &TextCompareArgs) -> Result<TextCompareRequest, ApiError> {
    let mut rules_patch = importance_patch(&args.importance, ctx.matches)?;
    rules_patch.algorithm = args.algorithm.as_deref().map(parse_algorithm).transpose()?;
    rules_patch.skew_tolerance = args.skew;
    rules_patch.closeness = tri(args.closeness, args.no_closeness);
    rules_patch.closeness_threshold_permille = args.closeness_threshold;
    rules_patch.never_align_differences = args.never_align.then_some(true);
    rules_patch.compare_line_endings = args.compare_line_endings.then_some(true);
    rules_patch.orphans_always_important = args.orphans_unimportant.then_some(false);
    Ok(TextCompareRequest {
        left: args.left.clone(),
        right: args.right.clone(),
        format: args.format.clone(),
        encodings: Pair::new(
            args.left_encoding.clone().or_else(|| args.encoding.clone()),
            args.right_encoding
                .clone()
                .or_else(|| args.encoding.clone()),
        ),
        rules_file: args.rules_file.clone(),
        rules_patch,
        ignore_unimportant: args.ignore_unimportant,
        base_rules: None,
    })
}

/// The parsed presentation words of one invocation.
struct Plan {
    /// Display of the JSON document (default `mismatches`).
    json_display: hikasa_report::DisplayMode,
    /// Display of the human report (default `all`).
    human_display: hikasa_report::DisplayMode,
    layout: TextLayout,
    wrap: Wrap,
    line_numbers: bool,
    width: u16,
    html: Option<bool>,
    settings: ReportSettings,
}

impl Plan {
    fn new(ctx: &Ctx<'_>, args: &TextCompareArgs) -> Result<Plan, ApiError> {
        let default_context = ctx.service.options().context_lines;
        let lines = args.context.unwrap_or(default_context);
        let display = args.display.as_deref();
        let json_display = resolve_text_display(display, args.context, default_context, true)?;
        let human_display = resolve_text_display(display, args.context, default_context, false)?;
        let patch = PatchStyle::parse(&args.patch_style, lines)?;
        let layout = TextLayout::parse(&args.report, patch, lines)?;
        let wrap = Wrap::parse(&args.wrap)?;
        let html = if args.html_mono {
            Some(true)
        } else if args.html {
            Some(false)
        } else {
            None
        };
        Ok(Plan {
            json_display,
            human_display,
            layout,
            wrap,
            line_numbers: !args.no_line_numbers,
            width: args.width,
            html,
            settings: ReportSettings {
                title: args.title.clone(),
                ..ReportSettings::default()
            },
        })
    }

    /// The report options; `ansi` only for a terminal (never for `--output`).
    fn report(&self, resp: &TextCompareResponse, ansi: bool) -> TextReportOptions {
        TextReportOptions {
            layout: self.layout,
            display: self.human_display,
            ignore_unimportant: resp.ignore_unimportant,
            line_numbers: self.line_numbers,
            width: self.width,
            wrap: self.wrap,
            format: match self.html {
                Some(mono) => OutputFormat::Html { mono },
                None => OutputFormat::Text { ansi },
            },
        }
    }
}
