//! `hikasa hex compare|dump|find` (M2 W12, §1.12.23): the words as typed go to the Service, which parses them (T11,
//! check 7). `hex compare` exits as D-331 (1, 2, 11, 13, 100); `hex dump` and `hex find` exit 0 (100 for an error);
//! `--gui` sends the words to the window's Hex Compare view after checking them here (D-350).

use hikasa_core::{JobKind, Pair};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{GUI_OPEN_SCHEMA, HEX_DUMP_SCHEMA, HEX_FIND_SCHEMA};
use hikasa_session::api::{
    ApiOp, GuiOpenRequest, HexCompareRequest, HexDumpRequest, HexFindEvent, HexFindRequest,
    HexFindSink, HexLayoutWords, HexRuleWords, parse_find_archives,
};

use super::report::{Shape, asks_report, run_hex};
use super::{Ctx, gui};
use crate::args::{HexCmd, HexCompareArgs, HexDumpArgs, HexFindArgs, HexLayoutArgs};

/// Runs `hex …`.
pub fn run(ctx: &mut Ctx<'_>, cmd: &HexCmd) -> u8 {
    match cmd {
        HexCmd::Compare(args) => compare(ctx, args),
        HexCmd::Dump(args) => dump(ctx, args),
        HexCmd::Find(args) => find(ctx, args),
    }
}

/// The layout words of the flags.
fn layout_words(l: &HexLayoutArgs) -> HexLayoutWords {
    HexLayoutWords {
        bytes_per_line: l.bytes_per_line.clone(),
        group: l.group.clone(),
        chars: l.chars.clone(),
        address: l.address.clone(),
        context: None,
        display: None,
    }
}

/// The request of `hex compare`'s flags (the words as typed).
pub fn request_of(ctx: &Ctx<'_>, args: &HexCompareArgs) -> HexCompareRequest {
    let mut layout = layout_words(&args.layout);
    layout.context.clone_from(&args.context);
    layout.display.clone_from(&args.display);
    HexCompareRequest {
        left: args.left.clone(),
        right: args.right.clone(),
        format: args.format.clone(),
        formats: Pair::new(args.left_format.clone(), args.right_format.clone()),
        rules: HexRuleWords {
            alignment: args.alignment.clone(),
            block: args.block.clone(),
            skew: args.skew.clone(),
            window: args.window.clone(),
            budget_mib: args.budget.clone(),
            unlocked_max_mib: args.unlocked_max.clone(),
        },
        layout,
        max_sections: args.max_sections.clone(),
        preview: args.preview.clone(),
        format_overrides: Default::default(),
        verbose: ctx
            .matches
            .try_get_one::<bool>("verbose")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
    }
}

fn compare(ctx: &mut Ctx<'_>, args: &HexCompareArgs) -> u8 {
    let request = request_of(ctx, args);
    if args.gui {
        let op = ApiOp::GuiOpen { wait: args.wait };
        // The window is the output (T-52's rule): a report, `--max-sections` or `--preview` has nowhere to go.
        if asks_report(&args.report.report_args())
            || args.max_sections.is_some()
            || args.preview.is_some()
        {
            let refused = ApiError::InvalidArgument(
                "the report flags, --max-sections and --preview do not apply with --gui (the window shows the \
                 comparison)"
                    .to_owned(),
            );
            return ctx.fail(GUI_OPEN_SCHEMA, &refused, op, &[]);
        }
        // Every word is checked here first: a bad word exits 100 without contacting a GUI (D-350).
        if let Err(e) = ctx.service.check_hex_words(&request) {
            return ctx.fail(GUI_OPEN_SCHEMA, &e, op, &[]);
        }
        let open = GuiOpenRequest::for_hex(&request, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    run_hex(ctx, &args.report.report_args(), request, Shape::Compare)
}

fn dump(ctx: &mut Ctx<'_>, args: &HexDumpArgs) -> u8 {
    let op = ApiOp::HexDump;
    let request = HexDumpRequest {
        spec: args.spec.clone(),
        offset: args.offset.clone(),
        length: args.length.clone(),
        layout: layout_words(&args.layout),
        values: args.values,
        format: args.format.clone(),
    };
    let json = ctx.out.json();
    if json && (args.layout.group.is_some() || args.layout.address.is_some()) {
        let refused = ApiError::InvalidArgument(
            "--group and --address shape the human output, not the JSON document".to_owned(),
        );
        return ctx.fail(HEX_DUMP_SCHEMA, &refused, op, &[]);
    }
    let job = ctx.job(JobKind::HexCompare);
    let resp = match ctx.service.hex_dump(&request, &job) {
        Ok(resp) => resp,
        Err(e) => return ctx.fail(HEX_DUMP_SCHEMA, &e, op, &[]),
    };
    let warnings = resp.warnings.clone();
    if json {
        return ctx.json_success(HEX_DUMP_SCHEMA, 0, resp.json(), &warnings, op);
    }
    let written = ctx.out.lines(resp.human_lines());
    ctx.human_done(written, 0, HEX_DUMP_SCHEMA, &warnings, op)
}

/// What `hex find` prints per event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `spec:0x…` per hit.
    Hits,
    /// `spec` once per file with a hit.
    Files,
    /// `spec:<n>` per file with a hit.
    Count,
}

/// Collects the human lines of the events.
struct Lines {
    mode: Mode,
    lines: Vec<String>,
}

impl HexFindSink for Lines {
    fn event(&mut self, e: HexFindEvent<'_>) {
        match (self.mode, e) {
            (Mode::Hits, HexFindEvent::Hit { spec, offset }) => {
                self.lines.push(format!("{spec}:0x{offset:X}"));
            }
            (Mode::Files, HexFindEvent::File(file)) => self.lines.push(file.spec.clone()),
            (Mode::Count, HexFindEvent::File(file)) => {
                self.lines.push(format!("{}:{}", file.spec, file.hit_count));
            }
            _ => {}
        }
    }
}

fn find(ctx: &mut Ctx<'_>, args: &HexFindArgs) -> u8 {
    let op = ApiOp::HexFind;
    let archives = match args
        .archives
        .as_deref()
        .map(parse_find_archives)
        .transpose()
    {
        Ok(a) => a.unwrap_or_default(),
        Err(e) => return ctx.fail(HEX_FIND_SCHEMA, &e, op, &[]),
    };
    let mode = if args.count {
        Mode::Count
    } else if args.files_with_matches {
        Mode::Files
    } else {
        Mode::Hits
    };
    let request = HexFindRequest {
        pattern: args.pattern.clone(),
        roots: args.paths.clone(),
        hex: args.hex,
        encoding: args.encoding.clone(),
        match_case: args.case,
        recursive: args.recursive,
        mask: args.mask.clone(),
        include_hidden: args.include_hidden,
        archives,
        max_matches: args.max_matches,
        max_matches_per_file: args.max_matches_per_file,
        keep_hits: ctx.out.json() && mode == Mode::Hits,
    };
    let job = ctx.job(JobKind::HexFind);
    let mut sink = Lines {
        mode,
        lines: Vec::new(),
    };
    let outcome = match ctx.service.hex_find(&request, &mut sink, &job) {
        Ok(outcome) => outcome,
        Err(e) => return ctx.fail(HEX_FIND_SCHEMA, &e, op, &[]),
    };
    let warnings = outcome.warnings.clone();
    if ctx.out.json() {
        return ctx.json_success(HEX_FIND_SCHEMA, 0, &outcome.json, &warnings, op);
    }
    let written = ctx.out.lines(sink.lines);
    // A skipped file is a warning-like note on stderr (the walk goes on).
    for skipped in &outcome.json.skipped {
        if let Some(error) = &skipped.error {
            crate::output::stderr_line(&format!("skipped: {}: {}", skipped.spec, error.message));
        }
    }
    ctx.human_done(written, 0, HEX_FIND_SCHEMA, &warnings, op)
}
