//! The subcommands (contract §1.9.2, §3.5). Each module turns its arguments into one request, calls one `Service`
//! method and prints the result as JSON (`--json`) or as human output; the exit code comes from the response or
//! from `ApiError::exit_code` (§2.21). Vocabulary words are parsed by the core parsers only (T11).

pub mod completions;
pub mod folder;
pub mod format;
pub mod gui;
pub mod help;
pub mod quick;
pub mod session;
pub mod settings;
pub mod text;
pub mod version;
pub mod vfs;
pub mod workspace;

use clap::ArgMatches;
use hikasa_core::{JobKind, Warning};
use hikasa_session::api::dto::{
    COMPLETIONS_SCHEMA, FOLDER_COMPARE_SCHEMA, FORMAT_DETECT_SCHEMA, FORMAT_EXPORT_SCHEMA,
    FORMAT_LIST_SCHEMA, FORMAT_RESTORE_SCHEMA, FORMAT_SHOW_SCHEMA, FORMAT_TOKENS_SCHEMA,
    FORMAT_WRITE_SCHEMA, GUI_CLOSE_SCHEMA, GUI_LIST_SCHEMA, GUI_OPEN_SCHEMA, GUI_PING_SCHEMA,
    GUI_QUIT_SCHEMA, HELP_SCHEMA, JsonEnvelope, QUICK_SCHEMA, SESSION_LIST_SCHEMA,
    SESSION_SHOW_SCHEMA, SESSION_WRITE_SCHEMA, SETTINGS_EXPORT_SCHEMA, SETTINGS_GET_SCHEMA,
    SETTINGS_IMPORT_SCHEMA, SETTINGS_LIST_SCHEMA, SETTINGS_RESTORE_SCHEMA, SETTINGS_SET_SCHEMA,
    TEXT_COMPARE_SCHEMA, VERSION_SCHEMA, VFS_CAT_SCHEMA, VFS_LIST_SCHEMA, VFS_OP_SCHEMA,
    VFS_STAT_SCHEMA, WORKSPACE_LIST_SCHEMA, WORKSPACE_SHOW_SCHEMA, WORKSPACE_WRITE_SCHEMA,
};
use hikasa_session::api::{ApiOp, OpenArgs};
use hikasa_session::jobs::JobControl;
use hikasa_session::{ApiError, Service};
use hikasa_text::{TextPattern, TextRulesPatch, UnimportantItem};
use serde::Serialize;

use crate::args::{
    Command, FolderCmd, FormatCmd, GuiCmd, ImportanceArgs, SessionCmd, SettingsCmd, TextCmd,
    VfsCmd, WorkspaceCmd,
};
use crate::output::{Output, WriteError};
use crate::progress::ProgressLine;

/// What a command needs: the service, the output, the progress line and the parsed arguments of the leaf
/// subcommand (for the order of repeated flags).
pub struct Ctx<'a> {
    /// The one service of this process.
    pub service: &'a Service,
    /// stdout/stderr.
    pub out: &'a mut Output,
    /// Progress on stderr.
    pub progress: &'a ProgressLine,
    /// clap's matches of the leaf subcommand.
    pub matches: &'a ArgMatches,
}

impl Ctx<'_> {
    /// A job control for a blocking service call, reporting to the progress line.
    pub fn job(&self, kind: JobKind) -> JobControl {
        self.service.job(kind, self.progress.sink())
    }

    /// The warnings of an envelope: the startup warnings, the command's own (§1.8.7 item 8), then what the
    /// back-ends raised (D-145).
    pub fn warnings(&self, own: &[Warning]) -> Vec<Warning> {
        let mut all = self.service.startup_warnings().to_vec();
        all.extend_from_slice(own);
        all.extend(self.service.take_warnings());
        all
    }

    /// Writes a successful JSON document with exit code `code` and returns the code (a broken pipe keeps it).
    pub fn json_success<T: Serialize>(
        &mut self,
        schema: &'static str,
        code: u8,
        result: T,
        own: &[Warning],
        op: ApiOp,
    ) -> u8 {
        let envelope = JsonEnvelope::success_raw(schema, code, result, self.warnings(own));
        self.json_envelope(&envelope, own, op)
    }

    /// Writes a JSON document built from a response and returns its exit code (a broken pipe keeps it; another
    /// write failure fails the command).
    pub fn json_envelope<T: Serialize>(
        &mut self,
        envelope: &JsonEnvelope<T>,
        own: &[Warning],
        op: ApiOp,
    ) -> u8 {
        self.progress.clear();
        match self.out.envelope(envelope) {
            Ok(()) | Err(WriteError::BrokenPipe) => envelope.exit_code,
            Err(WriteError::Other(e)) => self.fail(envelope.schema, &e, op, own),
        }
    }

    /// The end of a human command: a broken pipe keeps `code`, another write failure fails the command; the
    /// command's warnings go to stderr.
    pub fn human_done(
        &mut self,
        written: Result<(), WriteError>,
        code: u8,
        schema: &'static str,
        own: &[Warning],
        op: ApiOp,
    ) -> u8 {
        self.progress.clear();
        match written {
            Ok(()) | Err(WriteError::BrokenPipe) => {
                self.out.warnings(own);
                self.out.warnings(&self.service.take_warnings());
                code
            }
            Err(WriteError::Other(e)) => self.fail(schema, &e, op, own),
        }
    }

    /// A failed command: one stderr line, the failure document with `--json` (unless stdout already carries
    /// output), and `err.exit_code(op)`.
    pub fn fail(&mut self, schema: &'static str, err: &ApiError, op: ApiOp, own: &[Warning]) -> u8 {
        self.progress.clear();
        let startup = self.service.startup_warnings();
        let mut own = own.to_vec();
        own.extend(self.service.take_warnings());
        fail(self.out, schema, err, op, startup, &own)
    }
}

/// A failed command: the stderr line, the failure document with `--json` (unless stdout already carries output;
/// its warnings are `startup` then `own`), and `err.exit_code(op)`. In human mode only `own` is printed: the
/// startup warnings were printed once when the service started.
pub fn fail(
    out: &mut Output,
    schema: &'static str,
    err: &ApiError,
    op: ApiOp,
    startup: &[Warning],
    own: &[Warning],
) -> u8 {
    out.error(err);
    if out.json() {
        if !out.wrote_stdout() {
            let mut warnings = startup.to_vec();
            warnings.extend_from_slice(own);
            let envelope = JsonEnvelope::<()>::failure(schema, err, op, warnings);
            let _ = out.envelope(&envelope);
        }
    } else {
        out.warnings(own);
    }
    err.exit_code(op).code()
}

/// Runs the commands that need no service and read no configuration (`completions`, `version`, `help`); `None`
/// for every other command.
pub fn run_without_service(out: &mut Output, command: &Command) -> Option<u8> {
    match command {
        Command::Completions { shell } => Some(completions::run(out, *shell)),
        Command::Version => Some(version::run(out)),
        Command::Help(args) => Some(help::run(out, args)),
        Command::Quick(_)
        | Command::Text { .. }
        | Command::Folder { .. }
        | Command::Format { .. }
        | Command::Vfs { .. }
        | Command::Gui { .. }
        | Command::Settings { .. }
        | Command::Session { .. }
        | Command::Workspace { .. } => None,
    }
}

/// The schema of a command's JSON document and the operation of its exit codes.
pub fn schema_and_op(command: &Command, open: Option<&OpenArgs>) -> (&'static str, ApiOp) {
    match command {
        Command::Quick(_) => (QUICK_SCHEMA, ApiOp::Quick),
        Command::Text {
            cmd: TextCmd::Compare(args),
        } => match args.gui {
            true => (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: args.wait }),
            false => (TEXT_COMPARE_SCHEMA, ApiOp::TextCompare),
        },
        Command::Folder {
            cmd: FolderCmd::Compare(args),
        } => match args.gui {
            true => (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: args.wait }),
            false => (FOLDER_COMPARE_SCHEMA, ApiOp::FolderCompare),
        },
        Command::Format { cmd } => match cmd {
            FormatCmd::List => (FORMAT_LIST_SCHEMA, ApiOp::Format),
            FormatCmd::Show { .. } => (FORMAT_SHOW_SCHEMA, ApiOp::Format),
            FormatCmd::Detect(_) => (FORMAT_DETECT_SCHEMA, ApiOp::Format),
            FormatCmd::Tokens(_) => (FORMAT_TOKENS_SCHEMA, ApiOp::Format),
            FormatCmd::New(_)
            | FormatCmd::Edit(_)
            | FormatCmd::Delete { .. }
            | FormatCmd::Import { .. }
            | FormatCmd::Enable { .. }
            | FormatCmd::Disable { .. }
            | FormatCmd::Move(_) => (FORMAT_WRITE_SCHEMA, ApiOp::Format),
            FormatCmd::Export { .. } => (FORMAT_EXPORT_SCHEMA, ApiOp::Format),
            FormatCmd::Restore(_) => (FORMAT_RESTORE_SCHEMA, ApiOp::Format),
        },
        Command::Settings { cmd } => match cmd {
            SettingsCmd::Get { .. } => (SETTINGS_GET_SCHEMA, ApiOp::Other),
            SettingsCmd::Set { .. } | SettingsCmd::Unset { .. } => {
                (SETTINGS_SET_SCHEMA, ApiOp::Other)
            }
            SettingsCmd::List { .. } => (SETTINGS_LIST_SCHEMA, ApiOp::Other),
            SettingsCmd::RestoreDefaults(_) => (SETTINGS_RESTORE_SCHEMA, ApiOp::Other),
            SettingsCmd::Export(_) => (SETTINGS_EXPORT_SCHEMA, ApiOp::Other),
            SettingsCmd::Import(_) => (SETTINGS_IMPORT_SCHEMA, ApiOp::Other),
        },
        Command::Session { cmd } => match cmd {
            SessionCmd::List { .. } => (SESSION_LIST_SCHEMA, ApiOp::Other),
            SessionCmd::Show { .. } => (SESSION_SHOW_SCHEMA, ApiOp::Other),
            SessionCmd::Run(_) => (FOLDER_COMPARE_SCHEMA, ApiOp::Other),
            SessionCmd::New { .. }
            | SessionCmd::Edit { .. }
            | SessionCmd::Delete { .. }
            | SessionCmd::Rename { .. }
            | SessionCmd::Lock { .. }
            | SessionCmd::Unlock { .. }
            | SessionCmd::Mkdir { .. }
            | SessionCmd::Rmdir { .. } => (SESSION_WRITE_SCHEMA, ApiOp::Other),
        },
        Command::Workspace { cmd } => match cmd {
            WorkspaceCmd::List => (WORKSPACE_LIST_SCHEMA, ApiOp::Other),
            WorkspaceCmd::Show { .. } => (WORKSPACE_SHOW_SCHEMA, ApiOp::Other),
            WorkspaceCmd::Delete { .. } | WorkspaceCmd::Rename { .. } => {
                (WORKSPACE_WRITE_SCHEMA, ApiOp::Other)
            }
        },
        Command::Vfs { cmd } => match cmd {
            VfsCmd::Ls(_) => (VFS_LIST_SCHEMA, ApiOp::Vfs),
            VfsCmd::Stat(_) => (VFS_STAT_SCHEMA, ApiOp::Vfs),
            VfsCmd::Cat { .. } => (VFS_CAT_SCHEMA, ApiOp::Vfs),
            VfsCmd::Cp(_) | VfsCmd::Mv(_) | VfsCmd::Rm(_) | VfsCmd::Mkdir(_) | VfsCmd::Touch(_) => {
                (VFS_OP_SCHEMA, ApiOp::Vfs)
            }
        },
        Command::Gui { cmd } => match cmd {
            GuiCmd::Open(_) => (
                GUI_OPEN_SCHEMA,
                ApiOp::GuiOpen {
                    wait: open.is_some_and(|o| o.request.wait),
                },
            ),
            GuiCmd::Ping => (GUI_PING_SCHEMA, ApiOp::GuiControl),
            GuiCmd::List => (GUI_LIST_SCHEMA, ApiOp::GuiControl),
            GuiCmd::Close { .. } => (GUI_CLOSE_SCHEMA, ApiOp::GuiControl),
            GuiCmd::Quit => (GUI_QUIT_SCHEMA, ApiOp::GuiControl),
            GuiCmd::Session { wait, .. } => (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: *wait }),
            GuiCmd::Workspace { .. } => (GUI_OPEN_SCHEMA, ApiOp::GuiOpen { wait: false }),
        },
        Command::Completions { .. } => (COMPLETIONS_SCHEMA, ApiOp::Other),
        Command::Version => (VERSION_SCHEMA, ApiOp::Other),
        Command::Help(_) => (HELP_SCHEMA, ApiOp::Other),
    }
}

/// Runs one command and returns its exit code. `open` is the parsed `gui open` command line (main parses it
/// before the service exists, §1.9.1).
pub fn dispatch(ctx: &mut Ctx<'_>, command: &Command, open: Option<&OpenArgs>) -> u8 {
    match command {
        Command::Quick(args) => quick::run(ctx, args),
        Command::Text {
            cmd: TextCmd::Compare(args),
        } => text::run(ctx, args),
        Command::Folder {
            cmd: FolderCmd::Compare(args),
        } => folder::run(ctx, args),
        Command::Format { cmd } => format::run(ctx, cmd),
        Command::Vfs { cmd } => vfs::run(ctx, cmd),
        Command::Gui { cmd } => gui::run(ctx, cmd, open),
        Command::Settings { cmd } => settings::run(ctx, cmd),
        Command::Session { cmd } => session::run(ctx, cmd),
        Command::Workspace { cmd } => workspace::run(ctx, cmd),
        Command::Completions { shell } => completions::run(ctx.out, *shell),
        Command::Version => version::run(ctx.out),
        Command::Help(args) => help::run(ctx.out, args),
    }
}

/// The text rules patch of the importance flags: `--unimportant` through `UnimportantItem::parse_list`,
/// `--unimportant-text` and `--unimportant-regex` as case-sensitive patterns in command-line order (§1.9.2).
pub fn importance_patch(
    importance: &ImportanceArgs,
    matches: &ArgMatches,
) -> Result<TextRulesPatch, ApiError> {
    let mut patch = TextRulesPatch::default();
    if let Some(list) = &importance.unimportant {
        patch.unimportant = UnimportantItem::parse_list(list)?;
    }
    patch.unimportant_text = ordered_patterns(importance, matches);
    Ok(patch)
}

/// The patterns of `--unimportant-text` and `--unimportant-regex`, in the order they were given.
fn ordered_patterns(importance: &ImportanceArgs, matches: &ArgMatches) -> Vec<TextPattern> {
    let indices = |id: &str| -> Vec<usize> {
        match matches.try_contains_id(id) {
            Ok(true) => matches
                .indices_of(id)
                .map(Iterator::collect)
                .unwrap_or_default(),
            Ok(false) | Err(_) => Vec::new(),
        }
    };
    let text_at = indices("unimportant_text");
    let regex_at = indices("unimportant_regex");
    let mut items: Vec<(usize, TextPattern)> = Vec::new();
    for (k, text) in importance.unimportant_text.iter().enumerate() {
        let at = text_at.get(k).copied().unwrap_or(usize::MAX);
        items.push((at, TextPattern::plain(text.clone())));
    }
    for (k, regex) in importance.unimportant_regex.iter().enumerate() {
        let at = regex_at.get(k).copied().unwrap_or(usize::MAX);
        items.push((at, TextPattern::regex(regex.clone())));
    }
    items.sort_by_key(|(at, _)| *at);
    items.into_iter().map(|(_, pattern)| pattern).collect()
}

/// `Some(true)` for the positive flag, `Some(false)` for the negative one, `None` for neither (clap's
/// `overrides_with` keeps only the last one given).
pub fn tri(on: bool, off: bool) -> Option<bool> {
    match (on, off) {
        (_, true) => Some(false),
        (true, false) => Some(true),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tri_state_flags() {
        assert_eq!(tri(false, false), None);
        assert_eq!(tri(true, false), Some(true));
        assert_eq!(tri(false, true), Some(false));
    }
}
