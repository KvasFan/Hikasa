//! The Home view (UI-001; M2 W4b, §1.4.11, D-124): the type buttons, the sessions tree with its search and the
//! management panel, whose Specs editor is the path form of M1. The tree is built from `session_list`
//! ([`HomeModel::load`]); paths go through the same resolution as the command line (`Service::resolve_open`: specs
//! against the GUI's current folder, the view kind from CLI-002 or the selected type) and the same mapping as an
//! IPC request (`Service::open_view_from_ipc`, which reads the GUI's own clipboard for `clipboard://`, D-101).
//!
//! The UI thread never blocks (contract §1.10): the fields are passed verbatim (spaces are legal in names,
//! §2.1), standard input (`-`) is refused at once with an inline message (a window has no standard input to
//! read; the command line can spool it), and the resolution, which stats the paths, runs as a job; the view
//! opens when the job's result arrives. Everything that touches a session file is a job of the app core.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::{JobKind, Spec};
use hikasa_session::api::{GuiOpenRequest, SessionListDto, ViewRequest};
use hikasa_session::ipc::{OpenOptions, ViewKind};
use hikasa_session::sessions::{AutoGroup, AutoId, Session, SessionRef, SessionType};
use hikasa_session::{ApiError, Service};

use crate::commands::Ctx;
use crate::job::Job;
use crate::menus::OPENABLE_TYPES;

/// Shown when a field is empty.
pub const NEED_TWO: &str = "Enter two files or two folders.";
/// Shown for `-`.
pub const NO_STDIN: &str = "Standard input (-) cannot be compared from Home: the window has no input to read. \
                            Pass - on the command line instead (hikasa gui open - FILE).";

/// The running resolution of a Compare (compared by identity, so the model stays a plain value for the UI).
#[derive(Clone)]
pub struct Pending(Arc<Job<HomeOpen>>);

impl PartialEq for Pending {
    fn eq(&self, other: &Pending) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Pending").field(&self.0.id()).finish()
    }
}

/// One node of the sessions tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeNode {
    /// A heading (*New*, *Auto-saved*).
    Heading(&'static str),
    /// *New*: a session type; `customized` when its defaults file exists.
    NewType {
        /// The type.
        kind: SessionType,
        /// `_defaults/<type>.toml` exists.
        customized: bool,
    },
    /// A folder (its full path).
    Folder(String),
    /// A named session.
    Named {
        /// The reference.
        reference: SessionRef,
        /// The type.
        kind: SessionType,
        /// The description.
        description: String,
        /// Locked.
        locked: bool,
        /// Favorite.
        favorite: bool,
    },
    /// A group of auto-saved sessions.
    AutoGroup(AutoGroup),
    /// An auto-saved session.
    AutoSaved {
        /// The id.
        id: AutoId,
        /// The type.
        kind: SessionType,
        /// The label.
        label: String,
    },
}

/// One row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    /// A key stable across reloads.
    pub key: String,
    /// Indentation.
    pub depth: usize,
    /// The node.
    pub node: TreeNode,
    /// The keys of its ancestors, outermost first.
    pub ancestors: Vec<String>,
}

impl TreeRow {
    /// True for a row that can hold others.
    pub fn expandable(&self) -> bool {
        matches!(
            self.node,
            TreeNode::Heading(_) | TreeNode::Folder(_) | TreeNode::AutoGroup(_)
        )
    }

    /// The caption of the row.
    pub fn caption(&self) -> String {
        match &self.node {
            TreeNode::Heading(text) => (*text).to_owned(),
            TreeNode::NewType { kind, .. } => type_caption(*kind).to_owned(),
            TreeNode::Folder(path) => path.rsplit('/').next().unwrap_or(path).to_owned(),
            TreeNode::Named { reference, .. } => reference.name.to_string(),
            TreeNode::AutoGroup(group) => match group {
                AutoGroup::Today => "Today",
                AutoGroup::Yesterday => "Yesterday",
                AutoGroup::Days2To6 => "2-6 days ago",
                AutoGroup::Older => "More than 6 days ago",
            }
            .to_owned(),
            TreeNode::AutoSaved { label, .. } => label.clone(),
        }
    }

    /// True when the search text (lower case) matches the row (D-124: name, folder path or description).
    fn matches(&self, needle: &str) -> bool {
        let has = |text: &str| text.to_lowercase().contains(needle);
        match &self.node {
            TreeNode::Heading(_) | TreeNode::AutoGroup(_) => false,
            TreeNode::NewType { kind, .. } => has(type_caption(*kind)),
            TreeNode::Folder(path) => has(path),
            TreeNode::Named {
                reference,
                description,
                ..
            } => has(&reference.to_string()) || has(description),
            TreeNode::AutoSaved { label, .. } => has(label),
        }
    }
}

/// The caption of a session type.
pub fn type_caption(kind: SessionType) -> &'static str {
    OPENABLE_TYPES
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or_else(|| kind.as_str(), |(_, caption)| caption)
}

/// What the management panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PanelTarget {
    /// Nothing selected: the free path form of M1.
    #[default]
    None,
    /// An empty session of a type.
    New(SessionType),
    /// A named session.
    Named(SessionRef),
    /// An auto-saved session (read-only).
    AutoSaved(AutoId),
}

/// Whether the management panel shows (`ShowManage`, `MaximizeManage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelMode {
    /// Beside the tree.
    #[default]
    Shown,
    /// Hidden.
    Hidden,
    /// The tree is hidden.
    Maximized,
}

/// The management panel (D-124): the selection's copy, its Specs editor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ManagementPanel {
    /// What it shows.
    pub target: PanelTarget,
    /// Left path as typed.
    pub left: String,
    /// Right path as typed.
    pub right: String,
    /// The description.
    pub description: String,
    /// The panel's copy of the session.
    pub session: Option<Session>,
    /// The session as read (Cancel returns to it).
    pub original: Option<Session>,
    /// A read of the selection runs.
    pub loading: bool,
}

impl ManagementPanel {
    /// Shows `session` (also resets the fields).
    pub fn show(&mut self, target: PanelTarget, session: Option<Session>) {
        self.target = target;
        self.loading = false;
        self.original.clone_from(&session);
        self.fill(session);
    }

    /// Fills the fields from a session, keeping the target and the original.
    pub fn fill(&mut self, session: Option<Session>) {
        let specs = session
            .as_ref()
            .map(|s| s.specs.clone())
            .unwrap_or_default();
        self.left = specs.left;
        self.right = specs.right;
        self.description = session
            .as_ref()
            .map(|s| s.description.clone())
            .unwrap_or_default();
        self.session = session;
    }

    /// True when the fields differ from the session as read.
    pub fn changed(&self) -> bool {
        match &self.original {
            Some(s) => {
                self.session != self.original
                    || self.left != s.specs.left
                    || self.right != s.specs.right
                    || self.description != s.description
            }
            None => false,
        }
    }

    /// The panel's session with the typed fields (specs as typed: the job stores them, D-80).
    pub fn edited(&self) -> Option<Session> {
        let mut session = self.session.clone()?;
        session.specs.left.clone_from(&self.left);
        session.specs.right.clone_from(&self.right);
        session.description.clone_from(&self.description);
        Some(session)
    }
}

/// The in-place rename of a tree row (`TreeRename`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameEdit {
    /// The row's key.
    pub key: String,
    /// The text.
    pub text: String,
}

/// The Home view model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HomeModel {
    /// The rows of the tree, in order.
    pub tree: Vec<TreeRow>,
    /// The search text.
    pub search: String,
    /// The selected row (an index into `tree`).
    pub selected: Option<usize>,
    /// The keys of the open rows.
    pub expanded: BTreeSet<String>,
    /// The keys seen so far: a row seen for the first time opens by itself.
    known: BTreeSet<String>,
    /// The management panel.
    pub panel: ManagementPanel,
    /// Whether it shows.
    pub panel_mode: PanelMode,
    /// The in-place rename.
    pub rename: Option<RenameEdit>,
    /// The last error (shown in the panel).
    pub error: Option<String>,
    /// `Find` asks the UI to focus the search box.
    pub focus_search: bool,
    /// The session being dragged onto a folder.
    pub dragging: Option<SessionRef>,
    /// The resolution started by Compare, while it runs.
    pending: Option<Pending>,
}

/// What Compare opens.
#[derive(Debug, Clone)]
pub struct HomeOpen {
    /// View kind.
    pub kind: ViewKind,
    /// The view's request.
    pub request: ViewRequest,
    /// Options (titles).
    pub options: OpenOptions,
    /// Canonical absolute specs.
    pub specs: Vec<String>,
}

const NEW_HEADING: &str = "h:new";
const AUTO_HEADING: &str = "h:auto";

impl HomeModel {
    /// True when both paths are filled in (blank fields count as empty; the paths themselves are not trimmed).
    pub fn ready(&self) -> bool {
        !self.panel.left.trim().is_empty() && !self.panel.right.trim().is_empty()
    }

    /// True when nothing was typed, selected or searched: an open replaces this tab.
    pub fn untouched(&self) -> bool {
        self.panel.left.is_empty()
            && self.panel.right.is_empty()
            && self.panel.target == PanelTarget::None
            && self.search.is_empty()
            && self.selected.is_none()
            && self.error.is_none()
            && self.pending.is_none()
    }

    /// True while Compare resolves the paths.
    pub fn resolving(&self) -> bool {
        self.pending.is_some()
    }

    /// True while the resolution has ended but its result is not published yet.
    pub fn settling(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.0.settling())
    }

    /// The selected row.
    pub fn selection(&self) -> Option<&TreeRow> {
        self.selected.and_then(|i| self.tree.get(i))
    }

    /// Fills the command context.
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.home_ready = self.ready() && !self.resolving();
        ctx.busy = self.resolving();
        ctx.ready = true;
        match self.selection().map(|r| &r.node) {
            Some(TreeNode::Named { locked, .. }) => {
                ctx.tree_named = true;
                ctx.tree_locked = *locked;
            }
            Some(TreeNode::AutoSaved { .. }) => ctx.tree_auto = true,
            Some(TreeNode::Folder(_)) => ctx.tree_folder = true,
            Some(TreeNode::NewType { .. }) => ctx.tree_new = true,
            _ => {}
        }
    }

    /// Builds the tree from `session_list` (D-124). The selection and the expansion survive by key; a row seen
    /// for the first time opens.
    pub fn load(&mut self, list: &SessionListDto) {
        let selected = self.selection().map(|r| r.key.clone());
        self.tree = build_tree(list);
        for row in &self.tree {
            if row.expandable() && self.known.insert(row.key.clone()) {
                self.expanded.insert(row.key.clone());
            }
        }
        self.selected = selected.and_then(|key| self.tree.iter().position(|r| r.key == key));
        // The panel must not hold a session that is gone: its Save would bring it back.
        let listed = match &self.panel.target {
            PanelTarget::Named(r) => {
                let wanted = r.to_string().to_lowercase();
                list.sessions
                    .iter()
                    .any(|s| s.reference.to_lowercase() == wanted)
            }
            PanelTarget::AutoSaved(id) => {
                let wanted = id.to_string();
                list.autosaved.iter().any(|a| a.id == wanted)
            }
            PanelTarget::None | PanelTarget::New(_) => true,
        };
        if !listed {
            self.panel.show(PanelTarget::None, None);
        }
    }

    /// The rows shown now. With a search: the matches with their ancestors, whatever is open; without: the rows
    /// whose ancestors are all open.
    pub fn visible(&self) -> Vec<&TreeRow> {
        let needle = self.search.trim().to_lowercase();
        if needle.is_empty() {
            return self
                .tree
                .iter()
                .filter(|r| r.ancestors.iter().all(|a| self.expanded.contains(a)))
                .collect();
        }
        let mut shown: BTreeSet<&str> = BTreeSet::new();
        for row in self.tree.iter().filter(|r| r.matches(&needle)) {
            shown.insert(&row.key);
            shown.extend(row.ancestors.iter().map(String::as_str));
        }
        self.tree
            .iter()
            .filter(|r| shown.contains(r.key.as_str()))
            .collect()
    }

    /// Sets the search text (the box edits `search` directly).
    #[cfg(test)]
    pub fn set_search(&mut self, text: &str) {
        text.clone_into(&mut self.search);
    }

    /// Opens or closes a row.
    pub fn toggle(&mut self, key: &str) {
        if !self.expanded.remove(key) {
            self.expanded.insert(key.to_owned());
        }
    }

    /// Opens every row.
    pub fn expand_all(&mut self) {
        self.expanded = self
            .tree
            .iter()
            .filter(|r| r.expandable())
            .map(|r| r.key.clone())
            .collect();
    }

    /// Closes every folder and group; the headings stay open.
    pub fn collapse_all(&mut self) {
        self.expanded = [NEW_HEADING.to_owned(), AUTO_HEADING.to_owned()]
            .into_iter()
            .collect();
    }

    /// Selects the row with `key`.
    pub fn select_key(&mut self, key: &str) -> bool {
        self.selected = self.tree.iter().position(|r| r.key == key);
        self.selected.is_some()
    }

    /// The tree as text (golden tests): `[+]`/`[-]` for closed and open rows, `*` favorites, `(locked)`,
    /// `(customized)`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn render_tree(&self) -> String {
        let mut out = String::new();
        for row in self.visible() {
            let pad = "  ".repeat(row.depth);
            let open = if !row.expandable() {
                ""
            } else if self.expanded.contains(&row.key) {
                "[-] "
            } else {
                "[+] "
            };
            let mut line = format!("{pad}{open}{}", row.caption());
            match &row.node {
                TreeNode::Named {
                    kind,
                    locked,
                    favorite,
                    description,
                    ..
                } => {
                    line.push_str(&format!(" <{kind}>"));
                    if *favorite {
                        line.push_str(" *");
                    }
                    if *locked {
                        line.push_str(" (locked)");
                    }
                    if !description.is_empty() {
                        line.push_str(&format!(" -- {description}"));
                    }
                }
                TreeNode::NewType {
                    customized: true, ..
                } => line.push_str(" (customized)"),
                TreeNode::AutoSaved { kind, .. } => line.push_str(&format!(" <{kind}>")),
                _ => {}
            }
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    /// Starts resolving the two paths (off the UI thread); `kind` is the selected session type, if any; `wake` is
    /// called when the result is there. Refuses `-` at once. The result is picked up by [`HomeModel::poll`].
    pub fn compare(
        &mut self,
        service: &Service,
        kind: Option<ViewKind>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        if self.resolving() {
            return;
        }
        if !self.ready() {
            self.error = Some(NEED_TWO.into());
            return;
        }
        let specs = vec![
            OsString::from(&self.panel.left),
            OsString::from(&self.panel.right),
        ];
        // Parsing is pure (no I/O, standard input is not touched): what a window cannot open is refused before
        // anything runs.
        let ctx = service.spec_context();
        for spec in &specs {
            if matches!(Spec::parse_os(spec, &ctx), Ok(Spec::Stdin)) {
                self.error = Some(NO_STDIN.into());
                return;
            }
        }
        self.error = None;
        let request = GuiOpenRequest {
            specs,
            kind,
            ..GuiOpenRequest::default()
        };
        let job = Job::spawn(
            service,
            JobKind::VfsRead,
            "resolve the Home paths".into(),
            &wake,
            move |svc, _job| resolve_as(svc, &request),
        );
        self.pending = Some(Pending(Arc::new(job)));
    }

    /// A copy of the typed fields (`TabDuplicate` of a Home tab, D-107): no error, nothing running.
    pub fn copy_fields(&self) -> HomeModel {
        HomeModel {
            error: None,
            pending: None,
            rename: None,
            ..self.clone()
        }
    }

    /// Picks up the result of a running Compare: `Some` opens the view; an error is kept for display. The
    /// second value is true when the model changed.
    pub fn poll(&mut self) -> (Option<HomeOpen>, bool) {
        let Some(result) = self.pending.as_ref().and_then(|p| p.0.try_result()) else {
            return (None, false);
        };
        self.pending = None;
        match result {
            Ok(open) => (Some(open), true),
            Err(e) => {
                self.error = Some(e.to_string());
                (None, true)
            }
        }
    }
}

/// The rows of the tree (D-124): *New*, the named sessions (folders first, then favorites, then the rest, each
/// sorted case-insensitively), *Auto-saved* in its four groups, newest first.
fn build_tree(list: &SessionListDto) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    rows.push(TreeRow {
        key: NEW_HEADING.into(),
        depth: 0,
        node: TreeNode::Heading("New"),
        ancestors: Vec::new(),
    });
    for (kind, _) in OPENABLE_TYPES {
        rows.push(TreeRow {
            key: format!("new:{kind}"),
            depth: 1,
            node: TreeNode::NewType {
                kind,
                customized: list.defaults.iter().any(|d| d == kind.as_str()),
            },
            ancestors: vec![NEW_HEADING.into()],
        });
    }
    let mut folders: BTreeSet<String> = list.folders.iter().cloned().collect();
    for item in &list.sessions {
        let mut path = item.reference.as_str();
        while let Some((parent, _)) = path.rsplit_once('/') {
            folders.insert(parent.to_owned());
            path = parent;
        }
    }
    let folders: Vec<String> = folders.into_iter().collect();
    named_rows("", &[], list, &folders, &mut rows);
    if !list.autosaved.is_empty() {
        rows.push(TreeRow {
            key: AUTO_HEADING.into(),
            depth: 0,
            node: TreeNode::Heading("Auto-saved"),
            ancestors: Vec::new(),
        });
        for group in [
            AutoGroup::Today,
            AutoGroup::Yesterday,
            AutoGroup::Days2To6,
            AutoGroup::Older,
        ] {
            let items: Vec<_> = list
                .autosaved
                .iter()
                .filter(|a| a.group == group.as_str())
                .collect();
            if items.is_empty() {
                continue;
            }
            let key = format!("g:{}", group.as_str());
            rows.push(TreeRow {
                key: key.clone(),
                depth: 1,
                node: TreeNode::AutoGroup(group),
                ancestors: vec![AUTO_HEADING.into()],
            });
            for item in items {
                let (Ok(id), Ok(kind)) = (AutoId::parse(&item.id), SessionType::parse(&item.kind))
                else {
                    continue;
                };
                rows.push(TreeRow {
                    key: format!("a:{id}"),
                    depth: 2,
                    node: TreeNode::AutoSaved {
                        id,
                        kind,
                        label: item.label.clone(),
                    },
                    ancestors: vec![AUTO_HEADING.into(), key.clone()],
                });
            }
        }
    }
    rows
}

/// The rows below folder `path` (`""`: the root).
fn named_rows(
    path: &str,
    ancestors: &[String],
    list: &SessionListDto,
    folders: &[String],
    rows: &mut Vec<TreeRow>,
) {
    let parent_of = |full: &str| full.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
    let mut children: Vec<&String> = folders.iter().filter(|f| parent_of(f) == path).collect();
    children.sort_by_key(|f| f.to_lowercase());
    for folder in children {
        let key = format!("f:{folder}");
        rows.push(TreeRow {
            key: key.clone(),
            depth: ancestors.len(),
            node: TreeNode::Folder(folder.clone()),
            ancestors: ancestors.to_vec(),
        });
        let mut inner = ancestors.to_vec();
        inner.push(key);
        named_rows(folder, &inner, list, folders, rows);
    }
    let mut sessions: Vec<_> = list
        .sessions
        .iter()
        .filter(|s| parent_of(&s.reference) == path)
        .collect();
    sessions.sort_by_key(|s| (!s.favorite, s.reference.to_lowercase()));
    for item in sessions {
        let (Ok(reference), Ok(kind)) = (
            SessionRef::parse(&item.reference),
            SessionType::parse(&item.kind),
        ) else {
            continue;
        };
        rows.push(TreeRow {
            key: format!("n:{}", item.reference),
            depth: ancestors.len(),
            node: TreeNode::Named {
                reference,
                kind,
                description: item.description.clone(),
                locked: item.locked,
                favorite: item.favorite,
            },
            ancestors: ancestors.to_vec(),
        });
    }
}

/// [`resolve`] for a chosen type: paths of another kind are the panel's error (§1.4.8), never a view that fails.
fn resolve_as(service: &Service, request: &GuiOpenRequest) -> Result<HomeOpen, ApiError> {
    let open = resolve(
        service,
        &GuiOpenRequest {
            kind: None,
            ..request.clone()
        },
    )?;
    match request.kind {
        Some(kind) if kind != open.kind => Err(ApiError::InvalidArgument(format!(
            "These paths open as {}, not as {}.",
            open.kind.as_str(),
            kind.as_str()
        ))),
        _ => Ok(open),
    }
}

/// `resolve_open` + `open_view_from_ipc` (on the job thread; also the window's drops, UI-008): `clipboard://` reads
/// the GUI's own clipboard (D-101).
pub fn resolve(service: &Service, request: &GuiOpenRequest) -> Result<HomeOpen, ApiError> {
    let resolved = service.resolve_open(request)?;
    let view = service.open_view_from_ipc(resolved.kind, &resolved.specs, &resolved.options)?;
    Ok(HomeOpen {
        kind: resolved.kind,
        request: view,
        options: resolved.options,
        specs: resolved.specs,
    })
}
