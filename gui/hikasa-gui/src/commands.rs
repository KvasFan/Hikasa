//! The window's commands (ARCH-070, UI-003; M2 W4a, D-109…D-111): the GUI keeps per command id only its
//! *presentation* — icon, short toolbar label, radio or check group, availability predicate and the context-menu
//! areas — in [`PRESENTATION`]. Captions, descriptions (hints), menus, toolbar flags and shortcuts come from the
//! core's effective command table (`hikasa_session::commands::effective`, the catalogue with `commands.toml`
//! applied), rebuilt into [`CommandTables`] at start and when `commands.toml` changes, never per frame. Key
//! presses are dispatched by [`dispatch`] (D-111); hints are [`hint_text`] and [`HintTiming`] (UI-007).
//!
//! This module holds no toolkit type (§1.4.16 rule 4); `ui/` draws what it describes.

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
}

impl Scope {
    /// The view types the GUI shows.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const VIEWS: [Scope; 3] = [Scope::Home, Scope::Folder, Scope::Text];

    /// The view type's name in the catalogue and `commands.toml` (`home`, `folder-compare`, `text-compare`;
    /// `Global` has none: its commands are in every view's table).
    pub fn view_name(self) -> Option<&'static str> {
        match self {
            Scope::Global => None,
            Scope::Home => Some("home"),
            Scope::Folder => Some("folder-compare"),
            Scope::Text => Some("text-compare"),
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
}

/// Availability predicates (ARCH-070 `enabled_when`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Predicate {
    /// Always available.
    Always,
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
}

impl Predicate {
    /// Evaluates the predicate.
    pub fn holds(self, ctx: &Ctx) -> bool {
        match self {
            Predicate::Always => true,
            Predicate::MultipleTabs => ctx.tabs > 1,
            Predicate::OtherWindows => ctx.windows > 1,
            Predicate::Busy => ctx.busy,
            Predicate::Ready => ctx.ready,
            Predicate::ClosedFolderSelected => ctx.closed_folder_selected,
            Predicate::OpenFolderSelected => ctx.open_folder_selected,
            Predicate::FolderSelected => ctx.closed_folder_selected || ctx.open_folder_selected,
            Predicate::PairSelected => ctx.pair_selected,
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
        }
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

/// The presentation table, one row per (scope, id). A view type's row wins over a global one.
pub const PRESENTATION: &[Row] = &[
    // Global
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
];

/// The presentation row of `id` in `scope` (or the global row).
pub fn lookup(scope: Scope, id: CommandId) -> Option<&'static Row> {
    PRESENTATION
        .iter()
        .find(|r| r.id == id && r.scope == scope && scope != Scope::Global)
        .or_else(|| {
            PRESENTATION
                .iter()
                .find(|r| r.id == id && r.scope == Scope::Global)
        })
}

/// The presentation rows of a view type: its own, then the global ones.
pub fn rows_for(scope: Scope) -> impl Iterator<Item = &'static Row> {
    let own = PRESENTATION
        .iter()
        .filter(move |r| r.scope == scope && scope != Scope::Global);
    let global = PRESENTATION.iter().filter(|r| r.scope == Scope::Global);
    own.chain(global)
}

/// The effective command tables of the view types the GUI shows (D-109), built from `commands.toml`.
#[derive(Debug, Clone)]
pub struct CommandTables {
    home: ViewCommands<'static>,
    folder: ViewCommands<'static>,
    text: ViewCommands<'static>,
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
        }
    }

    /// The table of a view type (`Global`: the Home table, which holds every global command).
    pub fn view(&self, scope: Scope) -> &ViewCommands<'static> {
        match scope {
            Scope::Global | Scope::Home => &self.home,
            Scope::Folder => &self.folder,
            Scope::Text => &self.text,
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
            return caption.to_owned();
        }
        lookup(scope, id)
            .and_then(|r| r.toolbar_label)
            .unwrap_or(id.name())
            .to_owned()
    }

    /// The description (hint) of `id`.
    pub fn description(&self, scope: Scope, id: CommandId) -> &'static str {
        self.entry(scope, id).map_or("", |e| e.description)
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
            hint_text(&caption, shortcut, description, true),
            format!("Reload Files (F5)\n{description}")
        );
        assert_eq!(
            hint_text(&caption, shortcut, description, false),
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
