//! `hikasa text convert <FILE>…` (TC-063, D-280; contract §1.10.15, §1.10.24): `Service::text_convert_plan`, the save
//! questions through the confirm driver (`--yes`, `--confirm`, `--force`; `--unmappable` decides the plan), then
//! `Service::text_convert_execute` (or `text_convert_dry_run`). The files are read, converted and written by the
//! Service (the save plan of §1.10.13): nothing here names the save internals (§1.10.25 check 7).
//!
//! Output: `hikasa.text_convert.v1` with `--json`; else one line per file (`converted <n> lines: <spec>`, `would
//! convert …` with `--dry-run`) and, with `--verbose`, the plan's lines. Without `--in-place` or `--output` the
//! converted bytes go to stdout and the lines to stderr (so `--json` needs `--dry-run` there). A run that wrote with
//! failed files is `ok: true` with exit 100; a refused request (nothing to convert, open questions without a terminal)
//! is `ok: false`, exit 100, nothing written.

use std::io::{self, IsTerminal, Write};

use hikasa_core::{JobKind, Warning};
use hikasa_session::ApiError;
use hikasa_session::api::dto::TEXT_CONVERT_SCHEMA;
use hikasa_session::api::{
    ApiOp, ConvertTarget, SaveAnswers, TabsConversion, TextConvertOutcome, TextConvertRequest,
    parse_confirm_mode, parse_line_ending, parse_line_range, parse_unmappable,
};
use hikasa_text::save::UnmappablePolicy;

use super::Ctx;
use crate::args::TextConvertArgs;
use crate::confirm::ask;
use crate::output::stderr_line;

/// The operation of the exit codes.
const OP: ApiOp = ApiOp::TextConvert;

/// Runs `text convert`.
pub fn run(ctx: &mut Ctx<'_>, args: &TextConvertArgs) -> u8 {
    match convert(ctx, args) {
        Ok(code) => code,
        Err(failure) => ctx.fail(TEXT_CONVERT_SCHEMA, &failure.0, OP, &failure.1),
    }
}

/// A failure, with the warnings seen so far.
type Failure = Box<(ApiError, Vec<Warning>)>;

fn bare<T>(result: Result<T, ApiError>) -> Result<T, Failure> {
    result.map_err(|e| Box::new((e, Vec::new())))
}

/// The request of the flags. `--unmappable` decides the plan's policy: without it a terminal asks (in a plan), and
/// without a terminal the character refuses its file.
fn request_of(args: &TextConvertArgs, terminal: bool) -> Result<TextConvertRequest, ApiError> {
    let mode = args
        .confirm
        .as_deref()
        .map(parse_confirm_mode)
        .transpose()?
        .unwrap_or_default();
    let mut answers = SaveAnswers::of_mode(mode);
    answers.yes |= args.yes;
    answers.force = args.force;
    let target = match (&args.output, args.in_place) {
        (Some(output), _) => ConvertTarget::Output(output.clone()),
        (None, true) => ConvertTarget::InPlace,
        (None, false) => ConvertTarget::Stdout,
    };
    let unmappable = match &args.unmappable {
        Some(word) => parse_unmappable(word)?,
        None if terminal => UnmappablePolicy::Ask,
        None => UnmappablePolicy::Refuse,
    };
    Ok(TextConvertRequest {
        files: args.files.clone(),
        trim_trailing: args.trim_trailing,
        tabs: if args.tabs_to_spaces {
            Some(TabsConversion::ToSpaces)
        } else if args.spaces_to_tabs {
            Some(TabsConversion::ToTabs)
        } else {
            None
        },
        tab_size: args.tab_size,
        line_endings: args
            .line_endings
            .as_deref()
            .map(parse_line_ending)
            .transpose()?,
        encoding: args.encoding.clone(),
        bom: if args.bom {
            Some(true)
        } else if args.no_bom {
            Some(false)
        } else {
            None
        },
        lines: args.lines.as_deref().map(parse_line_range).transpose()?,
        format: args.format.clone(),
        encoding_in: args.encoding_in.clone(),
        unmappable,
        target,
        backup: if args.backup {
            Some(true)
        } else if args.no_backup {
            Some(false)
        } else {
            None
        },
        answers,
        dry_run: args.dry_run,
    })
}

fn convert(ctx: &mut Ctx<'_>, args: &TextConvertArgs) -> Result<u8, Failure> {
    let terminal = io::stdin().is_terminal();
    let req = bare(request_of(args, terminal))?;
    let to_stdout = req.target == ConvertTarget::Stdout && !req.dry_run;
    if to_stdout && ctx.out.json() {
        return Err(Box::new((
            ApiError::InvalidArgument(
                "--json needs --in-place, --output or --dry-run: the converted text goes to stdout"
                    .to_owned(),
            ),
            Vec::new(),
        )));
    }
    let job = ctx.job(JobKind::TextConvert);
    let plan = bare(ctx.service.text_convert_plan(&req, &job))?;
    ctx.progress.clear();
    let outcome = if req.dry_run {
        ctx.service.text_convert_dry_run(plan)
    } else {
        let plan = match ctx.service.text_convert_confirmation(&plan) {
            Some(mut driver) => {
                if driver.next().is_some() {
                    if !terminal && let Some(refusal) = driver.refusal() {
                        return Err(Box::new((refusal, Vec::new())));
                    }
                    let mut stderr = io::stderr();
                    ask(&mut driver, &job.cancel, &mut stderr);
                    let _ = stderr.flush();
                }
                bare(plan.answered(driver))?
            }
            None => plan,
        };
        let out = ctx.out.stdout();
        bare(ctx.service.text_convert_execute(plan, out, &job))?
    };
    ctx.progress.clear();
    Ok(print(ctx, &outcome, to_stdout))
}

/// `hikasa.text_convert.v1`, or one line per file (and the plan's lines with `--verbose`); on stderr when stdout holds
/// the converted bytes.
fn print(ctx: &mut Ctx<'_>, outcome: &TextConvertOutcome, to_stdout: bool) -> u8 {
    let code = outcome.exit_code.code();
    if ctx.out.json() {
        return ctx.json_success(
            TEXT_CONVERT_SCHEMA,
            code,
            &outcome.dto,
            &outcome.warnings,
            OP,
        );
    }
    let mut lines = outcome.human_lines();
    if ctx.matches.get_flag("verbose") {
        lines.extend(outcome.plan_lines.iter().cloned());
    }
    if to_stdout {
        if !ctx.out.quiet() {
            for line in &lines {
                stderr_line(line);
            }
        }
        return ctx.human_done(Ok(()), code, TEXT_CONVERT_SCHEMA, &outcome.warnings, OP);
    }
    let written = ctx.out.lines(lines);
    ctx.human_done(written, code, TEXT_CONVERT_SCHEMA, &outcome.warnings, OP)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::args::{Cli, Command, TextCmd};

    fn request(argv: &[&str], terminal: bool) -> Result<TextConvertRequest, ApiError> {
        let mut all = vec!["hikasa", "text", "convert"];
        all.extend_from_slice(argv);
        let cli = Cli::try_parse_from(all).unwrap();
        let Command::Text {
            cmd: TextCmd::Convert(args),
        } = cli.command
        else {
            panic!("not text convert");
        };
        request_of(&args, terminal)
    }

    /// A10.15's CLI drivers: the flags answer their classes; `--unmappable` sets the plan's policy, and without it a
    /// terminal asks while no terminal refuses.
    #[test]
    fn a10_15_the_flags_answer_their_questions() {
        let req = request(&["a", "--trim-trailing", "--force"], false).unwrap();
        assert!(req.answers.force && !req.answers.yes);
        assert_eq!(req.unmappable, UnmappablePolicy::Refuse);
        let req = request(&["a", "--trim-trailing", "--yes"], true).unwrap();
        assert!(req.answers.yes && !req.answers.force);
        assert_eq!(req.unmappable, UnmappablePolicy::Ask);
        let req = request(&["a", "--trim-trailing", "--confirm", "yes-to-all"], true).unwrap();
        assert!(req.answers.yes);
        let req = request(&["a", "--trim-trailing", "--confirm", "no-to-all"], true).unwrap();
        assert!(req.answers.no_to_all && !req.answers.yes);
        let req = request(
            &["a", "--encoding", "latin1", "--unmappable", "replace"],
            false,
        )
        .unwrap();
        assert_eq!(req.unmappable, UnmappablePolicy::Replace);
        let req = request(
            &["a", "--encoding", "latin1", "--unmappable", "refuse"],
            true,
        )
        .unwrap();
        assert_eq!(req.unmappable, UnmappablePolicy::Refuse);
        assert!(request(&["a", "--trim-trailing", "--confirm", "maybe"], true).is_err());
    }

    #[test]
    fn the_flags_make_the_request() {
        let req = request(
            &[
                "a",
                "b",
                "--trim-trailing",
                "--spaces-to-tabs",
                "--tab-size",
                "8",
                "--line-endings",
                "CRLF",
                "--encoding",
                "UTF-16LE",
                "--no-bom",
                "--lines",
                "2-5",
                "--format",
                "c",
                "--encoding-in",
                "latin1",
                "--in-place",
                "--backup",
                "--dry-run",
            ],
            false,
        )
        .unwrap();
        assert_eq!(req.files.len(), 2);
        assert!(req.trim_trailing);
        assert_eq!(req.tabs, Some(TabsConversion::ToTabs));
        assert_eq!(req.tab_size, Some(8));
        assert_eq!(req.line_endings, Some(hikasa_text::LineEnding::CrLf));
        assert_eq!(req.encoding.as_deref(), Some("UTF-16LE"));
        assert_eq!(req.bom, Some(false));
        assert_eq!(req.lines, Some(2..=5));
        assert_eq!(req.format.as_deref(), Some("c"));
        assert_eq!(req.encoding_in.as_deref(), Some("latin1"));
        assert_eq!(req.target, ConvertTarget::InPlace);
        assert_eq!(req.backup, Some(true));
        assert!(req.dry_run);
        let req = request(&["a", "--tabs-to-spaces", "-o", "x", "--no-backup"], false).unwrap();
        assert_eq!(req.tabs, Some(TabsConversion::ToSpaces));
        assert_eq!(req.target, ConvertTarget::Output("x".into()));
        assert_eq!(req.backup, Some(false));
        let req = request(&["a", "--bom"], false).unwrap();
        assert_eq!(req.target, ConvertTarget::Stdout);
        assert_eq!(req.bom, Some(true));
        let parse = |argv: &[&str]| Cli::try_parse_from(argv).is_err();
        assert!(parse(&[
            "hikasa", "text", "convert", "a", "--bom", "--no-bom"
        ]));
        assert!(parse(&[
            "hikasa",
            "text",
            "convert",
            "a",
            "--in-place",
            "--output",
            "x"
        ]));
        assert!(parse(&["hikasa", "text", "convert"]));
    }
}
