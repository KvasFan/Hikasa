//! `hikasa text find <PATTERN> <PATH>…` (TC-064, D-285; contract §1.10.19, §1.10.24): `Service::text_find` with a
//! printer as the sink, or with `--replace` `Service::text_find_replace_plan`, the questions through the confirm driver
//! (the operation question *Replace N occurrences in M files?*, then the save questions) and
//! `Service::text_find_replace_execute` (or `text_find_replace_dry_run`). Files are read, searched and written by the
//! Service: nothing here names the find or save internals (§1.10.25 check 7).
//!
//! Output: `hikasa.text_find.v1` with `--json`; else the hits as they are found, `<spec>:<line>:<column>: <preview>`
//! (`--files-with-matches`: each file with a hit once; `--count`: `<spec>:<n>` for each file with a hit), the skipped
//! files on stderr with `--verbose`; with `--replace` one line per file with a hit (`replaced <n> occurrences: <spec>`,
//! `would replace …` with `--dry-run`) and, with `--dry-run` or `--verbose`, the plan's lines. File content and names
//! are escaped (no control character reaches the terminal). A reader that goes away stops the search (exit 0).
//!
//! Exit codes: 0 whether or not anything matched; 100 for an invalid pattern, a missing root, a cancel, open questions
//! without a terminal (nothing written) and a replace with a failed file (`ok: true` when it wrote others).

use std::io::{self, IsTerminal, Write};

use hikasa_core::{CancelToken, JobKind, Warning};
use hikasa_report::{escape_controls, escape_terminal};
use hikasa_session::ApiError;
use hikasa_session::api::dto::TEXT_FIND_SCHEMA;
use hikasa_session::api::{
    ApiOp, NoEvents, SaveAnswers, TextFindOutcome, TextFindReplace, TextFindReplacePlan,
    TextFindRequest, parse_byte_size, parse_confirm_mode, parse_find_archives, parse_unmappable,
};
use hikasa_text::find::files::{FilesEvent, FilesSink};
use hikasa_text::save::UnmappablePolicy;

use super::Ctx;
use crate::args::TextFindArgs;
use crate::confirm::ask;
use crate::output::{WriteError, stderr_line};
use crate::progress::ProgressLine;

/// The operation of the exit codes.
const OP: ApiOp = ApiOp::TextFind;

/// Runs `text find`.
pub fn run(ctx: &mut Ctx<'_>, args: &TextFindArgs) -> u8 {
    match find(ctx, args) {
        Ok(code) => code,
        Err(failure) => ctx.fail(TEXT_FIND_SCHEMA, &failure.0, OP, &failure.1),
    }
}

/// A failure, with the warnings seen so far.
type Failure = Box<(ApiError, Vec<Warning>)>;

fn bare<T>(result: Result<T, ApiError>) -> Result<T, Failure> {
    result.map_err(|e| Box::new((e, Vec::new())))
}

/// How the hits are printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// `<spec>:<line>:<column>: <preview>` per hit.
    Lines,
    /// `--files-with-matches`: `<spec>` once per file with a hit.
    Files,
    /// `--count`: `<spec>:<n>` per file with a hit.
    Count,
}

/// The request of the flags (`json`: the document keeps the hits). `--unmappable` decides the plan's policy: without
/// it a terminal asks (in the plan), and without a terminal the character fails its file.
fn request_of(
    args: &TextFindArgs,
    terminal: bool,
    json: bool,
) -> Result<TextFindRequest, ApiError> {
    let replace = match &args.replace {
        None => None,
        Some(text) => {
            let mode = args
                .confirm
                .as_deref()
                .map(parse_confirm_mode)
                .transpose()?
                .unwrap_or_default();
            let mut answers = SaveAnswers::of_mode(mode);
            answers.yes |= args.yes;
            let unmappable = match &args.unmappable {
                Some(word) => parse_unmappable(word)?,
                None if terminal => UnmappablePolicy::Ask,
                None => UnmappablePolicy::Refuse,
            };
            Some(TextFindReplace {
                text: text.clone(),
                dry_run: args.dry_run,
                answers,
                backup: if args.backup {
                    Some(true)
                } else if args.no_backup {
                    Some(false)
                } else {
                    None
                },
                unmappable,
            })
        }
    };
    Ok(TextFindRequest {
        pattern: args.pattern.clone(),
        roots: args.paths.clone(),
        regex: args.regex,
        match_case: args.case,
        whole_word: args.word,
        recursive: args.recursive,
        mask: args.mask.clone(),
        include_hidden: args.include_hidden,
        archives: args
            .archives
            .as_deref()
            .map(parse_find_archives)
            .transpose()?
            .unwrap_or_default(),
        encoding: args.encoding.clone(),
        format: args.format.clone(),
        convert: args.convert,
        max_matches: args.max_matches,
        max_matches_per_file: args.max_matches_per_file,
        max_file_size: args
            .max_file_size
            .as_deref()
            .map(|word| parse_byte_size(word, "--max-file-size"))
            .transpose()?,
        keep_hits: json,
        replace,
    })
}

/// Prints the events as they come (the sink of the search).
struct Printer<'a> {
    out: &'a mut dyn Write,
    mode: Mode,
    /// Skipped files on stderr.
    verbose: bool,
    /// Flush each line and clear the progress line first (stdout is a terminal).
    terminal: Option<&'a ProgressLine>,
    /// The current file, escaped.
    spec: String,
    /// The current file's name was printed (`--files-with-matches`).
    printed: bool,
    /// Cancelled when stdout fails: nobody reads the rest.
    stop: CancelToken,
    /// The first write error.
    error: Option<io::Error>,
}

impl Printer<'_> {
    fn line(&mut self, text: &str) {
        if self.error.is_some() {
            return;
        }
        if let Some(progress) = self.terminal {
            progress.clear();
        }
        let mut written = self
            .out
            .write_all(text.as_bytes())
            .and_then(|()| self.out.write_all(b"\n"));
        if written.is_ok() && self.terminal.is_some() {
            written = self.out.flush();
        }
        if let Err(e) = written {
            self.error = Some(e);
            self.stop.cancel();
        }
    }
}

impl FilesSink for Printer<'_> {
    fn event(&mut self, e: FilesEvent<'_>) {
        match e {
            FilesEvent::File { spec, .. } => {
                self.spec = escape_controls(&spec.canonical()).into_owned();
                self.printed = false;
            }
            FilesEvent::Hit {
                line,
                column,
                preview,
                ..
            } => match self.mode {
                Mode::Lines => {
                    let text = format!(
                        "{}:{line}:{column}: {}",
                        self.spec,
                        escape_terminal(preview)
                    );
                    self.line(&text);
                }
                Mode::Files if !self.printed => {
                    self.printed = true;
                    let text = self.spec.clone();
                    self.line(&text);
                }
                Mode::Files | Mode::Count => {}
            },
            FilesEvent::Done { hits, .. } => {
                if self.mode == Mode::Count && hits > 0 {
                    let text = format!("{}:{hits}", self.spec);
                    self.line(&text);
                }
            }
            FilesEvent::Skipped { spec, reason } => {
                if self.verbose {
                    if let Some(progress) = self.terminal {
                        progress.clear();
                    }
                    let why = match &reason {
                        hikasa_text::find::files::SkipWhy::Unreadable(error) => {
                            format!(": {}", error.message)
                        }
                        _ => String::new(),
                    };
                    stderr_line(&format!(
                        "skipped ({}): {}{why}",
                        reason.as_str(),
                        spec.canonical()
                    ));
                }
            }
        }
    }
}

/// What the search came to.
enum Found {
    /// A search.
    Outcome(Box<TextFindOutcome>),
    /// A replace, planned.
    Plan(Box<TextFindReplacePlan>),
}

fn find(ctx: &mut Ctx<'_>, args: &TextFindArgs) -> Result<u8, Failure> {
    let terminal = io::stdin().is_terminal();
    let json = ctx.out.json();
    let req = bare(request_of(args, terminal, json))?;
    let mut job = ctx.job(JobKind::TextFind);
    // A child of Ctrl-C's token: a reader that goes away stops this search only.
    let stop = job.cancel.child();
    job.cancel = stop.clone();
    let mode = if args.files_with_matches {
        Mode::Files
    } else if args.count {
        Mode::Count
    } else {
        Mode::Lines
    };
    let verbose = ctx.matches.get_flag("verbose");
    let quiet = ctx.out.quiet();
    let stdout_terminal = !json && io::stdout().is_terminal();
    let service = ctx.service;
    let progress = ctx.progress;
    let (result, write_error) = {
        let mut printer = Printer {
            out: ctx.out.stdout(),
            mode,
            verbose: verbose && !quiet,
            terminal: stdout_terminal.then_some(progress),
            spec: String::new(),
            printed: false,
            stop,
            error: None,
        };
        let mut none = NoEvents;
        let sink: &mut dyn FilesSink = if json { &mut none } else { &mut printer };
        let result = match &req.replace {
            None => service
                .text_find(&req, sink, &job)
                .map(|outcome| Found::Outcome(Box::new(outcome))),
            Some(_) => service
                .text_find_replace_plan(&req, sink, &job)
                .map(|plan| Found::Plan(Box::new(plan))),
        };
        (result, printer.error.take())
    };
    ctx.progress.clear();
    if let Some(error) = write_error {
        return match WriteError::from_io(error) {
            // The reader went away: the search stopped there; a search keeps its code (D-35), a replace did nothing.
            WriteError::BrokenPipe if req.replace.is_none() => {
                Ok(ctx.human_done(Err(WriteError::BrokenPipe), 0, TEXT_FIND_SCHEMA, &[], OP))
            }
            WriteError::BrokenPipe => Err(Box::new((ApiError::Cancelled, Vec::new()))),
            WriteError::Other(e) => Err(Box::new((e, Vec::new()))),
        };
    }
    let outcome = match bare(result)? {
        Found::Outcome(outcome) => *outcome,
        Found::Plan(plan) => replace(ctx, *plan, req.replace.as_ref(), terminal, &job)?,
    };
    ctx.progress.clear();
    let dry_run = req.replace.as_ref().is_some_and(|r| r.dry_run);
    Ok(print(ctx, &outcome, dry_run || verbose))
}

/// The questions, then the dry run or the execution of a planned replace.
fn replace(
    ctx: &mut Ctx<'_>,
    plan: TextFindReplacePlan,
    replace: Option<&TextFindReplace>,
    terminal: bool,
    job: &hikasa_session::jobs::JobControl,
) -> Result<TextFindOutcome, Failure> {
    if replace.is_some_and(|r| r.dry_run) {
        return Ok(ctx.service.text_find_replace_dry_run(plan));
    }
    let plan = match ctx.service.text_find_replace_confirmation(&plan) {
        Some(mut driver) => {
            if driver.next_question().is_some() {
                if !terminal && let Some(refusal) = driver.refusal() {
                    return Err(Box::new((refusal, Vec::new())));
                }
                ctx.progress.clear();
                let mut stderr = io::stderr();
                ask(&mut driver, &job.cancel, &mut stderr);
                let _ = stderr.flush();
            }
            bare(plan.answered(driver))?
        }
        None => plan,
    };
    bare(ctx.service.text_find_replace_execute(plan, job))
}

/// `hikasa.text_find.v1`, or the replace's lines (and the plan's with `plan`); the hits were printed as they came.
fn print(ctx: &mut Ctx<'_>, outcome: &TextFindOutcome, plan: bool) -> u8 {
    let code = outcome.exit_code.code();
    if ctx.out.json() {
        return ctx.json_success(TEXT_FIND_SCHEMA, code, &outcome.dto, &outcome.warnings, OP);
    }
    let mut lines = outcome.replace_lines();
    if plan {
        lines.extend(outcome.plan_lines.iter().cloned());
    }
    let written = ctx
        .out
        .lines(lines.iter().map(|line| escape_controls(line).into_owned()));
    ctx.human_done(written, code, TEXT_FIND_SCHEMA, &outcome.warnings, OP)
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use hikasa_text::find::files::FindArchives;

    use super::*;
    use crate::args::{Cli, Command, TextCmd};

    /// `hikasa text find <line>` (the words of `line` split at spaces) parsed by clap.
    fn parse(line: &str) -> Result<Cli, clap::Error> {
        let argv = format!("hikasa text find {line}");
        Cli::try_parse_from(argv.split(' ').filter(|word| !word.is_empty()))
    }

    /// The request of `hikasa text find <line>`.
    fn request(line: &str, terminal: bool) -> Result<TextFindRequest, ApiError> {
        let Command::Text {
            cmd: TextCmd::Find(args),
        } = parse(line).unwrap().command
        else {
            panic!("not text find");
        };
        request_of(&args, terminal, false)
    }

    #[test]
    fn the_flags_make_the_request() {
        let req = request(
            "foo a b --regex --case --word -r --mask *.c;-obj/ --include-hidden --archives opened --encoding \
             latin1 --format c --convert --max-matches 5 --max-matches-per-file 2 --max-file-size 3KB",
            false,
        )
        .unwrap();
        assert_eq!(req.pattern, "foo");
        assert_eq!(req.roots.len(), 2);
        assert!(req.regex && req.match_case && req.whole_word && req.recursive);
        assert_eq!(req.mask.as_deref(), Some("*.c;-obj/"));
        assert!(req.include_hidden && req.convert);
        assert_eq!(req.archives, FindArchives::Opened);
        assert_eq!(req.encoding.as_deref(), Some("latin1"));
        assert_eq!(req.format.as_deref(), Some("c"));
        assert_eq!(req.max_matches, Some(5));
        assert_eq!(req.max_matches_per_file, Some(2));
        assert_eq!(req.max_file_size, Some(3 * 1024));
        assert!(req.replace.is_none());
        // The defaults: case ignored, no words, not recursive, archives as files.
        let req = request("x .", false).unwrap();
        assert!(!req.regex && !req.match_case && !req.whole_word && !req.recursive);
        assert_eq!(req.archives, FindArchives::Files);
        assert_eq!(req.max_file_size, None);
        // A pattern may start with a hyphen.
        assert_eq!(request("-x .", false).unwrap().pattern, "-x");
        assert!(request("x . --archives maybe", false).is_err());
        assert!(request("x . --max-file-size >3", false).is_err());
        assert!(parse("x").is_err());
        assert!(parse("x . -l -c").is_err());
    }

    #[test]
    fn the_replace_flags_answer_their_questions() {
        let req = request("a . --replace b --yes", false).unwrap();
        let r = req.replace.unwrap();
        assert_eq!(r.text, "b");
        assert!(r.answers.yes && !r.dry_run);
        assert_eq!(r.unmappable, UnmappablePolicy::Refuse);
        let r = request("a . --replace b", true).unwrap().replace.unwrap();
        assert!(!r.answers.yes);
        assert_eq!(r.unmappable, UnmappablePolicy::Ask);
        let r = request("a . --replace b --confirm no-to-all --no-backup", true)
            .unwrap()
            .replace
            .unwrap();
        assert!(r.answers.no_to_all && !r.answers.yes);
        assert_eq!(r.backup, Some(false));
        let r = request(
            "a . --replace b --confirm yes-to-all --backup --unmappable replace",
            false,
        )
        .unwrap()
        .replace
        .unwrap();
        assert!(r.answers.yes);
        assert_eq!(r.backup, Some(true));
        assert_eq!(r.unmappable, UnmappablePolicy::Replace);
        let r = request("a . --replace -b --dry-run", false)
            .unwrap()
            .replace
            .unwrap();
        assert!(r.dry_run);
        assert_eq!(r.text, "-b");
        // The replace flags need --replace; --yes and --dry-run exclude each other.
        assert!(parse("a . --dry-run").is_err());
        assert!(parse("a . --yes").is_err());
        assert!(parse("a . --replace b --yes --dry-run").is_err());
        assert!(parse("a . --replace b --backup --no-backup").is_err());
        assert!(request("a . --replace b --confirm maybe", true).is_err());
    }

    /// A writer that fails as a closed pipe does.
    struct Broken;

    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn printer<'a>(out: &'a mut dyn Write, mode: Mode, stop: &CancelToken) -> Printer<'a> {
        Printer {
            out,
            mode,
            verbose: false,
            terminal: None,
            spec: String::new(),
            printed: false,
            stop: stop.clone(),
            error: None,
        }
    }

    fn hit(preview: &str) -> FilesEvent<'_> {
        FilesEvent::Hit {
            file: 0,
            line: 2,
            column: 3,
            len_chars: 1,
            preview,
            preview_range: 2..3,
            truncated: false,
        }
    }

    /// A failed write stops the search (the job's token), once, and keeps the error for the exit code (D-35).
    #[test]
    fn a_failed_stdout_cancels_the_search() {
        let stop = CancelToken::new();
        let mut out = Broken;
        let mut p = printer(&mut out, Mode::Lines, &stop);
        let spec = hikasa_core::Spec::Stdin;
        p.event(FilesEvent::File {
            index: 0,
            spec: &spec,
            encoding: hikasa_text::TextEncoding::UTF_8,
        });
        assert!(!stop.is_cancelled());
        p.event(hit("x"));
        assert!(stop.is_cancelled());
        assert_eq!(p.error.unwrap().kind(), io::ErrorKind::BrokenPipe);
    }

    /// The three modes and the escaping of a line's control characters.
    #[test]
    fn the_modes_print_their_lines() {
        let spec = hikasa_core::Spec::Stdin;
        let file = || FilesEvent::File {
            index: 0,
            spec: &spec,
            encoding: hikasa_text::TextEncoding::UTF_8,
        };
        let done = |hits| FilesEvent::Done {
            file: 0,
            hits,
            stopped: false,
        };
        for (mode, expected) in [
            (Mode::Lines, "-:2:3: a\\u{1b}[0m\tb\n-:2:3: c\n"),
            (Mode::Files, "-\n"),
            (Mode::Count, "-:2\n"),
        ] {
            let stop = CancelToken::new();
            let mut out = Vec::new();
            let mut p = printer(&mut out, mode, &stop);
            p.event(file());
            p.event(hit("a\u{1b}[0m\tb"));
            p.event(hit("c"));
            p.event(done(2));
            p.event(file());
            p.event(done(0));
            assert!(p.error.is_none());
            assert_eq!(String::from_utf8(out).unwrap(), expected, "{mode:?}");
        }
    }
}
