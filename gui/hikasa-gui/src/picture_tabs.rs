//! The rule tabs of a Picture Compare session (SES-020; M2 W13b, §1.13.18, D-370): **Picture** (mode, tolerance,
//! blend percentage, *Ignore unimportant differences*, *Auto Scale*, tone map, maximum megapixels) and **Format** (each
//! side's Picture format, *Auto* = none). The values are words parsed by core (`PictureRulesFile::of_words`,
//! `Service::check_picture_words`); a refusal is shown in the tab and disables OK. A tab writes only the keys it changed
//! (T-61's identity: open and OK without edits writes nothing). Toolkit-free.

use std::any::Any;

use hikasa_session::Service;
use hikasa_session::api::picture::PictureRulesFile;
use hikasa_session::api::{PictureCompareRequest, PictureRuleWords};
use hikasa_session::sessions::{SessionRules, SessionType, Specs};

use crate::session_dialog::SessionTab;

/// The Picture tab's title (`Rules` opens the dialog on it).
pub const PICTURE: &str = "Picture";
/// The Format tab's title.
pub const FORMAT: &str = "Format";
/// The help topic of the tabs.
pub const TOPIC: &str = "dialog-session-settings";

/// The mode words with their captions.
pub const MODES: [(&str, &str); 3] = [
    ("tolerance", "Tolerance"),
    ("range", "Mismatch range"),
    ("blend", "Blend"),
];

/// The tone map words with their captions.
pub const TONE_MAPS: [(&str, &str); 2] = [
    ("clamp", "Clamp (SDR as authored)"),
    ("reinhard", "Reinhard (keeps highlights)"),
];

/// The rule tabs of a session type: Picture and Format for `picture-compare`.
pub fn rule_tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>> {
    if kind != SessionType::PictureCompare {
        return Vec::new();
    }
    vec![
        Box::new(PictureTab::default()),
        Box::new(PictureFormatTab::default()),
    ]
}

/// The picture words of a session's rules.
fn file_of(rules: &SessionRules) -> Option<&PictureRulesFile> {
    match rules {
        SessionRules::Picture(p) => Some(&p.picture),
        _ => None,
    }
}

/// The Picture tab's fields, as written (numbers as text).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PictureFields {
    /// `tolerance`, `range`, `blend`.
    pub mode: String,
    /// 0…255.
    pub tolerance: String,
    /// 0…100.
    pub blend: String,
    /// Ignore unimportant differences.
    pub ignore_unimportant: bool,
    /// Auto Scale.
    pub auto_scale: bool,
    /// `clamp`, `reinhard`.
    pub tone_map: String,
    /// 1…4,096.
    pub max_megapixels: String,
}

impl PictureFields {
    /// The fields of a file.
    pub fn of(file: &PictureRulesFile) -> PictureFields {
        PictureFields {
            mode: file.mode.clone(),
            tolerance: file.tolerance.to_string(),
            blend: file.blend.to_string(),
            ignore_unimportant: file.ignore_unimportant,
            auto_scale: file.auto_scale,
            tone_map: file.tone_map.clone(),
            max_megapixels: file.max_megapixels.to_string(),
        }
    }

    /// Writes the fields that differ from `read` into `file`, each parsed by core; the first refusal is the error.
    pub fn write(&self, read: &PictureFields, file: &mut PictureRulesFile) -> Result<(), String> {
        let parse =
            |words: PictureRuleWords| PictureRulesFile::of_words(&words).map_err(|e| e.to_string());
        if self.mode != read.mode {
            file.mode = parse(PictureRuleWords {
                mode: Some(self.mode.clone()),
                ..PictureRuleWords::default()
            })?
            .mode;
        }
        if self.tolerance != read.tolerance {
            file.tolerance = parse(PictureRuleWords {
                tolerance: Some(self.tolerance.clone()),
                ..PictureRuleWords::default()
            })?
            .tolerance;
        }
        if self.blend != read.blend {
            // The percentage is kept in any mode (D-349); checked as `--mode blend --blend <n>`.
            file.blend = parse(PictureRuleWords {
                mode: Some("blend".to_owned()),
                blend: Some(self.blend.clone()),
                ..PictureRuleWords::default()
            })?
            .blend;
        }
        if self.ignore_unimportant != read.ignore_unimportant {
            file.ignore_unimportant = self.ignore_unimportant;
        }
        if self.auto_scale != read.auto_scale {
            file.auto_scale = self.auto_scale;
        }
        if self.tone_map != read.tone_map {
            file.tone_map = parse(PictureRuleWords {
                tone_map: Some(self.tone_map.clone()),
                ..PictureRuleWords::default()
            })?
            .tone_map;
        }
        if self.max_megapixels != read.max_megapixels {
            file.max_megapixels = parse(PictureRuleWords {
                max_megapixels: Some(self.max_megapixels.clone()),
                ..PictureRuleWords::default()
            })?
            .max_megapixels;
        }
        Ok(())
    }
}

/// The Picture tab.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PictureTab {
    /// The working copy.
    pub fields: PictureFields,
    original: PictureFields,
    /// Core's message.
    pub error: Option<String>,
}

impl PictureTab {
    fn with_fields(&self, current: &SessionRules) -> Result<SessionRules, String> {
        let mut out = current.clone();
        if let SessionRules::Picture(p) = &mut out {
            self.fields.write(&self.original, &mut p.picture)?;
        }
        Ok(out)
    }
}

impl SessionTab for PictureTab {
    fn title(&self) -> String {
        PICTURE.to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn changes_rules(&self) -> bool {
        self.fields != self.original
    }

    fn rules(&self, current: &SessionRules) -> Option<SessionRules> {
        if !self.changes_rules() {
            return None;
        }
        self.with_fields(current).ok()
    }

    fn load(&mut self, rules: &SessionRules) {
        if let Some(file) = file_of(rules) {
            self.fields = PictureFields::of(file);
            self.original = self.fields.clone();
        }
    }

    fn check(&mut self, svc: &Service, current: &SessionRules) {
        self.error = match self.with_fields(current) {
            Err(e) => Some(e),
            Ok(rules) => file_of(&rules).and_then(|file| {
                let request = PictureCompareRequest {
                    rules: file.words(),
                    ..PictureCompareRequest::default()
                };
                svc.check_picture_words(&request)
                    .err()
                    .map(|e| e.to_string())
            }),
        };
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

/// The Format tab's fields: each side's format name (`""`: *Auto*).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PictureFormatFields {
    /// The left side's format.
    pub left: String,
    /// The right side's format.
    pub right: String,
}

/// The Format tab: each side's Picture format (this view only, D-82).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PictureFormatTab {
    /// The working copy.
    pub fields: PictureFormatFields,
    original: PictureFormatFields,
    /// The names the window offers (the enabled formats).
    pub formats: Vec<String>,
    /// The specs are disabled (a defaults entry): nothing shows or is written.
    pub disabled: bool,
    /// Core's message.
    pub error: Option<String>,
}

impl PictureFormatTab {
    /// Fills the tab from the session's specs.
    pub fn load_specs(&mut self, specs: &Specs, disabled: bool) {
        let both = specs.format.clone();
        let side = |own: &str| {
            if own.is_empty() {
                both.clone()
            } else {
                own.to_owned()
            }
        };
        self.fields = PictureFormatFields {
            left: side(&specs.left_format),
            right: side(&specs.right_format),
        };
        self.original = self.fields.clone();
        self.disabled = disabled;
    }
}

impl SessionTab for PictureFormatTab {
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
        if self.disabled || self.fields == self.original {
            return;
        }
        specs.format.clear();
        specs.left_format = self.fields.left.trim().to_owned();
        specs.right_format = self.fields.right.trim().to_owned();
    }

    fn prepare(&mut self, extras: &crate::session_dialog::TabExtras) {
        self.formats = extras
            .formats
            .iter()
            .filter(|(_, _, enabled)| *enabled)
            .map(|(_, name, _)| name.clone())
            .collect();
    }

    fn check(&mut self, svc: &Service, _current: &SessionRules) {
        if self.disabled {
            self.error = None;
            return;
        }
        let word = |t: &str| {
            let t = t.trim();
            (!t.is_empty()).then(|| t.to_owned())
        };
        let request = PictureCompareRequest {
            formats: hikasa_core::Pair::new(word(&self.fields.left), word(&self.fields.right)),
            ..PictureCompareRequest::default()
        };
        self.error = svc
            .check_picture_words(&request)
            .err()
            .map(|e| e.to_string());
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
