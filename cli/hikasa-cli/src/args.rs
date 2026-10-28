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
    /// Context lines (default from the options, 3).
    #[arg(long, value_name = "N")]
    pub context: Option<u32>,
    /// Display filter: all, mismatches, context, matches.
    #[arg(long, value_name = "MODE")]
    pub display: Option<String>,
    /// Report layout: unified, side-by-side, summary, patch, statistics.
    #[arg(long, value_name = "LAYOUT", default_value = "unified")]
    pub report: String,
    /// Patch style of `--report patch`: normal, context, unified.
    #[arg(long, value_name = "STYLE", default_value = "unified")]
    pub patch_style: String,
    /// HTML output (side-by-side and summary).
    #[arg(long, conflicts_with = "html_mono")]
    pub html: bool,
    /// Monochrome HTML output.
    #[arg(long)]
    pub html_mono: bool,
    /// Show line numbers (side-by-side; the default).
    #[arg(long, overrides_with = "no_line_numbers")]
    pub line_numbers: bool,
    /// Hide line numbers (side-by-side).
    #[arg(long)]
    pub no_line_numbers: bool,
    /// Total width of side-by-side text (columns).
    #[arg(long, value_name = "N", default_value_t = 160)]
    pub width: u16,
    /// Line wrapping of side-by-side text: none, character, word.
    #[arg(long, value_name = "MODE", default_value = "character")]
    pub wrap: String,
    /// Report title.
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// Write the report to FILE (atomically) instead of stdout.
    #[arg(long, value_name = "FILE")]
    pub output: Option<OsString>,
    /// File name of standard input, for format detection.
    #[arg(long, value_name = "NAME")]
    pub stdin_name: Option<String>,
    // Hands the comparison to the GUI (D-50).
    /// Show the comparison in the GUI instead of comparing here.
    #[arg(
        long,
        conflicts_with_all = ["output", "report", "patch_style", "html", "html_mono", "line_numbers",
                              "no_line_numbers", "width", "wrap", "title"]
    )]
    pub gui: bool,
    /// With `--gui`: wait until the view closes and exit with its code.
    #[arg(long, requires = "gui")]
    pub wait: bool,
    /// With `--gui`: do not start the GUI when it is not running.
    #[arg(long, requires = "gui")]
    pub no_launch: bool,
}

/// `hikasa folder …`.
#[derive(Debug, clap::Subcommand)]
pub enum FolderCmd {
    /// Compare two folders.
    Compare(Box<FolderCompareArgs>),
}

/// `hikasa folder compare <left> <right>`.
#[derive(Debug, clap::Args)]
pub struct FolderCompareArgs {
    /// Left base folder.
    pub left: OsString,
    /// Right base folder.
    pub right: OsString,
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
    /// Display filter (default differences).
    #[arg(long, value_name = "FILTER")]
    pub display: Option<String>,
    /// Folder mode of the display: always, structure, files-only.
    #[arg(long, value_name = "MODE")]
    pub folders: Option<String>,
    /// Treat unimportant differences as same (verdict, exit code, report).
    #[arg(long, visible_alias = "iu")]
    pub ignore_unimportant: bool,
    /// Show the items hidden by file filters.
    #[arg(long)]
    pub suppress_filters: bool,
    /// Expand all subfolders (the printed reports are always fully expanded).
    #[arg(long)]
    pub expand_all: bool,
    /// Report layout: list, side-by-side, summary, json.
    #[arg(long, value_name = "LAYOUT", default_value = "list")]
    pub report: String,
    /// Columns of the side-by-side report (e.g. `size,modified`).
    #[arg(long, value_name = "LIST")]
    pub columns: Option<String>,
    /// HTML output (side-by-side and summary).
    #[arg(long, conflicts_with = "html_mono")]
    pub html: bool,
    /// Monochrome HTML output.
    #[arg(long)]
    pub html_mono: bool,
    /// Report title.
    #[arg(long, value_name = "TEXT")]
    pub title: Option<String>,
    /// Write the report to FILE (atomically) instead of stdout.
    #[arg(long, value_name = "FILE")]
    pub output: Option<OsString>,
    // Hands the comparison to the GUI (D-50).
    /// Show the comparison in the GUI instead of comparing here.
    #[arg(
        long,
        conflicts_with_all = ["output", "report", "columns", "html", "html_mono", "title"]
    )]
    pub gui: bool,
    /// With `--gui`: wait until the view closes and exit with its code.
    #[arg(long, requires = "gui")]
    pub wait: bool,
    /// With `--gui`: do not start the GUI when it is not running.
    #[arg(long, requires = "gui")]
    pub no_launch: bool,
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
}

/// `hikasa settings restore-defaults`.
#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("what").required(true).arg("all").arg("file")))]
pub struct RestoreDefaultsArgs {
    /// Every settings file (not sessions, workspaces or formats).
    #[arg(long)]
    pub all: bool,
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

    #[test]
    fn the_command_tree_is_valid() {
        Cli::command().debug_assert();
    }
}
