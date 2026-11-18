//! The window's commands (ARCH-070, UI-003; M2 W4a, D-109…D-111): the GUI keeps per command id only its
//! *presentation* — icon, short toolbar label, radio or check group, availability predicate and the context-menu
//! areas — in [`PRESENTATION`]. Captions, descriptions (hints), menus, toolbar flags and shortcuts come from the
//! core's effective command table (`hikasa_session::commands::effective`, the catalogue with `commands.toml`
//! applied), rebuilt into [`CommandTables`] at start and when `commands.toml` changes, never per frame. Key
//! presses are dispatched by [`dispatch`] (D-111); hints are [`hint_text`] and [`HintTiming`] (UI-007).
//!
//! This module holds no toolkit type (§1.4.16 rule 4); `ui/` draws what it describes.

use hikasa_core::Side;
use hikasa_session::commands::{EffectiveCommand, ViewCommands, catalogue, effective};
use hikasa_session::settings::{CommandOverrides, Shortcut};

use crate::ui::icons::IconId;

/// The view type a command belongs to (commands are registered per view type, ARCH-070).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub enum Scope {
    /// Every view.
    #[default]
    Global,
    /// The Home view (the entry screen).
    Home,
    /// Folder Compare.
    Folder,
    /// Text Compare.
    Text,
    /// Folder Sync (M2 W9b, D-247).
    Sync,
    /// Table Compare (Table Compare phase 3, W13).
    Table,
    /// Text Edit (M2 W11b, D-313): one text pane, no comparison.
    TextEdit,
    /// Hex Compare (M2 W13a, D-371).
    Hex,
}

impl Scope {
    /// The view types the GUI shows.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const VIEWS: [Scope; 7] = [
        Scope::Home,
        Scope::Folder,
        Scope::Text,
        Scope::Sync,
        Scope::Table,
        Scope::TextEdit,
        Scope::Hex,
    ];

    /// The view type's title (`<Type>` in a catalogue caption).
    pub fn type_title(self) -> &'static str {
        match self {
            Scope::Global => "",
            Scope::Home => "Home",
            Scope::Folder => "Folder Compare",
            Scope::Text => "Text Compare",
            Scope::Sync => "Folder Sync",
            Scope::Table => "Table Compare",
            Scope::TextEdit => "Text Edit",
            Scope::Hex => "Hex Compare",
        }
    }

    /// The view type's name in the catalogue and `commands.toml` (`home`, `folder-compare`, `text-compare`;
    /// `Global` has none: its commands are in every view's table).
    pub fn view_name(self) -> Option<&'static str> {
        match self {
            Scope::Global => None,
            Scope::Home => Some("home"),
            Scope::Folder => Some("folder-compare"),
            Scope::Text => Some("text-compare"),
            Scope::Sync => Some("folder-sync"),
            Scope::Table => Some("table-compare"),
            Scope::TextEdit => Some("text-edit"),
            Scope::Hex => Some("hex-compare"),
        }
    }

    /// The sections of the core catalogue where this view type's commands live, own section first.
    pub fn catalogue_sections(self) -> &'static [&'static str] {
        match self.view_name() {
            None => &["global"],
            Some(view) => hikasa_session::commands::view_sections(view),
        }
    }

    /// The help topic of this view type (the core catalogue's section topic; F1, UI-006).
    pub fn help_topic(self) -> &'static str {
        if self == Scope::TextEdit {
            // The catalogue has no section of its own for Text Edit (its commands are the text views').
            return "view-text-edit";
        }
        let key = self.catalogue_sections()[0];
        catalogue()
            .section(key)
            .map_or(hikasa_session::help::INDEX_TOPIC, |s| {
                hikasa_session::help::topic(&s.help)
                    .map_or(hikasa_session::help::INDEX_TOPIC, |t| t.id)
            })
    }
}

macro_rules! command_ids {
    ($($(#[$doc:meta])* $id:ident),* $(,)?) => {
        /// Stable command ids (the ids of the core catalogue, `docs/reference/commands.md`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum CommandId {
            $($(#[$doc])* $id,)*
        }

        impl CommandId {
            /// Every id.
            pub const ALL: &'static [CommandId] = &[$(CommandId::$id,)*];

            /// The id as written in the catalogue (`DiffNext`, `FavCompareAll`, …).
            pub fn name(self) -> &'static str {
                match self {
                    $(CommandId::$id => stringify!($id),)*
                }
            }

            /// The id of a catalogue name the GUI knows.
            pub fn from_name(name: &str) -> Option<CommandId> {
                CommandId::ALL.iter().copied().find(|id| id.name() == name)
            }
        }
    };
}

command_ids! {
    /// Opens a Home view in a new tab.
    TabNew,
    /// Opens a Home view in a new window (M2 W4a).
    WindowNew,
    /// Closes the active tab (its view).
    CloseTab,
    /// Opens the active tab's session again in a new tab (M2 W4a, D-107).
    TabDuplicate,
    /// Moves the tab into a new window (D-107).
    TabMoveToNewWindow,
    /// Moves the tab into another window (D-107).
    TabMoveToWindow,
    /// Closes the other tabs of the window (D-107).
    TabCloseOthers,
    /// Quits the application.
    Exit,
    /// Selects the next tab.
    TabNext,
    /// Selects the previous tab.
    TabPrev,
    /// Program information (a dialog, D-112).
    About,
    /// The Options dialog (D-128).
    ProgramOptions,
    /// Tools ▸ File Formats… (M2 W11c, D-318).
    FileFormatManager,
    /// The Export wizard (D-131).
    Export,
    /// The Import wizard (D-131).
    Import,
    /// The Restore wizard (D-131).
    Restore,
    /// The Run Script dialog (M2 W8b, D-229).
    RunScript,
    /// Help: the contents topic.
    HelpContents,
    /// Help: the topic of the active view (F1).
    ContextHelp,
    /// Home: compares the two typed paths.
    HomeCompare,
    /// Folder: terminates the comparison.
    Stop,
    /// Folder: refreshes the entire comparison.
    FullRefresh,
    /// Text: reloads and recompares the files.
    Reload,
    /// Folder: opens the selected folder.
    Expand,
    /// Folder: closes the selected folder.
    Collapse,
    /// Folder: opens the selected folder and all its subfolders.
    ExpandSubfolders,
    /// Folder: closes the selected folder and its subfolders.
    CollapseSubfolders,
    /// Folder: opens all folders.
    ExpandAll,
    /// Folder: closes all folders.
    CollapseAll,
    /// Folder: opens the selected file pair in a Text Compare view (FC-062).
    Open,
    /// Folder: copies the selected row from left to right (M2 W6d, FC-070).
    CopyRight,
    /// Folder: copies the selected row from right to left (M2 W6d, FC-070).
    CopyLeft,
    /// Folder: deletes the selected row on the sides the Delete dialog names (M2 W6d, FC-073).
    Delete,
    /// Folder: deletes the selected row without the recycle bin (M2 W6d, FC-073).
    PermanentDelete,
    /// Folder (M2 W9a): selects every shown file row on every side it has.
    SelectAllFiles,
    /// Folder (M2 W9a): the Select Newer submenu.
    SelectNewerList,
    /// Folder (M2 W9a): the Select Orphans submenu.
    SelectOrphanList,
    /// Folder (M2 W9a): selects the files newer on the left.
    SelectNewerLeft,
    /// Folder (M2 W9a): selects the files newer on the right.
    SelectNewerRight,
    /// Folder (M2 W9a): selects the newer files of both sides.
    SelectNewerBoth,
    /// Folder (M2 W9a): selects the left orphans.
    SelectOrphanLeft,
    /// Folder (M2 W9a): selects the right orphans.
    SelectOrphanRight,
    /// Folder (M2 W9a): selects the orphans of both sides.
    SelectOrphanBoth,
    /// Folder (M2 W9a): inverts the selection over the shown rows.
    SelectInvert,
    /// Folder (M2 W9a): the next shown row present on the focus side.
    ExistsNext,
    /// Folder (M2 W9a): the previous shown row present on the focus side.
    ExistsPrev,
    /// Folder (M2 W9a): Ignore Folder Structure (FC-014).
    DirFlatten,
    /// Folder (M2 W9a): shows or hides the column of the argument (`CommandArg::Column`, D-234).
    ColList,
    /// Folder (M2 W9a): shows or hides the Legend panel (FC-030, D-242).
    Legend,
    /// Folder (M2 W9a): the Info dialog of the comparison's statistics (FC-064, D-242).
    Info,
    /// Folder (M2 W9a): the Up One Level submenu (FC-004).
    UpOneLevelList,
    /// Folder (M2 W9a): the left base becomes its parent.
    UpOneLevelLeft,
    /// Folder (M2 W9a): the right base becomes its parent.
    UpOneLevelRight,
    /// Folder (M2 W9a): both bases become their parents.
    UpOneLevelBoth,
    /// Folder (M2 W9a): the selected folders become the bases.
    SetAsBaseFolders,
    /// Folder (M2 W9a): the selected folder becomes the other side's base.
    SetAsOtherBaseFolder,
    /// Folder and Text (M2 W9a): a new Folder Compare of the folders holding the row's (the files') items.
    CompareParents,
    /// Folder and Text (M2 W9a): exchanges the sides (SES-008).
    Swap,
    /// Folder (M2 W9a): the view becomes an empty session of its type.
    SessionClear,
    /// Folder (M2 W9a): the Browse submenu (D-240).
    BrowseList,
    /// Folder (M2 W9a): the browse dialog for the left base.
    BrowseLeft,
    /// Folder (M2 W9a): the browse dialog for the right base.
    BrowseRight,
    /// Folder (M2 W9a): opens the Find Filename panel (FC-061, D-241).
    FindFilename,
    /// Folder (M2 W9a): the next row whose name matches the last query.
    FindFilenameNext,
    /// Folder (M2 W9a): the previous row whose name matches the last query.
    FindFilenamePrev,
    /// Folder (M2 W9b): asks the side, then copies the selection there (FC-070).
    CopyPrompt,
    /// Folder (M2 W9b): copies the selection to the side opposite the pane (`CommandArg::Side`).
    CopySide,
    /// Folder (M2 W9b): moves the selection from left to right (FC-071).
    MoveRight,
    /// Folder (M2 W9b): moves the selection from right to left.
    MoveLeft,
    /// Folder (M2 W9b): asks the side, then moves the selection there.
    MovePrompt,
    /// Folder (M2 W9b): moves the selection to the side opposite the pane.
    MoveSide,
    /// Folder (M2 W9b): exchanges the selected items of both sides (Exchange).
    MoveBoth,
    /// Folder (M2 W9b): the Copy to Folder dialog (FC-072).
    CopyTo,
    /// Folder (M2 W9b): the Move to Folder dialog.
    MoveTo,
    /// Folder (M2 W9b): renames one row inline, several in the Rename dialog (FC-074).
    Rename,
    /// Folder (M2 W9b): the Touch dialog (FC-076).
    Touch,
    /// Folder (M2 W9b): the New Folder dialog (FC-077).
    NewFolder,
    /// Folder (M2 W9b): marks or unmarks the selection as Ignored (FC-033; check).
    Ignored,
    /// Folder and Sync (M2 W9b): the Exclude dialog (FC-052).
    Exclude,
    /// Folder (M2 W9b): the Compare Contents dialog (FC-029 b).
    Compare,
    /// Folder (M2 W9b): the Quick Compare dialog of the selected pairs.
    QuickCompare,
    /// Folder (M2 W9b): compares two items, or one and the mark, or one and a browsed one (FC-062).
    CompareTo,
    /// Folder and Sync (M2 W9b): the Open With submenu (`CommandArg::OpenWith`).
    OpenWithList,
    /// Folder and Sync (M2 W9b): opens each selected item in its associated application.
    AssocApp,
    /// Folder (M2 W9b): the Synchronize submenu (FC-078).
    SyncList,
    /// Folder (M2 W9b): the Synchronize preview of Update Right.
    UpdateRight,
    /// Folder (M2 W9b): Update Left.
    UpdateLeft,
    /// Folder (M2 W9b): Update Both.
    UpdateBoth,
    /// Folder (M2 W9b): Mirror to Right.
    MirrorRight,
    /// Folder (M2 W9b): Mirror to Left.
    MirrorLeft,
    /// Folder (M2 W9b): a new Folder Sync view of the same bases (FC-004).
    SyncBaseView,
    /// Sync (M2 W9b): a new Folder Compare view of the same bases.
    CompareBaseView,
    /// Sync (M2 W9b): runs the planned sync (Run Sync).
    Run,
    /// Sync (M2 W9b): the selected rows are left alone (an override, FS-004).
    SyncLeave,
    /// Sync (M2 W9b): the selected rows are copied right to left.
    SyncCopyLeft,
    /// Sync (M2 W9b): the selected rows are copied left to right.
    SyncCopyRight,
    /// Sync (M2 W9b): the selected rows are deleted on the left.
    SyncDeleteLeft,
    /// Sync (M2 W9b): the selected rows are deleted on the right.
    SyncDeleteRight,
    /// Sync (M2 W9b): the next conflict.
    ConflictNext,
    /// Sync (M2 W9b): the previous conflict.
    ConflictPrev,
    /// Folder: next difference.
    DiffNext,
    /// Folder: previous difference.
    DiffPrev,
    /// Text: next difference section.
    DiffSectionNext,
    /// Text: previous difference section.
    DiffSectionPrev,
    /// Display filter: everything.
    FavCompareAll,
    /// Display filter: differences.
    FavDiff,
    /// Display filter: same items.
    FavSame,
    /// Display filter: no orphans.
    FavNoOrphan,
    /// Display filter: differences but no orphans.
    FavDiffNoOrphan,
    /// Display filter: orphans.
    FavOrphan,
    /// Display filter: left newer.
    FavLeftNewer,
    /// Display filter: right newer.
    FavRightNewer,
    /// Display filter: left newer and left orphans.
    FavLeftNewerOrphan,
    /// Display filter: right newer and right orphans.
    FavRightNewerOrphan,
    /// Display filter: left orphans.
    FavLeftOrphan,
    /// Display filter: right orphans.
    FavRightOrphan,
    /// Display filter: nothing.
    FavCompareNone,
    /// Text display filter: differences with context lines.
    ShowContext,
    /// Toggle: same items.
    TogSame,
    /// Toggle: left orphans.
    TogLeftOrphan,
    /// Toggle: right orphans.
    TogRightOrphan,
    /// Toggle: left newer.
    TogLeftNewer,
    /// Toggle: right newer.
    TogRightNewer,
    /// Toggle: differences.
    TogDiff,
    /// Folder mode: always show folders.
    DirShowAll,
    /// Folder mode: compare files and folder structure.
    DirCompareStructure,
    /// Folder mode: only compare files.
    DirCompareFilesOnly,
    /// Treats unimportant differences as same.
    InsigIgnored,
    /// Folder: temporarily shows filtered items.
    SuppressFilters,
    /// Text: shows or hides the thumbnail.
    ShowThumbnail,
    /// Text: larger font.
    DisplayFontIncrease,
    /// Text: smaller font.
    DisplayFontDecrease,
    /// Text: default font size.
    DisplayFontReset,
    /// Shows or hides the view's toolbar (M2 W4a, `[layout] toolbar`).
    ShowToolbar,
    /// Folder: shows or hides the log panel (M2 W4a, FC-100).
    ShowLog,
    /// A Home tab with the picked session type selected (M2 W4b).
    SessionNewList,
    /// Opens Home in a new tab.
    SessionOpen,
    /// Loads a workspace (the argument; D-125).
    WorkspaceLoadList,
    /// The Manage Workspaces dialog.
    WorkspaceManager,
    /// The Save Workspace As dialog.
    WorkspaceSaveAs,
    /// Replaces the view by Home after its close action.
    SessionHome,
    /// Opens the picked named session of the view's type in this tab.
    SessionsCombo,
    /// Saves the view's session (D-120).
    SessionSave,
    /// The Save Session As dialog.
    SessionSaveAs,
    /// The Session Settings dialog of the view (D-122).
    SessionEdit,
    /// Locks or unlocks the view's named session.
    SessionLocked,
    /// Home: shows or hides the management panel.
    ShowManage,
    /// Home: maximizes or restores the management panel.
    MaximizeManage,
    /// Home: focuses the search box.
    Find,
    /// Sessions tree: opens the selection.
    TreeOpen,
    /// Sessions tree: opens the selection as a new unnamed session.
    TreeOpenAsNew,
    /// Sessions tree: opens every session directly in the folder.
    TreeOpenAll,
    /// Sessions tree: a new session.
    TreeNewSession,
    /// Sessions tree: a new folder.
    TreeNewFolder,
    /// Sessions tree: resets a named session to the defaults.
    TreeClear,
    /// Sessions tree: deletes the selection.
    TreeDelete,
    /// Sessions tree: saves a copy of the selection.
    TreeSaveAs,
    /// Sessions tree: renames the selection in place.
    TreeRename,
    /// Sessions tree: the Session Settings dialog of a session or a type's defaults.
    TreeEdit,
    /// Sessions tree: opens every folder.
    TreeExpandAll,
    /// Sessions tree: closes every folder.
    TreeCollapseAll,
    /// Sessions tree: toggles the favorite mark.
    TreeFavorite,
    /// The selected items as files on the clipboard (M2 W9c, D-252).
    Copy,
    /// The clipboard's files dropped on the pane's folder (D-252).
    Paste,
    /// Selects every shown row (D-238, D-252).
    SelectAll,
    /// The selected items' specs as text (FC-063).
    CopyFilenames,
    /// The file-manager entries (FC-063, D-254).
    ExplorerList,
    /// Session Settings on the Comparison tab (SES-020, D-249).
    Rules,
    /// Session Settings on the Name Filters tab (FC-050).
    NameFilter,
    /// Focuses the toolbar's Filters edit (D-250).
    NameFilterEdit,
    /// Undoes the focus pane's last edit (M2 W11a).
    Undo,
    /// Redoes the focus pane's last undone edit.
    Redo,
    /// Copies the selection (or the caret's line) and deletes it.
    Cut,
    /// Saves the focus pane's file (D-300).
    Save,
    /// Saves every modified file of the view as one save.
    SaveAll,
    /// Saves the focus pane's file under a new name.
    SaveAs,
    /// Save As without archives or non-local places.
    SaveAsLocal,
    /// Shows or hides the File Info header (D-302).
    ShowSpecInfo,
    /// The pane's format (FF-003; the header's picker).
    FileFormat,
    /// Word wrap of the text panes (D-295).
    WordWrap,
    /// Overwrite mode of the text panes (D-296).
    Overwriting,
    /// Deletes the caret's line or the selected lines.
    DeleteItem,
    /// Deletes to the start of the line.
    DeleteItemBegin,
    /// Deletes to the end of the line.
    DeleteItemEnd,
    /// Deletes the word at the caret.
    DeleteWord,
    /// Deletes to the start of the word.
    DeleteWordBegin,
    /// Deletes to the end of the word.
    DeleteWordEnd,
    /// Inserts a line before the caret's line.
    InsertItemBefore,
    /// Inserts a line after the caret's line.
    InsertItemAfter,
    /// Indents the selected lines.
    IncreaseIndent,
    /// Unindents the selected lines.
    DecreaseIndent,
    /// Selects the lines of the current section.
    SelectSection,
    /// Copies the current row to the right pane.
    CopyItemRight,
    /// Copies the current row to the left pane.
    CopyItemLeft,
    /// Copies the current row to the pane without focus.
    CopyItemSide,
    /// Table Compare (phase 3): the next difference row (TB-022).
    DiffItemNext,
    /// Table Compare: the previous difference row.
    DiffItemPrev,
    /// Table Compare: the next sheet with differences.
    DiffSheetNext,
    /// Table Compare: the previous sheet with differences.
    DiffSheetPrev,
    /// Table Compare: Go To row, column and sheet (Ctrl+G).
    GoTo,
    /// Table Compare: Hide Same Columns (TB-020).
    ColHideSame,
    /// Table Compare: Resize Columns to Fit.
    ColDetectSize,
    /// Table Compare: the Text Details panel.
    ShowDetails,
    /// File views: the side-by-side layout (Table Compare).
    SideBySide,
    /// File views: the over-under layout (Table Compare).
    OverUnder,
    /// `<Type> Report...` (Table Compare: the table report, T-56).
    Report,
    /// Syntax colours of the text panes (M2 W11b, D-305).
    ShowSyntaxHighlighting,
    /// Visible whitespace.
    ShowWS,
    /// The focus pane alone fills the view (F11).
    ZoomPane,
    /// The splitter moves 5 % to the right (the left pane wider).
    ZoomLeft,
    /// The splitter moves 5 % to the left (the right pane wider).
    ZoomRight,
    /// The Display Font dialog.
    DisplayFont,
    /// The Text Details panel (D-306).
    ShowTextDetails,
    /// The Hex Details panel.
    ShowHexDetails,
    /// The Alignment Details panel.
    ShowAlignDetails,
    /// The ruler above the panes.
    ShowRuler,
    /// The next hit of the find panel's query (D-307).
    FindNext,
    /// The previous hit of the find panel's query.
    FindPrev,
    /// The Replace panel.
    Replace,
    /// The Find in Files dialog (D-308).
    FindInFiles,
    /// The next edited lines of the focus pane (D-309).
    EditNext,
    /// The previous edited lines of the focus pane.
    EditPrev,
    /// What the toolbar's arrows step by (Browse By).
    BrowseMode,
    /// Toggle Bookmark ▸.
    BookmarkToggleList,
    /// Go to Bookmark ▸.
    BookmarkGoToList,
    /// Toggles bookmark 0 at the caret's line.
    BookmarkToggle0,
    /// Toggles bookmark 1.
    BookmarkToggle1,
    /// Toggles bookmark 2.
    BookmarkToggle2,
    /// Toggles bookmark 3.
    BookmarkToggle3,
    /// Toggles bookmark 4.
    BookmarkToggle4,
    /// Toggles bookmark 5.
    BookmarkToggle5,
    /// Toggles bookmark 6.
    BookmarkToggle6,
    /// Toggles bookmark 7.
    BookmarkToggle7,
    /// Toggles bookmark 8.
    BookmarkToggle8,
    /// Toggles bookmark 9.
    BookmarkToggle9,
    /// Goes to bookmark 0.
    BookmarkGoTo0,
    /// Goes to bookmark 1.
    BookmarkGoTo1,
    /// Goes to bookmark 2.
    BookmarkGoTo2,
    /// Goes to bookmark 3.
    BookmarkGoTo3,
    /// Goes to bookmark 4.
    BookmarkGoTo4,
    /// Goes to bookmark 5.
    BookmarkGoTo5,
    /// Goes to bookmark 6.
    BookmarkGoTo6,
    /// Goes to bookmark 7.
    BookmarkGoTo7,
    /// Goes to bookmark 8.
    BookmarkGoTo8,
    /// Goes to bookmark 9.
    BookmarkGoTo9,
    /// Clears the focus pane's bookmarks.
    BookmarkClear,
    /// The left pane takes the keyboard.
    GoLeft,
    /// The right pane takes the keyboard.
    GoRight,
    /// The next pane takes the keyboard.
    GoNext,
    /// The previous pane takes the keyboard.
    GoPrev,
    /// Align With (an alignment override, D-310).
    AlignWith,
    /// Isolates the selected lines.
    Isolate,
    /// Clears the alignment overrides.
    AnchorsClear,
    /// Convert File ▸.
    ConvertList,
    /// Trims trailing whitespace.
    TrimTrailWS,
    /// Leading spaces to tabs.
    LeadSpaceToTab,
    /// Tabs to spaces.
    TabToSpace,
    /// Line Endings ▸.
    EndingStyleList,
    /// CRLF line endings.
    ToWindows,
    /// LF line endings.
    ToUnix,
    /// CR line endings.
    ToMac,
    /// Compares the selection to the clipboard's text.
    CompareToClipboard,
    /// Reopens the focus pane with the clipboard's text.
    OpenClipboard,
    /// Open File on File System (the focus pane).
    OpenLocal,
    /// The next difference files of the parent folder comparison (D-311).
    DiffFileNext,
    /// The previous difference files.
    DiffFilePrev,
    /// Copies the file to the right and opens the next difference files.
    CopyFileRight,
    /// Copies the file to the left and opens the next difference files.
    CopyFileLeft,
    /// Tools ▸ Edit Text File (a Text Edit view, D-313).
    EditTextFile,
    /// Hex Compare: the little endian values submenu (M2 W13a, D-360).
    LittleEndianList,
    /// Hex Compare: the big endian values submenu.
    BigEndianList,
    /// Hex Compare: the Layout dialog (D-356).
    Layout,
    /// Hex Compare: Edit ▸ Current Byte Address (HX-007, D-357).
    CurrentAddress,
    /// Hex Compare: Edit ▸ Copy as Text (D-357).
    CopyAsText,
    /// Hex Compare: View ▸ Alignment (None, Fast, Complete; D-355).
    AlignModeList,
}

/// What the active view offers, for [`Predicate::holds`] (computed by the app from the window and its view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ctx {
    /// The active view's type.
    pub scope: Scope,
    /// Open tabs of the window.
    pub tabs: usize,
    /// Open windows.
    pub windows: usize,
    /// A dialog of the window is open (the window's commands take no input, D-112).
    pub dialog_open: bool,
    /// A comparison of the active view is running.
    pub busy: bool,
    /// The active view has a model to act on (a folder model, a finished text comparison).
    pub ready: bool,
    /// A folder row is selected and it is closed.
    pub closed_folder_selected: bool,
    /// A folder row is selected and it is open.
    pub open_folder_selected: bool,
    /// A file pair (both sides present) is selected.
    pub pair_selected: bool,
    /// Folder (M2 W6d): one row is selected in a finished comparison, and no job of the view and no operation runs.
    pub row_selected: bool,
    /// Folder (M2 W6d): the selected row has an item on the left, on the right.
    pub row_on: [bool; 2],
    /// Folder (M2 W6d, FC-003): the view may write the left side, the right side (Disable editing, `--ro`).
    pub writable: [bool; 2],
    /// A difference exists after the current position.
    pub next_difference: bool,
    /// A difference exists before the current position.
    pub prev_difference: bool,
    /// Home: both paths are filled in.
    pub home_ready: bool,
    /// The active view's session is a named one.
    pub named: bool,
    /// Sessions tree: a named session is selected.
    pub tree_named: bool,
    /// Sessions tree: a locked named session is selected.
    pub tree_locked: bool,
    /// Sessions tree: an auto-saved session is selected.
    pub tree_auto: bool,
    /// Sessions tree: a folder is selected.
    pub tree_folder: bool,
    /// Sessions tree: a *New* type is selected.
    pub tree_new: bool,
    /// Folder (M2 W9a): the base of each side has a parent folder (`UpOneLevel*`).
    pub base_parent: [bool; 2],
    /// Folder: the selection names two base folders (`SetAsBaseFolders`).
    pub bases_selected: bool,
    /// Folder: one folder is selected on one side (`SetAsOtherBaseFolder`).
    pub other_base_selected: bool,
    /// Folder: one row is selected (`CompareParents`); a text view: always.
    pub parents_available: bool,
    /// Folder (M2 W9a after review, §1.9.2 rule 4): no operation, refresh or plan of the view runs and no question of
    /// one waits (`Predicate::Idle`); the base commands re-root the view and wait for it.
    pub idle: bool,
    /// Folder (M2 W9b): the comparison is flat (*Ignore Folder Structure*): no sync (`NotFlat`).
    pub flat: bool,
    /// Folder: two items are selected, or one and the Space mark (`TwoItems`, Compare To).
    pub two_items: bool,
    /// Sync (M2 W9b): a plan is ready and has an action (`PlanReady`, Run Sync).
    pub plan_ready: bool,
    /// Sync: a shown row is a conflict (`HasConflict`).
    pub has_conflict: bool,
    /// Folder: the selection is two folders (Open With ▸ *Folder Compare*).
    pub two_folders: bool,
    /// Folder (M2 W9c, D-252): a folder or sync view is active and no text field of the window has the keyboard
    /// (`FolderPaneFocused`).
    pub folder_pane: bool,
    /// Folder (D-254): the current item on the left, on the right is on a local disk or in an archive there (the
    /// file-manager entries).
    pub explorer: [bool; 2],
    /// Folder (M2 W9c after review F10): the side a pane's context menu was opened on; `CopySide`, `MoveSide`,
    /// `Paste` and `NewFolder` act on it.
    pub clicked: Option<Side>,
    /// Folder: the focus side when one side only is focused (what `CopySide`, `MoveSide` and `Paste` act on without a
    /// clicked side). A text view (M2 W11a): the pane with the keyboard.
    pub focus_side: Option<Side>,
    /// Text (M2 W11a, §1.11.28): a text pane has the keyboard (no text field of the window has it).
    pub text_pane: bool,
    /// Text: the view shows two panes (Text Compare).
    pub two_panes: bool,
    /// Text: the left, the right pane may be edited (a pane whose only block is `Binary` counts: it asks).
    pub pane_editable: [bool; 2],
    /// Text: the focus pane has a selection.
    pub has_selection: bool,
    /// Text: the focus pane can undo, redo.
    pub can_undo: bool,
    /// Text: the focus pane can redo.
    pub can_redo: bool,
    /// Text: the focus pane is modified.
    pub modified: bool,
    /// Text: a pane of the view is modified.
    pub any_modified: bool,
    /// Text: a save of the view runs.
    pub saving: bool,
    /// Text: no provisional rows (the last recompare is installed).
    pub installed: bool,
    /// Text: the live comparison has alignment overrides.
    pub has_anchors: bool,
    /// Text (M2 W11b): the view carries a parent link (D-311).
    pub child_view: bool,
    /// Text: a step of the browse mode exists forward, backward (the toolbar arrows, D-309).
    pub diff_available: [bool; 2],
    /// Text: the browse mode (the radio of `BrowseMode`).
    pub browse_mode: Option<hikasa_text::nav::BrowseMode>,
    /// Hex: an alignment landed (M2 W13a, D-371).
    pub hex_landed: bool,
    /// Hex: the documents are open, a selection exists (one column at least).
    pub byte_selection: bool,
    /// Hex: the requested alignment mode (the radio of `AlignModeList`).
    pub hex_mode: Option<hikasa_hex::AlignMode>,
    /// Hex: the value panel's blocks and radix (the checks of the values submenus).
    pub hex_values: Option<(hikasa_session::api::HexValues, hikasa_hex::Radix)>,
}

impl Ctx {
    /// The index of the side `CopySide`, `MoveSide` and `Paste` act on: the clicked one, else the focus side.
    fn acting_side(&self) -> Option<usize> {
        self.clicked.or(self.focus_side).and_then(side_index)
    }
}

/// The submenus of the catalogue's `…List` entries (M2 W9a): a list's children show in its submenu, not in the menu.
pub const LIST_CHILDREN: &[(CommandId, &[CommandId])] = &[
    (
        CommandId::BrowseList,
        &[CommandId::BrowseLeft, CommandId::BrowseRight],
    ),
    (
        CommandId::UpOneLevelList,
        &[
            CommandId::UpOneLevelLeft,
            CommandId::UpOneLevelRight,
            CommandId::UpOneLevelBoth,
        ],
    ),
    (
        CommandId::SyncList,
        &[
            CommandId::UpdateRight,
            CommandId::UpdateLeft,
            CommandId::UpdateBoth,
            CommandId::MirrorRight,
            CommandId::MirrorLeft,
        ],
    ),
    (CommandId::OpenWithList, &[CommandId::AssocApp]),
    // M2 W11b (§1.11.19, §1.11.18): Convert File ▸, Line Endings ▸ and the bookmarks' lists.
    (
        CommandId::ConvertList,
        &[
            CommandId::TrimTrailWS,
            CommandId::LeadSpaceToTab,
            CommandId::TabToSpace,
        ],
    ),
    (
        CommandId::EndingStyleList,
        &[CommandId::ToWindows, CommandId::ToUnix, CommandId::ToMac],
    ),
    (
        CommandId::BookmarkToggleList,
        &[
            CommandId::BookmarkToggle0,
            CommandId::BookmarkToggle1,
            CommandId::BookmarkToggle2,
            CommandId::BookmarkToggle3,
            CommandId::BookmarkToggle4,
            CommandId::BookmarkToggle5,
            CommandId::BookmarkToggle6,
            CommandId::BookmarkToggle7,
            CommandId::BookmarkToggle8,
            CommandId::BookmarkToggle9,
        ],
    ),
    (
        CommandId::BookmarkGoToList,
        &[
            CommandId::BookmarkGoTo0,
            CommandId::BookmarkGoTo1,
            CommandId::BookmarkGoTo2,
            CommandId::BookmarkGoTo3,
            CommandId::BookmarkGoTo4,
            CommandId::BookmarkGoTo5,
            CommandId::BookmarkGoTo6,
            CommandId::BookmarkGoTo7,
            CommandId::BookmarkGoTo8,
            CommandId::BookmarkGoTo9,
        ],
    ),
    (
        CommandId::SelectNewerList,
        &[
            CommandId::SelectNewerLeft,
            CommandId::SelectNewerRight,
            CommandId::SelectNewerBoth,
        ],
    ),
    (
        CommandId::SelectOrphanList,
        &[
            CommandId::SelectOrphanLeft,
            CommandId::SelectOrphanRight,
            CommandId::SelectOrphanBoth,
        ],
    ),
];

/// The children of a `…List` entry (empty for other ids).
pub fn list_children(id: CommandId) -> &'static [CommandId] {
    LIST_CHILDREN
        .iter()
        .find(|(list, _)| *list == id)
        .map_or(&[], |(_, children)| children)
}

/// True when `id` shows in a `…List` entry's submenu.
pub fn in_a_list(id: CommandId) -> bool {
    LIST_CHILDREN
        .iter()
        .any(|(_, children)| children.contains(&id))
}

/// Availability predicates (ARCH-070 `enabled_when`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Predicate {
    /// Always available.
    Always,
    /// Never available: a session command whose row in this view waits for a later wave (§1.9.19: a text view's
    /// `Info` and `SessionClear` are W11's).
    Never,
    /// More than one tab is open in the window.
    MultipleTabs,
    /// Another window is open.
    OtherWindows,
    /// A comparison is running.
    Busy,
    /// The view has a model or result.
    Ready,
    /// A closed folder is selected.
    ClosedFolderSelected,
    /// An open folder is selected.
    OpenFolderSelected,
    /// A folder is selected.
    FolderSelected,
    /// A file pair is selected.
    PairSelected,
    /// A selected row can be written on this side from the other (M2 W6d): the side is writable and the row has an
    /// item on the other side (`CopyRight`, `CopyLeft`).
    SideWritable(Side),
    /// A selected row has an item on a writable side (M2 W6d: `Delete`, `PermanentDelete`).
    RowSelected,
    /// A next difference exists.
    NextDifference,
    /// A previous difference exists.
    PrevDifference,
    /// Home: both paths are given.
    HomeReady,
    /// The view's session is named.
    Named,
    /// Tree: a named session, an auto-saved one or a *New* type.
    TreeOpenable,
    /// Tree: a named or auto-saved session.
    TreeSession,
    /// Tree: a folder.
    TreeFolder,
    /// Tree: a named session that is not locked.
    TreeUnlocked,
    /// Tree: a named session, a folder or an auto-saved session.
    TreeDeletable,
    /// Tree: a named session or a folder.
    TreeRenamable,
    /// Tree: a named session or a *New* type (its defaults).
    TreeEditable,
    /// Folder (M2 W9a): the base of this side has a parent (`Center`: each side's has).
    BaseParent(Side),
    /// Folder: the selection names two base folders.
    BasesSelected,
    /// Folder: one folder is selected on one side.
    OtherBaseSelected,
    /// `CompareParents` can run.
    ParentsAvailable,
    /// Folder (§1.9.19): no operation, refresh or plan of the view runs (the base commands, Clear Session).
    Idle,
    /// Folder: the view has a model and is idle (`Swap`).
    ReadyIdle,
    /// Folder (M2 W9b, §1.9.19): at least one item is selected in a finished comparison, nothing runs.
    Selection,
    /// A selected item can be moved to this side: both sides writable, items on the other side.
    Movable(Side),
    /// A selection on both sides, both writable (Exchange).
    BothWritable,
    /// A writable side and a finished comparison, nothing running (New Folder: no selection needed); from a pane's
    /// context menu, that pane's side writable (review F10).
    AnyWritable,
    /// A selection in a comparison with its folder structure (the Synchronize submenu).
    NotFlat,
    /// A finished comparison and nothing running (Sync Base Folders, Compare Base Folders).
    Loaded,
    /// Sync: a plan with an action is ready and nothing runs (Run Sync).
    PlanReady,
    /// Sync: a conflict is shown (Next/Previous Conflict).
    HasConflict,
    /// M2 W9c (D-252): a folder or sync view's pane has the keyboard (no text field has it): `Paste`, `SelectAll`.
    FolderPaneFocused,
    /// `FolderPaneFocused` with at least one selected item (`Copy`; the items not on a local disk are left out when it
    /// runs, with a message).
    LocalSelection,
    /// M2 W9c after review F10 (`CopySide`): the side acted on (clicked, else the focus side) has selected items and
    /// its other side is writable.
    CopyFromSide,
    /// `MoveSide`: [`Predicate::CopyFromSide`] and the side acted on writable too.
    MoveFromSide,
    /// `Paste`: [`Predicate::FolderPaneFocused`], a finished comparison, nothing running, and the side acted on writable
    /// (with both sides focused and none clicked the command asks for a row on one side).
    PasteSide,
    /// Text (M2 W11a): a text pane has the keyboard and the comparison is there.
    PaneFocused,
    /// Text: this pane may be edited (`Center`: the focus pane).
    PaneEditable(Side),
    /// Text: the focus pane may be edited and has the keyboard.
    FocusEditable,
    /// Text: the focus pane has a selection.
    HasSelection,
    /// Text: the focus pane can undo (and may be edited).
    CanUndo,
    /// Text: the focus pane can redo (and may be edited).
    CanRedo,
    /// Text: two panes and the pane without focus may be edited (`CopySide`, `CopyItemSide`).
    TargetEditable,
    /// Text: the focus pane is modified and may be saved (`Save`).
    Modified,
    /// Text: a pane is modified and no save runs (`SaveAll`).
    AnyModified,
    /// Text: no save runs and the comparison is there (`SaveAs`).
    NotSaving,
    /// Text (M2 W11b): two panes (Text Compare).
    TwoPanes,
    /// Text: the view carries a parent link (D-311).
    ChildView,
    /// Text: a parent link and this side's pane may be written (`CopyFileRight`: the right; D-184).
    CopyFileTo(Side),
    /// Text: the browse mode has a step forward (`true`) or backward.
    DiffAvailable(bool),
    /// Text: the live comparison has alignment overrides.
    HasAnchors,
    /// Hex: an alignment landed (M2 W13a, D-371).
    HexLanded,
    /// Hex: the documents are open (a selection of one column at least exists).
    ByteSelection,
}

impl Predicate {
    /// Evaluates the predicate.
    pub fn holds(self, ctx: &Ctx) -> bool {
        match self {
            Predicate::Always => true,
            Predicate::Never => false,
            Predicate::MultipleTabs => ctx.tabs > 1,
            Predicate::OtherWindows => ctx.windows > 1,
            Predicate::Busy => ctx.busy,
            Predicate::Ready => ctx.ready,
            Predicate::ClosedFolderSelected => ctx.closed_folder_selected,
            Predicate::OpenFolderSelected => ctx.open_folder_selected,
            Predicate::FolderSelected => ctx.closed_folder_selected || ctx.open_folder_selected,
            Predicate::PairSelected => ctx.pair_selected,
            Predicate::SideWritable(side) => match side_index(side) {
                Some(k) => ctx.row_selected && ctx.writable[k] && ctx.row_on[1 - k],
                None => false,
            },
            Predicate::RowSelected => {
                ctx.row_selected && (0..2).any(|k| ctx.row_on[k] && ctx.writable[k])
            }
            Predicate::NextDifference => ctx.next_difference,
            Predicate::PrevDifference => ctx.prev_difference,
            Predicate::HomeReady => ctx.home_ready,
            Predicate::Named => ctx.named,
            Predicate::TreeOpenable => ctx.tree_named || ctx.tree_auto || ctx.tree_new,
            Predicate::TreeSession => ctx.tree_named || ctx.tree_auto,
            Predicate::TreeFolder => ctx.tree_folder,
            Predicate::TreeUnlocked => ctx.tree_named && !ctx.tree_locked,
            Predicate::TreeDeletable => ctx.tree_named || ctx.tree_auto || ctx.tree_folder,
            Predicate::TreeRenamable => ctx.tree_named || ctx.tree_folder,
            Predicate::TreeEditable => ctx.tree_named || ctx.tree_new,
            // The base commands re-root the view: never while an operation of it runs (§1.9.2 rule 4).
            Predicate::BaseParent(side) => {
                ctx.idle
                    && match side_index(side) {
                        Some(k) => ctx.base_parent[k],
                        None => ctx.base_parent[0] && ctx.base_parent[1],
                    }
            }
            Predicate::BasesSelected => ctx.idle && ctx.bases_selected,
            Predicate::OtherBaseSelected => ctx.idle && ctx.other_base_selected,
            Predicate::ParentsAvailable => ctx.parents_available,
            Predicate::Idle => ctx.idle,
            Predicate::ReadyIdle => ctx.ready && ctx.idle,
            Predicate::Selection => ctx.row_selected,
            Predicate::Movable(side) => match side_index(side) {
                Some(k) => {
                    ctx.row_selected && ctx.writable[k] && ctx.writable[1 - k] && ctx.row_on[1 - k]
                }
                None => false,
            },
            Predicate::BothWritable => {
                ctx.row_selected
                    && ctx.writable[0]
                    && ctx.writable[1]
                    && ctx.row_on[0]
                    && ctx.row_on[1]
            }
            Predicate::AnyWritable => {
                ctx.ready
                    && ctx.idle
                    && !ctx.busy
                    && match ctx.clicked.and_then(side_index) {
                        Some(k) => ctx.writable[k],
                        None => ctx.writable[0] || ctx.writable[1],
                    }
            }
            Predicate::NotFlat => ctx.ready && ctx.idle && !ctx.busy && !ctx.flat,
            Predicate::Loaded => ctx.ready && !ctx.busy,
            Predicate::PlanReady => ctx.plan_ready && ctx.idle,
            Predicate::HasConflict => ctx.has_conflict,
            Predicate::FolderPaneFocused => ctx.folder_pane,
            Predicate::LocalSelection => ctx.folder_pane && ctx.row_selected,
            Predicate::CopyFromSide => {
                ctx.row_selected
                    && ctx
                        .acting_side()
                        .is_some_and(|k| ctx.row_on[k] && ctx.writable[1 - k])
            }
            Predicate::MoveFromSide => {
                Predicate::CopyFromSide.holds(ctx)
                    && ctx.acting_side().is_some_and(|k| ctx.writable[k])
            }
            Predicate::PasteSide => {
                ctx.folder_pane
                    && ctx.ready
                    && ctx.idle
                    && !ctx.busy
                    && ctx.acting_side().is_none_or(|k| ctx.writable[k])
            }
            Predicate::PaneFocused => ctx.text_pane && ctx.ready,
            Predicate::PaneEditable(side) => {
                ctx.ready
                    && match side_index(side) {
                        Some(k) => ctx.pane_editable[k],
                        None => ctx
                            .focus_side
                            .and_then(side_index)
                            .is_some_and(|k| ctx.pane_editable[k]),
                    }
            }
            Predicate::FocusEditable => {
                ctx.text_pane && Predicate::PaneEditable(Side::Center).holds(ctx)
            }
            Predicate::HasSelection => ctx.text_pane && ctx.has_selection,
            Predicate::CanUndo => Predicate::FocusEditable.holds(ctx) && ctx.can_undo,
            Predicate::CanRedo => Predicate::FocusEditable.holds(ctx) && ctx.can_redo,
            Predicate::TargetEditable => {
                ctx.ready
                    && ctx.two_panes
                    && ctx
                        .focus_side
                        .and_then(side_index)
                        .is_some_and(|k| ctx.pane_editable[1 - k])
            }
            Predicate::Modified => ctx.ready && ctx.modified && !ctx.saving,
            Predicate::AnyModified => ctx.ready && ctx.any_modified && !ctx.saving,
            Predicate::NotSaving => ctx.ready && !ctx.saving,
            Predicate::TwoPanes => ctx.ready && ctx.two_panes,
            Predicate::ChildView => ctx.ready && ctx.child_view,
            Predicate::CopyFileTo(side) => {
                ctx.ready
                    && ctx.child_view
                    && !ctx.saving
                    && side_index(side).is_some_and(|k| ctx.pane_editable[k])
            }
            Predicate::DiffAvailable(forward) => {
                ctx.ready && ctx.diff_available[usize::from(!forward)]
            }
            Predicate::HasAnchors => ctx.ready && ctx.has_anchors,
            Predicate::HexLanded => ctx.hex_landed,
            Predicate::ByteSelection => ctx.byte_selection,
        }
    }
}

/// The index of a folder side in [`Ctx::row_on`] and [`Ctx::writable`] (`None`: not a side of a folder view).
pub fn side_index(side: Side) -> Option<usize> {
    match side {
        Side::Left => Some(0),
        Side::Right => Some(1),
        Side::Center | Side::Output => None,
    }
}

/// Where a command appears in a context menu (D-110; W9+ add pane areas).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    /// The tab's context menu.
    Tab,
    /// The toolbar's context menu.
    Toolbar,
    /// The Home sessions tree (W4b).
    SessionTree,
    /// A folder view's column header (M2 W9a, D-234): the `ColList` items.
    FolderHeader,
    /// A folder view's pane of a side (M2 W9c, D-253): the list of [`pane_menu_ids`].
    FolderPane(Side),
    /// A folder view's centre column (D-253).
    FolderCentre,
    /// A Folder Sync view's pane of a side (D-253).
    SyncPane(Side),
    /// A text pane of a side (M2 W11b, D-312).
    TextPane(Side),
    /// The centre strip or a line-number gutter of a text view.
    TextGutter,
    /// The File Info header of a text pane.
    TextHeader(Side),
    /// A hex pane of a side (M2 W13a, D-360).
    HexPane(Side),
    /// The hex view's thumbnail strip.
    HexThumbnail,
    /// The hex view's value panel.
    HexValues,
    /// The File Info header of a hex (or picture) pane.
    FileHeader(Side),
}

/// D-253's normative lists of the pane context menus, groups in order (a separator between non-empty groups); an
/// entry that is not shown in the view (planned, no row) is left out, one that is not available is disabled.
pub fn pane_menu_ids(area: Area) -> &'static [&'static [CommandId]] {
    use CommandId as C;
    const PANE: &[&[CommandId]] = &[
        &[C::Open, C::OpenWithList, C::CompareTo, C::QuickCompare],
        &[C::CopySide, C::MoveSide, C::CopyTo, C::MoveTo, C::MoveBoth],
        &[
            C::Delete,
            C::PermanentDelete,
            C::Rename,
            C::Touch,
            C::NewFolder,
        ],
        &[C::Compare, C::Ignored, C::Exclude, C::SyncList],
        &[
            C::SetAsBaseFolders,
            C::SetAsOtherBaseFolder,
            C::CompareParents,
        ],
        &[C::Copy, C::Paste, C::CopyFilenames, C::ExplorerList],
        &[
            C::Expand,
            C::Collapse,
            C::ExpandSubfolders,
            C::CollapseSubfolders,
        ],
    ];
    const CENTRE: &[&[CommandId]] = &[
        &[C::Open, C::OpenWithList, C::CompareTo, C::QuickCompare],
        &[C::CopyTo, C::MoveTo, C::MoveBoth],
        &[
            C::Delete,
            C::PermanentDelete,
            C::Rename,
            C::Touch,
            C::NewFolder,
        ],
        &[C::Compare, C::Ignored, C::Exclude, C::SyncList],
        &[C::SetAsBaseFolders, C::CompareParents],
        &[C::Copy, C::CopyFilenames, C::ExplorerList],
        &[
            C::Expand,
            C::Collapse,
            C::ExpandSubfolders,
            C::CollapseSubfolders,
        ],
    ];
    const SYNC: &[&[CommandId]] = &[
        &[
            C::SyncLeave,
            C::SyncCopyRight,
            C::SyncCopyLeft,
            C::SyncDeleteLeft,
            C::SyncDeleteRight,
        ],
        &[C::Exclude],
        &[C::Open, C::OpenWithList],
        &[C::SetAsBaseFolders, C::SetAsOtherBaseFolder],
        &[C::Copy, C::CopyFilenames, C::ExplorerList],
        &[C::Expand, C::Collapse],
    ];
    const HEADER_IDS: &[&[CommandId]] = &[&[C::ColList]];
    // M2 W11b (D-312): the text views' lists.
    const TEXT_PANE: &[&[CommandId]] = &[
        &[C::Undo, C::Redo],
        &[C::Cut, C::Copy, C::Paste, C::Delete, C::SelectAll],
        &[C::CopySide, C::CopyItemSide, C::SelectSection],
        &[C::AlignWith, C::Isolate, C::Ignored],
        &[C::BookmarkToggleList, C::CompareToClipboard],
        &[C::ConvertList],
        &[C::Find, C::Replace, C::GoTo],
    ];
    const TEXT_GUTTER: &[&[CommandId]] = &[
        &[C::CopyLeft, C::CopyRight, C::CopyItemLeft, C::CopyItemRight],
        &[C::SelectSection, C::Ignored, C::AlignWith, C::Isolate],
        &[C::BookmarkToggleList],
    ];
    const TEXT_HEADER: &[&[CommandId]] = &[
        &[C::Save, C::SaveAs],
        &[
            C::OpenWithList,
            C::AssocApp,
            C::ExplorerList,
            C::CopyFilenames,
        ],
        &[C::EndingStyleList, C::FileFormat],
        &[C::Open],
    ];
    // M2 W13a (D-360): the hex view's lists.
    const HEX_PANE: &[&[CommandId]] = &[
        &[C::Copy, C::CopyAsText, C::CurrentAddress, C::SelectAll],
        &[C::Find, C::GoTo],
        &[C::DiffItemNext, C::DiffItemPrev],
        &[C::Layout, C::LittleEndianList, C::BigEndianList],
        &[C::Open, C::OpenWithList, C::ExplorerList],
    ];
    const HEX_THUMBNAIL: &[&[CommandId]] = &[&[C::ShowThumbnail]];
    const HEX_VALUES: &[&[CommandId]] = &[&[C::Copy], &[C::LittleEndianList, C::BigEndianList]];
    const FILE_HEADER: &[&[CommandId]] = &[
        &[C::Open, C::OpenLocal, C::OpenClipboard],
        &[C::OpenWithList, C::ExplorerList],
        &[C::FileFormat],
    ];
    match area {
        Area::HexPane(_) => HEX_PANE,
        Area::HexThumbnail => HEX_THUMBNAIL,
        Area::HexValues => HEX_VALUES,
        Area::FileHeader(_) => FILE_HEADER,
        Area::FolderPane(_) => PANE,
        Area::FolderCentre => CENTRE,
        Area::SyncPane(_) => SYNC,
        Area::FolderHeader => HEADER_IDS,
        Area::TextPane(_) => TEXT_PANE,
        Area::TextGutter => TEXT_GUTTER,
        Area::TextHeader(_) => TEXT_HEADER,
        Area::Tab | Area::Toolbar | Area::SessionTree => &[],
    }
}

/// The entries of a pane menu that act on the clicked side (D-253): they carry it as `CommandArg::Side`.
pub fn takes_side(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::CopySide
            | CommandId::MoveSide
            | CommandId::SetAsOtherBaseFolder
            | CommandId::Copy
            | CommandId::Paste
            | CommandId::CopyFilenames
            | CommandId::ExplorerList
            | CommandId::NewFolder
    )
}

/// A radio or check group of the presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// A plain command.
    None,
    /// A display filter preset (radio).
    Filter,
    /// A folder mode (radio).
    FolderMode,
    /// An atomic display toggle (check).
    Toggles,
    /// A check command (its state comes from the view).
    Check,
}

/// One presentation row: what the GUI adds to a catalogue entry (D-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Id.
    pub id: CommandId,
    /// View type (`Global`: every view).
    pub scope: Scope,
    /// Icon (toolbar, menus).
    pub icon: Option<IconId>,
    /// Short label under the icon with large buttons, and the caption of an entry the catalogue gives none.
    pub toolbar_label: Option<&'static str>,
    /// Radio or check group.
    pub group: Group,
    /// Availability.
    pub enabled_when: Predicate,
    /// Context-menu areas.
    pub areas: &'static [Area],
}

#[allow(clippy::too_many_arguments)] // one positional row per command
const fn row(
    id: CommandId,
    scope: Scope,
    icon: Option<IconId>,
    toolbar_label: Option<&'static str>,
    group: Group,
    enabled_when: Predicate,
    areas: &'static [Area],
) -> Row {
    Row {
        id,
        scope,
        icon,
        toolbar_label,
        group,
        enabled_when,
        areas,
    }
}

use Area as A;
use CommandId as C;
use Group as G;
use IconId as I;
use Predicate as P;
use Scope as S;

const NONE: &[Area] = &[];
const TAB: &[Area] = &[A::Tab];
const TREE: &[Area] = &[A::SessionTree];
const HEADER: &[Area] = &[A::FolderHeader];

/// The presentation table, one row per (scope, id). A view type's row wins over a global one.
pub const PRESENTATION: &[Row] = &[
    // Global
    // M2 W9c (D-252): the Edit entries act in a folder pane; the text views take them in W11.
    row(
        C::Copy,
        S::Global,
        None,
        None,
        G::None,
        P::LocalSelection,
        NONE,
    ),
    row(C::Paste, S::Global, None, None, G::None, P::PasteSide, NONE),
    // M2 W11b (D-313): Tools ▸ Edit Text File opens a Text Edit view from any view.
    row(
        C::EditTextFile,
        S::Global,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    // M2 W11a: Undo, Redo and Cut act in a text pane (other views: never available).
    row(C::Undo, S::Global, None, None, G::None, P::CanUndo, NONE),
    row(C::Redo, S::Global, None, None, G::None, P::CanRedo, NONE),
    row(
        C::Cut,
        S::Global,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::SelectAll,
        S::Global,
        None,
        None,
        G::None,
        P::FolderPaneFocused,
        NONE,
    ),
    row(
        C::TabNew,
        S::Global,
        Some(I::NewTab),
        Some("New Tab"),
        G::None,
        P::Always,
        TAB,
    ),
    row(
        C::WindowNew,
        S::Global,
        Some(I::NewWindow),
        Some("New Window"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::TabDuplicate,
        S::Global,
        None,
        None,
        G::None,
        P::Always,
        TAB,
    ),
    row(
        C::TabMoveToNewWindow,
        S::Global,
        Some(I::NewWindow),
        None,
        G::None,
        P::MultipleTabs,
        TAB,
    ),
    row(
        C::TabMoveToWindow,
        S::Global,
        None,
        None,
        G::None,
        P::OtherWindows,
        TAB,
    ),
    row(
        C::TabCloseOthers,
        S::Global,
        None,
        None,
        G::None,
        P::MultipleTabs,
        TAB,
    ),
    row(
        C::CloseTab,
        S::Global,
        Some(I::Close),
        Some("Close"),
        G::None,
        P::Always,
        TAB,
    ),
    row(C::Exit, S::Global, None, None, G::None, P::Always, NONE),
    row(
        C::TabNext,
        S::Global,
        None,
        None,
        G::None,
        P::MultipleTabs,
        NONE,
    ),
    row(
        C::TabPrev,
        S::Global,
        None,
        None,
        G::None,
        P::MultipleTabs,
        NONE,
    ),
    row(
        C::HelpContents,
        S::Global,
        Some(I::Help),
        Some("Help"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::ContextHelp,
        S::Global,
        Some(I::Help),
        Some("Help"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::ProgramOptions,
        S::Global,
        Some(I::Options),
        Some("Options"),
        G::None,
        P::Always,
        NONE,
    ),
    // M2 W11c (§1.11.26): Tools ▸ File Formats….
    row(
        C::FileFormatManager,
        S::Global,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Export,
        S::Global,
        Some(I::Export),
        Some("Export"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Import,
        S::Global,
        Some(I::Import),
        Some("Import"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Restore,
        S::Global,
        Some(I::Restore),
        Some("Restore"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::RunScript,
        S::Global,
        None,
        Some("Run Script"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::About,
        S::Global,
        Some(I::Info),
        Some("About"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionNewList,
        S::Global,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionOpen,
        S::Global,
        Some(I::Home),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::WorkspaceLoadList,
        S::Global,
        Some(I::Workspace),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::WorkspaceManager,
        S::Global,
        Some(I::Workspace),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::WorkspaceSaveAs,
        S::Global,
        Some(I::SaveAs),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    // Home
    row(
        C::ShowManage,
        S::Home,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::MaximizeManage,
        S::Home,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::Find,
        S::Home,
        Some(I::Search),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::TreeOpen,
        S::Home,
        Some(I::Open),
        None,
        G::None,
        P::TreeOpenable,
        TREE,
    ),
    row(
        C::TreeOpenAsNew,
        S::Home,
        None,
        None,
        G::None,
        P::TreeSession,
        TREE,
    ),
    row(
        C::TreeOpenAll,
        S::Home,
        None,
        None,
        G::None,
        P::TreeFolder,
        TREE,
    ),
    row(
        C::TreeNewSession,
        S::Home,
        None,
        None,
        G::None,
        P::Always,
        TREE,
    ),
    row(
        C::TreeNewFolder,
        S::Home,
        Some(I::Folder),
        None,
        G::None,
        P::Always,
        TREE,
    ),
    row(
        C::TreeClear,
        S::Home,
        None,
        None,
        G::None,
        P::TreeUnlocked,
        TREE,
    ),
    row(
        C::TreeDelete,
        S::Home,
        None,
        None,
        G::None,
        P::TreeDeletable,
        TREE,
    ),
    row(
        C::TreeSaveAs,
        S::Home,
        Some(I::SaveAs),
        None,
        G::None,
        P::TreeSession,
        TREE,
    ),
    row(
        C::TreeRename,
        S::Home,
        None,
        None,
        G::None,
        P::TreeRenamable,
        TREE,
    ),
    row(
        C::TreeEdit,
        S::Home,
        Some(I::Settings),
        None,
        G::None,
        P::TreeEditable,
        TREE,
    ),
    row(
        C::TreeExpandAll,
        S::Home,
        Some(I::ExpandAll),
        None,
        G::None,
        P::Always,
        TREE,
    ),
    row(
        C::TreeCollapseAll,
        S::Home,
        Some(I::CollapseAll),
        None,
        G::None,
        P::Always,
        TREE,
    ),
    row(
        C::TreeFavorite,
        S::Home,
        Some(I::Favorite),
        None,
        G::None,
        P::TreeUnlocked,
        TREE,
    ),
    row(
        C::HomeCompare,
        S::Home,
        Some(I::OpenPair),
        Some("Compare"),
        G::None,
        P::HomeReady,
        NONE,
    ),
    // Folder Compare
    row(
        C::Stop,
        S::Folder,
        Some(I::Stop),
        Some("Stop"),
        G::None,
        P::Busy,
        NONE,
    ),
    row(
        C::FullRefresh,
        S::Folder,
        Some(I::Reload),
        Some("Refresh"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::ExpandAll,
        S::Folder,
        Some(I::ExpandAll),
        Some("Expand"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::CollapseAll,
        S::Folder,
        Some(I::CollapseAll),
        Some("Collapse"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::Open,
        S::Folder,
        Some(I::OpenPair),
        Some("Open"),
        G::None,
        P::PairSelected,
        NONE,
    ),
    row(
        C::Expand,
        S::Folder,
        Some(I::Expand),
        None,
        G::None,
        P::ClosedFolderSelected,
        NONE,
    ),
    row(
        C::Collapse,
        S::Folder,
        Some(I::Collapse),
        None,
        G::None,
        P::OpenFolderSelected,
        NONE,
    ),
    row(
        C::ExpandSubfolders,
        S::Folder,
        None,
        None,
        G::None,
        P::FolderSelected,
        NONE,
    ),
    row(
        C::CollapseSubfolders,
        S::Folder,
        None,
        None,
        G::None,
        P::FolderSelected,
        NONE,
    ),
    row(
        C::SelectAllFiles,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectNewerList,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectOrphanList,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectNewerLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectNewerRight,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectNewerBoth,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectOrphanLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectOrphanRight,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectOrphanBoth,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::SelectInvert,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::ExistsNext,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::ExistsPrev,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::DirFlatten,
        S::Folder,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ColList,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        HEADER,
    ),
    row(
        C::CopyRight,
        S::Folder,
        None,
        None,
        G::None,
        P::SideWritable(Side::Right),
        NONE,
    ),
    row(
        C::CopyLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::SideWritable(Side::Left),
        NONE,
    ),
    row(
        C::Delete,
        S::Folder,
        None,
        None,
        G::None,
        P::RowSelected,
        NONE,
    ),
    row(
        C::PermanentDelete,
        S::Folder,
        None,
        None,
        G::None,
        P::RowSelected,
        NONE,
    ),
    row(
        C::CopyPrompt,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::CopySide,
        S::Folder,
        None,
        None,
        G::None,
        P::CopyFromSide,
        NONE,
    ),
    row(
        C::MoveRight,
        S::Folder,
        None,
        None,
        G::None,
        P::Movable(Side::Right),
        NONE,
    ),
    row(
        C::MoveLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::Movable(Side::Left),
        NONE,
    ),
    row(
        C::MovePrompt,
        S::Folder,
        None,
        None,
        G::None,
        P::RowSelected,
        NONE,
    ),
    row(
        C::MoveSide,
        S::Folder,
        None,
        None,
        G::None,
        P::MoveFromSide,
        NONE,
    ),
    row(
        C::MoveBoth,
        S::Folder,
        None,
        None,
        G::None,
        P::BothWritable,
        NONE,
    ),
    row(
        C::CopyTo,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::MoveTo,
        S::Folder,
        None,
        None,
        G::None,
        P::RowSelected,
        NONE,
    ),
    row(
        C::Rename,
        S::Folder,
        None,
        None,
        G::None,
        P::RowSelected,
        NONE,
    ),
    row(
        C::Touch,
        S::Folder,
        None,
        None,
        G::None,
        P::RowSelected,
        NONE,
    ),
    row(
        C::NewFolder,
        S::Folder,
        None,
        None,
        G::None,
        P::AnyWritable,
        NONE,
    ),
    row(
        C::Ignored,
        S::Folder,
        None,
        None,
        G::Check,
        P::Selection,
        NONE,
    ),
    row(
        C::Exclude,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::Compare,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::QuickCompare,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::CompareTo,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::OpenWithList,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::AssocApp,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::SyncList,
        S::Folder,
        Some(I::FolderSync),
        None,
        G::None,
        P::NotFlat,
        NONE,
    ),
    row(
        C::UpdateRight,
        S::Folder,
        None,
        None,
        G::None,
        P::NotFlat,
        NONE,
    ),
    row(
        C::UpdateLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::NotFlat,
        NONE,
    ),
    row(
        C::UpdateBoth,
        S::Folder,
        None,
        None,
        G::None,
        P::NotFlat,
        NONE,
    ),
    row(
        C::MirrorRight,
        S::Folder,
        None,
        None,
        G::None,
        P::NotFlat,
        NONE,
    ),
    row(
        C::MirrorLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::NotFlat,
        NONE,
    ),
    row(
        C::SyncBaseView,
        S::Folder,
        Some(I::FolderSync),
        None,
        G::None,
        P::Loaded,
        NONE,
    ),
    row(
        C::Run,
        S::Sync,
        Some(I::Run),
        Some("Run"),
        G::None,
        P::PlanReady,
        NONE,
    ),
    row(
        C::CompareBaseView,
        S::Sync,
        Some(I::FolderCompare),
        None,
        G::None,
        P::Loaded,
        NONE,
    ),
    row(
        C::SyncLeave,
        S::Sync,
        Some(I::SyncLeave),
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::SyncCopyLeft,
        S::Sync,
        Some(I::SyncCopyLeft),
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::SyncCopyRight,
        S::Sync,
        Some(I::SyncCopyRight),
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::SyncDeleteLeft,
        S::Sync,
        Some(I::SyncDeleteLeft),
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::SyncDeleteRight,
        S::Sync,
        Some(I::SyncDeleteRight),
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::ConflictNext,
        S::Sync,
        Some(I::NextDiff),
        None,
        G::None,
        P::HasConflict,
        NONE,
    ),
    row(
        C::ConflictPrev,
        S::Sync,
        Some(I::PrevDiff),
        None,
        G::None,
        P::HasConflict,
        NONE,
    ),
    row(
        C::DiffNext,
        S::Folder,
        Some(I::NextDiff),
        Some("Next"),
        G::None,
        P::NextDifference,
        NONE,
    ),
    row(
        C::DiffPrev,
        S::Folder,
        Some(I::PrevDiff),
        Some("Prev"),
        G::None,
        P::PrevDifference,
        NONE,
    ),
    row(
        C::FavCompareAll,
        S::Folder,
        Some(I::FilterAll),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavDiff,
        S::Folder,
        Some(I::FilterDiff),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavSame,
        S::Folder,
        Some(I::FilterSame),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavNoOrphan,
        S::Folder,
        None,
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavDiffNoOrphan,
        S::Folder,
        None,
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavOrphan,
        S::Folder,
        Some(I::FilterOrphans),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavLeftNewer,
        S::Folder,
        Some(I::FilterLeftNewer),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavRightNewer,
        S::Folder,
        Some(I::FilterRightNewer),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavLeftNewerOrphan,
        S::Folder,
        None,
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavRightNewerOrphan,
        S::Folder,
        None,
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavLeftOrphan,
        S::Folder,
        Some(I::FilterLeftOrphan),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavRightOrphan,
        S::Folder,
        Some(I::FilterRightOrphan),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavCompareNone,
        S::Folder,
        Some(I::FilterNone),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::TogSame,
        S::Folder,
        Some(I::FilterSame),
        Some("Same Items"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogLeftOrphan,
        S::Folder,
        Some(I::FilterLeftOrphan),
        Some("Left Orphans"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogRightOrphan,
        S::Folder,
        Some(I::FilterRightOrphan),
        Some("Right Orphans"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogLeftNewer,
        S::Folder,
        Some(I::FilterLeftNewer),
        Some("Left Newer"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogRightNewer,
        S::Folder,
        Some(I::FilterRightNewer),
        Some("Right Newer"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogDiff,
        S::Folder,
        Some(I::FilterDiff),
        Some("Different Items"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::DirShowAll,
        S::Folder,
        None,
        None,
        G::FolderMode,
        P::Always,
        NONE,
    ),
    row(
        C::DirCompareStructure,
        S::Folder,
        None,
        None,
        G::FolderMode,
        P::Always,
        NONE,
    ),
    row(
        C::DirCompareFilesOnly,
        S::Folder,
        None,
        None,
        G::FolderMode,
        P::Always,
        NONE,
    ),
    row(
        C::InsigIgnored,
        S::Folder,
        Some(I::Unimportant),
        Some("Ignore Unimportant"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::SuppressFilters,
        S::Folder,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ShowToolbar,
        S::Folder,
        None,
        None,
        G::Check,
        P::Always,
        &[A::Toolbar],
    ),
    row(
        C::ShowLog,
        S::Folder,
        Some(I::Log),
        Some("Log"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::Legend,
        S::Folder,
        Some(I::Legend),
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(C::Info, S::Folder, None, None, G::None, P::Ready, NONE),
    // M2 W9c (FC-063, D-254, D-249, D-250).
    row(
        C::CopyFilenames,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::ExplorerList,
        S::Folder,
        None,
        None,
        G::None,
        P::Selection,
        NONE,
    ),
    row(
        C::Rules,
        S::Folder,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Rules,
        S::Text,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::NameFilter,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::NameFilterEdit,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(C::Info, S::Text, None, None, G::None, P::Ready, NONE),
    row(
        C::UpOneLevelList,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::UpOneLevelLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::BaseParent(Side::Left),
        NONE,
    ),
    row(
        C::UpOneLevelRight,
        S::Folder,
        None,
        None,
        G::None,
        P::BaseParent(Side::Right),
        NONE,
    ),
    row(
        C::UpOneLevelBoth,
        S::Folder,
        None,
        None,
        G::None,
        P::BaseParent(Side::Center),
        NONE,
    ),
    row(
        C::SetAsBaseFolders,
        S::Folder,
        None,
        None,
        G::None,
        P::BasesSelected,
        NONE,
    ),
    row(
        C::SetAsOtherBaseFolder,
        S::Folder,
        None,
        None,
        G::None,
        P::OtherBaseSelected,
        NONE,
    ),
    row(
        C::CompareParents,
        S::Folder,
        None,
        None,
        G::None,
        P::ParentsAvailable,
        NONE,
    ),
    row(
        C::CompareParents,
        S::Text,
        None,
        None,
        G::None,
        P::ParentsAvailable,
        NONE,
    ),
    row(C::Swap, S::Folder, None, None, G::None, P::ReadyIdle, NONE),
    row(C::Swap, S::Text, None, None, G::None, P::Always, NONE),
    row(
        C::SessionClear,
        S::Folder,
        None,
        None,
        G::None,
        P::Idle,
        NONE,
    ),
    row(
        C::SessionClear,
        S::Text,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::BrowseList,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::BrowseLeft,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::BrowseRight,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::FindFilename,
        S::Folder,
        Some(I::Search),
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::FindFilenameNext,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::FindFilenamePrev,
        S::Folder,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    // M2 W11a (§1.11.28): the editor, the clipboard, the copies, the saves.
    row(C::Undo, S::Text, None, None, G::None, P::CanUndo, NONE),
    row(C::Redo, S::Text, None, None, G::None, P::CanRedo, NONE),
    row(C::Cut, S::Text, None, None, G::None, P::FocusEditable, NONE),
    row(C::Copy, S::Text, None, None, G::None, P::PaneFocused, NONE),
    row(
        C::Paste,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::SelectAll,
        S::Text,
        None,
        None,
        G::None,
        P::PaneFocused,
        NONE,
    ),
    row(
        C::Delete,
        S::Text,
        None,
        None,
        G::None,
        P::HasSelection,
        NONE,
    ),
    row(
        C::DeleteItem,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::DeleteItemBegin,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::DeleteItemEnd,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::DeleteWord,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::DeleteWordBegin,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::DeleteWordEnd,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::InsertItemBefore,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::InsertItemAfter,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::IncreaseIndent,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::DecreaseIndent,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::SelectSection,
        S::Text,
        None,
        None,
        G::None,
        P::PaneFocused,
        NONE,
    ),
    row(
        C::CopyRight,
        S::Text,
        Some(I::CopyToRight),
        Some("Copy to Right"),
        G::None,
        P::PaneEditable(Side::Right),
        NONE,
    ),
    row(
        C::CopyLeft,
        S::Text,
        Some(I::CopyToLeft),
        Some("Copy to Left"),
        G::None,
        P::PaneEditable(Side::Left),
        NONE,
    ),
    row(
        C::CopySide,
        S::Text,
        None,
        None,
        G::None,
        P::TargetEditable,
        NONE,
    ),
    row(
        C::CopyItemRight,
        S::Text,
        None,
        None,
        G::None,
        P::PaneEditable(Side::Right),
        NONE,
    ),
    row(
        C::CopyItemLeft,
        S::Text,
        None,
        None,
        G::None,
        P::PaneEditable(Side::Left),
        NONE,
    ),
    row(
        C::CopyItemSide,
        S::Text,
        None,
        None,
        G::None,
        P::TargetEditable,
        NONE,
    ),
    row(
        C::WordWrap,
        S::Text,
        Some(I::WordWrap),
        Some("Wrap"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::Overwriting,
        S::Text,
        None,
        None,
        G::Check,
        P::Ready,
        NONE,
    ),
    row(
        C::ShowSpecInfo,
        S::Text,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::Save,
        S::Text,
        Some(I::Save),
        Some("Save"),
        G::None,
        P::Modified,
        NONE,
    ),
    row(
        C::SaveAll,
        S::Text,
        Some(I::SaveAll),
        Some("Save All"),
        G::None,
        P::AnyModified,
        NONE,
    ),
    row(
        C::SaveAs,
        S::Text,
        Some(I::SaveAs),
        None,
        G::None,
        P::NotSaving,
        NONE,
    ),
    row(
        C::SaveAsLocal,
        S::Text,
        None,
        None,
        G::None,
        P::NotSaving,
        NONE,
    ),
    row(
        C::FileFormat,
        S::Text,
        Some(I::Format),
        Some("Format"),
        G::None,
        P::Ready,
        NONE,
    ),
    // M2 W11b (§1.11.28): layouts, panels, search, navigation, marks, conversions, clipboard texts, the session link.
    row(
        C::SideBySide,
        S::Text,
        None,
        None,
        G::Filter,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::OverUnder,
        S::Text,
        None,
        None,
        G::Filter,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::ShowSyntaxHighlighting,
        S::Text,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ShowWS,
        S::Text,
        Some(I::Whitespace),
        Some("Show Whitespace"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ZoomPane,
        S::Text,
        None,
        None,
        G::Check,
        P::TwoPanes,
        NONE,
    ),
    row(C::ZoomLeft, S::Text, None, None, G::None, P::TwoPanes, NONE),
    row(
        C::ZoomRight,
        S::Text,
        None,
        None,
        G::None,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::DisplayFont,
        S::Text,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::ShowTextDetails,
        S::Text,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ShowHexDetails,
        S::Text,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ShowAlignDetails,
        S::Text,
        None,
        None,
        G::Check,
        P::TwoPanes,
        NONE,
    ),
    row(C::ShowRuler, S::Text, None, None, G::Check, P::Always, NONE),
    row(
        C::Find,
        S::Text,
        Some(I::Search),
        Some("Find"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::FindNext, S::Text, None, None, G::None, P::Ready, NONE),
    row(C::FindPrev, S::Text, None, None, G::None, P::Ready, NONE),
    row(
        C::Replace,
        S::Text,
        Some(I::Replace),
        Some("Replace"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::FindInFiles,
        S::Text,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(C::DiffNext, S::Text, None, None, G::None, P::Ready, NONE),
    row(C::DiffPrev, S::Text, None, None, G::None, P::Ready, NONE),
    row(C::EditNext, S::Text, None, None, G::None, P::Ready, NONE),
    row(C::EditPrev, S::Text, None, None, G::None, P::Ready, NONE),
    row(
        C::ExistsNext,
        S::Text,
        None,
        None,
        G::None,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::ExistsPrev,
        S::Text,
        None,
        None,
        G::None,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::DiffItemNext,
        S::Text,
        Some(I::NextDiff),
        Some("Next"),
        G::None,
        P::DiffAvailable(true),
        NONE,
    ),
    row(
        C::DiffItemPrev,
        S::Text,
        Some(I::PrevDiff),
        Some("Previous"),
        G::None,
        P::DiffAvailable(false),
        NONE,
    ),
    row(
        C::BrowseMode,
        S::Text,
        Some(I::Mark),
        Some("Browse By"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::GoTo, S::Text, None, None, G::None, P::Ready, NONE),
    row(
        C::BookmarkToggleList,
        S::Text,
        Some(I::Bookmark),
        Some("Bookmark"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoToList,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle0,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle1,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle2,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle3,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle4,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle5,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle6,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle7,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle8,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkToggle9,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo0,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo1,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo2,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo3,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo4,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo5,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo6,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo7,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo8,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkGoTo9,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::BookmarkClear,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::GoLeft, S::Text, None, None, G::None, P::TwoPanes, NONE),
    row(C::GoRight, S::Text, None, None, G::None, P::TwoPanes, NONE),
    row(C::GoNext, S::Text, None, None, G::None, P::TwoPanes, NONE),
    row(C::GoPrev, S::Text, None, None, G::None, P::TwoPanes, NONE),
    row(
        C::Ignored,
        S::Text,
        Some(I::Ignored),
        None,
        G::Check,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::AlignWith,
        S::Text,
        Some(I::Anchor),
        None,
        G::None,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::Isolate,
        S::Text,
        Some(I::Isolated),
        None,
        G::None,
        P::TwoPanes,
        NONE,
    ),
    row(
        C::AnchorsClear,
        S::Text,
        None,
        None,
        G::None,
        P::HasAnchors,
        NONE,
    ),
    row(
        C::ConvertList,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::TrimTrailWS,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::LeadSpaceToTab,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::TabToSpace,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::EndingStyleList,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::ToWindows,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::ToUnix,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::ToMac,
        S::Text,
        None,
        None,
        G::None,
        P::FocusEditable,
        NONE,
    ),
    row(
        C::CompareToClipboard,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::OpenClipboard,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::Open, S::Text, None, None, G::None, P::Always, NONE),
    row(C::OpenLocal, S::Text, None, None, G::None, P::Always, NONE),
    row(
        C::OpenWithList,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::AssocApp, S::Text, None, None, G::None, P::Ready, NONE),
    row(
        C::ExplorerList,
        S::Text,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::DiffFileNext,
        S::Text,
        None,
        None,
        G::None,
        P::ChildView,
        NONE,
    ),
    row(
        C::DiffFilePrev,
        S::Text,
        None,
        None,
        G::None,
        P::ChildView,
        NONE,
    ),
    row(
        C::CopyFileRight,
        S::Text,
        None,
        None,
        G::None,
        P::CopyFileTo(Side::Right),
        NONE,
    ),
    row(
        C::CopyFileLeft,
        S::Text,
        None,
        None,
        G::None,
        P::CopyFileTo(Side::Left),
        NONE,
    ),
    // Table Compare: the file-shared entries of the text views (W11a, W11b) are not the table view's.
    row(C::ShowWS, S::Table, None, None, G::None, P::Never, NONE),
    row(
        C::DisplayFont,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(C::Find, S::Table, None, None, G::None, P::Never, NONE),
    row(C::FindNext, S::Table, None, None, G::None, P::Never, NONE),
    row(C::FindPrev, S::Table, None, None, G::None, P::Never, NONE),
    row(C::GoLeft, S::Table, None, None, G::None, P::Never, NONE),
    row(C::GoRight, S::Table, None, None, G::None, P::Never, NONE),
    row(C::GoNext, S::Table, None, None, G::None, P::Never, NONE),
    row(C::GoPrev, S::Table, None, None, G::None, P::Never, NONE),
    row(
        C::OpenClipboard,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(C::Open, S::Table, None, None, G::None, P::Never, NONE),
    row(C::OpenLocal, S::Table, None, None, G::None, P::Never, NONE),
    row(
        C::OpenWithList,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(C::AssocApp, S::Table, None, None, G::None, P::Never, NONE),
    row(
        C::ExplorerList,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::DiffFileNext,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::DiffFilePrev,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::CopyFileRight,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::CopyFileLeft,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    // Table Compare: the file-shared entries of the text views (W11a) are not the table view's.
    row(C::Save, S::Table, None, None, G::None, P::Never, NONE),
    row(C::SaveAll, S::Table, None, None, G::None, P::Never, NONE),
    row(C::SaveAs, S::Table, None, None, G::None, P::Never, NONE),
    row(
        C::SaveAsLocal,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::ShowSpecInfo,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::FileFormat,
        S::Table,
        Some(I::Format),
        Some("Format"),
        G::None,
        P::Never,
        NONE,
    ),
    // Text Compare
    row(
        C::Reload,
        S::Text,
        Some(I::Reload),
        Some("Reload"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DiffSectionNext,
        S::Text,
        Some(I::NextSection),
        Some("Next"),
        G::None,
        P::NextDifference,
        NONE,
    ),
    row(
        C::DiffSectionPrev,
        S::Text,
        Some(I::PrevSection),
        Some("Prev"),
        G::None,
        P::PrevDifference,
        NONE,
    ),
    row(
        C::FavCompareAll,
        S::Text,
        Some(I::FilterAll),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavDiff,
        S::Text,
        Some(I::FilterDiff),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavSame,
        S::Text,
        Some(I::FilterSame),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::ShowContext,
        S::Text,
        Some(I::Context),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavCompareNone,
        S::Text,
        Some(I::FilterNone),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::TogSame,
        S::Text,
        Some(I::FilterSame),
        Some("Same Lines"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogLeftOrphan,
        S::Text,
        Some(I::FilterLeftOrphan),
        Some("Left Orphans"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogRightOrphan,
        S::Text,
        Some(I::FilterRightOrphan),
        Some("Right Orphans"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogDiff,
        S::Text,
        Some(I::FilterDiff),
        Some("Different Lines"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::InsigIgnored,
        S::Text,
        Some(I::Unimportant),
        Some("Ignore Unimportant"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ShowThumbnail,
        S::Text,
        Some(I::Thumbnail),
        Some("Thumbnail"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontIncrease,
        S::Text,
        Some(I::FontLarger),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontDecrease,
        S::Text,
        Some(I::FontSmaller),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontReset,
        S::Text,
        Some(I::FontReset),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::ShowToolbar,
        S::Text,
        None,
        None,
        G::Check,
        P::Always,
        &[A::Toolbar],
    ),
    // The session commands of the views.
    row(
        C::SessionHome,
        S::Folder,
        Some(I::Home),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionsCombo,
        S::Folder,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSave,
        S::Folder,
        Some(I::Save),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSaveAs,
        S::Folder,
        Some(I::SaveAs),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionEdit,
        S::Folder,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionLocked,
        S::Folder,
        Some(I::Lock),
        None,
        G::Check,
        P::Named,
        NONE,
    ),
    row(
        C::SessionHome,
        S::Text,
        Some(I::Home),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionsCombo,
        S::Text,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSave,
        S::Text,
        Some(I::Save),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSaveAs,
        S::Text,
        Some(I::SaveAs),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionEdit,
        S::Text,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionLocked,
        S::Text,
        Some(I::Lock),
        None,
        G::Check,
        P::Named,
        NONE,
    ),
    // Hex Compare (M2 W13a, D-371): the view's own entries, the file-shared and session ones it shows; editing
    // (HX-006), Replace and the report (W13c) stay planned.
    row(
        C::SelectAll,
        S::Hex,
        None,
        None,
        G::None,
        P::ByteSelection,
        NONE,
    ),
    row(
        C::DiffItemNext,
        S::Hex,
        Some(I::NextSection),
        Some("Next"),
        G::None,
        P::NextDifference,
        NONE,
    ),
    row(
        C::DiffItemPrev,
        S::Hex,
        Some(I::PrevSection),
        Some("Prev"),
        G::None,
        P::PrevDifference,
        NONE,
    ),
    row(C::ExistsNext, S::Hex, None, None, G::None, P::Ready, NONE),
    row(C::ExistsPrev, S::Hex, None, None, G::None, P::Ready, NONE),
    row(
        C::FavCompareAll,
        S::Hex,
        Some(I::FilterAll),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavDiff,
        S::Hex,
        Some(I::FilterDiff),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavSame,
        S::Hex,
        Some(I::FilterSame),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::GoTo,
        S::Hex,
        Some(I::Search),
        Some("Go To"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::LittleEndianList,
        S::Hex,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::BigEndianList,
        S::Hex,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::Layout,
        S::Hex,
        Some(I::Layout),
        Some("Layout"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::ShowThumbnail,
        S::Hex,
        Some(I::Thumbnail),
        Some("Thumbnail"),
        G::Check,
        P::Always,
        &[A::HexThumbnail],
    ),
    row(
        C::CurrentAddress,
        S::Hex,
        None,
        None,
        G::None,
        P::ByteSelection,
        NONE,
    ),
    row(
        C::CopyAsText,
        S::Hex,
        None,
        None,
        G::None,
        P::ByteSelection,
        NONE,
    ),
    row(
        C::AlignModeList,
        S::Hex,
        Some(I::Alignment),
        Some("Alignment"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::Copy, S::Hex, None, None, G::None, P::ByteSelection, NONE),
    row(
        C::FileFormat,
        S::Hex,
        Some(I::Format),
        Some("Format"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Reload,
        S::Hex,
        Some(I::Reload),
        Some("Reload"),
        G::None,
        P::Always,
        NONE,
    ),
    row(C::Open, S::Hex, None, None, G::None, P::Always, NONE),
    row(C::OpenLocal, S::Hex, None, None, G::None, P::Always, NONE),
    row(
        C::OpenClipboard,
        S::Hex,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(C::OpenWithList, S::Hex, None, None, G::None, P::Ready, NONE),
    row(C::AssocApp, S::Hex, None, None, G::None, P::Ready, NONE),
    row(C::ExplorerList, S::Hex, None, None, G::None, P::Ready, NONE),
    row(
        C::DiffFileNext,
        S::Hex,
        None,
        None,
        G::None,
        P::ChildView,
        NONE,
    ),
    row(
        C::DiffFilePrev,
        S::Hex,
        None,
        None,
        G::None,
        P::ChildView,
        NONE,
    ),
    row(C::Find, S::Hex, None, None, G::None, P::Ready, NONE),
    row(C::FindNext, S::Hex, None, None, G::None, P::Ready, NONE),
    row(C::FindPrev, S::Hex, None, None, G::None, P::Ready, NONE),
    row(C::GoLeft, S::Hex, None, None, G::None, P::Always, NONE),
    row(C::GoRight, S::Hex, None, None, G::None, P::Always, NONE),
    row(C::GoNext, S::Hex, None, None, G::None, P::Always, NONE),
    row(C::GoPrev, S::Hex, None, None, G::None, P::Always, NONE),
    row(C::DisplayFont, S::Hex, None, None, G::None, P::Always, NONE),
    row(
        C::DisplayFontIncrease,
        S::Hex,
        Some(I::FontLarger),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontDecrease,
        S::Hex,
        Some(I::FontSmaller),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontReset,
        S::Hex,
        Some(I::FontReset),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SideBySide,
        S::Hex,
        None,
        Some("Side by Side"),
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::OverUnder,
        S::Hex,
        None,
        Some("Over-Under"),
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::ShowSpecInfo,
        S::Hex,
        None,
        None,
        G::Check,
        P::Always,
        NONE,
    ),
    row(C::Save, S::Hex, None, None, G::None, P::Never, NONE),
    row(C::SaveAll, S::Hex, None, None, G::None, P::Never, NONE),
    row(C::SaveAs, S::Hex, None, None, G::None, P::Never, NONE),
    row(C::SaveAsLocal, S::Hex, None, None, G::None, P::Never, NONE),
    row(C::ShowWS, S::Hex, None, None, G::None, P::Never, NONE),
    row(
        C::CopyFileRight,
        S::Hex,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(C::CopyFileLeft, S::Hex, None, None, G::None, P::Never, NONE),
    row(
        C::Rules,
        S::Hex,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Info,
        S::Hex,
        Some(I::Info),
        None,
        G::None,
        P::HexLanded,
        NONE,
    ),
    row(
        C::Swap,
        S::Hex,
        Some(I::Swap),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::CompareParents,
        S::Hex,
        None,
        None,
        G::None,
        P::ParentsAvailable,
        NONE,
    ),
    row(
        C::SessionClear,
        S::Hex,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(C::InsigIgnored, S::Hex, None, None, G::None, P::Never, NONE),
    row(
        C::SessionHome,
        S::Hex,
        Some(I::Home),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionsCombo,
        S::Hex,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSave,
        S::Hex,
        Some(I::Save),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSaveAs,
        S::Hex,
        Some(I::SaveAs),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionEdit,
        S::Hex,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionLocked,
        S::Hex,
        Some(I::Lock),
        None,
        G::Check,
        P::Named,
        NONE,
    ),
    row(
        C::ShowToolbar,
        S::Hex,
        None,
        None,
        G::Check,
        P::Always,
        &[A::Toolbar],
    ),
    // Table Compare (phase 3, W13): the grid's commands; editing (TB-023) stays planned.
    row(
        C::Reload,
        S::Table,
        Some(I::Reload),
        Some("Reload"),
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DiffItemNext,
        S::Table,
        Some(I::NextSection),
        Some("Next"),
        G::None,
        P::NextDifference,
        NONE,
    ),
    row(
        C::DiffItemPrev,
        S::Table,
        Some(I::PrevSection),
        Some("Prev"),
        G::None,
        P::PrevDifference,
        NONE,
    ),
    row(
        C::DiffNext,
        S::Table,
        Some(I::NextDiff),
        Some("Next Cell"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::DiffPrev,
        S::Table,
        Some(I::PrevDiff),
        Some("Prev Cell"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::DiffSheetNext,
        S::Table,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::DiffSheetPrev,
        S::Table,
        None,
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::GoTo, S::Table, None, None, G::None, P::Ready, NONE),
    row(
        C::FavCompareAll,
        S::Table,
        Some(I::FilterAll),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavDiff,
        S::Table,
        Some(I::FilterDiff),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavSame,
        S::Table,
        Some(I::FilterSame),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::FavCompareNone,
        S::Table,
        Some(I::FilterNone),
        None,
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::TogSame,
        S::Table,
        Some(I::FilterSame),
        Some("Same Rows"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogLeftOrphan,
        S::Table,
        Some(I::FilterLeftOrphan),
        Some("Left Orphans"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogRightOrphan,
        S::Table,
        Some(I::FilterRightOrphan),
        Some("Right Orphans"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::TogDiff,
        S::Table,
        Some(I::FilterDiff),
        Some("Different Rows"),
        G::Toggles,
        P::Always,
        NONE,
    ),
    row(
        C::InsigIgnored,
        S::Table,
        Some(I::Unimportant),
        Some("Ignore Unimportant"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ColHideSame,
        S::Table,
        None,
        Some("Hide Same Columns"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ColDetectSize,
        S::Table,
        None,
        Some("Fit Columns"),
        G::None,
        P::Ready,
        NONE,
    ),
    row(
        C::ShowThumbnail,
        S::Table,
        Some(I::Thumbnail),
        Some("Thumbnail"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::ShowDetails,
        S::Table,
        None,
        Some("Details"),
        G::Check,
        P::Always,
        NONE,
    ),
    row(
        C::SideBySide,
        S::Table,
        None,
        Some("Side by Side"),
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::OverUnder,
        S::Table,
        None,
        Some("Over-Under"),
        G::Filter,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontIncrease,
        S::Table,
        Some(I::FontLarger),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontDecrease,
        S::Table,
        Some(I::FontSmaller),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::DisplayFontReset,
        S::Table,
        Some(I::FontReset),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(C::Copy, S::Table, None, None, G::None, P::Ready, NONE),
    row(
        C::Rules,
        S::Table,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::Info,
        S::Table,
        Some(I::Info),
        None,
        G::None,
        P::Ready,
        NONE,
    ),
    row(C::Report, S::Table, None, None, G::None, P::Ready, NONE),
    row(
        C::Swap,
        S::Table,
        Some(I::Swap),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::CompareParents,
        S::Table,
        None,
        None,
        G::None,
        P::ParentsAvailable,
        NONE,
    ),
    row(
        C::SessionClear,
        S::Table,
        None,
        None,
        G::None,
        P::Never,
        NONE,
    ),
    row(
        C::SessionHome,
        S::Table,
        Some(I::Home),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionsCombo,
        S::Table,
        None,
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSave,
        S::Table,
        Some(I::Save),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionSaveAs,
        S::Table,
        Some(I::SaveAs),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionEdit,
        S::Table,
        Some(I::Settings),
        None,
        G::None,
        P::Always,
        NONE,
    ),
    row(
        C::SessionLocked,
        S::Table,
        Some(I::Lock),
        None,
        G::Check,
        P::Named,
        NONE,
    ),
    row(
        C::ShowToolbar,
        S::Table,
        None,
        None,
        G::Check,
        P::Always,
        &[A::Toolbar],
    ),
];

/// The presentation row of `id` in `scope` (or the global row). A Folder Sync view shares the Folder Compare rows of
/// the sections both views have (`folder-shared`, `session`): its own row wins, then the folder row; which entries it
/// shows is still its own catalogue table's (M2 W9b).
pub fn lookup(scope: Scope, id: CommandId) -> Option<&'static Row> {
    PRESENTATION
        .iter()
        .find(|r| r.id == id && r.scope == scope && scope != Scope::Global)
        .or_else(|| {
            (scope == Scope::Sync)
                .then(|| {
                    PRESENTATION
                        .iter()
                        .find(|r| r.id == id && r.scope == Scope::Folder)
                })
                .flatten()
        })
        .or_else(|| {
            (scope == Scope::TextEdit && one_pane(id))
                .then(|| {
                    PRESENTATION
                        .iter()
                        .find(|r| r.id == id && r.scope == Scope::Text)
                })
                .flatten()
        })
        .or_else(|| {
            PRESENTATION
                .iter()
                .find(|r| r.id == id && r.scope == Scope::Global)
        })
}

/// True for a Text Compare command a Text Edit view has too (D-313: one pane suffices). The two-pane commands, the
/// comparison's display filter, the alignment overrides, the session link and the rule tabs are absent there.
pub fn one_pane(id: CommandId) -> bool {
    use CommandId as C;
    !matches!(
        id,
        C::CopyRight
            | C::CopyLeft
            | C::CopySide
            | C::CopyItemRight
            | C::CopyItemLeft
            | C::CopyItemSide
            | C::SelectSection
            | C::SideBySide
            | C::OverUnder
            | C::ZoomPane
            | C::ZoomLeft
            | C::ZoomRight
            | C::ShowAlignDetails
            | C::DiffNext
            | C::DiffPrev
            | C::DiffSectionNext
            | C::DiffSectionPrev
            | C::DiffItemNext
            | C::DiffItemPrev
            | C::BrowseMode
            | C::ExistsNext
            | C::ExistsPrev
            | C::GoLeft
            | C::GoRight
            | C::GoNext
            | C::GoPrev
            | C::Ignored
            | C::AlignWith
            | C::Isolate
            | C::AnchorsClear
            | C::DiffFileNext
            | C::DiffFilePrev
            | C::CopyFileRight
            | C::CopyFileLeft
            | C::FavCompareAll
            | C::FavDiff
            | C::FavSame
            | C::FavCompareNone
            | C::ShowContext
            | C::TogSame
            | C::TogLeftOrphan
            | C::TogRightOrphan
            | C::TogDiff
            | C::InsigIgnored
            | C::ShowThumbnail
            | C::Rules
            | C::SaveAll
            | C::Swap
            | C::CompareParents
    )
}

/// The presentation rows of a view type: its own, then (a Folder Sync view) the folder rows it has no row of its own
/// for, then the global ones.
pub fn rows_for(scope: Scope) -> impl Iterator<Item = &'static Row> {
    // A Text Edit view (D-313) has the Text Compare rows that one pane suffices for.
    let own = PRESENTATION.iter().filter(move |r| {
        scope != Scope::Global
            && (r.scope == scope
                || (scope == Scope::TextEdit && r.scope == Scope::Text && one_pane(r.id)))
    });
    let shared = PRESENTATION.iter().filter(move |r| {
        scope == Scope::Sync
            && r.scope == Scope::Folder
            && !PRESENTATION
                .iter()
                .any(|own| own.scope == Scope::Sync && own.id == r.id)
    });
    let global = PRESENTATION.iter().filter(|r| r.scope == Scope::Global);
    own.chain(shared).chain(global)
}

/// The effective command tables of the view types the GUI shows (D-109), built from `commands.toml`.
#[derive(Debug, Clone)]
pub struct CommandTables {
    home: ViewCommands<'static>,
    folder: ViewCommands<'static>,
    text: ViewCommands<'static>,
    sync: ViewCommands<'static>,
    table: ViewCommands<'static>,
    text_edit: ViewCommands<'static>,
    hex: ViewCommands<'static>,
}

impl Default for CommandTables {
    fn default() -> CommandTables {
        CommandTables::new(&CommandOverrides::default())
    }
}

impl CommandTables {
    /// The tables with the overrides of `commands.toml`.
    pub fn new(overrides: &CommandOverrides) -> CommandTables {
        let table = |scope: Scope| {
            effective(
                catalogue(),
                scope.view_name().unwrap_or_default(),
                overrides,
            )
        };
        CommandTables {
            home: table(Scope::Home),
            folder: table(Scope::Folder),
            text: table(Scope::Text),
            sync: table(Scope::Sync),
            table: table(Scope::Table),
            text_edit: table(Scope::TextEdit),
            hex: table(Scope::Hex),
        }
    }

    /// The table of a view type (`Global`: the Home table, which holds every global command).
    pub fn view(&self, scope: Scope) -> &ViewCommands<'static> {
        match scope {
            Scope::Global | Scope::Home => &self.home,
            Scope::Folder => &self.folder,
            Scope::Text => &self.text,
            Scope::Sync => &self.sync,
            Scope::Table => &self.table,
            Scope::TextEdit => &self.text_edit,
            Scope::Hex => &self.hex,
        }
    }

    /// The effective entry of `id` in `scope`'s table.
    pub fn entry(&self, scope: Scope, id: CommandId) -> Option<&EffectiveCommand<'static>> {
        self.view(scope).get(id.name())
    }

    /// The caption of `id` (the catalogue's, else the presentation's label, else the id).
    pub fn caption(&self, scope: Scope, id: CommandId) -> String {
        let caption = self.entry(scope, id).map_or("", |e| e.caption);
        if !caption.is_empty() {
            // `<Type> Info`, `<Type> Report...`: the window writes the view's type (§1.9.21).
            return caption.replace("<Type>", scope.type_title());
        }
        lookup(scope, id)
            .and_then(|r| r.toolbar_label)
            .unwrap_or(id.name())
            .to_owned()
    }

    /// The description (hint) of `id`; a catalogue description's `<type>` is the view's type (`Info`: "Displays
    /// folder compare statistics."), as a caption's `<Type>`.
    pub fn description(&self, scope: Scope, id: CommandId) -> String {
        let text = self.entry(scope, id).map_or("", |e| e.description);
        if text.contains("<type>") {
            text.replace("<type>", &scope.type_title().to_lowercase())
        } else {
            text.to_owned()
        }
    }

    /// The first effective shortcut of `id` (menus show it; hints with `show_shortcut_in_hints`).
    pub fn shortcut(&self, scope: Scope, id: CommandId) -> Option<&Shortcut> {
        self.entry(scope, id).and_then(|e| e.shortcuts.first())
    }

    /// The command a key press runs (D-111), see [`dispatch`].
    pub fn dispatch(
        &self,
        scope: Scope,
        ctx: &Ctx,
        key: &Shortcut,
        at: FocusArea,
    ) -> Option<CommandId> {
        dispatch(self, scope, ctx, key, at)
    }
}

/// True for a command the GUI shows in `scope`: an `implemented` catalogue entry with a presentation row.
pub fn shown(tables: &CommandTables, scope: Scope, id: CommandId) -> bool {
    lookup(scope, id).is_some() && tables.entry(scope, id).is_some_and(|e| e.implemented())
}

/// Where a key event is handled (D-111): the window root takes what no focus area owns.
#[allow(dead_code)] // variants for W4b/W4c (contract seams)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusArea {
    /// The window root: the event bubbled up from whatever has the focus.
    Root,
    /// The view's main area (the folder tree, the text panes).
    ViewMain,
    /// A path field (Home, dialogs).
    PathField,
    /// The Home sessions tree (W4b).
    SessionsTree,
    /// The Home search box (W4b).
    HomeSearch,
    /// A dialog (it handles its keys itself).
    Dialog,
    /// A text field of a view (M2 W9c, D-252): a folder view's path edit, the Filters edit, the find panel. Like a
    /// path field it keeps Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+A.
    TextField,
}

impl FocusArea {
    /// True for an area whose control edits text (D-252: it keeps the clipboard keys and Ctrl+A).
    pub fn edits_text(self) -> bool {
        matches!(
            self,
            FocusArea::PathField | FocusArea::HomeSearch | FocusArea::TextField | FocusArea::Dialog
        )
    }
}

/// The keys a focused text field keeps (D-252, D-111 refinement): Ctrl+C, Ctrl+X, Ctrl+V and Ctrl+A, without Shift or
/// Alt.
pub fn is_text_edit_key(s: &Shortcut) -> bool {
    s.ctrl && !s.shift && !s.alt && matches!(s.key.as_str(), "C" | "X" | "V" | "A")
}

/// A key that focused controls use themselves (D-111): no Ctrl, Alt or Cmd, and a letter, digit, punctuation,
/// Enter, Tab, Esc, Space, BkSp, Del, Ins, Home, End, PgUp, PgDn or an arrow.
pub fn is_focus_bound(s: &Shortcut) -> bool {
    if s.ctrl || s.alt || s.cmd {
        return false;
    }
    let mut chars = s.key.chars();
    let single = matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii_graphic());
    single
        || matches!(
            s.key.as_str(),
            "Enter"
                | "Tab"
                | "Esc"
                | "Space"
                | "BkSp"
                | "Del"
                | "Ins"
                | "Home"
                | "End"
                | "PgUp"
                | "PgDn"
                | "Up"
                | "Down"
                | "Left"
                | "Right"
        )
}

/// The focus area a restricted shortcut belongs to: a context naming a path field → [`FocusArea::PathField`],
/// a command of the sessions tree → [`FocusArea::SessionsTree`], else the view's main area.
fn area_of(shortcut: &Shortcut, row: &Row) -> FocusArea {
    match &shortcut.context {
        Some(context) if context.contains("path field") => FocusArea::PathField,
        _ if row.areas.contains(&Area::SessionTree) => FocusArea::SessionsTree,
        _ => FocusArea::ViewMain,
    }
}

/// The command a key press runs in the active view of `scope` (D-111). Candidates are the shown, available
/// entries whose effective shortcuts contain the key. (1) An entry whose matching shortcut has a context, whose
/// menu is empty, or whose shortcut is focus-bound is taken only by its focus area; (2) the others only at the
/// window root, the first in `view_sections` then catalogue order winning. (3) Workspace shortcuts (W4b) apply
/// when this returns `None`. While a dialog is open nothing is taken.
pub fn dispatch(
    tables: &CommandTables,
    scope: Scope,
    ctx: &Ctx,
    key: &Shortcut,
    at: FocusArea,
) -> Option<CommandId> {
    if ctx.dialog_open || at == FocusArea::Dialog {
        return None;
    }
    // A focused text field keeps its editing keys, whatever binds them (D-252).
    if at.edits_text() && is_text_edit_key(key) {
        return None;
    }
    let bare = |s: &Shortcut| Shortcut {
        context: None,
        ..s.clone()
    };
    let wanted = bare(key);
    for entry in tables.view(scope).with_shortcut(key) {
        let Some(id) = CommandId::from_name(entry.id()) else {
            continue;
        };
        let Some(row) = lookup(scope, id) else {
            continue;
        };
        if !entry.implemented() || !row.enabled_when.holds(ctx) {
            continue;
        }
        let Some(matched) = entry.shortcuts.iter().find(|s| bare(s) == wanted) else {
            continue;
        };
        let restricted =
            matched.context.is_some() || entry.menu.is_empty() || is_focus_bound(matched);
        let here = if restricted {
            at == area_of(matched, row)
        } else {
            at == FocusArea::Root
        };
        if here {
            return Some(id);
        }
    }
    None
}

/// The hint of a command (UI-007): the caption, then the first effective shortcut in parentheses when
/// `show_shortcut`, then the description on a second line.
pub fn hint_text(
    caption: &str,
    shortcut: Option<&Shortcut>,
    description: &str,
    show_shortcut: bool,
) -> String {
    let mut text = caption.to_owned();
    if show_shortcut && let Some(s) = shortcut {
        text.push_str(&format!(" ({s})"));
    }
    if !description.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(description);
    }
    text
}

/// When hints show (OPT-017, OPT-019): after `show_ms` of hover, hidden after `hide_ms` more (0 = never).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HintTiming {
    /// Hints at all (`show_hints`).
    pub enabled: bool,
    /// The shortcut in the hint (`show_shortcut_in_hints`).
    pub show_shortcut: bool,
    /// Milliseconds of hover before a hint appears (`hint_show_ms`).
    pub show_ms: u32,
    /// Milliseconds a hint stays (`hint_hide_ms`; 0: until the pointer leaves).
    pub hide_ms: u32,
}

impl Default for HintTiming {
    fn default() -> HintTiming {
        HintTiming::from_options(&hikasa_session::settings::Options::default())
    }
}

impl HintTiming {
    /// The timing of the options.
    pub fn from_options(o: &hikasa_session::settings::Options) -> HintTiming {
        HintTiming {
            enabled: o.tweaks.interface.show_hints,
            show_shortcut: o.tweaks.interface.show_shortcut_in_hints,
            show_ms: o.tweaks.metrics.hint_show_ms,
            hide_ms: o.tweaks.metrics.hint_hide_ms,
        }
    }

    /// True when a hint is visible after `hovered_s` seconds of hover (`hovered_s` from the frame times).
    pub fn visible(&self, hovered_s: f64) -> bool {
        if !self.enabled {
            return false;
        }
        let ms = hovered_s * 1000.0;
        let shown = ms >= f64::from(self.show_ms);
        let hidden = self.hide_ms > 0 && ms >= f64::from(self.show_ms) + f64::from(self.hide_ms);
        shown && !hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_session::commands::CommandStatus;
    use std::collections::HashSet;

    fn sc(text: &str) -> Shortcut {
        Shortcut::parse(text).expect("a shortcut")
    }

    #[test]
    fn rows_are_unique_per_scope_and_names_round_trip() {
        for scope in [Scope::Global, Scope::Home, Scope::Folder, Scope::Text] {
            let mut ids = HashSet::new();
            for r in PRESENTATION.iter().filter(|r| r.scope == scope) {
                assert!(ids.insert(r.id), "{:?} twice in {scope:?}", r.id);
            }
        }
        for id in CommandId::ALL {
            assert_eq!(CommandId::from_name(id.name()), Some(*id));
            assert!(
                PRESENTATION.iter().any(|r| r.id == *id),
                "{} has no presentation row",
                id.name()
            );
        }
        assert_eq!(CommandId::from_name("NoSuch"), None);
    }

    /// §1.4.16 rule 1 (A4.3): every command the window executes is an `implemented` entry of the core catalogue
    /// in a section of its view type, every `implemented` entry of a section a shown view uses has a row, and
    /// every row on a default toolbar has an icon.
    #[test]
    fn the_window_matches_the_core_catalogue() {
        let core = catalogue();
        for r in PRESENTATION {
            let (_, entry) = core
                .find(r.scope.catalogue_sections(), r.id.name())
                .unwrap_or_else(|| {
                    panic!("{} ({:?}) is not in the catalogue", r.id.name(), r.scope)
                });
            assert_eq!(entry.status, CommandStatus::Implemented, "{}", r.id.name());
            if entry.toolbar {
                assert!(
                    r.icon.is_some(),
                    "{} is on a toolbar without an icon",
                    r.id.name()
                );
            }
        }
        for scope in Scope::VIEWS {
            for section in scope.catalogue_sections() {
                let Some(s) = core.section(section) else {
                    continue;
                };
                for entry in s
                    .commands
                    .iter()
                    .filter(|e| e.status == CommandStatus::Implemented)
                {
                    let id = CommandId::from_name(&entry.id).unwrap_or_else(|| {
                        panic!(
                            "{} ({section}) is implemented but unknown to the GUI",
                            entry.id
                        )
                    });
                    // A Text Edit view has no row for the commands that need two panes (D-313).
                    if scope == Scope::TextEdit && !one_pane(id) {
                        assert!(lookup(scope, id).is_none(), "{} in Text Edit", entry.id);
                        continue;
                    }
                    assert!(
                        lookup(scope, id).is_some(),
                        "{} ({section}) is implemented but {scope:?} has no row",
                        entry.id
                    );
                }
            }
        }
    }

    #[test]
    fn every_view_type_has_a_help_topic() {
        assert_eq!(Scope::Home.help_topic(), "view-home");
        assert_eq!(Scope::Folder.help_topic(), "view-folder-compare");
        assert_eq!(Scope::Text.help_topic(), "view-text-compare");
        assert_eq!(Scope::Global.help_topic(), "main-window");
    }

    #[test]
    fn tables_read_captions_and_shortcuts_from_the_core() {
        let tables = CommandTables::default();
        assert_eq!(
            tables.caption(Scope::Folder, CommandId::DiffNext),
            "Next Difference"
        );
        assert_eq!(
            tables.shortcut(Scope::Folder, CommandId::DiffNext),
            Some(&sc("Ctrl+N"))
        );
        assert_eq!(
            tables.shortcut(Scope::Text, CommandId::DiffSectionNext),
            Some(&sc("Ctrl+N"))
        );
        // An entry without a catalogue caption takes the presentation label.
        assert_eq!(
            tables.caption(Scope::Folder, CommandId::TogSame),
            "Same Items"
        );
        assert_eq!(
            tables.caption(Scope::Text, CommandId::FavDiff),
            "Show Differences"
        );
        assert!(
            !tables
                .description(Scope::Text, CommandId::Reload)
                .is_empty()
        );
        assert!(shown(&tables, Scope::Text, CommandId::Reload));
        assert!(!shown(&tables, Scope::Text, CommandId::Stop));
    }

    /// §1.4.16 rule 3 (A4.3): the D-111 dispatch table.
    #[test]
    fn dispatch_follows_focus_order_overrides_and_dialogs() {
        let tables = CommandTables::default();
        let ctx = |scope| Ctx {
            scope,
            tabs: 2,
            windows: 1,
            next_difference: true,
            prev_difference: true,
            pair_selected: true,
            closed_folder_selected: true,
            home_ready: true,
            ..Ctx::default()
        };
        let run =
            |t: &CommandTables, scope, key: &str, at| t.dispatch(scope, &ctx(scope), &sc(key), at);
        // Own section over global: Ctrl+N is the view's next difference.
        assert_eq!(
            run(&tables, Scope::Folder, "Ctrl+N", FocusArea::Root),
            Some(CommandId::DiffNext)
        );
        assert_eq!(
            run(&tables, Scope::Text, "Ctrl+N", FocusArea::Root),
            Some(CommandId::DiffSectionNext)
        );
        assert_eq!(run(&tables, Scope::Home, "Ctrl+N", FocusArea::Root), None);
        // Global commands everywhere, at the root only.
        for scope in Scope::VIEWS {
            assert_eq!(
                run(&tables, scope, "Ctrl+W", FocusArea::Root),
                Some(CommandId::CloseTab)
            );
            assert_eq!(run(&tables, scope, "Ctrl+W", FocusArea::ViewMain), None);
            assert_eq!(
                run(&tables, scope, "Ctrl+Tab", FocusArea::Root),
                Some(CommandId::TabNext)
            );
        }
        // Focus-bound keys belong to their area.
        for (key, id) in [("Enter", CommandId::Open), ("Right", CommandId::Expand)] {
            assert_eq!(
                run(&tables, Scope::Folder, key, FocusArea::Root),
                None,
                "{key}"
            );
            assert_eq!(
                run(&tables, Scope::Folder, key, FocusArea::ViewMain),
                Some(id),
                "{key}"
            );
        }
        // A context: Enter in a path field is Home's Compare, nowhere else.
        assert_eq!(
            run(&tables, Scope::Home, "Enter", FocusArea::PathField),
            Some(CommandId::HomeCompare)
        );
        assert_eq!(run(&tables, Scope::Home, "Enter", FocusArea::Root), None);
        assert_eq!(
            run(&tables, Scope::Home, "Enter", FocusArea::ViewMain),
            None
        );
        // Unavailable commands take no key.
        let off = Ctx {
            scope: Scope::Folder,
            tabs: 1,
            ..Ctx::default()
        };
        assert_eq!(
            tables.dispatch(Scope::Folder, &off, &sc("Ctrl+N"), FocusArea::Root),
            None
        );
        assert_eq!(
            tables.dispatch(Scope::Folder, &off, &sc("Ctrl+Tab"), FocusArea::Root),
            None
        );
        // A dialog takes every key away from the window's commands.
        let dialog = Ctx {
            dialog_open: true,
            ..ctx(Scope::Folder)
        };
        assert_eq!(
            tables.dispatch(Scope::Folder, &dialog, &sc("Ctrl+W"), FocusArea::Root),
            None
        );
        assert_eq!(
            run(&tables, Scope::Folder, "Ctrl+W", FocusArea::Dialog),
            None
        );
        // Overrides: a removed shortcut, a replaced one, an added one; another view keeps its defaults.
        let mut overrides = CommandOverrides::default();
        let custom = overrides.view.entry("folder-compare".into()).or_default();
        custom.command.insert(
            "DiffNext".into(),
            hikasa_session::settings::CommandOverride {
                shortcuts: Some(vec!["Alt+N".into()]),
                ..Default::default()
            },
        );
        custom.command.insert(
            "CloseTab".into(),
            hikasa_session::settings::CommandOverride {
                shortcuts: Some(Vec::new()),
                ..Default::default()
            },
        );
        custom.command.insert(
            "ShowLog".into(),
            hikasa_session::settings::CommandOverride {
                shortcuts: Some(vec!["Ctrl+L".into()]),
                ..Default::default()
            },
        );
        let custom = CommandTables::new(&overrides);
        assert_eq!(run(&custom, Scope::Folder, "Ctrl+N", FocusArea::Root), None);
        assert_eq!(
            run(&custom, Scope::Folder, "Alt+N", FocusArea::Root),
            Some(CommandId::DiffNext)
        );
        assert_eq!(run(&custom, Scope::Folder, "Ctrl+W", FocusArea::Root), None);
        assert_eq!(
            run(&custom, Scope::Folder, "Ctrl+L", FocusArea::Root),
            Some(CommandId::ShowLog)
        );
        assert_eq!(
            run(&custom, Scope::Text, "Ctrl+W", FocusArea::Root),
            Some(CommandId::CloseTab)
        );
        // Planned commands never take a key (Find is Ctrl+F in the catalogue, implemented in Home only).
        assert_eq!(run(&tables, Scope::Folder, "Ctrl+F", FocusArea::Root), None);
        assert_eq!(
            run(&tables, Scope::Home, "Ctrl+F", FocusArea::Root),
            Some(CommandId::Find)
        );
        // M2 W6d: Del and Shift+Del delete the selected row of the focused folder tree; a read-only side takes no key.
        let row = Ctx {
            row_selected: true,
            row_on: [true, true],
            writable: [true, true],
            ..ctx(Scope::Folder)
        };
        let at = |ctx: &Ctx, key: &str, area| tables.dispatch(Scope::Folder, ctx, &sc(key), area);
        assert_eq!(
            at(&row, "Del", FocusArea::ViewMain),
            Some(CommandId::Delete)
        );
        assert_eq!(
            at(&row, "Shift+Del", FocusArea::ViewMain),
            Some(CommandId::PermanentDelete)
        );
        assert_eq!(at(&row, "Del", FocusArea::Root), None);
        assert_eq!(
            at(&row, "Ctrl+R", FocusArea::Root),
            Some(CommandId::CopyRight)
        );
        let read_only = Ctx {
            writable: [false, false],
            ..row
        };
        assert_eq!(at(&read_only, "Del", FocusArea::ViewMain), None);
        assert_eq!(at(&read_only, "Ctrl+R", FocusArea::Root), None);
        let right_only = Ctx {
            writable: [true, false],
            ..row
        };
        assert_eq!(at(&right_only, "Ctrl+R", FocusArea::Root), None);
        assert_eq!(
            at(&right_only, "Ctrl+L", FocusArea::Root),
            Some(CommandId::CopyLeft)
        );
        // The tree's keys belong to the tree (W4b).
        let tree = Ctx {
            tree_named: true,
            ..ctx(Scope::Home)
        };
        for (key, id) in [
            ("Enter", CommandId::TreeOpen),
            ("Del", CommandId::TreeDelete),
            ("F2", CommandId::TreeRename),
        ] {
            let at = |area| tables.dispatch(Scope::Home, &tree, &sc(key), area);
            assert_eq!(at(FocusArea::SessionsTree), Some(id), "{key}");
            assert_eq!(at(FocusArea::Root), None, "{key}");
        }
    }

    #[test]
    fn focus_bound_keys() {
        for key in [
            "Enter",
            "Right",
            "A",
            "7",
            "/",
            "Shift+Tab",
            "Space",
            "PgDn",
            "Esc",
        ] {
            assert!(is_focus_bound(&sc(key)), "{key}");
        }
        for key in ["Ctrl+A", "F5", "Alt+Left", "Cmd+Q", "Num +", "Ctrl+Num 0"] {
            assert!(!is_focus_bound(&sc(key)), "{key}");
        }
    }

    /// A4.3: hint text with and without `show_shortcut_in_hints`, timing from `hint_show_ms`/`hint_hide_ms`.
    #[test]
    fn hints_text_and_timing() {
        let tables = CommandTables::default();
        let (caption, shortcut, description) = (
            tables.caption(Scope::Text, CommandId::Reload),
            tables.shortcut(Scope::Text, CommandId::Reload),
            tables.description(Scope::Text, CommandId::Reload),
        );
        assert_eq!(
            hint_text(&caption, shortcut, &description, true),
            format!("Reload Files (F5)\n{description}")
        );
        assert_eq!(
            hint_text(&caption, shortcut, &description, false),
            format!("Reload Files\n{description}")
        );
        assert_eq!(hint_text("X", None, "", true), "X");
        let mut timing = HintTiming::default();
        assert_eq!((timing.show_ms, timing.hide_ms), (500, 5000));
        assert!(!timing.visible(0.4));
        assert!(timing.visible(0.5));
        assert!(timing.visible(5.4));
        assert!(!timing.visible(5.5));
        timing.hide_ms = 0;
        assert!(timing.visible(3600.0));
        timing.enabled = false;
        assert!(!timing.visible(1.0));
    }
}
