//! Menu, toolbar and context-menu assembly (M2 W4a, D-110; UI-002, UI-004, UI-005): deterministic lists derived
//! from the effective command table, drawn by `ui/menus.rs` and tested as text ([`render_text`]).
//!
//! Menu bar: Session, File, Edit, Actions, Search, View, Tools, Help; a menu without a visible entry is hidden.
//! Inside a menu the entries are grouped by catalogue section in `view_sections` order (own section first,
//! `global` last) with a separator between groups, catalogue order inside a group; the Session menu puts the
//! `global` entries before `CloseTab` first, then the view's groups, then `CloseTab` and `Exit`. An entry is
//! visible when it is `implemented`, the GUI has a row for it, it has a caption and a menu, and its effective
//! menu flag (or `show_all_in_menu`) is set. `MacWindow` entries are keyboard-only; the pseudo-menus `DropDown`,
//! `Favorites` and `Toggles` make the View ▸ Display Filter submenu.

use hikasa_session::commands::EffectiveCommand;
use hikasa_session::sessions::{SessionName, SessionType};
use hikasa_session::settings::commands::DisplayFilterMode;

use crate::commands::{CommandId, CommandTables, Ctx, Group, Scope, lookup};
use crate::folder::columns::ColumnId;
use crate::window::{CommandArg, Invocation, WindowId};

/// The session types the GUI can open, with their captions (`SessionNewList`, Home's type buttons).
pub const OPENABLE_TYPES: [(SessionType, &str); 6] = [
    (SessionType::FolderCompare, "Folder Compare"),
    (SessionType::FolderSync, "Folder Sync"),
    (SessionType::TextCompare, "Text Compare"),
    (SessionType::TableCompare, "Table Compare"),
    (SessionType::TextEdit, "Text Edit"),
    (SessionType::PictureCompare, "Picture Compare"),
];

/// The menus of the menu bar, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuTitle {
    /// Session.
    Session,
    /// File.
    File,
    /// Edit.
    Edit,
    /// Actions.
    Actions,
    /// Search.
    Search,
    /// View.
    View,
    /// Tools.
    Tools,
    /// Help.
    Help,
}

impl MenuTitle {
    /// Menu-bar order.
    pub const ORDER: [MenuTitle; 8] = [
        MenuTitle::Session,
        MenuTitle::File,
        MenuTitle::Edit,
        MenuTitle::Actions,
        MenuTitle::Search,
        MenuTitle::View,
        MenuTitle::Tools,
        MenuTitle::Help,
    ];

    /// The caption (also the catalogue's `menu` value).
    pub fn caption(self) -> &'static str {
        match self {
            MenuTitle::Session => "Session",
            MenuTitle::File => "File",
            MenuTitle::Edit => "Edit",
            MenuTitle::Actions => "Actions",
            MenuTitle::Search => "Search",
            MenuTitle::View => "View",
            MenuTitle::Tools => "Tools",
            MenuTitle::Help => "Help",
        }
    }
}

/// The check mark of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// A plain command.
    None,
    /// A check box.
    Toggle(bool),
    /// A radio button.
    Radio(bool),
}

/// One menu item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    /// A command.
    Command {
        /// What it runs.
        invocation: Invocation,
        /// Caption.
        caption: String,
        /// The first effective shortcut.
        shortcut: Option<String>,
        /// Check mark.
        check: Check,
        /// Available.
        enabled: bool,
        /// The hint.
        description: String,
    },
    /// A separator.
    Separator,
    /// A submenu.
    Submenu {
        /// Caption.
        caption: String,
        /// Items.
        items: Vec<MenuItem>,
    },
}

/// One menu of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// Which menu.
    pub title: MenuTitle,
    /// Items.
    pub items: Vec<MenuItem>,
}

/// Everything menu assembly needs from the window.
pub struct MenuContext<'a> {
    /// The command tables.
    pub tables: &'a CommandTables,
    /// The active view's type.
    pub scope: Scope,
    /// Availability.
    pub ctx: Ctx,
    /// The check state of a command in the active view (`None`: not checkable).
    pub checked: &'a dyn Fn(CommandId) -> Option<bool>,
    /// The saved workspaces (`WorkspaceLoadList`).
    pub workspaces: &'a [String],
    /// The columns a folder view offers and whether each shows (`ColList`, the header menu; D-234).
    pub columns: &'a [(ColumnId, bool)],
    /// The applications of `openwith.toml`, by description (Open With ▸, M2 W9b).
    pub open_with: &'a [String],
    /// The enabled text formats a text header's Format ▸ offers (FF-003; empty in other views).
    pub formats: &'a [String],
}

impl MenuContext<'_> {
    /// The id and row of an entry the GUI shows in this view.
    fn shown(&self, entry: &EffectiveCommand<'_>) -> Option<CommandId> {
        let id = CommandId::from_name(entry.id())?;
        (entry.implemented() && lookup(self.scope, id).is_some()).then_some(id)
    }

    /// The item of a command.
    pub fn item(&self, id: CommandId, arg: Option<CommandArg>) -> MenuItem {
        let row = lookup(self.scope, id);
        let enabled = !self.ctx.dialog_open && row.is_some_and(|r| r.enabled_when.holds(&self.ctx));
        let check = match (row.map(|r| r.group), (self.checked)(id)) {
            (Some(Group::Filter | Group::FolderMode), Some(on)) => Check::Radio(on),
            (_, Some(on)) => Check::Toggle(on),
            _ => Check::None,
        };
        MenuItem::Command {
            invocation: Invocation { id, arg },
            caption: self.tables.caption(self.scope, id),
            shortcut: self
                .tables
                .shortcut(self.scope, id)
                .map(ToString::to_string),
            check,
            enabled,
            description: self.tables.description(self.scope, id).to_owned(),
        }
    }

    /// The item of a menu entry: the dynamic commands of the Session menu are submenus (§1.4.3).
    fn entry_item(&self, id: CommandId) -> MenuItem {
        let plain = |invocation: Invocation, caption: String| MenuItem::Command {
            invocation,
            caption,
            shortcut: None,
            check: Check::None,
            enabled: !self.ctx.dialog_open,
            description: String::new(),
        };
        match id {
            CommandId::SessionNewList => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: OPENABLE_TYPES
                    .iter()
                    .map(|(kind, caption)| {
                        plain(
                            Invocation::with(id, CommandArg::SessionType(*kind)),
                            (*caption).to_owned(),
                        )
                    })
                    .collect(),
            },
            CommandId::WorkspaceLoadList => {
                let mut items: Vec<MenuItem> = self
                    .workspaces
                    .iter()
                    .filter_map(|name| {
                        let name = SessionName::parse(name).ok()?;
                        let caption = name.to_string();
                        Some(plain(
                            Invocation::with(id, CommandArg::Workspace(name)),
                            caption,
                        ))
                    })
                    .collect();
                if !items.is_empty() {
                    items.push(MenuItem::Separator);
                }
                items.push(self.item(CommandId::WorkspaceManager, None));
                MenuItem::Submenu {
                    caption: self.tables.caption(self.scope, id),
                    items,
                }
            }
            CommandId::OpenWithList if matches!(self.scope, Scope::Text | Scope::TextEdit) => {
                MenuItem::Submenu {
                    caption: self.tables.caption(self.scope, id),
                    items: text_open_with_items(self),
                }
            }
            CommandId::OpenWithList => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: open_with_items(self),
            },
            CommandId::ColList => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: column_items(self),
            },
            CommandId::BrowseMode => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: browse_mode_items(self),
            },
            CommandId::ZoomList => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: zoom_items(self),
            },
            CommandId::ExplorerList => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: explorer_items(self, None),
            },
            id if !crate::commands::list_children(id).is_empty() => MenuItem::Submenu {
                caption: self.tables.caption(self.scope, id),
                items: crate::commands::list_children(id)
                    .iter()
                    .filter(|c| lookup(self.scope, **c).is_some())
                    .map(|c| self.item(*c, None))
                    .collect(),
            },
            id => self.item(id, None),
        }
    }

    /// The visible entries of `menu` in the active view, as (section, id).
    fn entries(&self, menu: &str) -> Vec<(&'static str, CommandId)> {
        let table = self.tables.view(self.scope);
        let show_all = table.custom.show_all_in_menu;
        table
            .entries
            .iter()
            .filter(|e| e.menu == menu && !e.caption.is_empty() && (e.in_menu || show_all))
            .filter_map(|e| self.shown(e).map(|id| (e.section, id)))
            .filter(|(_, id)| !crate::commands::in_a_list(*id))
            .collect()
    }
}

/// The items of View ▸ Browse By ▸ (D-309): the browse modes as a radio (a Text Edit view: edits and find hits).
pub fn browse_mode_items(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    use hikasa_text::nav::BrowseMode as B;
    let enabled = !ctx.ctx.dialog_open
        && lookup(ctx.scope, CommandId::BrowseMode).is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
    crate::text::layout::BROWSE_MODES
        .iter()
        .filter(|m| ctx.scope != Scope::TextEdit || matches!(m, B::Edit | B::Find))
        .map(|mode| MenuItem::Command {
            invocation: Invocation::with(CommandId::BrowseMode, CommandArg::BrowseMode(*mode)),
            caption: crate::text::layout::browse_caption(*mode).to_owned(),
            shortcut: None,
            check: Check::Radio(ctx.ctx.browse_mode == Some(*mode)),
            enabled,
            description: ctx
                .tables
                .description(ctx.scope, CommandId::BrowseMode)
                .to_owned(),
        })
        .collect()
}

/// The items of View ▸ Zoom ▸ of a Picture Compare view (M2 W13b, D-362): Fit to Window, Actual Size, 25 %…800 %.
pub fn zoom_items(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    let enabled = !ctx.ctx.dialog_open
        && lookup(ctx.scope, CommandId::ZoomList).is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
    crate::picture::ZoomChoice::LIST
        .iter()
        .map(|choice| MenuItem::Command {
            invocation: Invocation::with(CommandId::ZoomList, CommandArg::ZoomStep(*choice)),
            caption: choice.caption(),
            shortcut: None,
            check: Check::None,
            enabled,
            description: ctx
                .tables
                .description(ctx.scope, CommandId::ZoomList)
                .to_owned(),
        })
        .collect()
}

/// The items of Open With ▸ of a text header (D-312): *Text Compare* (with the other pane's file), *Text Edit*,
/// *Table Compare* where both are tables, then the `openwith.toml` entries and the associated application.
pub fn text_open_with_items(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    use crate::window::OpenWithItem;
    let enabled = !ctx.ctx.dialog_open
        && lookup(ctx.scope, CommandId::OpenWithList)
            .is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
    let item = |what: OpenWithItem, caption: String, on: bool| MenuItem::Command {
        invocation: Invocation::with(CommandId::OpenWithList, CommandArg::OpenWith(what)),
        caption,
        shortcut: None,
        check: Check::None,
        enabled: enabled && on,
        description: ctx
            .tables
            .description(ctx.scope, CommandId::OpenWithList)
            .to_owned(),
    };
    let mut items = Vec::new();
    if ctx.ctx.two_panes {
        items.push(item(
            OpenWithItem::TextCompare,
            Scope::Text.type_title().to_owned(),
            true,
        ));
    }
    items.push(item(
        OpenWithItem::TextEdit,
        Scope::TextEdit.type_title().to_owned(),
        true,
    ));
    if !ctx.open_with.is_empty() {
        items.push(MenuItem::Separator);
        for (index, name) in ctx.open_with.iter().enumerate() {
            items.push(item(OpenWithItem::App(index), name.clone(), true));
        }
    }
    if crate::commands::shown(ctx.tables, ctx.scope, CommandId::AssocApp) {
        items.push(MenuItem::Separator);
        items.push(ctx.item(CommandId::AssocApp, None));
    }
    items
}

/// The items of Open With ▸ (§1.9.13): *Text Compare*, *Table Compare* (two files; T-57) and *Folder Compare* (two
/// folders), each application of
/// `openwith.toml`, then the associated application. Views of later waves are not listed until they exist.
pub fn open_with_items(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    use crate::window::OpenWithItem;
    let enabled = !ctx.ctx.dialog_open
        && lookup(ctx.scope, CommandId::OpenWithList)
            .is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
    let item = |what: OpenWithItem, caption: String, on: bool| MenuItem::Command {
        invocation: Invocation::with(CommandId::OpenWithList, CommandArg::OpenWith(what)),
        caption,
        shortcut: None,
        check: Check::None,
        enabled: enabled && on,
        description: ctx
            .tables
            .description(ctx.scope, CommandId::OpenWithList)
            .to_owned(),
    };
    let mut items = vec![item(
        OpenWithItem::TextCompare,
        Scope::Text.type_title().to_owned(),
        ctx.ctx.two_items,
    )];
    // FC-062 (Table Compare phase 3): two files in the table grid, whatever their format.
    items.push(item(
        OpenWithItem::TableCompare,
        Scope::Table.type_title().to_owned(),
        ctx.ctx.two_items && !ctx.ctx.two_folders,
    ));
    // M2 W13b (D-369): two files in Picture Compare.
    items.push(item(
        OpenWithItem::PictureCompare,
        Scope::Picture.type_title().to_owned(),
        ctx.ctx.two_items && !ctx.ctx.two_folders,
    ));
    // M2 W11b (D-313): the selected files, each in a Text Edit view.
    items.push(item(
        OpenWithItem::TextEdit,
        Scope::TextEdit.type_title().to_owned(),
        !ctx.ctx.two_folders,
    ));
    if ctx.ctx.two_folders {
        items.push(item(
            OpenWithItem::FolderCompare,
            Scope::Folder.type_title().to_owned(),
            true,
        ));
    }
    if !ctx.open_with.is_empty() {
        items.push(MenuItem::Separator);
        for (index, name) in ctx.open_with.iter().enumerate() {
            items.push(item(OpenWithItem::App(index), name.clone(), true));
        }
    }
    if crate::commands::shown(ctx.tables, ctx.scope, CommandId::AssocApp) {
        items.push(MenuItem::Separator);
        items.push(ctx.item(CommandId::AssocApp, None));
    }
    items
}

/// The column check items of a folder view (`ColList`'s submenu and the header's context menu, `Area::FolderHeader`;
/// D-234): one per offered column but Name, checked when it shows.
pub fn column_items(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    let enabled = !ctx.ctx.dialog_open
        && lookup(ctx.scope, CommandId::ColList).is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
    ctx.columns
        .iter()
        .filter(|(id, _)| *id != ColumnId::Name)
        .map(|&(id, on)| MenuItem::Command {
            invocation: Invocation::with(CommandId::ColList, CommandArg::Column(id)),
            caption: crate::folder::columns::title(id).to_owned(),
            shortcut: None,
            check: Check::Toggle(on),
            enabled,
            description: ctx
                .tables
                .description(ctx.scope, CommandId::ColList)
                .to_owned(),
        })
        .chain(
            // *Reset Columns* (M2 W9c, §1.9.18): the shown columns, their order and widths back to the defaults.
            (!ctx.columns.is_empty())
                .then(|| {
                    [
                        MenuItem::Separator,
                        MenuItem::Command {
                            invocation: Invocation::with(
                                CommandId::ColList,
                                CommandArg::ResetColumns,
                            ),
                            caption: RESET_COLUMNS.to_owned(),
                            shortcut: None,
                            check: Check::None,
                            enabled,
                            description: ctx
                                .tables
                                .description(ctx.scope, CommandId::ColList)
                                .to_owned(),
                        },
                    ]
                })
                .into_iter()
                .flatten(),
        )
        .collect()
}

/// The caption of *Reset Columns* (part of `ColList`, §1.9.18).
pub const RESET_COLUMNS: &str = "Reset Columns";

/// The items of File Manager ▸ (FC-063, D-254): *Open Folder in File Manager* and *Show in File Manager*, on the
/// current item of `side` (`None`: the focus side); disabled where that item is not on a local disk.
pub fn explorer_items(ctx: &MenuContext<'_>, side: Option<hikasa_core::Side>) -> Vec<MenuItem> {
    use crate::window::{ExplorerAction, ExplorerItem};
    let enabled = !ctx.ctx.dialog_open
        && lookup(ctx.scope, CommandId::ExplorerList)
            .is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
    let local = match side.and_then(crate::commands::side_index) {
        Some(k) => ctx.ctx.explorer[k],
        None => ctx.ctx.explorer[0] || ctx.ctx.explorer[1],
    };
    [ExplorerAction::OpenFolder, ExplorerAction::ShowItem]
        .into_iter()
        .map(|action| MenuItem::Command {
            invocation: Invocation::with(
                CommandId::ExplorerList,
                CommandArg::Explorer(ExplorerItem { action, side }),
            ),
            caption: action.caption().to_owned(),
            shortcut: None,
            check: Check::None,
            enabled: enabled && local,
            description: ctx
                .tables
                .description(ctx.scope, CommandId::ExplorerList)
                .to_owned(),
        })
        .collect()
}

/// The context menu of a folder view's pane area (D-253): D-110's list of the area ([`crate::commands::pane_menu_ids`]),
/// groups separated; an entry the view does not show (planned, no row) is left out, an unavailable one disabled; the
/// entries that act on a side carry the clicked pane's (`CommandArg::Side`); `…List` entries are submenus.
pub fn pane_menu(ctx: &MenuContext<'_>, area: crate::commands::Area) -> Vec<MenuItem> {
    use crate::commands::Area;
    let side = match area {
        Area::FolderPane(side) | Area::SyncPane(side) => Some(side),
        _ => None,
    };
    if area == Area::FolderHeader {
        return header_menu(ctx);
    }
    let mut items = Vec::new();
    for group in crate::commands::pane_menu_ids(area) {
        let mut entries = Vec::new();
        for &id in *group {
            if !crate::commands::shown(ctx.tables, ctx.scope, id) {
                continue;
            }
            let arg = side
                .filter(|_| crate::commands::takes_side(id))
                .map(CommandArg::Side);
            let item = match id {
                CommandId::ExplorerList => MenuItem::Submenu {
                    caption: ctx.tables.caption(ctx.scope, id),
                    items: explorer_items(ctx, side),
                },
                CommandId::OpenWithList if matches!(area, Area::TextHeader(_)) => {
                    MenuItem::Submenu {
                        caption: ctx.tables.caption(ctx.scope, id),
                        items: text_open_with_items(ctx),
                    }
                }
                // A text header's Encoding ▸ and Format ▸ (D-312): the header picker's command with its argument.
                CommandId::FileFormat if matches!(area, Area::TextHeader(_)) => {
                    let enabled = !ctx.ctx.dialog_open
                        && lookup(ctx.scope, id).is_some_and(|r| r.enabled_when.holds(&ctx.ctx));
                    let plain = |arg: CommandArg, caption: String| MenuItem::Command {
                        invocation: Invocation::with(id, arg),
                        caption,
                        shortcut: None,
                        check: Check::None,
                        enabled,
                        description: String::new(),
                    };
                    entries.push(MenuItem::Submenu {
                        caption: "Encoding".to_owned(),
                        items: crate::text::header::ENCODINGS
                            .iter()
                            .map(|label| {
                                plain(
                                    CommandArg::Encoding((*label).to_owned()),
                                    (*label).to_owned(),
                                )
                            })
                            .collect(),
                    });
                    MenuItem::Submenu {
                        caption: ctx.tables.caption(ctx.scope, id),
                        items: std::iter::once(plain(
                            CommandArg::Format(None),
                            "Detect".to_owned(),
                        ))
                        .chain(ctx.formats.iter().map(|name| {
                            plain(CommandArg::Format(Some(name.clone())), name.clone())
                        }))
                        .collect(),
                    }
                }
                CommandId::OpenWithList => MenuItem::Submenu {
                    caption: ctx.tables.caption(ctx.scope, id),
                    items: open_with_items(ctx),
                },
                id if !crate::commands::list_children(id).is_empty() => MenuItem::Submenu {
                    caption: ctx.tables.caption(ctx.scope, id),
                    items: crate::commands::list_children(id)
                        .iter()
                        .filter(|c| crate::commands::shown(ctx.tables, ctx.scope, **c))
                        .map(|c| ctx.item(*c, None))
                        .collect(),
                },
                id => ctx.item(id, arg),
            };
            entries.push(item);
        }
        if entries.is_empty() {
            continue;
        }
        if !items.is_empty() {
            items.push(MenuItem::Separator);
        }
        items.extend(entries);
    }
    items
}

/// The header's context menu of a folder view (`Area::FolderHeader`): the commands of that area, `ColList` as its
/// column items.
pub fn header_menu(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    let mut items = Vec::new();
    for row in crate::commands::PRESENTATION
        .iter()
        .filter(|r| r.scope == ctx.scope && r.areas.contains(&crate::commands::Area::FolderHeader))
    {
        if !crate::commands::shown(ctx.tables, ctx.scope, row.id) {
            continue;
        }
        if row.id == CommandId::ColList {
            items.extend(column_items(ctx));
        } else {
            items.push(ctx.item(row.id, None));
        }
    }
    items
}

/// Groups by section with separators between groups.
fn grouped(ctx: &MenuContext<'_>, entries: &[(&'static str, CommandId)]) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let mut last: Option<&str> = None;
    for (section, id) in entries {
        if last.is_some_and(|s| s != *section) {
            items.push(MenuItem::Separator);
        }
        last = Some(section);
        items.push(ctx.entry_item(*id));
    }
    items
}

/// The menu bar of the active view (D-110).
pub fn menu_bar(ctx: &MenuContext<'_>) -> Vec<Menu> {
    let mut menus = Vec::new();
    for title in MenuTitle::ORDER {
        let entries = ctx.entries(title.caption());
        let mut items = if title == MenuTitle::Session {
            let close = entries
                .iter()
                .position(|(s, id)| *s == "global" && *id == CommandId::CloseTab)
                .unwrap_or(entries.len());
            let (head, tail): (Vec<_>, Vec<_>) = entries
                .iter()
                .enumerate()
                .partition(|(i, (s, _))| *s == "global" && *i < close);
            let own: Vec<_> = tail
                .iter()
                .filter(|(_, (s, _))| *s != "global")
                .map(|(_, e)| **e)
                .collect();
            let end: Vec<_> = tail
                .iter()
                .filter(|(_, (s, _))| *s == "global")
                .map(|(_, e)| **e)
                .collect();
            let head: Vec<_> = head.into_iter().map(|(_, e)| *e).collect();
            let mut items = Vec::new();
            for part in [head, own, end] {
                if part.is_empty() {
                    continue;
                }
                if !items.is_empty() {
                    items.push(MenuItem::Separator);
                }
                items.extend(grouped(ctx, &part));
            }
            items
        } else {
            grouped(ctx, &entries)
        };
        if title == MenuTitle::View {
            let filters = display_filter_submenu(ctx);
            if !filters.is_empty() {
                if !items.is_empty() {
                    items.push(MenuItem::Separator);
                }
                items.push(MenuItem::Submenu {
                    caption: "Display Filter".to_owned(),
                    items: filters,
                });
            }
        }
        if !items.is_empty() {
            menus.push(Menu { title, items });
        }
    }
    menus
}

/// The View ▸ Display Filter submenu: the presets of the pseudo-menus `DropDown` and `Favorites` (radio), then
/// the `Toggles` (check).
fn display_filter_submenu(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    let table = ctx.tables.view(ctx.scope);
    let pick = |menus: &[&str]| -> Vec<CommandId> {
        table
            .entries
            .iter()
            .filter(|e| menus.contains(&e.menu))
            .filter_map(|e| ctx.shown(e))
            .collect()
    };
    let presets = pick(&["DropDown", "Favorites"]);
    let toggles = pick(&["Toggles"]);
    let mut items: Vec<MenuItem> = presets.iter().map(|id| ctx.item(*id, None)).collect();
    if !presets.is_empty() && !toggles.is_empty() {
        items.push(MenuItem::Separator);
    }
    items.extend(toggles.iter().map(|id| ctx.item(*id, None)));
    items
}

/// The tab context menu of tab `tab` (D-107): New Tab, Duplicate, Move to New Window, Move to Window ▸ (the
/// other windows), Close Other Tabs, Close Tab. `others` lists the other windows with their captions.
pub fn tab_menu(ctx: &MenuContext<'_>, tab: usize, others: &[(WindowId, String)]) -> Vec<MenuItem> {
    let arg = || Some(CommandArg::Tab(tab));
    let mut items = vec![
        ctx.item(CommandId::TabNew, None),
        ctx.item(CommandId::TabDuplicate, arg()),
        ctx.item(CommandId::TabMoveToNewWindow, arg()),
    ];
    if !others.is_empty() {
        items.push(MenuItem::Submenu {
            caption: ctx.tables.caption(ctx.scope, CommandId::TabMoveToWindow),
            items: others
                .iter()
                .map(|(id, caption)| MenuItem::Command {
                    invocation: Invocation::with(
                        CommandId::TabMoveToWindow,
                        CommandArg::Window(*id),
                    ),
                    caption: caption.clone(),
                    shortcut: None,
                    check: Check::None,
                    enabled: !ctx.ctx.dialog_open,
                    description: String::new(),
                })
                .collect(),
        });
    }
    items.push(ctx.item(CommandId::TabCloseOthers, arg()));
    items.push(MenuItem::Separator);
    items.push(ctx.item(CommandId::CloseTab, arg()));
    items
}

/// The context menu of the sessions tree (D-110, SES-031): the commands of `Area::SessionTree`, in catalogue
/// order, those available for the selection.
pub fn tree_menu(ctx: &MenuContext<'_>) -> Vec<MenuItem> {
    ctx.tables
        .view(Scope::Home)
        .entries
        .iter()
        .filter_map(|e| CommandId::from_name(e.id()))
        .filter(|id| {
            lookup(Scope::Home, *id).is_some_and(|r| {
                r.areas.contains(&crate::commands::Area::SessionTree)
                    && r.enabled_when.holds(&ctx.ctx)
            })
        })
        .map(|id| ctx.item(id, None))
        .collect()
}

/// One element of a toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolItem {
    /// A command button (its icon; the label under it with large buttons).
    Command(CommandId),
    /// The display filter control of `display_filter_mode`.
    DisplayFilter,
    /// The folder mode combo (Folder Compare).
    FolderMode,
    /// `SessionsCombo`: the named sessions of the view's type (W4b).
    Sessions,
}

/// The toolbar of a view type: the visible `on_toolbar` entries with an icon (all of them with
/// `show_all_on_toolbar`), then the display filter control and the folder mode combo.
pub fn toolbar_items(tables: &CommandTables, scope: Scope) -> Vec<ToolItem> {
    let table = tables.view(scope);
    let show_all = table.custom.show_all_on_toolbar;
    let mut items: Vec<ToolItem> = table
        .entries
        .iter()
        .filter(|e| e.implemented() && (e.on_toolbar || show_all))
        .filter_map(|e| CommandId::from_name(e.id()))
        .filter(|id| lookup(scope, *id).is_some_and(|r| r.icon.is_some()))
        .filter(|id| {
            // With show-all, the filter presets and toggles stay in their control.
            !show_all
                || !matches!(
                    lookup(scope, *id).map(|r| r.group),
                    Some(Group::Filter | Group::Toggles)
                )
                || tables.entry(scope, *id).is_some_and(|e| e.on_toolbar)
        })
        .map(ToolItem::Command)
        .collect();
    if !display_filter(tables, scope).presets.is_empty() {
        items.push(ToolItem::DisplayFilter);
    }
    if scope == Scope::Folder {
        items.push(ToolItem::FolderMode);
    }
    if lookup(scope, CommandId::SessionsCombo).is_some() {
        items.push(ToolItem::Sessions);
    }
    items
}

/// The display filter control of a view type (D-110): by `display_filter_mode`, a drop-down of every preset
/// with the toggle buttons (`dropdown`, the M1 toolbar), the `Favorites` presets as buttons beside a drop-down
/// of the others (`favorites`), or the toggle buttons alone (`toggles`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayFilterControl {
    /// The mode.
    pub mode: DisplayFilterMode,
    /// Presets in the drop-down.
    pub presets: Vec<CommandId>,
    /// Presets shown as buttons.
    pub buttons: Vec<CommandId>,
    /// Toggle buttons.
    pub toggles: Vec<CommandId>,
}

/// The display filter control of `scope`.
pub fn display_filter(tables: &CommandTables, scope: Scope) -> DisplayFilterControl {
    let table = tables.view(scope);
    let mode = table.custom.display_filter_mode;
    let of = |group: Group| -> Vec<(CommandId, &str)> {
        table
            .entries
            .iter()
            .filter(|e| e.implemented())
            .filter_map(|e| {
                let id = CommandId::from_name(e.id())?;
                (lookup(scope, id)?.group == group).then_some((id, e.menu))
            })
            .collect()
    };
    let presets = of(Group::Filter);
    let toggles: Vec<CommandId> = of(Group::Toggles).into_iter().map(|(id, _)| id).collect();
    match mode {
        DisplayFilterMode::Dropdown => DisplayFilterControl {
            mode,
            presets: presets.into_iter().map(|(id, _)| id).collect(),
            buttons: Vec::new(),
            toggles,
        },
        DisplayFilterMode::Favorites => {
            let (buttons, rest): (Vec<_>, Vec<_>) = presets
                .into_iter()
                .partition(|(_, menu)| *menu == "Favorites");
            DisplayFilterControl {
                mode,
                presets: rest.into_iter().map(|(id, _)| id).collect(),
                buttons: buttons.into_iter().map(|(id, _)| id).collect(),
                toggles: Vec::new(),
            }
        }
        DisplayFilterMode::Toggles => DisplayFilterControl {
            mode,
            presets: presets.into_iter().map(|(id, _)| id).collect(),
            buttons: Vec::new(),
            toggles,
        },
    }
}

/// One entry of the toolbar's context menu (UI-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarMenuItem {
    /// A display filter mode (radio).
    Mode(DisplayFilterMode, bool),
    /// "Large buttons with text labels" (check).
    LargeButtons(bool),
    /// "Toolbar Options…" (`ProgramOptions` on the commands page with the view preselected).
    Options,
    /// "Hide Toolbar" (`ShowToolbar`).
    HideToolbar,
}

/// The toolbar's context menu of a view type.
pub fn toolbar_menu(tables: &CommandTables, scope: Scope) -> Vec<ToolbarMenuItem> {
    let custom = &tables.view(scope).custom;
    let mut items: Vec<ToolbarMenuItem> = [
        DisplayFilterMode::Dropdown,
        DisplayFilterMode::Favorites,
        DisplayFilterMode::Toggles,
    ]
    .into_iter()
    .map(|m| ToolbarMenuItem::Mode(m, custom.display_filter_mode == m))
    .collect();
    items.push(ToolbarMenuItem::LargeButtons(custom.large_buttons));
    items.push(ToolbarMenuItem::Options);
    items.push(ToolbarMenuItem::HideToolbar);
    items
}

impl ToolbarMenuItem {
    /// The caption.
    pub fn caption(self) -> &'static str {
        match self {
            ToolbarMenuItem::Mode(DisplayFilterMode::Dropdown, _) => "Display Filters as Drop-down",
            ToolbarMenuItem::Mode(DisplayFilterMode::Favorites, _) => {
                "Display Filters as Favorites"
            }
            ToolbarMenuItem::Mode(DisplayFilterMode::Toggles, _) => "Display Filters as Toggles",
            ToolbarMenuItem::LargeButtons(_) => "Large Buttons with Text Labels",
            ToolbarMenuItem::Options => "Toolbar Options…",
            ToolbarMenuItem::HideToolbar => "Hide Toolbar",
        }
    }
}

/// The menus as indented text (golden tests, §1.4.16 rule 2): `[x]`/`[ ]` check boxes, `(*)`/`( )` radio
/// buttons, `---` separators, `>` submenus, the shortcut after a tab, `(disabled)` for unavailable entries.
#[cfg_attr(not(test), allow(dead_code))]
pub fn render_text(menus: &[Menu]) -> String {
    fn items(out: &mut String, list: &[MenuItem], depth: usize) {
        let pad = "  ".repeat(depth);
        for item in list {
            match item {
                MenuItem::Separator => out.push_str(&format!("{pad}---\n")),
                MenuItem::Submenu {
                    caption,
                    items: sub,
                } => {
                    out.push_str(&format!("{pad}> {caption}\n"));
                    items(out, sub, depth + 1);
                }
                MenuItem::Command {
                    caption,
                    shortcut,
                    check,
                    enabled,
                    ..
                } => {
                    let mark = match check {
                        Check::None => "",
                        Check::Toggle(true) => "[x] ",
                        Check::Toggle(false) => "[ ] ",
                        Check::Radio(true) => "(*) ",
                        Check::Radio(false) => "( ) ",
                    };
                    let mut line = format!("{pad}{mark}{caption}");
                    if let Some(s) = shortcut {
                        line.push('\t');
                        line.push_str(s);
                    }
                    if !enabled {
                        line.push_str(" (disabled)");
                    }
                    out.push_str(&line);
                    out.push('\n');
                }
            }
        }
    }
    let mut out = String::new();
    for menu in menus {
        out.push_str(menu.title.caption());
        out.push('\n');
        items(&mut out, &menu.items, 1);
    }
    out
}
