//! The page table of the Options dialog (M2 W4c, §1.4.13): data only. Every P0 key of `options.toml` is on exactly
//! one page and no P1/P2 key on any (D-128; the tests walk `key_infos`). Label wording is original, after docs/15.

use hikasa_session::settings::theme::SchemeGroup;

/// What fills the right side of a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    /// Only the fields (and nothing at all for a heading such as Tweaks).
    Fields,
    /// Mode and schemes, the scheme buttons (D-130).
    Appearance,
    /// The elements of a scheme group (D-130).
    Scheme(SchemeGroup),
    /// The four fonts (D-130).
    Fonts,
    /// The masks of `archives.toml` (OPT-010).
    ArchiveTypes,
    /// Toolbars, Shortcuts, Menus (D-129).
    Commands,
    /// The list of `openwith.toml` (OPT-012).
    OpenWith,
    /// The list of `filters.toml` (OPT-020).
    NameFilters,
}

/// A widget that is not the one of the key's `ValueKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Widget {
    /// By `ValueKind` (D-128).
    Auto,
    /// A combo of the saved workspaces (empty: none).
    Workspace,
}

/// One option key on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    /// The dotted key inside `options.toml`.
    pub path: &'static str,
    /// The label.
    pub label: &'static str,
    /// The widget.
    pub widget: Widget,
}

const fn f(path: &'static str, label: &'static str) -> FieldDef {
    FieldDef {
        path,
        label,
        widget: Widget::Auto,
    }
}

const fn w(path: &'static str, label: &'static str) -> FieldDef {
    FieldDef {
        path,
        label,
        widget: Widget::Workspace,
    }
}

/// One page of the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// The id (`CommandArg::Page`, `state.dialogs.options.page`).
    pub id: &'static str,
    /// The title in the tree.
    pub title: &'static str,
    /// The parent page.
    pub parent: Option<&'static str>,
    /// A note above the fields.
    pub note: Option<&'static str>,
    /// The fields.
    pub fields: &'static [FieldDef],
    /// What the page shows.
    pub body: Body,
    /// The OPT ids of the page's lists and editors (the fields bring their own), for the search.
    pub opts: &'static [&'static str],
}

const fn page(
    id: &'static str,
    title: &'static str,
    parent: Option<&'static str>,
    fields: &'static [FieldDef],
) -> Page {
    Page {
        id,
        title,
        parent,
        note: None,
        fields,
        body: Body::Fields,
        opts: &[],
    }
}

const fn editor(
    id: &'static str,
    title: &'static str,
    parent: Option<&'static str>,
    body: Body,
    opts: &'static [&'static str],
) -> Page {
    Page {
        id,
        title,
        parent,
        note: None,
        fields: &[],
        body,
        opts,
    }
}

/// The page shown when nothing is remembered.
pub const FIRST_PAGE: &str = "startup";

/// Every page, in tree order.
pub const PAGES: &[Page] = &[
    Page {
        note: Some("The quick compare keys are stored now and used from a later version."),
        ..page(
            "startup",
            "Startup",
            None,
            &[
                w("startup.load_workspace", "Load workspace"),
                w(
                    "startup.on_exit_save_workspace_as",
                    "On exit, save workspace as",
                ),
                f(
                    "startup.show_quick_compare_dialog",
                    "When starting with a file comparison, show quick compare dialog",
                ),
                f("startup.quick_compare_method", "Quick compare method"),
                f(
                    "startup.open_view_if_different",
                    "Open view automatically if files are different",
                ),
            ],
        )
    },
    page(
        "tabs",
        "Tabs",
        None,
        &[
            f("tabs.open_sessions_in", "Open sessions in"),
            f("tabs.open_child_sessions_in", "Open child sessions in"),
            f(
                "tabs.open_new_tab_next_to_active",
                "Open new tab next to active tab",
            ),
            f(
                "tabs.closing_last_tab_closes_window",
                "Closing last remaining tab closes the window",
            ),
            f(
                "tabs.warn_when_closing_multiple_tabs",
                "Warn when closing multiple tabs",
            ),
            f(
                "tabs.warn_when_closing_multiple_windows",
                "Warn when closing multiple windows",
            ),
            f(
                "tabs.hide_tab_bar_if_single_tab",
                "Hide tab bar if single tab",
            ),
        ],
    ),
    Page {
        body: Body::Appearance,
        ..page(
            "appearance",
            "Theme, Colors, Fonts",
            None,
            &[f("appearance.theme_mode", "Theme mode")],
        )
    },
    editor(
        "appearance.folder",
        "Folder Views",
        Some("appearance"),
        Body::Scheme(SchemeGroup::Folder),
        &["OPT-005", "UI-021"],
    ),
    editor(
        "appearance.file",
        "File Views",
        Some("appearance"),
        Body::Scheme(SchemeGroup::File),
        &["OPT-005", "UI-022"],
    ),
    editor(
        "appearance.picture",
        "Picture Compare",
        Some("appearance"),
        Body::Scheme(SchemeGroup::Picture),
        &["OPT-005", "UI-023"],
    ),
    editor(
        "appearance.fonts",
        "Fonts",
        Some("appearance"),
        Body::Fonts,
        &["OPT-005"],
    ),
    page(
        "text_editing",
        "Text Editing",
        None,
        &[
            f("text_editing.auto_indent", "Auto indent"),
            f("text_editing.backspace_unindents", "Backspace unindents"),
            f(
                "text_editing.init_find_with_current_word",
                "Initialize \"Text to find\" to be the current word",
            ),
            f(
                "text_editing.show_filtered_line_counts",
                "Show filtered line counts",
            ),
            f(
                "text_editing.number_of_context_lines",
                "Number of context lines",
            ),
        ],
    ),
    page(
        "next_difference",
        "Next Difference",
        None,
        &[
            f(
                "next_difference.go_to_first_difference_on_load",
                "When loading new files, go to first difference",
            ),
            f(
                "next_difference.go_to_next_difference_after_copy",
                "Go to next difference after copying to other side",
            ),
            f(
                "next_difference.limit_next_difference_files_to_current_folder",
                "Limit \"Next Difference Files\" to current folder",
            ),
            f(
                "next_difference.wrap_around",
                "When no more differences: wrap around to first difference",
            ),
            f(
                "next_difference.show_message_panel",
                "When no more differences: show message panel",
            ),
        ],
    ),
    page(
        "backups",
        "Backups",
        None,
        &[
            f("backups.back_up_before_copy", "Back up files before copy"),
            f("backups.back_up_before_save", "Back up files before save"),
            f("backups.naming", "Back up \"abc.txt\" as"),
            f("backups.folder", "Backup folder (created on demand)"),
        ],
    ),
    page(
        "file_operations",
        "File Operations",
        None,
        &[
            f("file_operations.confirm_copy", "Confirm file copy"),
            f("file_operations.confirm_move", "Confirm file move"),
            f(
                "file_operations.confirm_read_only",
                "Confirm replacing or deleting read-only files",
            ),
            f(
                "file_operations.confirm_system",
                "Confirm replacing or deleting system files",
            ),
            f(
                "file_operations.confirm_overwrite_newer",
                "Confirm overwriting newer files",
            ),
            f(
                "file_operations.confirm_replace_during_move",
                "Confirm replacing files during move",
            ),
            f(
                "file_operations.confirm_content_compare",
                "Confirm content compare",
            ),
            f("file_operations.confirm_delete", "Confirm file delete"),
            f(
                "file_operations.require_side_selection_in_delete",
                "Require explicit side selection in Delete dialog",
            ),
            f("file_operations.confirm_merge", "Confirm file merge"),
            f(
                "file_operations.sync_confirmations",
                "Synchronize confirmations",
            ),
            f(
                "file_operations.include_hidden_by_default",
                "Include hidden items by default",
            ),
        ],
    ),
    Page {
        note: Some("Masks apply to comparisons started after the change."),
        ..editor(
            "archive_types",
            "Archive Types",
            None,
            Body::ArchiveTypes,
            &["OPT-010"],
        )
    },
    editor(
        "commands",
        "Toolbars, Shortcuts, Menus",
        None,
        Body::Commands,
        &["OPT-011"],
    ),
    editor("open_with", "Open With", None, Body::OpenWith, &["OPT-012"]),
    page("tweaks", "Tweaks", None, &[]),
    page(
        "tweaks.editor_display",
        "Editor Display",
        Some("tweaks"),
        &[
            f(
                "tweaks.editor_display.syntax_on_difference_lines",
                "Show syntax highlighting on difference lines",
            ),
            f(
                "tweaks.editor_display.crosshatch_beyond_eof",
                "Use crosshatching for lines beyond the end of file",
            ),
            f(
                "tweaks.editor_display.right_gutter_for_left_editor",
                "Right side gutter for left editor",
            ),
            f("tweaks.editor_display.use_orphan_color", "Use orphan color"),
            f(
                "tweaks.editor_display.extra_line_spacing_px",
                "Extra line spacing (px)",
            ),
            f(
                "tweaks.editor_display.column_line_at",
                "Show column line at (0 = off)",
            ),
            f(
                "tweaks.editor_display.dim_inactive_pane_percent",
                "Dim inactive pane by (%)",
            ),
        ],
    ),
    page(
        "tweaks.file_operations",
        "File Operations",
        Some("tweaks"),
        &[
            f(
                "tweaks.file_operations.beep_after_long_operations",
                "Beep after long file operations",
            ),
            f(
                "tweaks.file_operations.remove_read_only_flag_from_read_only_media",
                "Remove read-only flag when copying from read-only media",
            ),
            f(
                "tweaks.file_operations.check_for_files_changed_on_disk",
                "Check for files changed on disk",
            ),
            f(
                "tweaks.file_operations.automatically_reload",
                "Automatically reload unless changes will be discarded",
            ),
            f(
                "tweaks.file_operations.comparison_priority",
                "Comparison priority",
            ),
            f(
                "tweaks.file_operations.buffer_size_for_binary_compare",
                "Buffer size for binary compare (bytes)",
            ),
            f(
                "tweaks.file_operations.content_memory_budget_mib",
                "Content-test memory budget (MiB)",
            ),
        ],
    ),
    page(
        "tweaks.interface",
        "Interface",
        Some("tweaks"),
        &[
            f(
                "tweaks.interface.show_hints",
                "Show hints for toolbar buttons",
            ),
            f(
                "tweaks.interface.show_shortcut_in_hints",
                "Show keyboard shortcut in hints",
            ),
            f(
                "tweaks.interface.esc_closes_file_views",
                "Esc key closes file views",
            ),
            f(
                "tweaks.interface.single_vertical_scrollbar",
                "Single vertical scrollbar",
            ),
            f(
                "tweaks.interface.sticky_splitter_position",
                "Sticky splitter position",
            ),
            f(
                "tweaks.interface.thumbnail_on_large_files",
                "Thumbnail on large files",
            ),
        ],
    ),
    page(
        "tweaks.log_panel",
        "Log Panel",
        Some("tweaks"),
        &[
            f(
                "tweaks.log_panel.operation_summaries",
                "Operation summaries",
            ),
            f("tweaks.log_panel.operation_details", "Operation details"),
            f("tweaks.log_panel.warnings", "Warnings"),
            f("tweaks.log_panel.debug_messages", "Debug messages"),
            f("tweaks.log_panel.display_up_to", "Display up to (messages)"),
            f(
                "tweaks.log_panel.save_to_file",
                "Automatically save messages to file",
            ),
        ],
    ),
    page(
        "tweaks.metrics",
        "Metrics",
        Some("tweaks"),
        &[
            f(
                "tweaks.metrics.max_auto_saved_sessions",
                "Max auto-saved sessions",
            ),
            f(
                "tweaks.metrics.max_mru_items",
                "Max most-recently-used items",
            ),
            f(
                "tweaks.metrics.hint_show_ms",
                "Milliseconds before a hint appears",
            ),
            f(
                "tweaks.metrics.hint_hide_ms",
                "Milliseconds before a hint disappears (0 = never)",
            ),
        ],
    ),
    editor(
        "tweaks.name_filter_presets",
        "Name Filter Presets",
        Some("tweaks"),
        Body::NameFilters,
        &["OPT-020"],
    ),
    page(
        "tweaks.scripts",
        "Scripts",
        Some("tweaks"),
        &[
            f("tweaks.scripts.beep_when_finished", "Beep when finished"),
            f("tweaks.scripts.close_when_finished", "Close when finished"),
            f(
                "tweaks.scripts.shared_scripts_folder",
                "Shared scripts folder",
            ),
        ],
    ),
];

/// The page `id`.
pub fn page_of(id: &str) -> Option<&'static Page> {
    PAGES.iter().find(|p| p.id == id)
}
