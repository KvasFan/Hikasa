//! The rule tabs of the Session Settings dialog for Folder Compare and Folder Sync sessions (M2 W9c, SES-020, SES-022,
//! FC-050, FF-002, FS-002; D-249): Comparison, Name Filters, Handling, Misc and (Folder Sync) Sync, each a toolkit-free
//! model over a typed working copy of its part of `FolderRules` (and `SyncOptions`). A tab shows the P0 keys only
//! (D-128's rule) and writes only those over the rules it is given, so every other key stays as stored. Each change is
//! checked by core (`Service::check_folder_rules`, `SyncMatrix::validate`): a refusal stays on its tab with core's
//! message and OK is off (`ui/session_dialog.rs` draws them).

use std::any::Any;
use std::collections::BTreeMap;

use hikasa_core::{CaseSensitivity, ContentMethod, MaskOptions, NameFilter, NameFilterLists, Side};
use hikasa_folder::sync::{
    OrphanReading, Situation, SyncAction, SyncMatrix, SyncOptions, SyncPreset,
};
use hikasa_folder::{ArchiveHandling, CaseMode, FolderRules, TzOffset};
use hikasa_session::Service;
use hikasa_session::sessions::{SessionRules, SessionType};

use crate::session_dialog::{SessionTab, TabExtras};

/// The tab titles (also what `Rules` and `NameFilter` open, §1.9.18).
pub const COMPARISON: &str = "Comparison";
/// Name Filters.
pub const NAME_FILTERS: &str = "Name Filters";
/// Handling.
pub const HANDLING: &str = "Handling";
/// Misc.
pub const MISC: &str = "Misc";
/// Sync (Folder Sync sessions; a function, as the word is also a vocabulary word that check 7 watches in constants).
pub fn sync_title() -> &'static str {
    "Sync"
}

/// The folder rules of a session's rules (`None`: not a folder session).
pub fn folder_of(rules: &SessionRules) -> Option<&FolderRules> {
    match rules {
        SessionRules::Folder(rules) => Some(rules),
        SessionRules::Sync(rules) => Some(&rules.folder),
        _ => None,
    }
}

/// `rules` with its folder rules changed by `edit` (unchanged for a session that has none).
fn with_folder(rules: &SessionRules, edit: impl FnOnce(&mut FolderRules)) -> SessionRules {
    let mut out = rules.clone();
    match &mut out {
        SessionRules::Folder(rules) => edit(rules),
        SessionRules::Sync(rules) => edit(&mut rules.folder),
        _ => {}
    }
    out
}

/// The core check of a folder tab's rules: `check_folder_rules`'s message, or nothing.
fn core_check(svc: &Service, rules: &SessionRules) -> Option<String> {
    let folder = folder_of(rules)?;
    svc.check_folder_rules(folder).err().map(|e| e.to_string())
}

/// The rule tabs of a session type (D-122's registry adds them after Specs).
pub fn rule_tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>> {
    let mut tabs: Vec<Box<dyn SessionTab>> = Vec::new();
    if matches!(kind, SessionType::FolderCompare | SessionType::FolderSync) {
        tabs.push(Box::new(ComparisonTab::default()));
        tabs.push(Box::new(NameFiltersTab::default()));
        tabs.push(Box::new(HandlingTab::default()));
        tabs.push(Box::new(MiscTab::default()));
    }
    if kind == SessionType::FolderSync {
        tabs.push(Box::new(SyncTab::default()));
    }
    tabs
}

macro_rules! tab_any {
    () => {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    };
}

// ---- Comparison ----------------------------------------------------------------------------------------------------

/// The Comparison tab's keys (FC-020…FC-022, FC-025…FC-027).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonFields {
    /// `criteria.size`.
    pub size: bool,
    /// `criteria.timestamps.enabled`.
    pub timestamps: bool,
    /// `criteria.timestamps.tolerance_secs`, as typed.
    pub tolerance: String,
    /// *Ignore daylight saving difference*.
    pub ignore_dst: bool,
    /// *Ignore timezone differences*.
    pub ignore_timezone: bool,
    /// The shifted side of the offset (`None`: no offset).
    pub offset_side: Option<Side>,
    /// The offset's hours, as typed.
    pub offset_hours: String,
    /// `criteria.filename_case`.
    pub filename_case: bool,
    /// `criteria.content.method`.
    pub method: ContentMethod,
    /// *Skip if quick tests indicate files are the same*.
    pub skip_if_quick_same: bool,
    /// *Override quick test results*.
    pub override_quick: bool,
}

impl Default for ComparisonFields {
    fn default() -> ComparisonFields {
        ComparisonFields::of(&FolderRules::default())
    }
}

impl ComparisonFields {
    fn of(rules: &FolderRules) -> ComparisonFields {
        let t = &rules.criteria.timestamps;
        ComparisonFields {
            size: rules.criteria.size,
            timestamps: t.enabled,
            tolerance: t.tolerance_secs.to_string(),
            ignore_dst: t.ignore_dst,
            ignore_timezone: t.ignore_timezone,
            offset_side: t.offset.map(|o| o.side),
            offset_hours: t
                .offset
                .map_or_else(|| "0".to_owned(), |o| o.hours.to_string()),
            filename_case: rules.criteria.filename_case,
            method: rules.criteria.content.method,
            skip_if_quick_same: rules.criteria.content.skip_if_quick_same,
            override_quick: rules.criteria.content.override_quick,
        }
    }

    /// Writes the keys into `rules`; `Err` with the message of a field that is not a number.
    fn write(&self, rules: &mut FolderRules) -> Result<(), String> {
        let tolerance: u32 = self.tolerance.trim().parse().map_err(|_| {
            format!(
                "The tolerance `{}` is not a whole number of seconds from 0 to {}.",
                self.tolerance.trim(),
                u32::MAX
            )
        })?;
        let offset = match self.offset_side {
            None => None,
            Some(side) => {
                let hours: i8 = self.offset_hours.trim().parse().map_err(|_| {
                    format!(
                        "The offset `{}` is not a whole number of hours.",
                        self.offset_hours.trim()
                    )
                })?;
                Some(TzOffset { side, hours })
            }
        };
        let c = &mut rules.criteria;
        c.size = self.size;
        c.timestamps.enabled = self.timestamps;
        c.timestamps.tolerance_secs = tolerance;
        c.timestamps.ignore_dst = self.ignore_dst;
        c.timestamps.ignore_timezone = self.ignore_timezone;
        c.timestamps.offset = offset;
        c.filename_case = self.filename_case;
        c.content.method = self.method;
        c.content.skip_if_quick_same = self.skip_if_quick_same;
        c.content.override_quick = self.override_quick;
        Ok(())
    }
}

/// The content methods in the order the tab shows them.
pub const METHODS: [ContentMethod; 4] = [
    ContentMethod::None,
    ContentMethod::Crc,
    ContentMethod::Binary,
    ContentMethod::Rules,
];

/// The caption of a content method.
pub fn method_caption(method: ContentMethod) -> &'static str {
    match method {
        ContentMethod::None => "No content test",
        ContentMethod::Crc => "CRC comparison",
        ContentMethod::Binary => "Binary comparison",
        ContentMethod::Rules => "Rules-based comparison",
    }
}

/// The Comparison tab.
#[derive(Debug, Clone, Default)]
pub struct ComparisonTab {
    /// The working copy.
    pub fields: ComparisonFields,
    original: ComparisonFields,
    /// Core's or the field's message (OK is off).
    pub error: Option<String>,
}

impl SessionTab for ComparisonTab {
    fn title(&self) -> String {
        COMPARISON.to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        let mut result = Ok(());
        let rules = with_folder(current, |r| result = self.fields.write(r));
        result.ok().map(|()| rules)
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(folder) = folder_of(rules) {
            self.fields = ComparisonFields::of(folder);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let mut result = Ok(());
        let rules = with_folder(current, |r| result = self.fields.write(r));
        self.error = match result {
            Err(e) => Some(e),
            Ok(()) => core_check(svc, &rules),
        };
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Name Filters --------------------------------------------------------------------------------------------------

/// The four lists of the Name Filters tab (FC-050), one mask a line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ListFields {
    /// Include files.
    pub include_files: String,
    /// Exclude files.
    pub exclude_files: String,
    /// Include folders.
    pub include_folders: String,
    /// Exclude folders.
    pub exclude_folders: String,
}

impl ListFields {
    fn of(lists: &NameFilterLists) -> ListFields {
        ListFields {
            include_files: lists.include_files.join("\n"),
            exclude_files: lists.exclude_files.join("\n"),
            include_folders: lists.include_folders.join("\n"),
            exclude_folders: lists.exclude_folders.join("\n"),
        }
    }

    /// The lists: one item a non-empty line, trimmed of its line break only.
    pub fn lists(&self) -> NameFilterLists {
        let items = |text: &str| -> Vec<String> {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect()
        };
        NameFilterLists {
            include_files: items(&self.include_files),
            exclude_files: items(&self.exclude_files),
            include_folders: items(&self.include_folders),
            exclude_folders: items(&self.exclude_folders),
        }
    }
}

/// The mask options of the toolbar text (the syntax does not depend on the case; the comparison compiles again).
pub const TEXT_OPTIONS: MaskOptions = MaskOptions {
    case: CaseSensitivity::Sensitive,
};

/// The canonical toolbar text of four lists (core's `NameFilter::to_filter_string`, D-07's order); `Err`: core's
/// message of a mask that does not parse.
pub fn toolbar_text(lists: &NameFilterLists) -> Result<String, String> {
    NameFilter::from_lists(lists, TEXT_OPTIONS)
        .map(|f| f.to_filter_string())
        .map_err(|e| e.to_string())
}

/// The Name Filters tab (FC-050): the four lists, *Clear*, the presets, *Add To Presets*, the toolbar text below.
#[derive(Debug, Clone, Default)]
pub struct NameFiltersTab {
    /// The working copy.
    pub fields: ListFields,
    original: ListFields,
    /// The presets of `filters.toml`: name, mask.
    pub presets: Vec<(String, String)>,
    /// Core's message (OK is off).
    pub error: Option<String>,
    /// *Add To Presets* was pressed: its mask (the window prompts for a name).
    pub add_to_presets: Option<String>,
}

impl NameFiltersTab {
    /// *Clear*: no name filter.
    pub fn clear(&mut self) {
        self.fields = ListFields::default();
    }

    /// A preset chosen: its mask parsed by core into the four lists; `Err`: core's message.
    pub fn take_preset(&mut self, mask: &str) -> Result<(), String> {
        let filter = NameFilter::parse(mask, TEXT_OPTIONS).map_err(|e| e.to_string())?;
        self.fields = ListFields::of(&filter.to_lists());
        Ok(())
    }

    /// The toolbar text of the working copy (shown under the lists).
    pub fn text(&self) -> Result<String, String> {
        toolbar_text(&self.fields.lists())
    }

    /// *Add To Presets*: the mask the prompt saves (nothing for an empty or bad filter).
    pub fn ask_add_to_presets(&mut self) {
        if let Ok(text) = self.text()
            && !text.is_empty()
        {
            self.add_to_presets = Some(text);
        }
    }
}

impl SessionTab for NameFiltersTab {
    fn title(&self) -> String {
        NAME_FILTERS.to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields.lists() != self.original.lists()
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_folder(current, |r| {
            r.filters.names = self.fields.lists()
        }))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(folder) = folder_of(rules) {
            self.fields = ListFields::of(&folder.filters.names);
            self.original = self.fields.clone();
        }
    }

    fn prepare(&mut self, extras: &TabExtras) {
        self.presets.clone_from(&extras.presets);
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let rules = with_folder(current, |r| r.filters.names = self.fields.lists());
        self.error = core_check(svc, &rules);
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Handling ------------------------------------------------------------------------------------------------------

/// The Handling tab's keys (FC-010, FC-012…FC-015).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HandlingFields {
    /// `alignment.case`.
    pub case: CaseMode,
    /// *Align filenames with different extensions*.
    pub align_extensions: bool,
    /// *Align filenames with different Unicode normalization forms*.
    pub align_normalization: bool,
    /// FC-012: subfolders are scanned without being asked.
    pub auto_scan_subfolders: bool,
    /// FC-012: the view opens the folders.
    pub expand_on_load: bool,
    /// FC-012: of those, the ones with differences.
    pub expand_differences_only: bool,
    /// FC-012: orphan subfolders are scanned.
    pub scan_orphan_folders: bool,
    /// FC-013 *Follow symbolic links*.
    pub follow_symlinks: bool,
    /// FC-014 *Ignore folder structure*.
    pub flatten: bool,
    /// FC-015 archive handling.
    pub archives: ArchiveHandling,
}

impl HandlingFields {
    fn of(rules: &FolderRules) -> HandlingFields {
        let h = &rules.handling;
        HandlingFields {
            case: rules.alignment.case,
            align_extensions: rules.alignment.align_extensions,
            align_normalization: rules.alignment.align_normalization,
            auto_scan_subfolders: h.auto_scan_subfolders,
            expand_on_load: h.expand_on_load,
            expand_differences_only: h.expand_differences_only,
            scan_orphan_folders: h.scan_orphan_folders,
            follow_symlinks: h.follow_symlinks,
            flatten: h.flatten,
            archives: h.archives,
        }
    }

    fn write(&self, rules: &mut FolderRules) {
        rules.alignment.case = self.case;
        rules.alignment.align_extensions = self.align_extensions;
        rules.alignment.align_normalization = self.align_normalization;
        let h = &mut rules.handling;
        h.auto_scan_subfolders = self.auto_scan_subfolders;
        h.expand_on_load = self.expand_on_load;
        h.expand_differences_only = self.expand_differences_only;
        h.scan_orphan_folders = self.scan_orphan_folders;
        h.follow_symlinks = self.follow_symlinks;
        h.flatten = self.flatten;
        h.archives = self.archives;
    }
}

/// The case modes in the order the tab shows them, with their captions.
pub fn case_caption(case: CaseMode) -> &'static str {
    match case {
        CaseMode::Auto => "Automatic (as the file systems say)",
        CaseMode::Sensitive => "Case sensitive",
        CaseMode::Insensitive => "Case insensitive",
    }
}

/// The case modes.
pub const CASES: [CaseMode; 3] = [CaseMode::Auto, CaseMode::Sensitive, CaseMode::Insensitive];

/// The archive handling modes, with their captions.
pub fn archives_caption(mode: ArchiveHandling) -> &'static str {
    match mode {
        ArchiveHandling::AsFiles => "Compare archives as files",
        ArchiveHandling::OnceOpened => "Archives are folders once opened",
        ArchiveHandling::Always => "Archives are folders",
    }
}

/// The archive handling modes.
pub const ARCHIVES: [ArchiveHandling; 3] = [
    ArchiveHandling::AsFiles,
    ArchiveHandling::OnceOpened,
    ArchiveHandling::Always,
];

/// The Handling tab.
#[derive(Debug, Clone, Default)]
pub struct HandlingTab {
    /// The working copy.
    pub fields: HandlingFields,
    original: HandlingFields,
    /// Core's message (OK is off).
    pub error: Option<String>,
    /// A Folder Sync session: *Ignore folder structure* is not offered (a sync keeps the structure).
    pub sync: bool,
}

impl SessionTab for HandlingTab {
    fn title(&self) -> String {
        HANDLING.to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_folder(current, |r| self.fields.write(r)))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(folder) = folder_of(rules) {
            self.fields = HandlingFields::of(folder);
            self.original = self.fields;
        }
        self.sync = matches!(rules, SessionRules::Sync(_));
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let rules = with_folder(current, |r| self.fields.write(r));
        self.error = core_check(svc, &rules);
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Misc ----------------------------------------------------------------------------------------------------------

/// One format of the Misc tab (FF-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRow {
    /// The id.
    pub id: String,
    /// The name.
    pub name: String,
    /// Enabled in the global state (`format list`).
    pub global: bool,
    /// The session's override (`None`: the global state). Shown **bold** while set.
    pub session: Option<bool>,
}

impl FormatRow {
    /// Enabled for this comparison.
    pub fn effective(&self) -> bool {
        self.session.unwrap_or(self.global)
    }
}

/// The Misc tab's keys: the session's format overrides and the Ignored paths (FC-033).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MiscFields {
    /// `format_overrides` (every id, the ones no format of the list names too: kept as stored).
    pub overrides: BTreeMap<String, bool>,
    /// `ignored`.
    pub ignored: Vec<String>,
}

/// The Misc tab.
#[derive(Debug, Clone, Default)]
pub struct MiscTab {
    /// The working copy.
    pub fields: MiscFields,
    original: MiscFields,
    /// The registry's formats, in `format list` order, with the global state.
    pub formats: Vec<(String, String, bool)>,
    /// Core's message (OK is off).
    pub error: Option<String>,
}

impl MiscTab {
    /// The rows: each format with the session's override.
    pub fn rows(&self) -> Vec<FormatRow> {
        self.formats
            .iter()
            .map(|(id, name, global)| FormatRow {
                id: id.clone(),
                name: name.clone(),
                global: *global,
                session: self.fields.overrides.get(id).copied(),
            })
            .collect()
    }

    /// The check box of a format: the session's state set to `on`; back to the global state, no override.
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        let global = self
            .formats
            .iter()
            .find(|(f, _, _)| f == id)
            .is_some_and(|(_, _, g)| *g);
        if on == global {
            self.fields.overrides.remove(id);
        } else {
            self.fields.overrides.insert(id.to_owned(), on);
        }
    }

    /// *Reset*: the format follows the global state.
    pub fn reset(&mut self, id: &str) {
        self.fields.overrides.remove(id);
    }

    /// *Remove* of an Ignored path.
    pub fn remove_ignored(&mut self, index: usize) {
        if index < self.fields.ignored.len() {
            self.fields.ignored.remove(index);
        }
    }
}

impl SessionTab for MiscTab {
    fn title(&self) -> String {
        MISC.to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        Some(with_folder(current, |r| {
            r.format_overrides.clone_from(&self.fields.overrides);
            r.ignored.clone_from(&self.fields.ignored);
        }))
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(folder) = folder_of(rules) {
            self.fields = MiscFields {
                overrides: folder.format_overrides.clone(),
                ignored: folder.ignored.clone(),
            };
            self.original = self.fields.clone();
        }
    }

    fn prepare(&mut self, extras: &TabExtras) {
        self.formats.clone_from(&extras.formats);
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        let rules = self.rules(current).unwrap_or_else(|| current.clone());
        self.error = core_check(svc, &rules);
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}

// ---- Sync ----------------------------------------------------------------------------------------------------------

/// The Sync tab's keys (FS-002, FS-004, FS-005): `[rules.sync]` without the matrix of a preset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncFields {
    /// The options.
    pub options: SyncOptions,
}

/// The Sync tab (Folder Sync sessions).
#[derive(Debug, Clone, Default)]
pub struct SyncTab {
    /// The working copy.
    pub fields: SyncFields,
    original: SyncFields,
    /// `SyncMatrix::validate`'s message (OK is off).
    pub error: Option<String>,
}

/// The presets in the order the tab shows them.
pub const PRESETS: [SyncPreset; 6] = SyncPreset::ALL;

impl SyncTab {
    /// The matrix the tab shows: the preset's, or the stored one of `custom`.
    pub fn matrix(&self) -> SyncMatrix {
        self.fields.options.effective_matrix()
    }

    /// True when a cell may take `action` (`Situation::allows`): the others are disabled.
    pub fn allows(situation: Situation, action: SyncAction) -> bool {
        situation.allows(action)
    }

    /// A preset chosen.
    pub fn set_preset(&mut self, preset: SyncPreset) {
        if preset == SyncPreset::Custom {
            self.fields.options.matrix = self.matrix();
        }
        self.fields.options.preset = preset;
    }

    /// A cell changed: the matrix becomes `custom`, from the matrix shown.
    pub fn set_cell(&mut self, situation: Situation, action: SyncAction) {
        let mut matrix = self.matrix();
        matrix.set(situation, action);
        self.fields.options.matrix = matrix;
        self.fields.options.preset = SyncPreset::Custom;
    }

    /// *Remove* of an override.
    pub fn remove_override(&mut self, path: &str) {
        self.fields.options.overrides.remove(path);
    }
}

/// The orphan readings, with their captions.
pub fn orphans_caption(reading: OrphanReading) -> &'static str {
    match reading {
        OrphanReading::Added => "Read items on one side only as added",
        OrphanReading::Deleted => "Read items on one side only as deleted",
    }
}

impl SessionTab for SyncTab {
    fn title(&self) -> String {
        sync_title().to_owned()
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        let mut out = current.clone();
        if let SessionRules::Sync(rules) = &mut out {
            rules.sync = self.fields.options.clone();
        }
        Some(out)
    }

    fn load(&mut self, rules: &SessionRules) {
        if let SessionRules::Sync(rules) = rules {
            self.fields = SyncFields {
                options: rules.sync.clone(),
            };
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, _svc: &Service, _current: &SessionRules) {
        self.error = self
            .fields
            .options
            .matrix
            .validate()
            .and_then(|()| self.fields.options.validate())
            .err()
            .map(|e| e.to_string());
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    tab_any!();
}
