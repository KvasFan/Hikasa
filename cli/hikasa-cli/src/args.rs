//! The clap derive tree of `hikasa` (contract §1.9.2): [`Cli`], [`GlobalArgs`], [`Command`] and every `*Args`
//! struct.
//!
//! Flags are plumbing only (T11): every vocabulary word (`--display`, `--criteria`, `--unimportant`, `--report`,
//! `--content`, `--algorithm`, …) is taken as a `String` and parsed by the core parser the command module calls;
//! no value list or word table lives here.

use std::ffi::OsString;
use std::path::PathBuf;

/// `hikasa`: compare folders and files from the command line.
#[derive(Debug, clap::Parser)]
#[command(
    name = "hikasa",
    version,
    propagate_version = true,
    disable_help_subcommand = true,
    max_term_width = 110
)]
pub struct Cli {
    /// Flags accepted by every subcommand.
    #[command(flatten)]
    pub global: GlobalArgs,
    /// The subcommand.
    #[command(subcommand)]
    pub command: Command,
}

/// Global flags (CLI-000, CLI-003 `--log`/`--verbose`).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct GlobalArgs {
    /// Machine-readable JSON on stdout (one document, also on error).
    #[arg(long, global = true)]
    pub json: bool,
    /// No progress, no non-error stderr output.
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,
    /// Write diagnostic logs to FILE.
    #[arg(long, global = true, value_name = "FILE")]
    pub log: Option<PathBuf>,
    /// Debug-level logs; also logs ignored option keys.
    #[arg(long, global = true)]
    pub verbose: bool,
    /// Use DIR as the settings folder (portable behaviour).
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    /// Disable ANSI colours.
    #[arg(long, global = true)]
    pub no_color: bool,
}

/// The subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Quick compare two files; exit code only (1/2/11/12/13/100).
    Quick(QuickArgs),
    /// Text tools.
    Text {
        /// The text subcommand.
        #[command(subcommand)]
        cmd: TextCmd,
    },
    /// Folder tools.
    Folder {
        /// The folder subcommand.
        #[command(subcommand)]
        cmd: FolderCmd,
    },
    /// File formats.
    Format {
        /// The format subcommand.
        #[command(subcommand)]
        cmd: FormatCmd,
    },
    /// Generic VFS access.
    Vfs {
        /// The VFS subcommand.
        #[command(subcommand)]
        cmd: VfsCmd,
    },
    /// Talk to the GUI over IPC.
    Gui {
        /// The GUI subcommand.
        #[command(subcommand)]
        cmd: GuiCmd,
    },
    /// Read and change the settings files.
    Settings {
        /// The settings subcommand.
        #[command(subcommand)]
        cmd: SettingsCmd,
    },
    /// Reports of a text or folder comparison, to a file, the clipboard, the printer or the browser.
    Report {
        /// The report subcommand.
        #[command(subcommand)]
        cmd: ReportCmd,
    },
    /// Named sessions, session folders, defaults and auto-saved sessions.
    Session {
        /// The session subcommand.
        #[command(subcommand)]
        cmd: SessionCmd,
    },
    /// Saved workspaces.
    Workspace {
        /// The workspace subcommand.
        #[command(subcommand)]
        cmd: WorkspaceCmd,
    },
    /// Print shell completions.
    Completions {
        /// The shell.
        shell: clap_complete::Shell,
    },
    /// Print version information.
    Version,
    /// Print a help topic (`hikasa help text compare`), or the contents.
    Help(HelpArgs),
}

/// `hikasa help [topic…]`.
#[derive(Debug, clap::Args)]
pub struct HelpArgs {
    /// The topic: words such as `text compare`, or an id such as `exit-codes`.
    #[arg(value_name = "TOPIC")]
    pub topic: Vec<String>,
    /// List every topic (id and title).
    #[arg(long, conflicts_with = "topic")]
    pub list: bool,
}

/// Importance overrides shared by `quick`, `text compare` and `folder compare --content rules`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct ImportanceArgs {
    /// Unimportant items, comma-separated: whitespace, leading-ws, embedded-ws, trailing-ws, case,
    /// everything-else, element:NAME.
    #[arg(long, value_name = "LIST")]
    pub unimportant: Option<String>,
    /// Text that is unimportant wherever it occurs (plain, case-sensitive; repeatable).
    #[arg(long, value_name = "TEXT")]
    pub unimportant_text: Vec<String>,
    /// A regular expression whose matches are unimportant (case-sensitive; repeatable).
    #[arg(long, value_name = "REGEX")]
    pub unimportant_regex: Vec<String>,
}

/// `hikasa quick <left> <right>`.
#[derive(Debug, clap::Args)]
pub struct QuickArgs {
    /// Left file (`-` reads standard input).
    pub left: OsString,
    /// Right file (`-` reads standard input).
    pub right: OsString,
    /// Compare the sizes only.
    #[arg(long, group = "quick_method")]
    pub size: bool,
    /// Compare CRC32 checksums.
    #[arg(long, group = "quick_method")]
    pub crc: bool,
    /// Compare byte by byte.
    #[arg(long, group = "quick_method")]
    pub binary: bool,
    /// Rules-based comparison (the default).
    #[arg(long, group = "quick_method")]
    pub rules: bool,
    /// Treat unimportant differences as same (exit 2 instead of 12).
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
    /// Force a file format (id or name).
    #[arg(long, value_name = "ID|NAME")]
    pub format: Option<String>,
    /// Text rules file (TOML).
    #[arg(long, value_name = "TOML")]
    pub rules_file: Option<OsString>,
    /// Importance overrides.
    #[command(flatten)]
    pub importance: ImportanceArgs,
    /// Compare line endings.
    #[arg(long)]
    pub compare_line_endings: bool,
    /// File name of standard input, for format detection.
    #[arg(long, value_name = "NAME")]
    pub stdin_name: Option<String>,
}

/// `hikasa text …`.
#[derive(Debug, clap::Subcommand)]
pub enum TextCmd {
    /// Compare two text files.
    Compare(Box<TextCompareArgs>),
}

/// `hikasa text compare <left> <right>`.
#[derive(Debug, clap::Args)]
pub struct TextCompareArgs {
    /// Left file (`-` reads standard input).
    pub left: OsString,
    /// Right file (`-` reads standard input).
    pub right: OsString,
    /// The rules of the comparison.
    #[command(flatten)]
    pub rule: TextRuleArgs,
    /// The report.
    #[command(flatten)]
    pub report: ReportArgs,
    // Hands the comparison to the GUI (D-50).
    /// Show the comparison in the GUI instead of comparing here.
    #[arg(
        long,
        conflicts_with_all = ["output", "output_to", "layout", "options", "output_options", "patch_style", "html",
                              "html_mono", "line_numbers", "no_line_numbers", "width", "wrap", "title", "lines",
                              "dry_run"]
    )]
    pub gui: bool,
    /// With `--gui`: wait until the view closes and exit with its code.
    #[arg(long, requires = "gui")]
    pub wait: bool,
    /// With `--gui`: do not start the GUI when it is not running.
    #[arg(long, requires = "gui")]
    pub no_launch: bool,
}

/// The rule flags of `text compare` and `report text` (one owner, like [`FolderRuleArgs`]).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct TextRuleArgs {
    /// Force a file format (id or name).
    #[arg(long, value_name = "ID|NAME")]
    pub format: Option<String>,
    /// Encoding of both files.
    #[arg(long, value_name = "LABEL")]
    pub encoding: Option<String>,
    /// Encoding of the left file.
    #[arg(long, value_name = "LABEL")]
    pub left_encoding: Option<String>,
    /// Encoding of the right file.
    #[arg(long, value_name = "LABEL")]
    pub right_encoding: Option<String>,
    /// Text rules file (TOML).
    #[arg(long, visible_alias = "rules", value_name = "TOML")]
    pub rules_file: Option<OsString>,
    /// Alignment algorithm: auto, unaligned, standard, myers, patience, histogram.
    #[arg(long, value_name = "NAME")]
    pub algorithm: Option<String>,
    /// Skew tolerance of the standard algorithm (lines).
    #[arg(long, value_name = "N")]
    pub skew: Option<u32>,
    /// Pair changed lines by closeness.
    #[arg(long, overrides_with = "no_closeness")]
    pub closeness: bool,
    /// Pair changed lines by position.
    #[arg(long)]
    pub no_closeness: bool,
    /// Closeness threshold (per mille).
    #[arg(long, value_name = "PERMILLE")]
    pub closeness_threshold: Option<u16>,
    /// Never align differing lines.
    #[arg(long)]
    pub never_align: bool,
    /// Importance overrides.
    #[command(flatten)]
    pub importance: ImportanceArgs,
    /// Compare line endings.
    #[arg(long)]
    pub compare_line_endings: bool,
    /// Orphan lines follow the importance rules (instead of always being important).
    #[arg(long)]
    pub orphans_unimportant: bool,
    /// Treat unimportant differences as same (verdict, exit code, report).
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
    /// File name of standard input, for format detection.
    #[arg(long, value_name = "NAME")]
    pub stdin_name: Option<String>,
}

/// REPORT (M2 W7, §1.7.15): the report flags of `text compare`, `folder compare`, `session run`, `report text` and
/// `report folder`, one owner. A flag that only one kind takes is refused for the other by the service, not by clap,
/// so the five commands parse alike. Every word is parsed by the core (T11).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct ReportArgs {
    /// Report layout (text: unified, side-by-side, summary, interleaved, patch, statistics, xml; folders: list,
    /// side-by-side, summary, json, xml).
    #[arg(long, visible_alias = "report", value_name = "WORD")]
    pub layout: Option<String>,
    /// Report option words, comma-separated (repeatable): display-*, ignore-unimportant, line-numbers, strikeout-*,
    /// patch-*, column-*, include-file-links.
    #[arg(long, value_name = "WORD,…")]
    pub options: Vec<String>,
    /// Output option words, comma-separated (repeatable): html-color, html-mono, html-custom:<css>, text, print-*,
    /// wrap-*.
    #[arg(long, value_name = "WORD,…")]
    pub output_options: Vec<String>,
    /// Where the report goes: `-` (standard output, the default), clipboard, printer, browser, or a file.
    #[arg(long, value_name = "TARGET", conflicts_with = "output")]
    pub output_to: Option<OsString>,
    /// Write the report to FILE (atomically) instead of stdout: --output-to FILE.
    #[arg(long, value_name = "FILE")]
    pub output: Option<OsString>,
    /// Report title.
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// Display filter (text: all, mismatches, context, matches; folders: a display filter).
    #[arg(long, value_name = "MODE")]
    pub display: Option<String>,
    /// Folders: folder mode of the display: always, structure, files-only.
    #[arg(long, value_name = "MODE")]
    pub folders: Option<String>,
    /// Folders: show the items hidden by file filters.
    #[arg(long)]
    pub suppress_filters: bool,
    /// Text: context lines (default from the options, 3).
    #[arg(long, value_name = "N")]
    pub context: Option<u32>,
    /// Text: patch style of the patch layout: normal, context, unified.
    #[arg(long, value_name = "STYLE")]
    pub patch_style: Option<String>,
    /// Folders: columns of the side-by-side report (e.g. `size,modified`).
    #[arg(long, value_name = "LIST")]
    pub columns: Option<String>,
    /// HTML output (the layouts with an HTML form).
    #[arg(long, conflicts_with = "html_mono")]
    pub html: bool,
    /// Monochrome HTML output.
    #[arg(long)]
    pub html_mono: bool,
    /// Text: show line numbers (side-by-side, interleaved; the default).
    #[arg(long, overrides_with = "no_line_numbers")]
    pub line_numbers: bool,
    /// Text: hide line numbers.
    #[arg(long)]
    pub no_line_numbers: bool,
    /// Text: total width of side-by-side and interleaved text (columns; default 160).
    #[arg(long, value_name = "N")]
    pub width: Option<u16>,
    /// Text: line wrapping of side-by-side and interleaved text: none, character, word.
    #[arg(long, value_name = "MODE")]
    pub wrap: Option<String>,
    /// Text: only the rows of a line range, `[left:|right:]<A>-<B>` (side-by-side, interleaved, xml).
    #[arg(long, value_name = "RANGE")]
    pub lines: Option<String>,
    /// Folders: only the selected rows, by mask (repeatable; list, side-by-side, xml).
    #[arg(long, value_name = "MASK")]
    pub select: Vec<String>,
    /// Folders: only the row at this relative path (repeatable).
    #[arg(long = "path", value_name = "REL")]
    pub paths: Vec<String>,
    /// Print what would be written; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `hikasa folder …`.
#[derive(Debug, clap::Subcommand)]
pub enum FolderCmd {
    /// Compare two folders.
    Compare(Box<FolderCompareArgs>),
    /// Compare, then run a file operation on what is selected.
    Op {
        /// The operation.
        #[command(subcommand)]
        verb: Box<FolderOpVerb>,
    },
    /// Compare, then synchronize the two sides by a preset or a matrix.
    Sync(Box<FolderSyncArgs>),
}

/// The preset of `folder sync`: exactly one, unless `--session` holds one.
#[derive(Debug, Clone, Default, clap::Args)]
#[group(multiple = false)]
pub struct SyncPresetArgs {
    /// Update Left: what is newer or new on the right goes left.
    #[arg(long)]
    pub update_left: bool,
    /// Update Right: what is newer or new on the left goes right.
    #[arg(long)]
    pub update_right: bool,
    /// Update Both.
    #[arg(long)]
    pub update_both: bool,
    /// Mirror to Left: the left becomes what the right is (deletes on the left).
    #[arg(long)]
    pub mirror_left: bool,
    /// Mirror to Right: the right becomes what the left is (deletes on the right).
    #[arg(long)]
    pub mirror_right: bool,
    /// A matrix: `<situation>=<action>,…`, a preset first as its base (`mirror-right,same=leave`).
    #[arg(long, value_name = "MATRIX")]
    pub custom: Option<String>,
}

/// `hikasa folder sync [<LEFT> <RIGHT>]` (§1.6.17).
#[derive(Debug, Clone, clap::Args)]
pub struct FolderSyncArgs {
    /// The sides, the rules and `SELECT` (with `--just-selection`).
    #[command(flatten)]
    pub common: FolderOpCommon,
    /// `CONFIRM`.
    #[command(flatten)]
    pub confirm: FolderConfirmArgs,
    /// The preset.
    #[command(flatten)]
    pub preset: SyncPresetArgs,
    /// An item on one side only is `added` there (default) or `deleted` on the other.
    #[arg(long, value_name = "READING")]
    pub orphans: Option<String>,
    /// Empty folders are made and deleted too.
    #[arg(long)]
    pub create_empty: bool,
    /// Archives are folders: their members are synced.
    #[arg(long)]
    pub within_archives: bool,
    /// Deletes go to the recycle bin where there is one.
    #[arg(long)]
    pub recycle: bool,
    /// A delete the bin refuses becomes permanent.
    #[arg(long, requires = "recycle")]
    pub recycle_fallback: bool,
    /// An action for a path and what lies below it (repeatable): `<PATH>=<ACTION>`.
    #[arg(long = "override", value_name = "PATH=ACTION")]
    pub overrides: Vec<String>,
    /// Only the rows `--select` and `--path` choose (else a sync ignores them).
    #[arg(long)]
    pub just_selection: bool,
    /// Only the rows `--display` shows.
    #[arg(long)]
    pub visible: bool,
    /// A side with nothing on it may make the other empty.
    #[arg(long)]
    pub allow_empty_source: bool,
    /// Keep what is replaced, as `[backups]` names it.
    #[arg(long, conflicts_with = "no_backup")]
    pub backup: bool,
    /// Keep no backup, whatever `[backups]` says.
    #[arg(long)]
    pub no_backup: bool,
    /// The output shows every row, `same` ones and folders too.
    #[arg(long)]
    pub all_rows: bool,
    /// Show the sync in the GUI instead of running it here.
    #[arg(long, conflicts_with_all = ["dry_run", "yes", "confirm", "all_rows"])]
    pub gui: bool,
    /// With `--gui`: wait until the view closes and exit with its code.
    #[arg(long, requires = "gui")]
    pub wait: bool,
    /// With `--gui`: do not start the GUI when it is not running.
    #[arg(long, requires = "gui")]
    pub no_launch: bool,
}

/// What every `folder op` verb takes: the sides and the rules, the selection.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct FolderOpCommon {
    /// Left base folder, or an archive.
    #[arg(requires = "right")]
    pub left: Option<OsString>,
    /// Right base folder, or an archive.
    pub right: Option<OsString>,
    /// A session gives the specs, the rules, read-only and Ignored marks; flags apply on top.
    #[arg(long, value_name = "REF", conflicts_with_all = ["left", "right"])]
    pub session: Option<String>,
    /// The rules of the comparison.
    #[command(flatten)]
    pub rule: FolderRuleArgs,
    /// The selection.
    #[command(flatten)]
    pub select: FolderSelectArgs,
}

/// `SELECT` of `folder op` (§1.6.17): nothing selected acts on nothing.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct FolderSelectArgs {
    /// A mask: `left.diff.files`, `all`, `empty.folders`… (repeatable; the masks add up).
    #[arg(long, value_name = "MASK")]
    pub select: Vec<String>,
    /// One row by its path; `left:`, `right:` for one side, else the verb's (repeatable).
    #[arg(long, value_name = "REL")]
    pub path: Vec<String>,
    /// A selected folder also brings what filters and the display hide.
    #[arg(long)]
    pub include_hidden: bool,
    /// A folder is one row: `--select` does not see below it.
    #[arg(long)]
    pub no_expand: bool,
    /// Only these folders are open for `--select` (repeatable).
    #[arg(long, value_name = "REL", conflicts_with = "no_expand")]
    pub expand: Vec<String>,
    /// What `--select` sees (default all).
    #[arg(long, value_name = "FILTER")]
    pub display: Option<String>,
    /// Treat unimportant differences as same.
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
}

/// `CONFIRM` of `folder op` (§1.6.17).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct FolderConfirmArgs {
    /// Answer every question yes.
    #[arg(long, short = 'y', conflicts_with_all = ["confirm", "dry_run"])]
    pub yes: bool,
    /// prompt, yes-to-all or no-to-all (no-to-all lets the items without a question go).
    #[arg(long, value_name = "MODE")]
    pub confirm: Option<String>,
    /// Print the plan and each item's conflicts; nothing is written.
    #[arg(long)]
    pub dry_run: bool,
    /// Refuse to write this side: left, right or both.
    #[arg(long, value_name = "SIDE")]
    pub read_only: Option<String>,
}

/// The flags of what copies (`folder op copy`, `move`).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct FolderCopyFlags {
    /// The side written: left or right.
    #[arg(long, value_name = "SIDE")]
    pub to: String,
    /// What an existing target means: fail, skip, always (default) or newer.
    #[arg(long, value_name = "POLICY")]
    pub overwrite: Option<String>,
    /// Keep no times or attributes.
    #[arg(long, conflicts_with = "preserve_created")]
    pub no_preserve: bool,
    /// Keep the creation date too.
    #[arg(long)]
    pub preserve_created: bool,
    /// Links: copy (default) or skip.
    #[arg(long, value_name = "POLICY")]
    pub symlinks: Option<String>,
    /// Keep what is replaced, as `[backups]` names it.
    #[arg(long, conflicts_with = "no_backup")]
    pub backup: bool,
    /// Keep no backup, whatever `[backups]` says.
    #[arg(long)]
    pub no_backup: bool,
}

/// The flags of `folder op copyto`, `moveto`.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct FolderCopyToFlags {
    /// The sides read: left, right or both.
    #[arg(long, value_name = "SIDE")]
    pub side: String,
    /// The folder, or archive, written; missing, it is made.
    #[arg(long, value_name = "SPEC")]
    pub target: OsString,
    /// relative (default), base or none.
    #[arg(long, value_name = "WORD")]
    pub structure: Option<String>,
    /// What an existing target means: fail (default), skip, always or newer.
    #[arg(long, value_name = "POLICY")]
    pub overwrite: Option<String>,
}

/// `hikasa folder op <VERB>`.
#[derive(Debug, clap::Subcommand)]
pub enum FolderOpVerb {
    /// Copy what is selected to the other side.
    Copy {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        #[command(flatten)]
        flags: FolderCopyFlags,
    },
    /// Move what is selected to the other side.
    Move {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        #[command(flatten)]
        flags: FolderCopyFlags,
    },
    /// Swap both sides of what is selected; nothing is lost at any step.
    Exchange {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
    },
    /// Delete what is selected, on the sides named.
    Delete {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        /// left, right or both.
        #[arg(long, value_name = "SIDE")]
        side: String,
        /// Use the recycle bin where there is one.
        #[arg(long)]
        recycle: bool,
        /// If the recycle bin fails, delete permanently.
        #[arg(long, requires = "recycle")]
        recycle_fallback: bool,
    },
    /// Rename what is selected: one name, a DOS mask or a regular expression.
    Rename {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        /// left, right or both.
        #[arg(long, value_name = "SIDE")]
        side: String,
        /// The new name of one row.
        #[arg(long, value_name = "NEW", conflicts_with_all = ["mask", "regex"])]
        name: Option<String>,
        /// A DOS mask: `*_old.*`, `abc?.bak`.
        #[arg(long, value_name = "MASK", conflicts_with = "regex")]
        mask: Option<String>,
        /// A regular expression over the whole name and what replaces it.
        #[arg(long, num_args = 2, value_names = ["OLD", "NEW"])]
        regex: Vec<String>,
    },
    /// Set the times of what is selected.
    Touch {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        /// left, right or both.
        #[arg(long, value_name = "SIDE")]
        side: String,
        /// This time (RFC 3339).
        #[arg(long, value_name = "TIME", conflicts_with_all = ["now", "from_other"])]
        time: Option<String>,
        /// The time now (default).
        #[arg(long, conflicts_with = "from_other")]
        now: bool,
        /// The other side's time, on `--side`.
        #[arg(long)]
        from_other: bool,
    },
    /// Make a folder, or an archive by its name; a trailing separator wants a folder.
    Mkdir {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        /// left, right or both.
        #[arg(long, value_name = "SIDE")]
        side: String,
        /// The row it goes into (default the base).
        #[arg(long = "in", value_name = "REL")]
        into: Option<String>,
        /// Its name.
        #[arg(long, value_name = "NAME")]
        name: String,
    },
    /// Copy what is selected to a folder outside the comparison.
    Copyto {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        #[command(flatten)]
        flags: FolderCopyToFlags,
    },
    /// Move what is selected to a folder outside the comparison.
    Moveto {
        #[command(flatten)]
        common: FolderOpCommon,
        #[command(flatten)]
        confirm: FolderConfirmArgs,
        #[command(flatten)]
        flags: FolderCopyToFlags,
    },
    /// Run a content test on what is selected.
    Compare {
        #[command(flatten)]
        common: FolderOpCommon,
        /// crc, binary or rules.
        #[arg(long, value_name = "METHOD")]
        method: String,
    },
    /// Compute CRC-32 of what is selected.
    Crc {
        #[command(flatten)]
        common: FolderOpCommon,
        /// left, right or both.
        #[arg(long, value_name = "SIDE")]
        side: String,
    },
    /// Add what is selected to the session's exclude lists.
    Exclude {
        #[command(flatten)]
        common: FolderOpCommon,
        /// items (default) or type.
        #[arg(long, value_name = "WORD")]
        what: Option<String>,
        /// Print the masks; nothing is written.
        #[arg(long)]
        dry_run: bool,
        /// Rewrite a locked session.
        #[arg(long)]
        force: bool,
    },
    /// Mark what is selected Ignored in the session.
    Ignored {
        #[command(flatten)]
        common: FolderOpCommon,
        /// Clear the mark.
        #[arg(long)]
        off: bool,
        /// Print the list; nothing is written.
        #[arg(long)]
        dry_run: bool,
        /// Rewrite a locked session.
        #[arg(long)]
        force: bool,
    },
    /// Open the rows of `--path` with an application of `openwith.toml`.
    OpenWith {
        #[command(flatten)]
        common: FolderOpCommon,
        /// An application: its description, or its place in the list.
        #[arg(
            long,
            value_name = "NAME|INDEX",
            required_unless_present = "associated"
        )]
        app: Option<String>,
        /// The associated application.
        #[arg(long, conflicts_with = "app")]
        associated: bool,
        /// left (default), right or both.
        #[arg(long, value_name = "SIDE")]
        side: Option<String>,
        /// Print the commands; nothing starts.
        #[arg(long)]
        dry_run: bool,
    },
}

/// The rule flags of `folder compare`, and of the folder commands that compare first: one owner, the same flags.
#[derive(Debug, Clone, Default, clap::Args)]
pub struct FolderRuleArgs {
    /// Archives inside the trees: files (always), opened (files, until named by --open-archive; default),
    /// folders (always).
    #[arg(long, value_name = "MODE")]
    pub archives: Option<String>,
    /// An archive to open as a folder: its path relative to the bases, `/`-separated (repeatable).
    #[arg(long = "open-archive", value_name = "PATH")]
    pub open_archive: Vec<String>,
    /// Name filter (masks separated by `;`, `-` excludes).
    #[arg(long, value_name = "MASKS")]
    pub filters: Option<String>,
    /// Folder rules file (TOML).
    #[arg(long, visible_alias = "rules", value_name = "TOML")]
    pub rules_file: Option<OsString>,
    /// Comparison criteria list (script syntax; unlisted tests are off).
    #[arg(long, value_name = "LIST")]
    pub criteria: Option<String>,
    /// Compare sizes.
    #[arg(long, overrides_with = "no_size")]
    pub size: bool,
    /// Do not compare sizes.
    #[arg(long)]
    pub no_size: bool,
    /// Compare timestamps.
    #[arg(long, overrides_with = "no_timestamps")]
    pub timestamps: bool,
    /// Do not compare timestamps.
    #[arg(long)]
    pub no_timestamps: bool,
    /// Timestamp tolerance (seconds).
    #[arg(long, value_name = "SECS")]
    pub tolerance: Option<u32>,
    /// Ignore daylight-saving-time differences.
    #[arg(long)]
    pub ignore_dst: bool,
    /// Ignore time-zone differences.
    #[arg(long)]
    pub ignore_timezone: bool,
    /// Compare the case of file names.
    #[arg(long)]
    pub filename_case: bool,
    /// Compare these DOS attributes (letters, e.g. `HRS`).
    #[arg(long, value_name = "LETTERS")]
    pub attributes: Option<String>,
    /// Compare Unix permissions.
    #[arg(long)]
    pub permissions: bool,
    /// Compare Unix owners.
    #[arg(long)]
    pub owner: bool,
    /// Compare Unix groups.
    #[arg(long)]
    pub group: bool,
    /// Content comparison: none, crc, binary, rules.
    #[arg(long, value_name = "METHOD")]
    pub content: Option<String>,
    /// Skip the content test when the quick tests already say same.
    #[arg(long)]
    pub skip_if_quick_same: bool,
    /// Do not let a content match override timestamp and size differences.
    #[arg(long)]
    pub no_override_quick: bool,
    /// Text rules file of `--content rules` (TOML).
    #[arg(long, value_name = "TOML")]
    pub text_rules_file: Option<OsString>,
    /// Importance overrides of `--content rules`.
    #[command(flatten)]
    pub importance: ImportanceArgs,
    /// Align names case-sensitively.
    #[arg(long, conflicts_with = "case_insensitive")]
    pub case_sensitive: bool,
    /// Align names case-insensitively.
    #[arg(long)]
    pub case_insensitive: bool,
    /// Align files whose names differ only in the extension.
    #[arg(long)]
    pub align_extensions: bool,
    /// Align names that differ only in Unicode normalisation.
    #[arg(long)]
    pub align_normalization: bool,
    /// Date filter: `[<|>](YYYY-MM-DD|RFC 3339 time|<n>days)`.
    #[arg(long, value_name = "EXPR")]
    pub cutoff: Option<String>,
    /// Size filter: `[<|>]<n>[KB|MB|GB|TB]`.
    #[arg(long, value_name = "EXPR")]
    pub size_filter: Option<String>,
    /// Include protected (system) files.
    #[arg(long)]
    pub include_protected: bool,
    /// Follow symbolic links.
    #[arg(long)]
    pub follow_symlinks: bool,
    /// Flatten the folder structure.
    #[arg(long)]
    pub flatten: bool,
    /// Do not scan folders that exist on one side only.
    #[arg(long)]
    pub no_scan_orphans: bool,
    /// Attribute filter on regular files: `(+|-)<letters>…`, e.g. `+rh-s`, or `none`.
    #[arg(long, value_name = "EXPR", allow_hyphen_values = true)]
    pub attrib_filter: Option<String>,
    /// Unix type filter: `(+|-)<letters>…` of `b c l p r s`, e.g. `-lp`, or `none`.
    #[arg(
        long = "unixtype-filter",
        value_name = "EXPR",
        allow_hyphen_values = true
    )]
    pub unix_type_filter: Option<String>,
    /// Do not scan subfolders by themselves: only the bases, and with their subtrees the orphan folders in a base.
    #[arg(long)]
    pub no_auto_scan: bool,
    /// A view opens the folders of the tree when the comparison is loaded.
    #[arg(long)]
    pub expand_on_load: bool,
    /// A view opens only the folders with differences.
    #[arg(long = "expand-differences")]
    pub expand_differences: bool,
    /// A path relative to the bases to mark Ignored, `!path` for an exception (repeatable).
    #[arg(long, value_name = "PATH", allow_hyphen_values = true)]
    pub ignored: Vec<String>,
    /// Turn on a file format for this comparison (repeatable).
    #[arg(long, value_name = "ID")]
    pub format_on: Vec<String>,
    /// Turn off a file format for this comparison (repeatable).
    #[arg(long, value_name = "ID")]
    pub format_off: Vec<String>,
}

/// `hikasa folder compare <left> <right>`.
#[derive(Debug, clap::Args)]
pub struct FolderCompareArgs {
    /// Left base folder, or an archive.
    pub left: OsString,
    /// Right base folder, or an archive.
    pub right: OsString,
    /// The rules of the comparison.
    #[command(flatten)]
    pub rule: FolderRuleArgs,
    /// Treat unimportant differences as same (verdict, exit code, report).
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
    /// Expand all subfolders (the printed reports are always fully expanded).
    #[arg(long)]
    pub expand_all: bool,
    /// The report.
    #[command(flatten)]
    pub report: ReportArgs,
    // Hands the comparison to the GUI (D-50).
    /// Show the comparison in the GUI instead of comparing here.
    #[arg(
        long,
        conflicts_with_all = ["output", "output_to", "layout", "options", "output_options", "columns", "html",
                              "html_mono", "title", "select", "paths", "dry_run"]
    )]
    pub gui: bool,
    /// With `--gui`: wait until the view closes and exit with its code.
    #[arg(long, requires = "gui")]
    pub wait: bool,
    /// With `--gui`: do not start the GUI when it is not running.
    #[arg(long, requires = "gui")]
    pub no_launch: bool,
}

/// `hikasa report …` (M2 W7, §1.7.15).
#[derive(Debug, clap::Subcommand)]
pub enum ReportCmd {
    /// A report of a text comparison (side-by-side by default).
    Text(Box<ReportTextArgs>),
    /// A report of a folder comparison (side-by-side by default).
    Folder(Box<ReportFolderArgs>),
}

/// `hikasa report text [<LEFT> <RIGHT>] [--session <REF>]`.
#[derive(Debug, clap::Args)]
pub struct ReportTextArgs {
    /// Left file (`-` reads standard input).
    #[arg(requires = "right", required_unless_present = "session")]
    pub left: Option<OsString>,
    /// Right file (`-` reads standard input).
    pub right: Option<OsString>,
    /// A saved Text Compare session instead of the two files (the flags apply on top).
    #[arg(long, value_name = "REF", conflicts_with = "left")]
    pub session: Option<String>,
    /// The rules of the comparison.
    #[command(flatten)]
    pub rule: TextRuleArgs,
    /// The report.
    #[command(flatten)]
    pub report: ReportArgs,
}

/// `hikasa report folder [<LEFT> <RIGHT>] [--session <REF>]`.
#[derive(Debug, clap::Args)]
pub struct ReportFolderArgs {
    /// Left base folder, or an archive.
    #[arg(requires = "right", required_unless_present = "session")]
    pub left: Option<OsString>,
    /// Right base folder, or an archive.
    pub right: Option<OsString>,
    /// A saved Folder Compare or Folder Sync session instead of the two folders (the flags apply on top).
    #[arg(long, value_name = "REF", conflicts_with = "left")]
    pub session: Option<String>,
    /// The rules of the comparison.
    #[command(flatten)]
    pub rule: FolderRuleArgs,
    /// Treat unimportant differences as same (verdict, exit code, report).
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
    /// The report.
    #[command(flatten)]
    pub report: ReportArgs,
}

/// `hikasa format …`.
#[derive(Debug, clap::Subcommand)]
pub enum FormatCmd {
    /// List the file formats in detection order.
    List,
    /// Show one file format.
    Show {
        /// Format id or name.
        name: String,
    },
    /// Detect the format of a file by its name.
    Detect(DetectArgs),
    /// Dump the grammar tokens of a file.
    Tokens(TokensArgs),
    /// Create a user format (empty, or a copy of another one).
    New(FormatNewArgs),
    /// Change a format (a built-in gets an override file).
    Edit(FormatEditArgs),
    /// Delete a user format.
    Delete {
        /// The format id.
        id: String,
    },
    /// Copy a format file into the formats folder.
    Import {
        /// The format file.
        path: PathBuf,
        /// Replace a format with the same id.
        #[arg(long)]
        replace: bool,
    },
    /// Print a format as TOML or write it to a file.
    Export {
        /// Format id or name.
        name: String,
        /// Write to PATH instead of stdout.
        #[arg(long, short = 'o', value_name = "PATH")]
        output: Option<PathBuf>,
    },
    /// Enable a format (detection uses it).
    Enable {
        /// The format id.
        id: String,
    },
    /// Disable a format (detection skips it).
    Disable {
        /// The format id.
        id: String,
    },
    /// Move a format in the detection order.
    Move(FormatMoveArgs),
    /// Remove user files that override built-in formats.
    Restore(FormatRestoreArgs),
}

/// `hikasa format new <id>`.
#[derive(Debug, clap::Args)]
pub struct FormatNewArgs {
    /// The new format id (a-z, 0-9 and `-`).
    pub id: String,
    /// Copy this format (id or name).
    #[arg(long, value_name = "ID")]
    pub from: Option<String>,
    /// The display name.
    #[arg(long, value_name = "TEXT")]
    pub name: Option<String>,
    /// The file name mask (`*.ext;*.other`).
    #[arg(long, value_name = "MASK")]
    pub mask: Option<String>,
    /// The kind: text, table, hex, picture, media, version, registry, external.
    #[arg(long, value_name = "KIND")]
    pub kind: Option<String>,
}

/// `hikasa format edit <id>`.
#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("edit").required(true).args(["from_file", "set"])))]
pub struct FormatEditArgs {
    /// The format id.
    pub id: String,
    /// Replace the format with this file (its id must match).
    #[arg(long, value_name = "PATH", conflicts_with = "set")]
    pub from_file: Option<PathBuf>,
    /// Set one key of the format's TOML (`mask=*.x`, `text.tab_size=8`; repeatable).
    #[arg(long, value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

/// `hikasa format move <id>`.
#[derive(Debug, clap::Args)]
#[command(group(
    clap::ArgGroup::new("to")
        .required(true)
        .args(["up", "down", "top", "bottom", "before", "after"])
))]
pub struct FormatMoveArgs {
    /// The format id.
    pub id: String,
    /// One place up.
    #[arg(long)]
    pub up: bool,
    /// One place down.
    #[arg(long)]
    pub down: bool,
    /// First.
    #[arg(long)]
    pub top: bool,
    /// Last.
    #[arg(long)]
    pub bottom: bool,
    /// Right before this format.
    #[arg(long, value_name = "ID")]
    pub before: Option<String>,
    /// Right after this format.
    #[arg(long, value_name = "ID")]
    pub after: Option<String>,
}

/// `hikasa format restore`.
#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("what").required(true).arg("ids").arg("all")))]
pub struct FormatRestoreArgs {
    /// Built-in format ids whose override files are removed.
    #[arg(value_name = "ID")]
    pub ids: Vec<String>,
    /// Remove every user format file and `order.toml`.
    #[arg(long)]
    pub all: bool,
    /// Print the plan only.
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,
    /// Delete without asking (required unless `--dry-run`).
    #[arg(long)]
    pub yes: bool,
}

/// `hikasa settings …`.
#[derive(Debug, clap::Subcommand)]
pub enum SettingsCmd {
    /// Print the value of a key (a table as TOML).
    Get {
        /// The key: `<file>.<path>`, such as `options.text_editing.auto_indent`.
        key: String,
    },
    /// Set a key (a value equal to the default removes it from the file).
    Set {
        /// The key.
        key: String,
        /// The value: TOML (`true`, `5`, `"text"`, `["a", "b"]`) or plain text for text keys.
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Overwrite a file changed on disk since it was read.
        #[arg(long)]
        force: bool,
    },
    /// Put a key back to its default.
    Unset {
        /// The key.
        key: String,
    },
    /// List keys with their values.
    List {
        /// Only the keys under this key or file.
        prefix: Option<String>,
        /// Only the keys that differ from their defaults.
        #[arg(long)]
        changed: bool,
    },
    /// Delete settings files so that their defaults apply.
    RestoreDefaults(RestoreDefaultsArgs),
    /// Write a settings package (`.hikasa-pkg`).
    Export(SettingsExportArgs),
    /// Import a settings package.
    Import(SettingsImportArgs),
}

/// `hikasa settings export`.
#[derive(Debug, clap::Args)]
pub struct SettingsExportArgs {
    /// The package to write.
    #[arg(long, short = 'o', value_name = "FILE.hikasa-pkg")]
    pub output: PathBuf,
    /// A category: options, theme, commands, state, sessions, formats, workspaces, system_info (repeatable;
    /// default: all but system_info).
    #[arg(long, value_name = "CAT")]
    pub category: Vec<String>,
    /// One item: `<CAT>:<NAME>` such as `sessions:work/api` (repeatable).
    #[arg(long, value_name = "CAT:NAME")]
    pub item: Vec<String>,
}

/// `hikasa settings import`.
#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("mode").required(true).arg("dry_run").arg("yes").arg("silent")))]
pub struct SettingsImportArgs {
    /// The package.
    pub path: PathBuf,
    /// A category to import (repeatable; default: every category of the package).
    #[arg(long, value_name = "CAT")]
    pub category: Vec<String>,
    /// One item: `<CAT>:<NAME>` (repeatable).
    #[arg(long, value_name = "CAT:NAME")]
    pub item: Vec<String>,
    /// Delete every existing item of a category first: sessions, formats or workspaces (repeatable).
    #[arg(long, value_name = "CAT")]
    pub delete_existing: Vec<String>,
    /// Print the plan only.
    #[arg(long)]
    pub dry_run: bool,
    /// Import without asking.
    #[arg(long)]
    pub yes: bool,
    /// Import every category and item, no deletes, no prompt.
    #[arg(long, conflicts_with_all = ["category", "item", "delete_existing"])]
    pub silent: bool,
    /// Also replace (or delete with --delete-existing) locked sessions.
    #[arg(long)]
    pub force: bool,
}

/// `hikasa session …`.
#[derive(Debug, clap::Subcommand)]
pub enum SessionCmd {
    /// List folders and named sessions (and defaults, auto-saved sessions).
    List {
        /// Only this folder.
        #[arg(long, value_name = "REF")]
        folder: Option<String>,
        /// Also the auto-saved sessions.
        #[arg(long)]
        autosaved: bool,
        /// Also the types with saved defaults.
        #[arg(long)]
        defaults: bool,
    },
    /// Print a session file.
    #[command(group(clap::ArgGroup::new("which").required(true).args(["reference", "defaults"])))]
    Show {
        /// The session: `folder/name`.
        #[arg(value_name = "REF")]
        reference: Option<String>,
        /// The defaults of a type instead.
        #[arg(long, value_name = "TYPE")]
        defaults: Option<String>,
    },
    /// Create a named session from the type's defaults.
    New {
        /// The session: `folder/name`.
        #[arg(value_name = "REF")]
        reference: String,
        /// folder-compare, folder-sync, text-compare, text-edit, table-compare, hex-compare, picture-compare.
        #[arg(long = "type", value_name = "TYPE")]
        kind: String,
        /// Left spec.
        #[arg(long, value_name = "SPEC")]
        left: Option<String>,
        /// Right spec.
        #[arg(long, value_name = "SPEC")]
        right: Option<String>,
        /// Description.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
    },
    /// Change a session, or the defaults of a type.
    #[command(group(clap::ArgGroup::new("which").required(true).args(["reference", "defaults"])))]
    #[command(group(clap::ArgGroup::new("change").required(true).multiple(true)
        .args(["set", "unset", "left", "right", "description"])))]
    Edit {
        /// The session: `folder/name`.
        #[arg(value_name = "REF")]
        reference: Option<String>,
        /// The defaults of a type instead (`--set` and `--unset` only).
        #[arg(long, value_name = "TYPE")]
        defaults: Option<String>,
        /// Set a key of the session file (`rules.criteria.size=false`; repeatable).
        #[arg(long, value_name = "KEY=VALUE")]
        set: Vec<String>,
        /// Remove a key of `rules`, `text_rules`, `display` or `view` (back to following the defaults;
        /// repeatable).
        #[arg(long, value_name = "KEY")]
        unset: Vec<String>,
        /// Left spec.
        #[arg(long, value_name = "SPEC", conflicts_with = "defaults")]
        left: Option<String>,
        /// Right spec.
        #[arg(long, value_name = "SPEC", conflicts_with = "defaults")]
        right: Option<String>,
        /// Description.
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,
        /// Change a locked session.
        #[arg(long)]
        force: bool,
    },
    /// Delete a named session.
    Delete {
        /// The session.
        #[arg(value_name = "REF")]
        reference: String,
        /// Delete a locked session.
        #[arg(long)]
        force: bool,
    },
    /// Rename or move a named session.
    Rename {
        /// The session.
        #[arg(value_name = "REF")]
        reference: String,
        /// Its new reference.
        #[arg(value_name = "NEW_REF")]
        new: String,
        /// Rename a locked session.
        #[arg(long)]
        force: bool,
    },
    /// Lock a session (changes are not saved).
    Lock {
        /// The session.
        #[arg(value_name = "REF")]
        reference: String,
    },
    /// Unlock a session.
    Unlock {
        /// The session.
        #[arg(value_name = "REF")]
        reference: String,
    },
    /// Create a session folder.
    Mkdir {
        /// The folder: `a/b`.
        folder: String,
    },
    /// Delete a session folder.
    Rmdir {
        /// The folder.
        folder: String,
        /// Also delete the sessions and folders inside.
        #[arg(long)]
        recursive: bool,
        /// Also delete locked sessions inside.
        #[arg(long)]
        force: bool,
    },
    /// Run a saved Folder Compare or Text Compare session headless.
    Run(Box<SessionRunArgs>),
}

/// `hikasa session run <ref>`: the report flags of `folder compare` and `text compare` (REPORT); `--dry-run` is the
/// sync's plan for a folder-sync session and the report's for the others.
#[derive(Debug, clap::Args)]
pub struct SessionRunArgs {
    /// The session.
    #[arg(value_name = "REF")]
    pub reference: String,
    /// Treat unimportant differences as same (verdict, exit code, report).
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
    /// The report.
    #[command(flatten)]
    pub report: ReportArgs,
    /// Folder Sync: answer every question yes.
    #[arg(long, short = 'y', conflicts_with_all = ["confirm", "dry_run"])]
    pub yes: bool,
    /// Folder Sync: prompt, yes-to-all or no-to-all.
    #[arg(long, value_name = "MODE")]
    pub confirm: Option<String>,
    /// Folder Sync: a side with nothing on it may make the other empty.
    #[arg(long)]
    pub allow_empty_source: bool,
    /// Folder Sync: the output shows every row.
    #[arg(long)]
    pub all_rows: bool,
}

/// `hikasa workspace …`.
#[derive(Debug, clap::Subcommand)]
pub enum WorkspaceCmd {
    /// List the workspaces.
    List,
    /// Print a workspace file.
    Show {
        /// The workspace.
        name: String,
    },
    /// Delete a workspace.
    Delete {
        /// The workspace.
        name: String,
    },
    /// Rename a workspace.
    Rename {
        /// The workspace.
        name: String,
        /// Its new name.
        new: String,
    },
}

/// `hikasa settings restore-defaults`.
#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("what").required(true).arg("all").arg("file").arg("category")))]
pub struct RestoreDefaultsArgs {
    /// Every settings file (not sessions, workspaces or formats).
    #[arg(long)]
    pub all: bool,
    /// A category of the factory-defaults plan: options, theme, commands, state, sessions (the defaults files),
    /// formats, workspaces (repeatable).
    #[arg(long, value_name = "CAT")]
    pub category: Vec<String>,
    /// With `--category`: also delete every named session.
    #[arg(long, requires = "category")]
    pub delete_sessions: bool,
    /// With `--category`: also delete every workspace.
    #[arg(long, requires = "category")]
    pub delete_workspaces: bool,
    /// With `--delete-sessions`: also delete locked sessions.
    #[arg(long, requires = "delete_sessions")]
    pub force: bool,
    /// One file: `options`, `state`, `commands`, `theme`, `themes`, `themes.<scheme>`, `filters`, `openwith`,
    /// `archives` (repeatable).
    #[arg(long, value_name = "FILE")]
    pub file: Vec<String>,
    /// Print the plan only.
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,
    /// Delete without asking (required unless `--dry-run`).
    #[arg(long)]
    pub yes: bool,
}

/// `hikasa format detect <file>`.
#[derive(Debug, clap::Args)]
pub struct DetectArgs {
    /// The file (`-` uses the `--stdin-name` hint).
    pub file: OsString,
    /// File name of standard input, for format detection.
    #[arg(long, value_name = "NAME")]
    pub stdin_name: Option<String>,
}

/// `hikasa format tokens <file>`.
#[derive(Debug, clap::Args)]
pub struct TokensArgs {
    /// The file (`-` reads standard input).
    pub file: OsString,
    /// Force a file format (id or name).
    #[arg(long, value_name = "ID|NAME")]
    pub format: Option<String>,
    /// Encoding of the file.
    #[arg(long, value_name = "LABEL")]
    pub encoding: Option<String>,
    /// Lines to print: `<first>-<last>` or one line number.
    #[arg(long, value_name = "A-B")]
    pub lines: Option<String>,
    /// File name of standard input, for format detection.
    #[arg(long, value_name = "NAME")]
    pub stdin_name: Option<String>,
}

/// `hikasa vfs …`.
#[derive(Debug, clap::Subcommand)]
pub enum VfsCmd {
    /// List a folder.
    Ls(VfsSpecArgs),
    /// Show the metadata of an item.
    Stat(VfsSpecArgs),
    /// Copy a file to stdout.
    Cat {
        /// The file.
        spec: OsString,
    },
    /// Copy files and folders (`--dry-run` prints the plan).
    Cp(VfsCpArgs),
    /// Move or rename files and folders.
    Mv(VfsMvArgs),
    /// Remove files, links and folders (needs --yes, or --dry-run).
    Rm(VfsRmArgs),
    /// Create folders.
    Mkdir(VfsMkdirArgs),
    /// Set the modification (and creation) time of items.
    Touch(VfsTouchArgs),
}

/// `vfs cp`.
#[derive(Debug, clap::Args)]
pub struct VfsCpArgs {
    /// The sources, then the target (an existing folder receives them; a trailing separator means a folder).
    #[arg(required = true, num_args = 2.., value_name = "SRC… DST")]
    pub paths: Vec<OsString>,
    /// Copy folders with their contents.
    #[arg(short, long)]
    pub recursive: bool,
    /// What an existing target means: fail (default), skip, always, newer.
    #[arg(long, value_name = "POLICY")]
    pub overwrite: Option<String>,
    /// How symbolic links are copied: copy (as links, default), skip.
    #[arg(long, value_name = "POLICY")]
    pub symlinks: Option<String>,
    /// Follow root arguments that are symbolic links (never links inside trees).
    #[arg(long)]
    pub follow_symlinks: bool,
    /// Copy no times or attributes.
    #[arg(long, conflicts_with = "preserve_created")]
    pub no_preserve: bool,
    /// Also copy the creation time (where the target supports it).
    #[arg(long)]
    pub preserve_created: bool,
    /// Create missing parent folders of the target.
    #[arg(short, long)]
    pub parents: bool,
    /// Print the plan; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `vfs mv`.
#[derive(Debug, clap::Args)]
pub struct VfsMvArgs {
    /// The sources, then the target.
    #[arg(required = true, num_args = 2.., value_name = "SRC… DST")]
    pub paths: Vec<OsString>,
    /// What an existing target means: fail (default), skip, always, newer.
    #[arg(long, value_name = "POLICY")]
    pub overwrite: Option<String>,
    /// Create missing parent folders of the target.
    #[arg(short, long)]
    pub parents: bool,
    /// Print the plan; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `vfs rm`.
#[derive(Debug, clap::Args)]
pub struct VfsRmArgs {
    /// The items.
    #[arg(required = true, value_name = "SPEC")]
    pub specs: Vec<OsString>,
    /// Remove folders with their contents.
    #[arg(short, long)]
    pub recursive: bool,
    /// Move each item to the recycle bin instead (never a permanent delete).
    #[arg(long)]
    pub trash: bool,
    /// Print the plan; remove nothing.
    #[arg(long, conflicts_with = "yes")]
    pub dry_run: bool,
    /// Confirm the removal.
    #[arg(long)]
    pub yes: bool,
}

/// `vfs mkdir`.
#[derive(Debug, clap::Args)]
pub struct VfsMkdirArgs {
    /// The folders.
    #[arg(required = true, value_name = "SPEC")]
    pub specs: Vec<OsString>,
    /// Create missing parents; an existing folder is no error.
    #[arg(short, long)]
    pub parents: bool,
    /// Print the plan; create nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// `vfs touch`.
#[derive(Debug, clap::Args)]
pub struct VfsTouchArgs {
    /// The items.
    #[arg(required = true, value_name = "SPEC")]
    pub specs: Vec<OsString>,
    /// The modification time (RFC 3339); default: now.
    #[arg(long, value_name = "RFC3339", conflicts_with = "reference")]
    pub time: Option<String>,
    /// Use the modification time of this item.
    #[arg(long, value_name = "SPEC")]
    pub reference: Option<OsString>,
    /// Also set the creation time (RFC 3339; where supported).
    #[arg(long, value_name = "RFC3339")]
    pub created: Option<String>,
    /// Create missing items as empty files.
    #[arg(long)]
    pub create: bool,
    /// Print the plan; change nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// The spec and link handling of `vfs ls` and `vfs stat`.
#[derive(Debug, clap::Args)]
pub struct VfsSpecArgs {
    /// The item.
    pub spec: OsString,
    /// Report the targets of symbolic links instead of the links.
    #[arg(long)]
    pub follow_symlinks: bool,
}

/// `hikasa gui …`.
#[derive(Debug, clap::Subcommand)]
pub enum GuiCmd {
    /// Open a view in the GUI (starting it when needed).
    #[command(disable_help_flag = true, disable_version_flag = true)]
    Open(GuiOpenArgs),
    /// Check that the GUI is running.
    Ping,
    /// List the open views.
    List,
    /// Close a view.
    Close {
        /// The view id (from `gui list`).
        id: u64,
    },
    /// Quit the GUI.
    Quit,
    /// Open a named session in the GUI.
    Session {
        /// The session: `folder/name`.
        #[arg(value_name = "REF")]
        reference: String,
        /// Wait until the view closes and exit with its code.
        #[arg(long)]
        wait: bool,
        /// Do not start the GUI when it is not running.
        #[arg(long)]
        no_launch: bool,
    },
    /// Load a workspace in the GUI.
    Workspace {
        /// The workspace.
        name: String,
        /// Do not start the GUI when it is not running.
        #[arg(long)]
        no_launch: bool,
    },
}

/// `gui open`: every argument after `open` (except the global flags clap sees directly after it) is passed raw to
/// `hikasa_session::api::gui::normalize_legacy_args` and `parse_open_args` (D-51), so that `hikasa gui open` and
/// `hikasa-gui` accept the same forms (`--help` prints their usage; `--version` is not one of their switches).
/// clap consumes a `--` that stands before the first value; main restores it (`raw_open_args`).
#[derive(Debug, clap::Args)]
pub struct GuiOpenArgs {
    /// Specs and switches (see `hikasa gui open --help`).
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        num_args = 0..,
        value_name = "ARGS"
    )]
    pub args: Vec<OsString>,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    /// The long flags of `command`, sorted.
    fn long_flags(command: &clap::Command) -> Vec<String> {
        let mut flags: Vec<String> = command
            .get_arguments()
            .filter_map(|arg| arg.get_long().map(str::to_owned))
            .collect();
        flags.sort();
        flags
    }

    #[test]
    fn a6_25_rule_flags_equal_folder_compares() {
        use clap::Args as _;
        let cli = Cli::command();
        let folder = cli.find_subcommand("folder").unwrap();
        let rules = long_flags(&FolderRuleArgs::augment_args(clap::Command::new("rules")));
        assert!(rules.contains(&"attrib-filter".to_owned()), "{rules:?}");
        let compare = long_flags(folder.find_subcommand("compare").unwrap());
        let op = folder.find_subcommand("op").unwrap();
        for verb in op.get_subcommands() {
            let own = long_flags(verb);
            for flag in &rules {
                assert!(compare.contains(flag), "compare: {flag}");
                assert!(own.contains(flag), "{}: {flag}", verb.get_name());
            }
        }
    }

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }
}

#[cfg(test)]
mod report_args_tests {
    use super::*;
    use clap::{Args, CommandFactory};

    /// A7.18: the five commands flatten one `ReportArgs`: every REPORT argument, with the same long name and value
    /// name, in each.
    #[test]
    fn a7_18_five_commands_flatten_one_report_args() {
        let group = ReportArgs::augment_args(clap::Command::new("group"));
        let report: Vec<&clap::Arg> = group.get_arguments().collect();
        assert_eq!(report.len(), 22, "the flags of REPORT");
        let cli = Cli::command();
        for (outer, inner) in [
            ("text", "compare"),
            ("folder", "compare"),
            ("session", "run"),
            ("report", "text"),
            ("report", "folder"),
        ] {
            let sub = cli
                .find_subcommand(outer)
                .and_then(|c| c.find_subcommand(inner))
                .unwrap_or_else(|| panic!("{outer} {inner}"));
            for arg in &report {
                let theirs = sub
                    .get_arguments()
                    .find(|a| a.get_id() == arg.get_id())
                    .unwrap_or_else(|| panic!("{outer} {inner} lacks --{:?}", arg.get_long()));
                assert_eq!(theirs.get_long(), arg.get_long(), "{outer} {inner}");
                assert_eq!(
                    theirs.get_value_names(),
                    arg.get_value_names(),
                    "{outer} {inner}"
                );
                assert_eq!(
                    theirs.get_visible_aliases(),
                    arg.get_visible_aliases(),
                    "{outer} {inner}"
                );
            }
        }
    }
}
