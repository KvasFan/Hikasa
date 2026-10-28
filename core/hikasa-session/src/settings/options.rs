//! `options.toml`: every key of docs/15 "Keys" (OPT-001…OPT-022; D-71, D-72). One Rust field per key, one struct
//! per table. The reserved sections `archive_types`, `commands` and `open_with` are known and hold nothing (their
//! data lives in `archives.toml`, `commands.toml` and `openwith.toml`); their keys are kept on save.

use toml::Table;

use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KindRule, ValueKind};
use super::theme::ThemeMode;
use crate::options::ProgramOptions;

settings_enum! {
    /// `[startup] quick_compare_method` (OPT-001).
    QuickCompareMethod { Binary = "binary", RulesBased = "rules_based" } default RulesBased
}

settings_enum! {
    /// Where a shell-integration command appears (OPT-003).
    MenuPosition { Off = "none", MainMenu = "main_menu", Submenu = "submenu" } default Submenu
}

settings_enum! {
    /// `[tabs] open_sessions_in`, `open_child_sessions_in` (OPT-004).
    OpenIn { Tab = "tab", Window = "window" } default Tab
}

settings_enum! {
    /// `[backups] naming` (OPT-008): how `abc.txt` is backed up.
    BackupNaming {
        NameTilde = "name_tilde",
        StemTilde = "stem_tilde",
        NameBak = "name_bak",
        TildeName = "tilde_name",
        StemBak = "stem_bak",
        StemBakExt = "stem_bak_ext",
        Folder = "folder",
    } default NameBak
}

settings_enum! {
    /// `[file_operations] sync_confirmations` (OPT-009).
    SyncConfirmations { Prompt = "prompt", YesToAll = "yes_to_all", NoToAll = "no_to_all" } default Prompt
}

settings_enum! {
    /// `[tweaks.file_operations] comparison_priority` (OPT-015).
    ComparisonPriority { Normal = "normal", BelowNormal = "below_normal", Idle = "idle" } default Normal
}

settings_enum! {
    /// `[tweaks.interface] thumbnail_on_large_files` (OPT-017).
    ThumbnailMode { Compress = "compress", Scroll = "scroll" } default Compress
}

settings_section! {
    /// `[startup]` (OPT-001…OPT-003).
    Startup {
        /// Workspace loaded at start (empty: the Home view).
        load_workspace: String = String::new(),
        /// Workspace saved on exit (empty: none).
        on_exit_save_workspace_as: String = String::new(),
        /// Show the quick compare dialog when starting with a file comparison.
        show_quick_compare_dialog: bool = false,
        /// Method of that quick compare.
        quick_compare_method: QuickCompareMethod = QuickCompareMethod::RulesBased,
        /// Open the view automatically if the files are different.
        open_view_if_different: bool = true,
        /// Clipboard history helper at login (P2).
        clipboard_helper_at_login: bool = false,
        /// Shell integration (P1).
        shell_integration: ShellIntegration = ShellIntegration::default(),
    }
}

settings_section! {
    /// `[startup.shell_integration]` (OPT-003, P1).
    ShellIntegration {
        /// Include Hikasa in the file manager's context menu.
        include_in_context_menu: bool = false,
        /// Compare.
        compare: MenuPosition = MenuPosition::MainMenu,
        /// Compare Using.
        compare_using: MenuPosition = MenuPosition::Submenu,
        /// Sync.
        sync: MenuPosition = MenuPosition::Submenu,
        /// Merge.
        merge: MenuPosition = MenuPosition::Submenu,
        /// Edit.
        edit: MenuPosition = MenuPosition::Submenu,
        /// Open for Compare.
        open_for_compare: MenuPosition = MenuPosition::Submenu,
        /// View Patch.
        view_patch: MenuPosition = MenuPosition::Submenu,
        /// Compare to Clipboard.
        compare_to_clipboard: MenuPosition = MenuPosition::Submenu,
        /// Compare to Registry (Windows).
        compare_to_registry: MenuPosition = MenuPosition::Off,
        /// Hide "Select Left" when a selection is already saved.
        hide_select_left_if_saved: bool = true,
    }
}

settings_section! {
    /// `[tabs]` (OPT-004).
    Tabs {
        /// Where sessions open.
        open_sessions_in: OpenIn = OpenIn::Tab,
        /// Where child sessions open.
        open_child_sessions_in: OpenIn = OpenIn::Tab,
        /// Open a new tab next to the active one.
        open_new_tab_next_to_active: bool = true,
        /// Closing the last tab closes the window.
        closing_last_tab_closes_window: bool = false,
        /// Warn when closing several tabs.
        warn_when_closing_multiple_tabs: bool = true,
        /// Warn when closing several windows.
        warn_when_closing_multiple_windows: bool = true,
        /// Hide the tab bar when there is one tab.
        hide_tab_bar_if_single_tab: bool = false,
        /// Taskbar previews (P1, deferred).
        show_taskbar_previews: bool = false,
    }
}

settings_section! {
    /// `[appearance]` (OPT-005; colours and fonts are in `theme.toml`, D-72).
    Appearance {
        /// Follow the OS, light or dark.
        theme_mode: ThemeMode = ThemeMode::FollowOs,
    }
}

settings_section! {
    /// `[text_editing]` (OPT-006).
    TextEditing {
        /// Auto indent.
        auto_indent: bool = true,
        /// Backspace unindents.
        backspace_unindents: bool = true,
        /// Initialize "Text to find" with the current word.
        init_find_with_current_word: bool = true,
        /// Show filtered line counts.
        show_filtered_line_counts: bool = false,
        /// Context lines of `--context` and the patch reports.
        number_of_context_lines: u32 = 3,
    }
}

settings_section! {
    /// `[next_difference]` (OPT-007).
    NextDifference {
        /// Go to the first difference when files load.
        go_to_first_difference_on_load: bool = true,
        /// Go to the next difference after copying to the other side.
        go_to_next_difference_after_copy: bool = false,
        /// Limit "Next Difference Files" to the current folder.
        limit_next_difference_files_to_current_folder: bool = false,
        /// Wrap around to the first difference.
        wrap_around: bool = false,
        /// Show the message panel when there are no more differences.
        show_message_panel: bool = true,
    }
}

settings_section! {
    /// `[backups]` (OPT-008).
    Backups {
        /// Back up files before copy.
        back_up_before_copy: bool = false,
        /// Back up files before save.
        back_up_before_save: bool = true,
        /// The backup name.
        naming: BackupNaming = BackupNaming::NameBak,
        /// The folder of `naming = "folder"` (created on demand).
        folder: String = String::new(),
    }
}

settings_section! {
    /// `[file_operations]` (OPT-009).
    FileOperations {
        /// Confirm file copy.
        confirm_copy: bool = true,
        /// Confirm file move.
        confirm_move: bool = true,
        /// Confirm replacing or deleting read-only files.
        confirm_read_only: bool = true,
        /// Confirm replacing or deleting system files.
        confirm_system: bool = true,
        /// Confirm overwriting newer files.
        confirm_overwrite_newer: bool = true,
        /// Confirm replacing files during move.
        confirm_replace_during_move: bool = true,
        /// Confirm content compare.
        confirm_content_compare: bool = false,
        /// Confirm file delete.
        confirm_delete: bool = true,
        /// Require an explicit side selection in the Delete dialog.
        require_side_selection_in_delete: bool = false,
        /// Confirm file merge.
        confirm_merge: bool = true,
        /// Synchronize confirmations.
        sync_confirmations: SyncConfirmations = SyncConfirmations::Prompt,
        /// Include hidden items by default.
        include_hidden_by_default: bool = true,
    }
}

settings_section! {
    /// `[tweaks]` (OPT-013…OPT-022).
    Tweaks {
        /// Check for updates (P1, deferred).
        check_for_updates: CheckForUpdates = CheckForUpdates::default(),
        /// Editor display.
        editor_display: EditorDisplay = EditorDisplay::default(),
        /// File operations.
        file_operations: TweakFileOperations = TweakFileOperations::default(),
        /// Home view (P1).
        home_view: HomeView = HomeView::default(),
        /// Interface.
        interface: Interface = Interface::default(),
        /// Log panel.
        log_panel: LogPanel = LogPanel::default(),
        /// Metrics.
        metrics: Metrics = Metrics::default(),
        /// Scripts.
        scripts: Scripts = Scripts::default(),
        /// Shared sessions file (P1).
        shared_sessions_file: SharedSessionsFile = SharedSessionsFile::default(),
    }
}

settings_section! {
    /// `[tweaks.check_for_updates]` (OPT-013, P1).
    CheckForUpdates {
        /// Days between checks (0: never).
        every_days: u32 = 7,
    }
}

settings_section! {
    /// `[tweaks.editor_display]` (OPT-014).
    EditorDisplay {
        /// Syntax highlighting on difference lines.
        syntax_on_difference_lines: bool = true,
        /// Crosshatch lines beyond the end of the file.
        crosshatch_beyond_eof: bool = true,
        /// Right-side gutter for the left editor.
        right_gutter_for_left_editor: bool = false,
        /// Use the orphan colour.
        use_orphan_color: bool = true,
        /// Extra line spacing in pixels.
        extra_line_spacing_px: u32 = 0,
        /// Column line position (0: off).
        column_line_at: u32 = 0,
        /// Dim the inactive pane by this percentage.
        dim_inactive_pane_percent: u32 = 0,
        /// A different font for merge input panes (P1).
        different_font_for_merge_inputs: bool = false,
    }
}

settings_section! {
    /// `[tweaks.file_operations]` (OPT-015).
    TweakFileOperations {
        /// Beep after long file operations.
        beep_after_long_operations: bool = false,
        /// Remove the read-only flag when copying from read-only media.
        remove_read_only_flag_from_read_only_media: bool = true,
        /// Check for files changed on disk.
        check_for_files_changed_on_disk: bool = true,
        /// Reload automatically unless changes would be discarded.
        automatically_reload: bool = true,
        /// Use IPv6 when available (P1).
        use_ipv6: bool = true,
        /// Comparison thread priority.
        comparison_priority: ComparisonPriority = ComparisonPriority::Normal,
        /// Buffer of the binary comparison in bytes (used clamped to 4 KiB … 64 MiB).
        buffer_size_for_binary_compare: u32 = 1 << 20,
        /// Content-test memory budget in MiB (D-25; 0 counts as 1).
        content_memory_budget_mib: u32 = 1024,
    }
}

settings_section! {
    /// `[tweaks.home_view]` (OPT-016, P1).
    HomeView {
        /// Sticky session selection.
        sticky_session_selection: bool = false,
    }
}

settings_section! {
    /// `[tweaks.interface]` (OPT-017).
    Interface {
        /// Show hints for toolbar buttons.
        show_hints: bool = true,
        /// Show the keyboard shortcut in hints.
        show_shortcut_in_hints: bool = true,
        /// Esc closes file views.
        esc_closes_file_views: bool = false,
        /// One vertical scroll bar.
        single_vertical_scrollbar: bool = false,
        /// Sticky splitter position.
        sticky_splitter_position: bool = true,
        /// Thumbnail of large files.
        thumbnail_on_large_files: ThumbnailMode = ThumbnailMode::Compress,
    }
}

settings_section! {
    /// `[tweaks.log_panel]` (OPT-018).
    LogPanel {
        /// Operation summaries.
        operation_summaries: bool = true,
        /// Operation details.
        operation_details: bool = false,
        /// Warnings.
        warnings: bool = true,
        /// Remote protocol trace (P1).
        remote_protocol_trace: bool = false,
        /// Debug messages.
        debug_messages: bool = false,
        /// Messages kept.
        display_up_to: u32 = 1000,
        /// Save messages to this file (empty: off).
        save_to_file: String = String::new(),
    }
}

settings_section! {
    /// `[tweaks.metrics]` (OPT-019).
    Metrics {
        /// Auto-saved sessions kept.
        max_auto_saved_sessions: u32 = 200,
        /// Items of each most-recently-used list.
        max_mru_items: u32 = 20,
        /// Milliseconds before a hint appears.
        hint_show_ms: u32 = 500,
        /// Milliseconds before a hint disappears (0: never).
        hint_hide_ms: u32 = 5000,
        /// Milliseconds before incremental search resets (P1).
        incremental_search_reset_ms: u32 = 1000,
    }
}

settings_section! {
    /// `[tweaks.scripts]` (OPT-021).
    Scripts {
        /// Beep when a script finishes.
        beep_when_finished: bool = false,
        /// Close the script window when it finishes.
        close_when_finished: bool = false,
        /// Shared scripts folder.
        shared_scripts_folder: String = String::new(),
    }
}

settings_section! {
    /// `[tweaks.shared_sessions_file]` (OPT-022, P1).
    SharedSessionsFile {
        /// Path of the settings package whose sessions appear read-only.
        path: String = String::new(),
    }
}

settings_section! {
    /// `options.toml` (docs/15 "Keys").
    Options {
        /// `[startup]`.
        startup: Startup = Startup::default(),
        /// `[tabs]`.
        tabs: Tabs = Tabs::default(),
        /// `[appearance]`.
        appearance: Appearance = Appearance::default(),
        /// `[text_editing]`.
        text_editing: TextEditing = TextEditing::default(),
        /// `[next_difference]`.
        next_difference: NextDifference = NextDifference::default(),
        /// `[backups]`.
        backups: Backups = Backups::default(),
        /// `[file_operations]`.
        file_operations: FileOperations = FileOperations::default(),
        /// `[tweaks]`.
        tweaks: Tweaks = Tweaks::default(),
    }
}

impl Options {
    /// The M1 view of the options (the three keys the comparison engines read).
    pub fn program_options(&self) -> ProgramOptions {
        ProgramOptions {
            context_lines: self.text_editing.number_of_context_lines,
            binary_buffer_size: self.tweaks.file_operations.buffer_size_for_binary_compare,
            content_memory_budget_mib: self.tweaks.file_operations.content_memory_budget_mib,
        }
    }
}

/// The reserved top-level sections (D-72): known, empty, their keys kept.
pub const RESERVED_SECTIONS: &[&str] = &["archive_types", "commands", "open_with"];

impl SettingsSchema for Options {
    const FILE: FileId = FileId::Options;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    fn validate(&self) -> Vec<Problem> {
        super::validate_by_rules(self, &FileId::Options)
    }

    fn known_sections() -> &'static [&'static str] {
        RESERVED_SECTIONS
    }
}

/// An `int a..b` rule.
const fn int(
    pattern: &'static str,
    min: i64,
    max: i64,
    opt: &'static str,
    p: &'static str,
) -> KindRule {
    rule(pattern, ValueKind::Int { min, max }, opt, p)
}

/// A rule with an OPT id and a priority.
const fn rule(
    pattern: &'static str,
    kind: ValueKind,
    opt: &'static str,
    p: &'static str,
) -> KindRule {
    KindRule {
        pattern,
        kind,
        opt: Some(opt),
        priority: Some(p),
    }
}

/// The largest `u32` as an `i64`.
const U32_MAX: i64 = u32::MAX as i64;

/// Kinds, OPT ids and priorities of every key (docs/15 "Keys"). Booleans need a rule only for their OPT id.
pub(crate) const RULES: &[KindRule] = &[
    rule("startup.load_workspace", ValueKind::String, "OPT-001", "P0"),
    rule(
        "startup.on_exit_save_workspace_as",
        ValueKind::String,
        "OPT-001",
        "P0",
    ),
    rule(
        "startup.show_quick_compare_dialog",
        ValueKind::Bool,
        "OPT-001",
        "P0",
    ),
    rule(
        "startup.quick_compare_method",
        ValueKind::Enum(QuickCompareMethod::ALL),
        "OPT-001",
        "P0",
    ),
    rule(
        "startup.open_view_if_different",
        ValueKind::Bool,
        "OPT-001",
        "P0",
    ),
    rule(
        "startup.clipboard_helper_at_login",
        ValueKind::Bool,
        "OPT-002",
        "P2",
    ),
    rule(
        "startup.shell_integration.include_in_context_menu",
        ValueKind::Bool,
        "OPT-003",
        "P1",
    ),
    rule(
        "startup.shell_integration.hide_select_left_if_saved",
        ValueKind::Bool,
        "OPT-003",
        "P1",
    ),
    rule(
        "startup.shell_integration.*",
        ValueKind::Enum(MenuPosition::ALL),
        "OPT-003",
        "P1",
    ),
    rule(
        "tabs.open_sessions_in",
        ValueKind::Enum(OpenIn::ALL),
        "OPT-004",
        "P0",
    ),
    rule(
        "tabs.open_child_sessions_in",
        ValueKind::Enum(OpenIn::ALL),
        "OPT-004",
        "P0",
    ),
    rule(
        "tabs.show_taskbar_previews",
        ValueKind::Bool,
        "OPT-004",
        "P1",
    ),
    rule("tabs.*", ValueKind::Bool, "OPT-004", "P0"),
    rule(
        "appearance.theme_mode",
        ValueKind::Enum(ThemeMode::ALL),
        "OPT-005",
        "P0",
    ),
    int(
        "text_editing.number_of_context_lines",
        0,
        U32_MAX,
        "OPT-006",
        "P0",
    ),
    rule("text_editing.*", ValueKind::Bool, "OPT-006", "P0"),
    rule("next_difference.*", ValueKind::Bool, "OPT-007", "P0"),
    rule(
        "backups.naming",
        ValueKind::Enum(BackupNaming::ALL),
        "OPT-008",
        "P0",
    ),
    rule("backups.folder", ValueKind::Path, "OPT-008", "P0"),
    rule("backups.*", ValueKind::Bool, "OPT-008", "P0"),
    rule(
        "file_operations.sync_confirmations",
        ValueKind::Enum(SyncConfirmations::ALL),
        "OPT-009",
        "P0",
    ),
    rule("file_operations.*", ValueKind::Bool, "OPT-009", "P0"),
    int(
        "tweaks.check_for_updates.every_days",
        0,
        365,
        "OPT-013",
        "P1",
    ),
    int(
        "tweaks.editor_display.extra_line_spacing_px",
        0,
        20,
        "OPT-014",
        "P0",
    ),
    int(
        "tweaks.editor_display.column_line_at",
        0,
        1000,
        "OPT-014",
        "P0",
    ),
    int(
        "tweaks.editor_display.dim_inactive_pane_percent",
        0,
        90,
        "OPT-014",
        "P0",
    ),
    rule(
        "tweaks.editor_display.different_font_for_merge_inputs",
        ValueKind::Bool,
        "OPT-014",
        "P1",
    ),
    rule("tweaks.editor_display.*", ValueKind::Bool, "OPT-014", "P0"),
    rule(
        "tweaks.file_operations.use_ipv6",
        ValueKind::Bool,
        "OPT-015",
        "P1",
    ),
    rule(
        "tweaks.file_operations.comparison_priority",
        ValueKind::Enum(ComparisonPriority::ALL),
        "OPT-015",
        "P0",
    ),
    int(
        "tweaks.file_operations.buffer_size_for_binary_compare",
        0,
        U32_MAX,
        "OPT-015",
        "P0",
    ),
    int(
        "tweaks.file_operations.content_memory_budget_mib",
        0,
        U32_MAX,
        "OPT-015",
        "P0",
    ),
    rule("tweaks.file_operations.*", ValueKind::Bool, "OPT-015", "P0"),
    rule(
        "tweaks.home_view.sticky_session_selection",
        ValueKind::Bool,
        "OPT-016",
        "P1",
    ),
    rule(
        "tweaks.interface.thumbnail_on_large_files",
        ValueKind::Enum(ThumbnailMode::ALL),
        "OPT-017",
        "P0",
    ),
    rule("tweaks.interface.*", ValueKind::Bool, "OPT-017", "P0"),
    rule(
        "tweaks.log_panel.remote_protocol_trace",
        ValueKind::Bool,
        "OPT-018",
        "P1",
    ),
    int(
        "tweaks.log_panel.display_up_to",
        10,
        100_000,
        "OPT-018",
        "P0",
    ),
    rule(
        "tweaks.log_panel.save_to_file",
        ValueKind::Path,
        "OPT-018",
        "P0",
    ),
    rule("tweaks.log_panel.*", ValueKind::Bool, "OPT-018", "P0"),
    int(
        "tweaks.metrics.max_auto_saved_sessions",
        0,
        10_000,
        "OPT-019",
        "P0",
    ),
    int("tweaks.metrics.max_mru_items", 1, 100, "OPT-019", "P0"),
    int("tweaks.metrics.hint_show_ms", 0, 10_000, "OPT-019", "P0"),
    int("tweaks.metrics.hint_hide_ms", 0, 60_000, "OPT-019", "P0"),
    int(
        "tweaks.metrics.incremental_search_reset_ms",
        100,
        10_000,
        "OPT-019",
        "P1",
    ),
    rule(
        "tweaks.scripts.shared_scripts_folder",
        ValueKind::Path,
        "OPT-021",
        "P0",
    ),
    rule("tweaks.scripts.*", ValueKind::Bool, "OPT-021", "P0"),
    rule(
        "tweaks.shared_sessions_file.path",
        ValueKind::Path,
        "OPT-022",
        "P1",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::file::{LoadMode, SettingsFile};

    #[test]
    fn defaults_write_only_the_version() {
        let file = SettingsFile::<Options>::parse("", "o", LoadMode::Strict).unwrap();
        assert_eq!(file.to_toml(), "schema_version = 1\n");
        assert_eq!(
            Options::default().program_options(),
            ProgramOptions::default()
        );
    }

    #[test]
    fn ranges_are_problems() {
        let mut o = Options::default();
        o.tweaks.metrics.max_mru_items = 0;
        o.tweaks.editor_display.dim_inactive_pane_percent = 91;
        let problems = o.validate();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(
            problems[0].key,
            "tweaks.editor_display.dim_inactive_pane_percent"
        );
        assert_eq!(problems[1].key, "tweaks.metrics.max_mru_items");
        o.repair(&problems);
        assert_eq!(o, Options::default());
    }
}
