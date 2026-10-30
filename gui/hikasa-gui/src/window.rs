//! Windows and tabs (M2 W4a, D-106…D-108; UI-001, SES-090): the toolkit-free [`WindowModel`] of each top-level
//! window, its [`Tab`]s, the [`Invocation`] of a command with its argument, and the drop rule of a dragged tab
//! ([`resolve_tab_drop`]). [`crate::app::AppCore`] owns every window; `ui/` draws them (`windows[0]` in eframe's
//! root viewport, the others as immediate viewports).

use std::path::PathBuf;

use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::{AutoId, SessionName, SessionRef, SessionType};
use hikasa_session::settings::state::WindowState;

use crate::commands::{CommandId, Scope};
use crate::dialog::DialogStack;
use crate::folder_view::FolderViewModel;
use crate::home::HomeModel;
use crate::notify::Notices;
use crate::status::Message;
use crate::text_view::TextViewModel;

/// A window's id: allocated once, never reused in a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// A command with its argument (menus, the toolbar, keys, context menus, dialogs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The command.
    pub id: CommandId,
    /// Its argument.
    pub arg: Option<CommandArg>,
}

impl Invocation {
    /// A command without an argument.
    pub fn new(id: CommandId) -> Invocation {
        Invocation { id, arg: None }
    }

    /// A command with an argument.
    pub fn with(id: CommandId, arg: CommandArg) -> Invocation {
        Invocation { id, arg: Some(arg) }
    }
}

impl From<CommandId> for Invocation {
    fn from(id: CommandId) -> Invocation {
        Invocation::new(id)
    }
}

/// The argument of a command.
#[allow(dead_code)] // variants for W4b/W4c (contract seams)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandArg {
    /// A window (`TabMoveToWindow`).
    Window(WindowId),
    /// A tab of the window (the tab context menu acts on that tab, not on the active one).
    Tab(usize),
    /// A session type (`SessionNewList`, W4b).
    SessionType(SessionType),
    /// A named session (W4b).
    Session(SessionRef),
    /// An auto-saved session (W4b).
    AutoSaved(AutoId),
    /// A workspace (W4b).
    Workspace(SessionName),
    /// A page of a dialog (`ProgramOptions`, W4c).
    Page(&'static str),
    /// Paths (drops).
    Paths(Vec<PathBuf>),
}

/// A tab's view.
pub enum ViewModel {
    /// Home.
    Home(HomeModel),
    /// Folder Compare.
    Folder(Box<FolderViewModel>),
    /// Text Compare.
    Text(Box<TextViewModel>),
}

impl ViewModel {
    /// The command scope of the view.
    pub fn scope(&self) -> Scope {
        match self {
            ViewModel::Home(_) => Scope::Home,
            ViewModel::Folder(_) => Scope::Folder,
            ViewModel::Text(_) => Scope::Text,
        }
    }
}

/// What a tab holds: a live view (W4b adds the pending tabs of a restored workspace, D-126).
pub enum TabContent {
    /// A view.
    Live(ViewModel),
}

/// One tab.
pub struct Tab {
    /// View id (registered in `Shared` for comparison views); unique across windows.
    pub id: ViewId,
    /// The content.
    pub content: TabContent,
    /// The view's message panel (D-115).
    pub message: Option<Message>,
    /// The view's toolbar is shown (`[layout.<view>] toolbar`, `ShowToolbar`).
    pub toolbar: bool,
}

impl Tab {
    /// A tab of a live view.
    pub fn live(id: ViewId, model: ViewModel, toolbar: bool) -> Tab {
        Tab {
            id,
            content: TabContent::Live(model),
            message: None,
            toolbar,
        }
    }

    /// The view.
    pub fn model(&self) -> Option<&ViewModel> {
        match &self.content {
            TabContent::Live(m) => Some(m),
        }
    }

    /// The view, mutable.
    pub fn model_mut(&mut self) -> Option<&mut ViewModel> {
        match &mut self.content {
            TabContent::Live(m) => Some(m),
        }
    }

    /// The command scope of the tab.
    pub fn scope(&self) -> Scope {
        self.model().map_or(Scope::Home, ViewModel::scope)
    }

    /// Tab caption.
    pub fn caption(&self) -> String {
        match self.model() {
            Some(ViewModel::Home(_)) | None => "Home".into(),
            Some(ViewModel::Folder(m)) => m.caption(),
            Some(ViewModel::Text(m)) => m.caption(),
        }
    }

    /// True for an untouched Home tab (an open replaces it, M1).
    pub fn untouched_home(&self) -> bool {
        matches!(self.model(), Some(ViewModel::Home(h)) if *h == HomeModel::default())
    }
}

/// One top-level window.
pub struct WindowModel {
    /// Id.
    pub id: WindowId,
    /// Tabs, in bar order.
    pub tabs: Vec<Tab>,
    /// Index of the active tab.
    pub active: usize,
    /// Geometry (x/y −1 = unknown), read from the platform each frame.
    pub geometry: WindowState,
    /// The platform reports this window's position (`ViewportInfo::outer_rect` is `Some`).
    pub position_known: bool,
    /// Its dialogs (modal to it).
    pub dialogs: DialogStack,
    /// Its notices.
    pub notices: Notices,
    /// Focus order: the window focused last has the highest number.
    pub focused_seq: u64,
    /// The help topic of its help panel (`None`: hidden).
    pub help: Option<&'static str>,
}

impl WindowModel {
    /// A window without tabs.
    pub fn new(id: WindowId, geometry: WindowState, focused_seq: u64) -> WindowModel {
        WindowModel {
            id,
            tabs: Vec::new(),
            active: 0,
            geometry,
            position_known: false,
            dialogs: DialogStack::default(),
            notices: Notices::default(),
            focused_seq,
            help: None,
        }
    }

    /// The active tab.
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// The active tab, mutable.
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    /// The title of the native window: `<active tab caption> — Hikasa`.
    pub fn title(&self) -> String {
        match self.active_tab() {
            Some(tab) => format!("{} — Hikasa", crate::shared::shown_name(&tab.caption())),
            None => "Hikasa".to_owned(),
        }
    }

    /// The index of a view's tab.
    pub fn index_of(&self, view: ViewId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == view)
    }
}

/// A rectangle in global screen points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenRect {
    /// Top-left corner.
    pub min: [f32; 2],
    /// Bottom-right corner.
    pub max: [f32; 2],
}

impl ScreenRect {
    /// True when `p` lies inside.
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.min[0] && p[0] <= self.max[0] && p[1] >= self.min[1] && p[1] <= self.max[1]
    }
}

/// Where a tab dragged out of its bar goes (D-107).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabDrop {
    /// Nothing (positions unknown, or released over a window but not over its tab bar).
    Nothing,
    /// Into another window's tab bar.
    Move(WindowId),
    /// Into a new window at this global position.
    NewWindow([f32; 2]),
}

/// The drop of a tab released at the global point `release` outside its own tab bar. `windows` lists every
/// window with its outer rectangle and its tab bar's rectangle (`None` where the platform reports no position,
/// Wayland, A17). Without every position nothing happens (the Move commands remain the way).
pub fn resolve_tab_drop(
    source: WindowId,
    windows: &[(WindowId, Option<ScreenRect>, Option<ScreenRect>)],
    release: Option<[f32; 2]>,
) -> TabDrop {
    let Some(at) = release else {
        return TabDrop::Nothing;
    };
    if windows
        .iter()
        .any(|(_, outer, bar)| outer.is_none() || bar.is_none())
    {
        return TabDrop::Nothing;
    }
    for (id, outer, bar) in windows {
        let (Some(outer), Some(bar)) = (outer, bar) else {
            continue;
        };
        if bar.contains(at) {
            return if *id == source {
                TabDrop::Nothing
            } else {
                TabDrop::Move(*id)
            };
        }
        if outer.contains(at) {
            return TabDrop::Nothing;
        }
    }
    TabDrop::NewWindow(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Option<ScreenRect> {
        Some(ScreenRect {
            min: [x, y],
            max: [x + w, y + h],
        })
    }

    /// A4.1: drag moves with known and unknown positions.
    #[test]
    fn tab_drops_need_known_positions() {
        let (a, b) = (WindowId(1), WindowId(2));
        let known = [
            (
                a,
                rect(0.0, 0.0, 800.0, 600.0),
                rect(0.0, 30.0, 800.0, 30.0),
            ),
            (
                b,
                rect(900.0, 0.0, 800.0, 600.0),
                rect(900.0, 30.0, 800.0, 30.0),
            ),
        ];
        assert_eq!(
            resolve_tab_drop(a, &known, Some([1000.0, 40.0])),
            TabDrop::Move(b)
        );
        assert_eq!(
            resolve_tab_drop(a, &known, Some([1000.0, 300.0])),
            TabDrop::Nothing
        );
        assert_eq!(
            resolve_tab_drop(a, &known, Some([100.0, 40.0])),
            TabDrop::Nothing
        );
        assert_eq!(
            resolve_tab_drop(a, &known, Some([2000.0, 900.0])),
            TabDrop::NewWindow([2000.0, 900.0])
        );
        assert_eq!(resolve_tab_drop(a, &known, None), TabDrop::Nothing);
        let unknown = [
            (a, None, rect(0.0, 30.0, 800.0, 30.0)),
            (
                b,
                rect(900.0, 0.0, 800.0, 600.0),
                rect(900.0, 30.0, 800.0, 30.0),
            ),
        ];
        assert_eq!(
            resolve_tab_drop(a, &unknown, Some([1000.0, 40.0])),
            TabDrop::Nothing
        );
        assert_eq!(
            resolve_tab_drop(a, &unknown, Some([2000.0, 900.0])),
            TabDrop::Nothing
        );
    }
}
