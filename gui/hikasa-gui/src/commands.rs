//! The command catalogue (ARCH-070, UI-003, docs/14): every user action of the window is a [`Command`] with a
//! stable id (the ids of `docs/reference/commands.md` where the reference has one), a menu, a caption, a hint, a
//! default shortcut, menu and toolbar visibility and an availability predicate. Menus, the toolbar and the
//! keyboard are generated from this table; the view models execute the ids (docs/17 §3.4: view logic is tested
//! through the catalogue).

use std::fmt;

/// The view type a command belongs to (commands are registered per view type, ARCH-070).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
    /// The sections of the core catalogue (`hikasa_session::commands`) where this view type's commands live, own
    /// section first (M2 W0).
    pub fn catalogue_sections(self) -> &'static [&'static str] {
        match self {
            Scope::Global => &["global"],
            Scope::Home => &["home", "global"],
            Scope::Folder => &["folder-compare", "folder-shared", "session", "global"],
            Scope::Text => &["text-compare", "file-shared", "session", "global"],
        }
    }

    /// The help topic of this view type (the core catalogue's section topic; F1, UI-006).
    pub fn help_topic(self) -> &'static str {
        let key = self.catalogue_sections()[0];
        hikasa_session::commands::catalogue().section(key).map_or(
            hikasa_session::help::INDEX_TOPIC,
            |s| {
                hikasa_session::help::topic(&s.help)
                    .map_or(hikasa_session::help::INDEX_TOPIC, |t| t.id)
            },
        )
    }
}

/// Menus of the menu bar (UI-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Menu {
    /// Session.
    Session,
    /// Edit.
    Edit,
    /// Actions.
    Actions,
    /// Search.
    Search,
    /// View.
    View,
    /// Help.
    Help,
}

impl Menu {
    /// Menus in menu-bar order.
    pub const ORDER: [Menu; 6] = [
        Menu::Session,
        Menu::Edit,
        Menu::Actions,
        Menu::Search,
        Menu::View,
        Menu::Help,
    ];

    /// Caption in the menu bar.
    pub fn caption(self) -> &'static str {
        match self {
            Menu::Session => "Session",
            Menu::Edit => "Edit",
            Menu::Actions => "Actions",
            Menu::Search => "Search",
            Menu::View => "View",
            Menu::Help => "Help",
        }
    }
}

/// A key of a shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A letter or digit (upper case).
    Char(char),
    /// A function key F1…F12.
    F(u8),
    /// Enter / Return.
    Enter,
    /// Tab.
    Tab,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Numeric keypad plus.
    NumPlus,
    /// Numeric keypad minus.
    NumMinus,
    /// Numeric keypad zero.
    NumZero,
}

/// A key with modifiers (Ctrl maps to Cmd on macOS in a later milestone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// Ctrl held.
    pub ctrl: bool,
    /// Shift held.
    pub shift: bool,
    /// Alt held.
    pub alt: bool,
    /// The key.
    pub key: Key,
}

impl Shortcut {
    /// The key alone.
    pub const fn plain(key: Key) -> Shortcut {
        Shortcut {
            ctrl: false,
            shift: false,
            alt: false,
            key,
        }
    }

    /// Ctrl + key.
    pub const fn ctrl(key: Key) -> Shortcut {
        Shortcut {
            ctrl: true,
            shift: false,
            alt: false,
            key,
        }
    }

    /// Shift + Ctrl + key.
    pub const fn shift_ctrl(key: Key) -> Shortcut {
        Shortcut {
            ctrl: true,
            shift: true,
            alt: false,
            key,
        }
    }

    /// A key that focused controls use themselves (Enter activates a button, the arrows move in a list or a
    /// text field, letters are typed), pressed without Ctrl or Alt. Such a shortcut belongs to the view's own
    /// focus area (the folder tree), never to the window root (UI-003).
    pub const fn is_focus_bound(self) -> bool {
        !self.ctrl
            && !self.alt
            && matches!(
                self.key,
                Key::Enter | Key::Tab | Key::Up | Key::Down | Key::Left | Key::Right | Key::Char(_)
            )
    }
}

/// Where a key event is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOrigin {
    /// The window root: the event bubbled up from whatever has the focus (a button, a list, a text field).
    Window,
    /// The view's own focus area (the folder tree has the focus).
    View,
}

impl fmt::Display for Shortcut {
    /// `Shift+Ctrl+N`, the reference spelling (docs/reference/commands.md).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Enter => f.write_str("Enter"),
            Key::Tab => f.write_str("Tab"),
            Key::Up => f.write_str("Up"),
            Key::Down => f.write_str("Down"),
            Key::Left => f.write_str("Left"),
            Key::Right => f.write_str("Right"),
            Key::NumPlus => f.write_str("Num +"),
            Key::NumMinus => f.write_str("Num -"),
            Key::NumZero => f.write_str("Num 0"),
        }
    }
}

macro_rules! command_ids {
    ($($(#[$doc:meta])* $id:ident),* $(,)?) => {
        /// Stable command ids (the reference ids of `docs/reference/commands.md`; `HomeCompare` is the Home
        /// view's Compare button, which the reference does not list).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum CommandId {
            $($(#[$doc])* $id,)*
        }

        impl CommandId {
            /// Every id.
            #[cfg(test)]
            pub const ALL: &'static [CommandId] = &[$(CommandId::$id,)*];

            /// The id as written in the reference (`DiffNext`, `FavCompareAll`, …).
            #[cfg(test)]
            pub fn name(self) -> &'static str {
                match self {
                    $(CommandId::$id => stringify!($id),)*
                }
            }
        }
    };
}

command_ids! {
    /// Opens a Home view in a new tab.
    TabNew,
    /// Closes the active tab (its view).
    CloseTab,
    /// Quits the application.
    Exit,
    /// Selects the next tab.
    TabNext,
    /// Selects the previous tab.
    TabPrev,
    /// Program information.
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
}

/// What the active view offers, for [`Predicate::holds`] (computed by the app from the active view model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ctx {
    /// The active view's type.
    pub scope: Scope,
    /// Open tabs.
    pub tabs: usize,
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
}

/// Availability predicates (ARCH-070 `enabled_when`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Predicate {
    /// Always available.
    Always,
    /// More than one tab is open.
    MultipleTabs,
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
}

impl Predicate {
    /// Evaluates the predicate.
    pub fn holds(self, ctx: &Ctx) -> bool {
        match self {
            Predicate::Always => true,
            Predicate::MultipleTabs => ctx.tabs > 1,
            Predicate::Busy => ctx.busy,
            Predicate::Ready => ctx.ready,
            Predicate::ClosedFolderSelected => ctx.closed_folder_selected,
            Predicate::OpenFolderSelected => ctx.open_folder_selected,
            Predicate::FolderSelected => ctx.closed_folder_selected || ctx.open_folder_selected,
            Predicate::PairSelected => ctx.pair_selected,
            Predicate::NextDifference => ctx.next_difference,
            Predicate::PrevDifference => ctx.prev_difference,
            Predicate::HomeReady => ctx.home_ready,
        }
    }
}

/// One catalogue entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// Id.
    pub id: CommandId,
    /// View type.
    pub scope: Scope,
    /// Menu.
    pub menu: Menu,
    /// Caption (menus, toolbar tooltips).
    pub caption: &'static str,
    /// Hint (status line, tooltips).
    pub hint: &'static str,
    /// Default shortcut.
    pub shortcut: Option<Shortcut>,
    /// Shown in its menu.
    pub in_menu: bool,
    /// Shown on the toolbar (with this short label).
    pub toolbar: Option<&'static str>,
    /// Radio group (0 = none): display filters (1), folder modes (2), toggles (3).
    pub group: u8,
    /// Availability.
    pub enabled_when: Predicate,
}

/// Radio group of the display filter presets.
pub const GROUP_FILTER: u8 = 1;
/// Radio group of the folder modes.
pub const GROUP_FOLDER_MODE: u8 = 2;
/// Check group of the atomic display toggles.
pub const GROUP_TOGGLES: u8 = 3;

#[allow(clippy::too_many_arguments)] // one positional row per catalogue entry
const fn cmd(
    id: CommandId,
    scope: Scope,
    menu: Menu,
    caption: &'static str,
    hint: &'static str,
    shortcut: Option<Shortcut>,
    toolbar: Option<&'static str>,
    group: u8,
    enabled_when: Predicate,
) -> Command {
    Command {
        id,
        scope,
        menu,
        caption,
        hint,
        shortcut,
        in_menu: true,
        toolbar,
        group,
        enabled_when,
    }
}

use CommandId as C;
use Key as K;
use Menu as M;
use Predicate as P;
use Scope as S;

/// The catalogue (per view type; a view type's entry wins over a global one with the same shortcut).
pub const CATALOGUE: &[Command] = &[
    // Global (Session, Help)
    cmd(
        C::TabNew,
        S::Global,
        M::Session,
        "New Tab",
        "Opens a Home view in a new tab.",
        Some(Shortcut::ctrl(K::Char('T'))),
        None,
        0,
        P::Always,
    ),
    cmd(
        C::CloseTab,
        S::Global,
        M::Session,
        "Close Tab",
        "Closes the active tab.",
        Some(Shortcut::ctrl(K::Char('W'))),
        None,
        0,
        P::Always,
    ),
    cmd(
        C::TabNext,
        S::Global,
        M::Session,
        "Next Tab",
        "Selects the tab after the active one.",
        Some(Shortcut::ctrl(K::Tab)),
        None,
        0,
        P::MultipleTabs,
    ),
    cmd(
        C::TabPrev,
        S::Global,
        M::Session,
        "Previous Tab",
        "Selects the tab before the active one.",
        Some(Shortcut::shift_ctrl(K::Tab)),
        None,
        0,
        P::MultipleTabs,
    ),
    cmd(
        C::Exit,
        S::Global,
        M::Session,
        "Exit",
        "Quits the application.",
        Some(Shortcut::ctrl(K::Char('Q'))),
        None,
        0,
        P::Always,
    ),
    cmd(
        C::HelpContents,
        S::Global,
        M::Help,
        "Contents",
        "Shows the contents of the help.",
        None,
        None,
        0,
        P::Always,
    ),
    cmd(
        C::ContextHelp,
        S::Global,
        M::Help,
        "Context Sensitive Help",
        "Shows the help topic of the active view.",
        Some(Shortcut::plain(K::F(1))),
        None,
        0,
        P::Always,
    ),
    cmd(
        C::About,
        S::Global,
        M::Help,
        "About",
        "Displays program information and the version number.",
        None,
        None,
        0,
        P::Always,
    ),
    // Home
    cmd(
        C::HomeCompare,
        S::Home,
        M::Session,
        "Compare",
        "Compares the two paths; the comparison replaces the Home view.",
        None,
        Some("Compare"),
        0,
        P::HomeReady,
    ),
    // Folder Compare
    cmd(
        C::Stop,
        S::Folder,
        M::Session,
        "Stop",
        "Terminates the comparison.",
        None,
        Some("Stop"),
        0,
        P::Busy,
    ),
    cmd(
        C::FullRefresh,
        S::Folder,
        M::Edit,
        "Full Refresh",
        "Refreshes the entire comparison.",
        Some(Shortcut::ctrl(K::F(5))),
        Some("Refresh"),
        0,
        P::Always,
    ),
    cmd(
        C::ExpandAll,
        S::Folder,
        M::Edit,
        "Expand All",
        "Opens all folders.",
        None,
        Some("Expand"),
        0,
        P::Ready,
    ),
    cmd(
        C::CollapseAll,
        S::Folder,
        M::Edit,
        "Collapse All",
        "Closes all folders.",
        None,
        Some("Collapse"),
        0,
        P::Ready,
    ),
    cmd(
        C::Open,
        S::Folder,
        M::Actions,
        "Open",
        "Opens the selected file pair in a Text Compare view.",
        Some(Shortcut::plain(K::Enter)),
        Some("Open"),
        0,
        P::PairSelected,
    ),
    cmd(
        C::Expand,
        S::Folder,
        M::Actions,
        "Open Folder",
        "Opens the selected folder.",
        Some(Shortcut::plain(K::Right)),
        None,
        0,
        P::ClosedFolderSelected,
    ),
    cmd(
        C::Collapse,
        S::Folder,
        M::Actions,
        "Close Folder",
        "Closes the selected folder.",
        Some(Shortcut::plain(K::Left)),
        None,
        0,
        P::OpenFolderSelected,
    ),
    cmd(
        C::ExpandSubfolders,
        S::Folder,
        M::Actions,
        "Open Subfolders",
        "Opens the selected folder and all its subfolders.",
        None,
        None,
        0,
        P::FolderSelected,
    ),
    cmd(
        C::CollapseSubfolders,
        S::Folder,
        M::Actions,
        "Close Subfolders",
        "Closes the selected folder and its subfolders.",
        None,
        None,
        0,
        P::FolderSelected,
    ),
    cmd(
        C::DiffNext,
        S::Folder,
        M::Search,
        "Next Difference",
        "Positions at the next difference.",
        Some(Shortcut::ctrl(K::Char('N'))),
        Some("Next"),
        0,
        P::NextDifference,
    ),
    cmd(
        C::DiffPrev,
        S::Folder,
        M::Search,
        "Previous Difference",
        "Positions at the previous difference.",
        Some(Shortcut::ctrl(K::Char('P'))),
        Some("Prev"),
        0,
        P::PrevDifference,
    ),
    cmd(
        C::FavCompareAll,
        S::Folder,
        M::View,
        "Show All",
        "Shows same, orphan, newer and different items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavDiff,
        S::Folder,
        M::View,
        "Show Differences",
        "Shows orphan, newer and different items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavSame,
        S::Folder,
        M::View,
        "Show Same",
        "Shows same items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavNoOrphan,
        S::Folder,
        M::View,
        "Show No Orphans",
        "Shows same, newer and different items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavDiffNoOrphan,
        S::Folder,
        M::View,
        "Show Differences but No Orphans",
        "Shows newer and different items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavOrphan,
        S::Folder,
        M::View,
        "Show Orphans",
        "Shows left and right orphan items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavLeftNewer,
        S::Folder,
        M::View,
        "Show Left Newer",
        "Shows left newer items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavRightNewer,
        S::Folder,
        M::View,
        "Show Right Newer",
        "Shows right newer items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavLeftNewerOrphan,
        S::Folder,
        M::View,
        "Show Left Newer and Left Orphans",
        "Shows left orphan and left newer items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavRightNewerOrphan,
        S::Folder,
        M::View,
        "Show Right Newer and Right Orphans",
        "Shows right orphan and right newer items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavLeftOrphan,
        S::Folder,
        M::View,
        "Show Left Orphans",
        "Shows left orphan items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavRightOrphan,
        S::Folder,
        M::View,
        "Show Right Orphans",
        "Shows right orphan items.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavCompareNone,
        S::Folder,
        M::View,
        "Show None",
        "Hides all files.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::TogSame,
        S::Folder,
        M::View,
        "Toggle Same Items",
        "Shows or hides same items.",
        None,
        Some("="),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogLeftOrphan,
        S::Folder,
        M::View,
        "Toggle Left Orphan Items",
        "Shows or hides left orphan items.",
        None,
        Some("◧"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogRightOrphan,
        S::Folder,
        M::View,
        "Toggle Right Orphan Items",
        "Shows or hides right orphan items.",
        None,
        Some("◨"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogLeftNewer,
        S::Folder,
        M::View,
        "Toggle Left Newer Items",
        "Shows or hides left newer items.",
        None,
        Some("◀"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogRightNewer,
        S::Folder,
        M::View,
        "Toggle Right Newer Items",
        "Shows or hides right newer items.",
        None,
        Some("▶"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogDiff,
        S::Folder,
        M::View,
        "Toggle Difference Items",
        "Shows or hides different, unknown and error items.",
        None,
        Some("≠"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::DirShowAll,
        S::Folder,
        M::View,
        "Always Show Folders",
        "Shows all folders, ignoring the display filter.",
        None,
        None,
        GROUP_FOLDER_MODE,
        P::Always,
    ),
    cmd(
        C::DirCompareStructure,
        S::Folder,
        M::View,
        "Compare Files and Folder Structure",
        "Shows folders if they, or items they contain, match the display filter.",
        None,
        None,
        GROUP_FOLDER_MODE,
        P::Always,
    ),
    cmd(
        C::DirCompareFilesOnly,
        S::Folder,
        M::View,
        "Only Compare Files",
        "Shows folders only if they contain files matching the display filter.",
        None,
        None,
        GROUP_FOLDER_MODE,
        P::Always,
    ),
    cmd(
        C::InsigIgnored,
        S::Folder,
        M::View,
        "Ignore Unimportant Differences",
        "Treats unimportant differences as same.",
        None,
        None,
        0,
        P::Always,
    ),
    cmd(
        C::SuppressFilters,
        S::Folder,
        M::View,
        "Suppress Filters",
        "Temporarily shows the items hidden by file filters.",
        None,
        None,
        0,
        P::Always,
    ),
    // Text Compare
    cmd(
        C::Reload,
        S::Text,
        M::Session,
        "Reload Files",
        "Reloads and recompares the displayed files.",
        Some(Shortcut::plain(K::F(5))),
        Some("Reload"),
        0,
        P::Always,
    ),
    cmd(
        C::DiffSectionNext,
        S::Text,
        M::Search,
        "Next Difference Section",
        "Positions at the next difference section.",
        Some(Shortcut::ctrl(K::Char('N'))),
        Some("Next"),
        0,
        P::NextDifference,
    ),
    cmd(
        C::DiffSectionPrev,
        S::Text,
        M::Search,
        "Previous Difference Section",
        "Positions at the previous difference section.",
        Some(Shortcut::ctrl(K::Char('P'))),
        Some("Prev"),
        0,
        P::PrevDifference,
    ),
    cmd(
        C::FavCompareAll,
        S::Text,
        M::View,
        "Show All",
        "Shows same, orphan and different lines.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavDiff,
        S::Text,
        M::View,
        "Show Differences",
        "Shows orphan and different lines.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavSame,
        S::Text,
        M::View,
        "Show Same",
        "Shows same lines.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::ShowContext,
        S::Text,
        M::View,
        "Show Context",
        "Shows the differences with the lines surrounding them.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::FavCompareNone,
        S::Text,
        M::View,
        "Show None",
        "Shows no lines.",
        None,
        None,
        GROUP_FILTER,
        P::Always,
    ),
    cmd(
        C::TogSame,
        S::Text,
        M::View,
        "Toggle Same Lines",
        "Shows or hides same lines.",
        None,
        Some("="),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogLeftOrphan,
        S::Text,
        M::View,
        "Toggle Left Orphan Lines",
        "Shows or hides left orphan lines.",
        None,
        Some("◧"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogRightOrphan,
        S::Text,
        M::View,
        "Toggle Right Orphan Lines",
        "Shows or hides right orphan lines.",
        None,
        Some("◨"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::TogDiff,
        S::Text,
        M::View,
        "Toggle Difference Lines",
        "Shows or hides different lines.",
        None,
        Some("≠"),
        GROUP_TOGGLES,
        P::Always,
    ),
    cmd(
        C::InsigIgnored,
        S::Text,
        M::View,
        "Ignore Unimportant Differences",
        "Treats unimportant differences as same.",
        None,
        None,
        0,
        P::Always,
    ),
    cmd(
        C::ShowThumbnail,
        S::Text,
        M::View,
        "Thumbnail",
        "Shows or hides the thumbnail.",
        None,
        None,
        0,
        P::Always,
    ),
    cmd(
        C::DisplayFontIncrease,
        S::Text,
        M::View,
        "Increase Display Font Size",
        "Makes the text larger.",
        Some(Shortcut::ctrl(K::NumPlus)),
        None,
        0,
        P::Always,
    ),
    cmd(
        C::DisplayFontDecrease,
        S::Text,
        M::View,
        "Decrease Display Font Size",
        "Makes the text smaller.",
        Some(Shortcut::ctrl(K::NumMinus)),
        None,
        0,
        P::Always,
    ),
    cmd(
        C::DisplayFontReset,
        S::Text,
        M::View,
        "Reset Display Font Size",
        "Restores the default text size.",
        Some(Shortcut::ctrl(K::NumZero)),
        None,
        0,
        P::Always,
    ),
];

/// The commands of a view type: its own entries, then the global ones.
pub fn commands_for(scope: Scope) -> impl Iterator<Item = &'static Command> {
    let own = CATALOGUE
        .iter()
        .filter(move |c| c.scope == scope && scope != Scope::Global);
    let global = CATALOGUE.iter().filter(|c| c.scope == Scope::Global);
    own.chain(global)
}

/// The command a view type binds to `shortcut` (its own binding wins over a global one).
pub fn find_shortcut(scope: Scope, shortcut: Shortcut) -> Option<&'static Command> {
    commands_for(scope).find(|c| c.shortcut == Some(shortcut))
}

/// The command a key event runs where it is handled: the window root takes every shortcut except the
/// focus-bound ones ([`Shortcut::is_focus_bound`]), which only the view's focus area takes, so plain Enter, Left
/// and Right still reach a focused button, list or check box. The caller runs the command (and prevents the
/// key's default action) only when the command is enabled.
pub fn shortcut_command(
    scope: Scope,
    shortcut: Shortcut,
    origin: KeyOrigin,
) -> Option<&'static Command> {
    let here = match origin {
        KeyOrigin::Window => !shortcut.is_focus_bound(),
        KeyOrigin::View => shortcut.is_focus_bound(),
    };
    if here {
        find_shortcut(scope, shortcut)
    } else {
        None
    }
}

/// The entry of `id` in `scope` (or the global entry).
pub fn lookup(scope: Scope, id: CommandId) -> Option<&'static Command> {
    commands_for(scope).find(|c| c.id == id)
}

/// The commands of one menu for a view type, in catalogue order.
pub fn menu_items(scope: Scope, menu: Menu) -> Vec<&'static Command> {
    commands_for(scope)
        .filter(|c| c.menu == menu && c.in_menu)
        .collect()
}

/// The toolbar of a view type, in catalogue order.
pub fn toolbar(scope: Scope) -> Vec<&'static Command> {
    commands_for(scope)
        .filter(|c| c.scope == scope && c.toolbar.is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_per_scope_and_shortcuts_do_not_clash() {
        for scope in [Scope::Home, Scope::Folder, Scope::Text] {
            let mut ids = HashSet::new();
            let mut keys = HashSet::new();
            for c in CATALOGUE.iter().filter(|c| c.scope == scope) {
                assert!(ids.insert(c.id), "{:?} twice in {scope:?}", c.id);
                if let Some(s) = c.shortcut {
                    assert!(keys.insert(s), "{s} bound twice in {scope:?}");
                }
            }
            // A view's own binding never hides a global command by accident.
            for g in CATALOGUE.iter().filter(|c| c.scope == Scope::Global) {
                if let Some(s) = g.shortcut {
                    assert!(
                        !keys.contains(&s),
                        "{s} of {:?} is shadowed in {scope:?}",
                        g.id
                    );
                }
            }
        }
    }

    #[test]
    fn every_command_has_text_and_a_scope_entry() {
        for c in CATALOGUE {
            assert!(!c.caption.is_empty() && !c.hint.is_empty(), "{:?}", c.id);
            assert!(c.hint.ends_with('.'), "{:?}", c.id);
        }
        for id in CommandId::ALL {
            assert!(
                CATALOGUE.iter().any(|c| c.id == *id),
                "{} is not in the catalogue",
                id.name()
            );
        }
    }

    /// M2 W0: every command the window executes is an `implemented` entry of the core catalogue (the source of
    /// `docs/reference/commands.md`) with the same default shortcut, and every `implemented` entry of a section
    /// the window shows is executed by the window.
    #[test]
    fn the_window_matches_the_core_catalogue() {
        use hikasa_session::commands::{CommandStatus, catalogue};
        let core = catalogue();
        for c in CATALOGUE {
            let (_, entry) = core
                .find(c.scope.catalogue_sections(), c.id.name())
                .unwrap_or_else(|| {
                    panic!(
                        "{} ({:?}) is not in the core catalogue",
                        c.id.name(),
                        c.scope
                    )
                });
            assert_eq!(entry.status, CommandStatus::Implemented, "{}", c.id.name());
            // A contextual key (`Enter (in a path field)`) is handled by its widget, not bound by the catalogue.
            if !entry.shortcut.contains('(') {
                let ours = c.shortcut.map(|s| s.to_string()).unwrap_or_default();
                assert_eq!(ours, entry.shortcut, "{} in {:?}", c.id.name(), c.scope);
            }
        }
        for (section, entry) in core.entries() {
            if entry.status != CommandStatus::Implemented {
                continue;
            }
            assert!(
                CATALOGUE.iter().any(|c| c.id.name() == entry.id
                    && c.scope.catalogue_sections().contains(&section.key.as_str())),
                "{} ({}) is implemented in core but the window does not execute it",
                entry.id,
                section.key
            );
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
    fn shortcuts_display_in_the_reference_spelling() {
        assert_eq!(
            Shortcut::shift_ctrl(Key::Char('N')).to_string(),
            "Shift+Ctrl+N"
        );
        assert_eq!(Shortcut::ctrl(Key::F(5)).to_string(), "Ctrl+F5");
        assert_eq!(Shortcut::ctrl(Key::NumPlus).to_string(), "Ctrl+Num +");
        assert_eq!(CommandId::DiffNext.name(), "DiffNext");
    }

    #[test]
    fn lookups_prefer_the_view_type() {
        let next = Shortcut::ctrl(Key::Char('N'));
        assert_eq!(
            find_shortcut(Scope::Folder, next).map(|c| c.id),
            Some(CommandId::DiffNext)
        );
        assert_eq!(
            find_shortcut(Scope::Text, next).map(|c| c.id),
            Some(CommandId::DiffSectionNext)
        );
        assert_eq!(find_shortcut(Scope::Home, next), None);
        let close = Shortcut::ctrl(Key::Char('W'));
        for scope in [Scope::Home, Scope::Folder, Scope::Text] {
            assert_eq!(
                find_shortcut(scope, close).map(|c| c.id),
                Some(CommandId::CloseTab)
            );
        }
        assert_eq!(lookup(Scope::Text, CommandId::Stop), None);
        assert!(menu_items(Scope::Folder, Menu::View).len() > 20);
        assert!(toolbar(Scope::Text).iter().all(|c| c.scope == Scope::Text));
    }

    #[test]
    fn plain_keys_belong_to_the_focus_area_not_to_the_window() {
        let enter = Shortcut::plain(Key::Enter);
        let right = Shortcut::plain(Key::Right);
        let left = Shortcut::plain(Key::Left);
        for key in [enter, right, left] {
            assert!(key.is_focus_bound(), "{key}");
            assert_eq!(
                shortcut_command(Scope::Folder, key, KeyOrigin::Window),
                None
            );
        }
        let at_tree = |s| shortcut_command(Scope::Folder, s, KeyOrigin::View).map(|c| c.id);
        assert_eq!(at_tree(enter), Some(CommandId::Open));
        assert_eq!(at_tree(right), Some(CommandId::Expand));
        assert_eq!(at_tree(left), Some(CommandId::Collapse));
        // Modified keys and function keys work wherever the focus is, and only once.
        let next = Shortcut::ctrl(Key::Char('N'));
        assert!(!next.is_focus_bound() && !Shortcut::plain(Key::F(5)).is_focus_bound());
        assert_eq!(
            shortcut_command(Scope::Folder, next, KeyOrigin::Window).map(|c| c.id),
            Some(CommandId::DiffNext)
        );
        assert_eq!(at_tree(next), None);
        assert!(
            shortcut_command(Scope::Text, Shortcut::plain(Key::F(5)), KeyOrigin::Window).is_some()
        );
        // Every catalogue shortcut is reachable from exactly one place.
        for c in CATALOGUE {
            if let Some(s) = c.shortcut {
                let window = shortcut_command(c.scope, s, KeyOrigin::Window).is_some();
                let view = shortcut_command(c.scope, s, KeyOrigin::View).is_some();
                assert!(window != view, "{:?} ({s})", c.id);
            }
        }
    }

    #[test]
    fn predicates_follow_the_context() {
        let mut ctx = Ctx {
            tabs: 1,
            ..Ctx::default()
        };
        assert!(Predicate::Always.holds(&ctx));
        assert!(!Predicate::MultipleTabs.holds(&ctx));
        ctx.tabs = 2;
        assert!(Predicate::MultipleTabs.holds(&ctx));
        assert!(!Predicate::FolderSelected.holds(&ctx));
        ctx.open_folder_selected = true;
        assert!(Predicate::FolderSelected.holds(&ctx));
        assert!(Predicate::OpenFolderSelected.holds(&ctx));
        assert!(!Predicate::ClosedFolderSelected.holds(&ctx));
        assert!(!Predicate::PairSelected.holds(&ctx));
        assert!(!Predicate::Busy.holds(&ctx) && !Predicate::Ready.holds(&ctx));
        assert!(!Predicate::NextDifference.holds(&ctx) && !Predicate::PrevDifference.holds(&ctx));
        assert!(!Predicate::HomeReady.holds(&ctx));
    }
}
