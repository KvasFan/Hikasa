//! The rule tabs of a Hex Compare session (M2 W13a, contract §1.13.18, D-370): **Alignment** (the mode None / Fast /
//! Complete; the block size, a power of two 1 KiB…1 MiB; skew; window; budget in MiB; *Load files up to N MiB into
//! memory without keeping them open* = `unlocked_max_mib`) and **Format** (per side *Auto* or an enabled Hex format).
//! The values are words parsed by core (`HexRulesFile::of_words`; sizes accept `8k`, `1MiB`): a refusal is shown in
//! the tab and disables OK. A tab writes only the keys it changed (T-61's identity): open and OK without edits writes
//! nothing. The GUI parses none of the words (check 7).

use std::any::Any;

use hikasa_session::Service;
use hikasa_session::api::HexRuleWords;
use hikasa_session::api::hex::{HexRulesFile, HexSessionRules};
use hikasa_session::sessions::{SessionRules, SessionType, Specs};

use crate::session_dialog::SessionTab;

/// The Alignment tab's title (`Rules` opens the dialog on it).
pub const ALIGNMENT: &str = "Alignment";
/// The Format tab's title.
pub const FORMAT: &str = "Format";
/// The help topic of the tabs.
pub const TOPIC: &str = "dialog-session-settings";

/// The modes with their captions.
pub const MODES: [(&str, &str); 3] = [
    ("none", "None (byte n with byte n)"),
    ("fast", "Fast (blocks, small gaps byte by byte)"),
    (
        "complete",
        "Complete (every gap byte by byte, within the budget)",
    ),
];

/// The block sizes offered (powers of two, 1 KiB…1 MiB).
pub const BLOCKS: [(&str, &str); 11] = [
    ("1024", "1 KiB"),
    ("2048", "2 KiB"),
    ("4096", "4 KiB"),
    ("8192", "8 KiB"),
    ("16384", "16 KiB"),
    ("32768", "32 KiB"),
    ("65536", "64 KiB"),
    ("131072", "128 KiB"),
    ("262144", "256 KiB"),
    ("524288", "512 KiB"),
    ("1048576", "1 MiB"),
];

/// The rule tabs of a session type: Alignment and Format for `hex-compare`.
pub fn rule_tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>> {
    if kind != SessionType::HexCompare {
        return Vec::new();
    }
    vec![
        Box::new(AlignmentTab::default()),
        Box::new(HexFormatTab::default()),
    ]
}

/// The hex rules of a session.
fn hex_of(rules: &SessionRules) -> Option<&HexRulesFile> {
    match rules {
        SessionRules::Hex(h) => Some(&h.hex),
        _ => None,
    }
}

/// The Alignment tab's fields: words as typed (the file's values as text when loaded).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlignmentFields {
    /// `alignment`.
    pub mode: String,
    /// `block` (bytes).
    pub block: String,
    /// `skew` (bytes; `16MiB` accepted).
    pub skew: String,
    /// `window` (bytes).
    pub window: String,
    /// `budget_mib`.
    pub budget_mib: String,
    /// `unlocked_max_mib`.
    pub unlocked_max_mib: String,
}

impl AlignmentFields {
    fn of(file: &HexRulesFile) -> AlignmentFields {
        let w = file.words();
        let s = |v: Option<String>| v.unwrap_or_default();
        AlignmentFields {
            mode: s(w.alignment),
            block: s(w.block),
            skew: s(w.skew),
            window: s(w.window),
            budget_mib: s(w.budget_mib),
            unlocked_max_mib: s(w.unlocked_max_mib),
        }
    }

    /// The words of `current` with the fields that differ from `original` in place (only the keys changed).
    fn words_over(&self, original: &AlignmentFields, current: &HexRulesFile) -> HexRuleWords {
        let mut words = current.words();
        let put = |mine: &str, orig: &str, slot: &mut Option<String>| {
            if mine != orig {
                *slot = Some(mine.to_owned());
            }
        };
        put(&self.mode, &original.mode, &mut words.alignment);
        put(&self.block, &original.block, &mut words.block);
        put(&self.skew, &original.skew, &mut words.skew);
        put(&self.window, &original.window, &mut words.window);
        put(
            &self.budget_mib,
            &original.budget_mib,
            &mut words.budget_mib,
        );
        put(
            &self.unlocked_max_mib,
            &original.unlocked_max_mib,
            &mut words.unlocked_max_mib,
        );
        words
    }
}

/// The Alignment tab.
#[derive(Debug, Clone, Default)]
pub struct AlignmentTab {
    /// The fields.
    pub fields: AlignmentFields,
    original: AlignmentFields,
    error: Option<String>,
}

impl AlignmentTab {
    /// The rules this tab leaves: `current` with the changed keys parsed by core.
    fn edited(&self, current: &SessionRules) -> Result<SessionRules, String> {
        let Some(file) = hex_of(current) else {
            return Ok(current.clone());
        };
        if self.fields == self.original {
            return Ok(current.clone());
        }
        let words = self.fields.words_over(&self.original, file);
        let parsed = HexRulesFile::of_words(&words).map_err(|e| e.to_string())?;
        // Only the changed keys: the others keep the session's values exactly (T-61).
        let mut out = file.clone();
        let changed = |a: &str, b: &str| a != b;
        if changed(&self.fields.mode, &self.original.mode) {
            out.alignment = parsed.alignment;
        }
        if changed(&self.fields.block, &self.original.block) {
            out.block = parsed.block;
        }
        if changed(&self.fields.skew, &self.original.skew) {
            out.skew = parsed.skew;
        }
        if changed(&self.fields.window, &self.original.window) {
            out.window = parsed.window;
        }
        if changed(&self.fields.budget_mib, &self.original.budget_mib) {
            out.budget_mib = parsed.budget_mib;
        }
        if changed(
            &self.fields.unlocked_max_mib,
            &self.original.unlocked_max_mib,
        ) {
            out.unlocked_max_mib = parsed.unlocked_max_mib;
        }
        Ok(SessionRules::Hex(Box::new(HexSessionRules { hex: out })))
    }
}

impl SessionTab for AlignmentTab {
    fn title(&self) -> String {
        ALIGNMENT.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        self.edited(current).ok()
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(file) = hex_of(rules) {
            self.fields = AlignmentFields::of(file);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, _svc: &Service, current: &SessionRules) {
        self.error = self.edited(current).err();
    }

    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The Format tab (W11c's Format tab over Hex formats): per side *Auto* (`""`) or an enabled Hex format; it edits the
/// session's `[specs] left_format` / `right_format`.
#[derive(Debug, Clone, Default)]
pub struct HexFormatTab {
    /// The left format (`""`: Auto).
    pub left: String,
    /// The right format.
    pub right: String,
    /// The enabled Hex formats: id, name (read once from the registry).
    pub formats: Option<Vec<(String, String)>>,
    original: (String, String),
    loaded: bool,
}

impl SessionTab for HexFormatTab {
    fn title(&self) -> String {
        FORMAT.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        false
    }

    fn rules(&self, _current: &SessionRules) -> Option<SessionRules> {
        None
    }

    fn specs(&self, specs: &mut Specs) {
        if !self.loaded {
            return;
        }
        if self.left != self.original.0 {
            specs.left_format.clone_from(&self.left);
        }
        if self.right != self.original.1 {
            specs.right_format.clone_from(&self.right);
        }
    }

    fn check(&mut self, svc: &Service, _current: &SessionRules) {
        if self.formats.is_none() {
            self.formats = Some(
                svc.format_list()
                    .formats
                    .into_iter()
                    .filter(|f| f.enabled && f.kind == "hex")
                    .map(|f| (f.id, f.name))
                    .collect(),
            );
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl HexFormatTab {
    /// Fills the tab from the session's specs (the dialog calls it with the session it opened).
    pub fn load_specs(&mut self, specs: &Specs) {
        self.left.clone_from(&specs.left_format);
        self.right.clone_from(&specs.right_format);
        self.original = (self.left.clone(), self.right.clone());
        self.loaded = true;
    }
}
