//! The browse dialog (M2 W9a, Q5 = a; D-240): an in-house dialog over the VFS, so archives, remote folders and every
//! back-end browse alike. Places (Home, the current folder, `/` or the drives, the view's bases, `mru.folders`), a
//! location with an Up button (out of an archive's root to its folder), the listing of `vfs_ls` in a job (folders first,
//! archives open as folders), hidden entries off by default, a substring filter of the shown list. Modes Folder, File and
//! SaveFile. Toolkit-free; `ui/browse.rs` draws it.

use std::any::Any;
use std::ffi::OsStr;

use hikasa_core::JobKind;
use hikasa_core::spec::Spec;
use hikasa_session::api::paths::{BrowseListing, PlaceDto, PlaceKind, browse_child};
use hikasa_session::ipc::ViewId;
use hikasa_session::{Platform, Service};
use hikasa_vfs::EntryKind;

use crate::dialog::{
    ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Followup, Prompt,
};
use crate::job::{Job, Wake};

/// The help topic of the dialog.
pub const BROWSE_TOPIC: &str = "dialog-browse";

/// The `[dialogs]` id of its options.
pub const BROWSE_DIALOG: &str = "browse";

/// What the dialog picks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseMode {
    /// A folder (an archive counts: it opens as one).
    Folder,
    /// An existing file.
    File,
    /// A file to write: a name in the chosen folder.
    SaveFile {
        /// The name offered.
        default_name: String,
    },
}

/// Where the answer goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseTarget {
    /// A path edit of a folder view (0 left, 1 right): committed as if typed and accepted.
    PathEdit {
        /// The view.
        view: ViewId,
        /// The side.
        side: usize,
    },
    /// The script path of the Run Script dialog.
    RunScript,
    /// The package file of the Export wizard (SaveFile).
    Export,
    /// A field of a Home tab's management panel (0 left, 1 right).
    Home {
        /// The Home tab.
        view: ViewId,
        /// The side.
        side: usize,
    },
    /// A spec of the Session Settings dialog's Specs tab (0 left, 1 right).
    Specs(usize),
    /// A path key of the Options dialog (its dotted path).
    OptionKey(&'static str),
    /// The target of a Copy/Move to Folder dialog of a view (M2 W9b).
    OpTarget(ViewId),
    /// The second item of Compare To (M2 W9b): the view whose selected item is the first.
    CompareTo(ViewId),
    /// Save As of a text view's pane (M2 W11a, D-300); `local`: Save on File System As.
    TextSaveAs {
        /// The view.
        view: ViewId,
        /// The pane.
        side: hikasa_core::Side,
        /// No archive member, no non-local place.
        local: bool,
    },
}

/// The last name of a typed path (the SaveFile name offered).
pub fn file_name_of(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The folder of a typed path (where SaveFile starts).
pub fn folder_of(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One entry of the listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseEntry {
    /// The name.
    pub name: String,
    /// A folder.
    pub folder: bool,
    /// A file `archives.toml` knows: shown with the zipper, opens as a folder.
    pub archive: bool,
    /// The name is not valid Unicode: shown with its replacement characters and marked, never picked (no spec can
    /// name it back).
    pub lossy: bool,
}

/// What OK answers while a folder is being listed.
pub const WAIT_LISTING: &str = "Wait for the listing to end.";

/// The tooltip and message of an entry whose name is not valid Unicode.
pub const LOSSY_NAME: &str = "This name is not valid Unicode: it cannot be picked here.";

impl BrowseEntry {
    /// Opens as a folder (a folder or an archive).
    pub fn opens(&self) -> bool {
        self.folder || self.archive
    }

    /// Hidden unless *Show hidden items*: the name starts with `.`.
    pub fn hidden(&self) -> bool {
        self.name.starts_with('.')
    }
}

/// The browse dialog.
pub struct BrowseDialog {
    /// The mode.
    pub mode: BrowseMode,
    /// The folder listed (a spec as typed or canonical).
    pub location: String,
    /// The text of the location field (Enter goes there).
    pub typed: String,
    /// The listing of `location` (folders first, then by name, as `vfs_ls` sorts).
    pub entries: Vec<BrowseEntry>,
    /// The filter of the shown list (a case-insensitive substring).
    pub filter: String,
    /// The selected entry (an index into `entries`).
    pub selected: Option<usize>,
    /// *Show hidden items*.
    pub show_hidden: bool,
    /// SaveFile: the name.
    pub name: String,
    /// The places list.
    pub places: Vec<PlaceDto>,
    /// The listing's error (shown in the list), or a refused OK.
    pub error: Option<String>,
    /// SaveFile: the name whose replacement was confirmed (another name asks again).
    pub replace_asked: Option<String>,
    /// The benchmark's scroll offset of the listing (G15 browse; `None`: the user's).
    pub bench_scroll: Option<f32>,
    requests: Vec<DialogRequest>,
    target: BrowseTarget,
    bases: Vec<String>,
    listing: Option<Job<BrowseListing>>,
    /// The listed folder (resolved) and its parent's spec, from the listing's job: a child's spec and Up need no
    /// reading on the window's thread.
    folder: Option<Spec>,
    parent: Option<String>,
    /// Names compare case-insensitively (Windows, macOS): SaveFile's "Replace?".
    fold_names: bool,
    places_job: Option<Job<Vec<PlaceDto>>>,
    /// A location to list once the dialog is polled (the listing needs the service).
    pending: Option<String>,
    wake: Wake,
}

impl BrowseDialog {
    /// A dialog in `mode` starting at `start` (a folder, or a file whose folder is listed), for `target`; `bases` are
    /// the view's bases (places).
    pub fn new(
        mode: BrowseMode,
        start: &str,
        target: BrowseTarget,
        bases: Vec<String>,
        show_hidden: bool,
        wake: Wake,
    ) -> BrowseDialog {
        let name = match &mode {
            BrowseMode::SaveFile { default_name } => default_name.clone(),
            _ => String::new(),
        };
        BrowseDialog {
            mode,
            location: start.to_owned(),
            typed: start.to_owned(),
            entries: Vec::new(),
            filter: String::new(),
            selected: None,
            show_hidden,
            name,
            places: Vec::new(),
            error: None,
            replace_asked: None,
            bench_scroll: None,
            requests: Vec::new(),
            target,
            bases,
            listing: None,
            folder: None,
            parent: None,
            fold_names: matches!(Platform::HOST, Platform::Windows | Platform::MacOs),
            places_job: None,
            pending: Some(start.to_owned()),
            wake,
        }
    }

    /// The indexes of the entries shown: hidden ones only with *Show hidden items*, then the filter.
    pub fn shown(&self) -> Vec<usize> {
        let filter = hikasa_core::case::fold(&self.filter).into_owned();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| self.show_hidden || !e.hidden())
            .filter(|(_, e)| {
                filter.is_empty() || hikasa_core::case::fold(&e.name).contains(filter.as_str())
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Lists `location` (Enter in the location field, a place, a double-click on a folder). The entries of the
    /// previous folder go at once: nothing is picked from them while the new listing runs.
    pub fn go(&mut self, location: &str) {
        if let Some(job) = self.listing.take() {
            job.cancel();
        }
        self.location = location.to_owned();
        self.typed = location.to_owned();
        self.pending = Some(location.to_owned());
        self.entries.clear();
        self.folder = None;
        self.parent = None;
        self.selected = None;
        self.replace_asked = None;
        self.error = None;
    }

    /// Names compare by the platform's rule (case-insensitive on Windows and macOS).
    pub fn same_name(&self, a: &str, b: &str) -> bool {
        if self.fold_names {
            hikasa_core::case::fold(a) == hikasa_core::case::fold(b)
        } else {
            a == b
        }
    }

    /// True while the listing runs.
    pub fn listing(&self) -> bool {
        self.listing.is_some() || self.pending.is_some()
    }

    /// The spec of entry `index` in the listed folder (pure: the listing's job resolved the folder); `None` for a name
    /// that is not valid Unicode (no spec names it back) or before the listing arrived.
    pub fn child(&self, index: usize) -> Option<String> {
        let entry = self.entries.get(index)?;
        if entry.lossy {
            return None;
        }
        browse_child(self.folder.as_ref()?, &entry.name)
    }

    /// Up: the parent of the listed folder (out of an archive's root into its folder), as its listing gave it.
    pub fn up(&mut self) {
        if let Some(parent) = self.parent.clone() {
            self.go(&parent);
        }
    }

    /// A double-click or Enter on entry `index`: a folder or an archive opens; a file in File mode is the answer. A
    /// name that is not valid Unicode is never picked.
    pub fn open(&mut self, index: usize) -> DialogStep {
        let Some(entry) = self.entries.get(index).cloned() else {
            return DialogStep::Stay;
        };
        if entry.lossy {
            self.error = Some(LOSSY_NAME.to_owned());
            return DialogStep::Stay;
        }
        let Some(spec) = self.child(index) else {
            return DialogStep::Stay;
        };
        // A folder or an archive opens (an archive's members can be picked, Q5).
        if entry.opens() {
            self.go(&spec);
            return DialogStep::Stay;
        }
        if self.mode == BrowseMode::File {
            return self.answer(spec, false);
        }
        self.selected = Some(index);
        DialogStep::Stay
    }

    /// The answer of OK, if the mode has one now. Nothing is answered while the listing runs (the entries of the
    /// folder are not known yet: a SaveFile name could replace one).
    fn ok(&mut self) -> DialogStep {
        self.error = None;
        // The folder is known once its listing arrived (`go` forgets the previous one).
        if self.folder.is_none() {
            self.error = Some(if self.listing() {
                WAIT_LISTING.to_owned()
            } else {
                "This folder cannot be listed.".to_owned()
            });
            return DialogStep::Stay;
        }
        if self
            .selected
            .and_then(|i| self.entries.get(i))
            .is_some_and(|e| e.lossy)
        {
            self.error = Some(LOSSY_NAME.to_owned());
            return DialogStep::Stay;
        }
        match self.mode.clone() {
            BrowseMode::Folder => {
                let spec = match self.selected {
                    Some(i) if self.entries.get(i).is_some_and(BrowseEntry::opens) => self.child(i),
                    _ => Some(self.location.clone()),
                };
                spec.map_or(DialogStep::Stay, |s| self.answer(s, true))
            }
            BrowseMode::File => match self.selected {
                Some(i) if self.entries.get(i).is_some_and(|e| !e.folder) => match self.child(i) {
                    Some(spec) => self.answer(spec, false),
                    None => DialogStep::Stay,
                },
                Some(i) => self.open(i),
                None => {
                    self.error = Some("Select a file.".to_owned());
                    DialogStep::Stay
                }
            },
            BrowseMode::SaveFile { .. } => {
                let name = self.name.trim().to_owned();
                if name.is_empty() {
                    self.error = Some("Type a file name.".to_owned());
                    return DialogStep::Stay;
                }
                let exists = self.entries.iter().any(|e| self.same_name(&e.name, &name));
                let asked = self
                    .replace_asked
                    .as_deref()
                    .is_some_and(|a| self.same_name(a, &name));
                if exists && !asked {
                    self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
                        "Replace",
                        format!("Replace {name}?"),
                        ButtonRole::No,
                        vec![
                            (ButtonRole::Yes, "Yes", vec![Followup::BrowseReplace]),
                            (ButtonRole::No, "No", Vec::new()),
                        ],
                        BROWSE_TOPIC,
                    ))));
                    return DialogStep::Stay;
                }
                match self.folder.as_ref().and_then(|f| browse_child(f, &name)) {
                    Some(spec) => self.answer(spec, false),
                    None => {
                        self.error = Some(format!("{name} is not a file name."));
                        DialogStep::Stay
                    }
                }
            }
        }
    }

    fn answer(&self, spec: String, folder: bool) -> DialogStep {
        DialogStep::CloseWith(vec![Followup::Browsed {
            target: self.target.clone(),
            answer: Some((spec, folder)),
            show_hidden: self.show_hidden,
        }])
    }
}

impl DialogModel for BrowseDialog {
    fn title(&self) -> String {
        match self.mode {
            BrowseMode::Folder => "Browse for Folder",
            BrowseMode::File => "Browse for File",
            BrowseMode::SaveFile { .. } => "Save As",
        }
        .to_owned()
    }

    fn help_topic(&self) -> &'static str {
        BROWSE_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: !self.listing(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => self.ok(),
            _ => DialogStep::CloseWith(vec![Followup::Browsed {
                target: self.target.clone(),
                answer: None,
                show_hidden: self.show_hidden,
            }]),
        }
    }

    fn poll(&mut self, svc: &Service) -> bool {
        let mut changed = false;
        if self.places.is_empty() && self.places_job.is_none() {
            self.places_job = Some(Job::spawn(
                svc,
                JobKind::VfsRead,
                "the places".into(),
                &self.wake,
                |svc, job| Ok(svc.browse_places(job)),
            ));
        }
        if let Some(result) = self.places_job.as_ref().and_then(Job::try_result) {
            let mut places = result.unwrap_or_default();
            // The view's bases come after the roots, before the recent folders.
            let at = places
                .iter()
                .position(|p| p.kind == PlaceKind::Recent)
                .unwrap_or(places.len());
            for (k, base) in self.bases.iter().enumerate().rev() {
                places.insert(
                    at,
                    PlaceDto {
                        label: format!("{} base: {base}", if k == 0 { "Left" } else { "Right" }),
                        spec: base.clone(),
                        kind: PlaceKind::Base,
                    },
                );
            }
            self.places = places;
            changed = true;
        }
        if let Some(location) = self.pending.take() {
            let wake = std::sync::Arc::clone(&self.wake);
            self.listing = Some(Job::spawn(
                svc,
                JobKind::VfsRead,
                "list the folder".into(),
                &wake,
                move |svc, job| svc.browse_list(OsStr::new(&location), job),
            ));
            changed = true;
        }
        if let Some(result) = self.listing.as_ref().and_then(Job::try_result) {
            self.listing = None;
            match result {
                Ok(listed) => {
                    self.error = None;
                    let dto = listed.listing;
                    self.entries = dto
                        .entries
                        .into_iter()
                        .map(|e| {
                            let folder = e.kind == EntryKind::Dir.as_str();
                            BrowseEntry {
                                archive: !folder && svc.archive_format_of(&e.name).is_some(),
                                folder,
                                lossy: e.name_lossy,
                                name: e.name,
                            }
                        })
                        .collect();
                    // Folders first; `vfs_ls` sorted by name (the D-20 key), the stable sort keeps it.
                    self.entries.sort_by_key(|e| !e.folder);
                    self.location = dto.spec;
                    self.typed.clone_from(&self.location);
                    self.folder = Some(listed.folder);
                    self.parent = listed.parent;
                }
                Err(e) => {
                    self.entries.clear();
                    self.error = Some(e.to_string());
                }
            }
            changed = true;
        }
        changed
    }

    fn busy(&self) -> bool {
        self.listing.is_some()
    }

    fn take_requests(&mut self) -> Vec<DialogRequest> {
        std::mem::take(&mut self.requests)
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((760.0, 420.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
