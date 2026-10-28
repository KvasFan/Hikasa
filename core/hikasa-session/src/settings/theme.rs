//! `theme.toml` and `themes/<name>.toml` (UI-020…023, D-78; docs/14 §2.1): the scheme chosen for light and for
//! dark mode, the four fonts, and the colour schemes.
//!
//! The built-in schemes `light` and `dark` are embedded (their values come from the M1 palettes of the GUI). A
//! file `themes/light.toml` or `themes/dark.toml` overrides elements of the built-in; any other scheme names a
//! `base` whose values fill the elements it does not set. [`Scheme::resolve`] gives every element a value.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml::Table;

use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KindRule, ValueKind, is_scheme_name};

settings_enum! {
    /// `[appearance] theme_mode` of `options.toml` (OPT-005, UI-020).
    ThemeMode { FollowOs = "follow_os", Light = "light", Dark = "dark" } default FollowOs
}

settings_enum! {
    /// The built-in scheme a scheme file is based on.
    SchemeBase { Light = "light", Dark = "dark" } default Light
}

/// The built-in scheme names.
pub const BUILTIN_SCHEMES: [&str; 2] = ["light", "dark"];

/// A colour: `#rrggbb` or `#rrggbbaa` (written in lower case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Colour {
    /// Red, green, blue.
    pub rgb: [u8; 3],
    /// Alpha, when the colour was written with one.
    pub alpha: Option<u8>,
}

impl Colour {
    /// An opaque colour from `0xrrggbb`.
    pub const fn hex(rgb: u32) -> Colour {
        Colour {
            rgb: [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8],
            alpha: None,
        }
    }

    /// Parses `#rrggbb` or `#rrggbbaa` (any case).
    pub fn parse(text: &str) -> Result<Colour, String> {
        let bad = || format!("{text:?} is not a colour (#rrggbb or #rrggbbaa)");
        let digits = text.strip_prefix('#').ok_or_else(bad)?;
        if !(digits.len() == 6 || digits.len() == 8)
            || !digits.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(bad());
        }
        let byte = |k: usize| u8::from_str_radix(&digits[k..k + 2], 16).map_err(|_| bad());
        Ok(Colour {
            rgb: [byte(0)?, byte(2)?, byte(4)?],
            alpha: if digits.len() == 8 {
                Some(byte(6)?)
            } else {
                None
            },
        })
    }

    /// Relative luminance (WCAG 2), alpha ignored.
    pub fn luminance(&self) -> f64 {
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(self.rgb[0])
            + 0.7152 * channel(self.rgb[1])
            + 0.0722 * channel(self.rgb[2])
    }

    /// The WCAG contrast ratio of two colours (1 to 21).
    pub fn contrast(&self, other: &Colour) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
}

impl fmt::Display for Colour {
    /// `#rrggbb` or `#rrggbbaa`, lower case.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.rgb;
        write!(f, "#{r:02x}{g:02x}{b:02x}")?;
        if let Some(a) = self.alpha {
            write!(f, "{a:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Colour, D::Error> {
        let text = String::deserialize(deserializer)?;
        Colour::parse(&text).map_err(serde::de::Error::custom)
    }
}

settings_section! {
    /// `theme.toml`: the schemes of light and dark mode and the fonts (UI-020, UI-022).
    ThemeSelection {
        /// Scheme used in light mode.
        light_scheme: String = "light".to_owned(),
        /// Scheme used in dark mode.
        dark_scheme: String = "dark".to_owned(),
        /// `[fonts.*]`.
        fonts: Fonts = Fonts::default(),
    }
}

impl ThemeSelection {
    /// The scheme of `mode` (`FollowOs` follows `os_dark`).
    pub fn scheme_for(&self, mode: ThemeMode, os_dark: bool) -> &str {
        let dark = match mode {
            ThemeMode::FollowOs => os_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        };
        if dark {
            &self.dark_scheme
        } else {
            &self.light_scheme
        }
    }
}

settings_section! {
    /// `[fonts]`: Editor, Listing, Hex and folder fonts.
    Fonts {
        /// The text editors.
        editor: FontSpec = FontSpec::default(),
        /// Listings (reports, logs).
        listing: FontSpec = FontSpec::default(),
        /// Hex Compare.
        hex: FontSpec = FontSpec::default(),
        /// Folder views (the system font unless `use_system = false`).
        folder: FolderFont = FolderFont::default(),
    }
}

settings_section! {
    /// One font.
    FontSpec {
        /// Family (empty: the built-in monospace).
        family: String = String::new(),
        /// Size in points (6.0..=72.0).
        size: f64 = 13.0,
    }
}

settings_section! {
    /// The folder-view font.
    FolderFont {
        /// Family (empty: the built-in font).
        family: String = String::new(),
        /// Size in points (6.0..=72.0).
        size: f64 = 13.0,
        /// Use the system font instead.
        use_system: bool = true,
    }
}

impl SettingsSchema for ThemeSelection {
    const FILE: FileId = FileId::Theme;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    /// Font sizes in range; scheme names usable as file names (whether the scheme exists is checked by
    /// [`super::SettingsStore::theme`]).
    fn validate(&self) -> Vec<Problem> {
        let mut problems = super::validate_by_rules(self, &FileId::Theme);
        for (key, name) in [
            ("light_scheme", &self.light_scheme),
            ("dark_scheme", &self.dark_scheme),
        ] {
            if !is_scheme_name(name) {
                problems.push(Problem::new(key, format!("{name:?} is not a scheme name")));
            }
        }
        problems
    }
}

/// True for a value equal to its type's default (an absent optional field of a scheme file).
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// Generates a resolved colour group (every element) and its file form (the elements a file sets).
macro_rules! scheme_group {
    (
        $(#[$meta:meta])* $resolved:ident, $file:ident {
            colours { $($(#[$cmeta:meta])* $colour:ident),* $(,)? }
            flags { $($(#[$fmeta:meta])* $flag:ident),* $(,)? }
            $(extra { $($(#[$xmeta:meta])* $extra:ident : $xty:ty, $xfile:ty),* $(,)? })?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize)]
        pub struct $resolved {
            $($(#[$cmeta])* pub $colour: Colour,)*
            $($(#[$fmeta])* pub $flag: bool,)*
            $($($(#[$xmeta])* pub $extra: $xty,)*)?
        }

        #[doc = concat!("The elements of [`", stringify!($resolved), "`] that a scheme file sets.")]
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct $file {
            $($(#[$cmeta])* #[serde(skip_serializing_if = "Option::is_none")] pub $colour: Option<Colour>,)*
            $($(#[$fmeta])* #[serde(skip_serializing_if = "Option::is_none")] pub $flag: Option<bool>,)*
            $($($(#[$xmeta])* #[serde(skip_serializing_if = "is_default")] pub $extra: $xfile,)*)?
        }

        impl $resolved {
            /// Writes the colours and flags the file sets over these.
            fn overlay_elements(&mut self, file: &$file) {
                $(if let Some(v) = file.$colour { self.$colour = v; })*
                $(if let Some(v) = file.$flag { self.$flag = v; })*
            }

            /// The colours and flags as a file that sets every one (the extra fields at their file defaults).
            fn elements_file(&self) -> $file {
                $file {
                    $($colour: Some(self.$colour),)*
                    $($flag: Some(self.$flag),)*
                    $($($extra: Default::default(),)*)?
                }
            }

            /// The colour elements by name.
            pub fn colours(&self) -> Vec<(&'static str, Colour)> {
                vec![$((stringify!($colour), self.$colour)),*]
            }
        }
    };
}

scheme_group! {
    /// `[folder]` (UI-021).
    FolderColours, FolderColoursFile {
        colours {
            /// Default text.
            default_text,
            /// Background.
            background,
            /// Folder information text.
            folder_info,
            /// Selection background.
            selection,
            /// Selection background of an inactive view.
            inactive_selection_background,
            /// Stripes background.
            stripes,
            /// Log panel background.
            log_panel,
            /// Filtered-out items.
            filtered_out,
            /// Unknown state.
            unknown,
            /// Same.
            same,
            /// Newer.
            newer,
            /// Older.
            older,
            /// Orphan.
            orphan,
            /// Different.
            different,
            /// Left change (merge).
            left_change,
            /// Right change (merge).
            right_change,
            /// Mergeable (merge).
            mergeable,
            /// Conflict (merge).
            conflict,
        }
        flags {
            /// Use the system selection colours.
            use_system_selection,
            /// Use the system log colours.
            use_system_log_colors,
            /// Use stripes.
            use_stripes,
            /// Colour folders to reflect their content.
            color_folders_by_content,
            /// Gradient center background.
            gradient_center_background,
        }
    }
}

scheme_group! {
    /// `[file]` (UI-022).
    FileColours, FileColoursFile {
        colours {
            /// Default text.
            default_text,
            /// Background.
            background,
            /// Stripes background.
            stripes,
            /// Edited lines (text).
            edited,
            /// Saved lines (text).
            saved,
            /// Line numbers.
            line_numbers,
            /// Ignored sections (text).
            ignored,
            /// Current line background.
            current_line,
            /// Selection background of an inactive view.
            inactive_selection,
            /// Selection background.
            selection,
            /// Important difference: text.
            important_text,
            /// Important difference: text background.
            important_text_background,
            /// Important difference: line background.
            important_line_background,
            /// Unimportant difference: text.
            unimportant_text,
            /// Unimportant difference: text background.
            unimportant_text_background,
            /// Unimportant difference: line background.
            unimportant_line_background,
            /// Replacement text.
            replacement,
            /// Orphan line background.
            orphan,
            /// Merge output text.
            output,
            /// Left change background.
            left_change,
            /// Right change background.
            right_change,
            /// Conflict background.
            conflict,
            /// Find highlight background.
            find_highlight,
            /// Gutter background.
            gutter,
            /// Visible whitespace.
            whitespace,
        }
        flags {
            /// The current line only in the over/under layout.
            current_line_only_over_under,
            /// Use stripes.
            use_stripes,
            /// Use the system selection colours.
            use_system_selection,
        }
        extra {
            /// Syntax colours by element id when an element has no entry (exactly 8; M1 slots 1…8).
            fallback: Vec<Colour>, Option<Vec<Colour>>,
            /// Syntax colours by grammar element name (lower case).
            syntax: BTreeMap<String, SyntaxStyle>, BTreeMap<String, SyntaxStyleFile>,
        }
    }
}

scheme_group! {
    /// `[picture]` (UI-023).
    PictureColours, PictureColoursFile {
        colours {
            /// Background.
            background,
            /// Same pixels (tolerance mode).
            same,
            /// Similar pixels (tolerance mode).
            similar,
            /// Different pixels (tolerance mode).
            different,
            /// Light squares of the transparency checkerboard.
            checker_light,
            /// Dark squares of the transparency checkerboard.
            checker_dark,
        }
        flags {}
    }
}

/// The style of one syntax element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SyntaxStyle {
    /// Colour.
    pub color: Colour,
    /// Bold.
    pub bold: bool,
    /// Italic.
    pub italic: bool,
}

/// The parts of a [`SyntaxStyle`] that a scheme file sets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyntaxStyleFile {
    /// Colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Colour>,
    /// Bold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Italic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
}

/// `themes/<name>.toml`: the elements a scheme sets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SchemeFile {
    /// The built-in this scheme is based on (ignored for `light` and `dark`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<SchemeBase>,
    /// `[folder]`.
    pub folder: FolderColoursFile,
    /// `[file]`.
    pub file: FileColoursFile,
    /// `[picture]`.
    pub picture: PictureColoursFile,
}

impl SettingsSchema for SchemeFile {
    const FILE: FileId = FileId::Themes(String::new());
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    /// The built-in of the scheme's base (the file name gives the scheme: `light` and `dark` are their own
    /// base), without `base`: a scheme file holds only what differs from it, whether `settings set`, a dialog
    /// or a change of `base` produced the value. Colours compare as colours (`#FFFFFF` is `#ffffff`).
    fn save_defaults(&self, path: &Path) -> Option<toml::Value> {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let base = Scheme::base_of(name, Some(self));
        toml::Value::try_from(Scheme::builtin(base).to_file(None)).ok()
    }

    /// `fallback` has exactly 8 colours; syntax element names are lower case.
    fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if let Some(fallback) = &self.file.fallback
            && fallback.len() != 8
        {
            problems.push(Problem::new(
                "file.fallback",
                format!("exactly 8 colours are needed, found {}", fallback.len()),
            ));
        }
        for name in self.file.syntax.keys() {
            if *name != name.to_lowercase() || name.is_empty() {
                problems.push(Problem::new(
                    format!(
                        "file.syntax.{}",
                        super::keys::KeySegment::Name(name.clone())
                    ),
                    "syntax element names are lower case",
                ));
            }
        }
        problems
    }
}

/// A scheme with every element (D-78).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scheme {
    /// The scheme name.
    pub name: String,
    /// `[folder]`.
    pub folder: FolderColours,
    /// `[file]`.
    pub file: FileColours,
    /// `[picture]`.
    pub picture: PictureColours,
}

impl Scheme {
    /// The built-in `light` or `dark` scheme.
    pub fn builtin(base: SchemeBase) -> Scheme {
        match base {
            SchemeBase::Light => light(),
            SchemeBase::Dark => dark(),
        }
    }

    /// The scheme `name` with the elements of `file` over its built-in: `light` and `dark` overlay
    /// themselves, any other name overlays its file's `base` (`light` without one).
    pub fn resolve(name: &str, file: Option<&SchemeFile>) -> Scheme {
        let base = Scheme::base_of(name, file);
        let mut scheme = Scheme::builtin(base);
        scheme.name = name.to_owned();
        if let Some(file) = file {
            scheme.folder.overlay_elements(&file.folder);
            scheme.file.overlay_elements(&file.file);
            scheme.picture.overlay_elements(&file.picture);
            if let Some(fallback) = file.file.fallback.as_ref().filter(|f| f.len() == 8) {
                scheme.file.fallback.clone_from(fallback);
            }
            for (element, style) in &file.file.syntax {
                let default = SyntaxStyle {
                    color: scheme.file.default_text,
                    bold: false,
                    italic: false,
                };
                let entry = scheme.file.syntax.entry(element.clone()).or_insert(default);
                if let Some(color) = style.color {
                    entry.color = color;
                }
                if let Some(bold) = style.bold {
                    entry.bold = bold;
                }
                if let Some(italic) = style.italic {
                    entry.italic = italic;
                }
            }
        }
        scheme
    }

    /// The built-in a scheme is based on.
    pub fn base_of(name: &str, file: Option<&SchemeFile>) -> SchemeBase {
        match name {
            "light" => SchemeBase::Light,
            "dark" => SchemeBase::Dark,
            _ => file.and_then(|f| f.base).unwrap_or_default(),
        }
    }

    /// The scheme as a file that sets every element (`base` only for a scheme that is not built in).
    pub fn to_file(&self, base: Option<SchemeBase>) -> SchemeFile {
        let mut file = self.file.elements_file();
        file.fallback = Some(self.file.fallback.clone());
        file.syntax = self
            .file
            .syntax
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    SyntaxStyleFile {
                        color: Some(s.color),
                        bold: Some(s.bold),
                        italic: Some(s.italic),
                    },
                )
            })
            .collect();
        SchemeFile {
            base,
            folder: self.folder.elements_file(),
            file,
            picture: self.picture.elements_file(),
        }
    }

    /// The colour of syntax element `name` with id `id` (≥ 1): its entry, else the fallback cycle (M1 slots).
    pub fn syntax_colour(&self, name: &str, id: u32) -> Colour {
        match self.file.syntax.get(name) {
            Some(style) => style.color,
            None => {
                let slot = usize::try_from(id.saturating_sub(1) % 8).unwrap_or(0);
                self.file
                    .fallback
                    .get(slot)
                    .copied()
                    .unwrap_or(self.file.default_text)
            }
        }
    }

    /// Every pair (element of default text, background element) whose contrast UI-009 requires (≥ 4.5:1).
    pub fn text_on_backgrounds(&self) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        let f = &self.folder;
        for (name, bg) in [
            ("folder.background", f.background),
            ("folder.stripes", f.stripes),
            ("folder.selection", f.selection),
            (
                "folder.inactive_selection_background",
                f.inactive_selection_background,
            ),
        ] {
            out.push((name.to_owned(), f.default_text.contrast(&bg)));
        }
        let t = &self.file;
        for (name, bg) in [
            ("file.background", t.background),
            ("file.stripes", t.stripes),
            ("file.current_line", t.current_line),
            ("file.inactive_selection", t.inactive_selection),
            ("file.selection", t.selection),
            (
                "file.important_text_background",
                t.important_text_background,
            ),
            (
                "file.important_line_background",
                t.important_line_background,
            ),
            (
                "file.unimportant_text_background",
                t.unimportant_text_background,
            ),
            (
                "file.unimportant_line_background",
                t.unimportant_line_background,
            ),
            ("file.orphan", t.orphan),
            ("file.left_change", t.left_change),
            ("file.right_change", t.right_change),
            ("file.conflict", t.conflict),
            ("file.find_highlight", t.find_highlight),
            ("file.gutter", t.gutter),
        ] {
            out.push((name.to_owned(), t.default_text.contrast(&bg)));
        }
        out
    }
}

/// A syntax style.
const fn style(rgb: u32, bold: bool, italic: bool) -> SyntaxStyle {
    SyntaxStyle {
        color: Colour::hex(rgb),
        bold,
        italic,
    }
}

/// The built-in syntax entries (comment, string, character, keyword, identifier, number, operator, directive).
fn syntax(colours: [SyntaxStyle; 8]) -> BTreeMap<String, SyntaxStyle> {
    [
        "comment",
        "string",
        "character",
        "keyword",
        "identifier",
        "number",
        "operator",
        "directive",
    ]
    .into_iter()
    .zip(colours)
    .map(|(name, style)| (name.to_owned(), style))
    .collect()
}

/// The built-in light scheme (the M1 light palette).
fn light() -> Scheme {
    let h = Colour::hex;
    Scheme {
        name: "light".into(),
        folder: FolderColours {
            default_text: h(0x1d2129),
            background: h(0xffffff),
            folder_info: h(0x69707d),
            selection: h(0xdbe7fb),
            inactive_selection_background: h(0xeef1f5),
            stripes: h(0xf6f7f9),
            log_panel: h(0xf3f4f6),
            filtered_out: h(0x0f8b8d),
            unknown: h(0x8a8f98),
            same: h(0x1d2129),
            newer: h(0xc62828),
            older: h(0x8a8f98),
            orphan: h(0x7b3fb5),
            different: h(0xc62828),
            left_change: h(0x1f5fbf),
            right_change: h(0x0a7d45),
            mergeable: h(0x7a6400),
            conflict: h(0xd9480f),
            use_system_selection: false,
            use_system_log_colors: false,
            use_stripes: true,
            color_folders_by_content: true,
            gradient_center_background: false,
        },
        file: FileColours {
            default_text: h(0x1d2129),
            background: h(0xffffff),
            stripes: h(0xf6f7f9),
            edited: h(0xb25e00),
            saved: h(0x0a7d45),
            line_numbers: h(0x9aa0aa),
            ignored: h(0x8a8f98),
            current_line: h(0xeef1f5),
            inactive_selection: h(0xe3e6eb),
            selection: h(0xdbe7fb),
            important_text: h(0xc62828),
            important_text_background: h(0xf6b3b3),
            important_line_background: h(0xfde7e7),
            unimportant_text: h(0x1f5fbf),
            unimportant_text_background: h(0xb8cff3),
            unimportant_line_background: h(0xe6effc),
            replacement: h(0x8c2bb3),
            orphan: h(0xf3e6fb),
            output: h(0x1d2129),
            left_change: h(0xe6f4ea),
            right_change: h(0xe6effc),
            conflict: h(0xfde7e7),
            find_highlight: h(0xfff3b0),
            gutter: h(0xf6f7f9),
            whitespace: h(0xc3c8d0),
            current_line_only_over_under: false,
            use_stripes: false,
            use_system_selection: false,
            fallback: [
                0x8c2bb3, 0x0a7d45, 0x6b6f76, 0xb25e00, 0x1f5fbf, 0xa1264f, 0x3d7a8a, 0x7a6400,
            ]
            .into_iter()
            .map(Colour::hex)
            .collect(),
            syntax: syntax([
                style(0x6b6f76, false, true),
                style(0x0a7d45, false, false),
                style(0x0a7d45, false, false),
                style(0x8c2bb3, true, false),
                style(0x1d2129, false, false),
                style(0xb25e00, false, false),
                style(0x3d7a8a, false, false),
                style(0x1f5fbf, false, false),
            ]),
        },
        picture: PictureColours {
            background: h(0xffffff),
            same: h(0x1f9d55),
            similar: h(0xd9a400),
            different: h(0xc62828),
            checker_light: h(0xffffff),
            checker_dark: h(0xe3e6eb),
        },
    }
}

/// The built-in dark scheme (the M1 dark palette).
fn dark() -> Scheme {
    let h = Colour::hex;
    Scheme {
        name: "dark".into(),
        folder: FolderColours {
            default_text: h(0xd7dae0),
            background: h(0x1b1d22),
            folder_info: h(0x8d94a0),
            selection: h(0x2c3e5f),
            inactive_selection_background: h(0x2a2e35),
            stripes: h(0x20232a),
            log_panel: h(0x23262c),
            filtered_out: h(0x3cc6c8),
            unknown: h(0x7f8691),
            same: h(0xd7dae0),
            newer: h(0xff6b6b),
            older: h(0x7f8691),
            orphan: h(0xc38fff),
            different: h(0xff6b6b),
            left_change: h(0x7aa7f5),
            right_change: h(0x6fcf97),
            mergeable: h(0xd9c45a),
            conflict: h(0xff9e5e),
            use_system_selection: false,
            use_system_log_colors: false,
            use_stripes: true,
            color_folders_by_content: true,
            gradient_center_background: false,
        },
        file: FileColours {
            default_text: h(0xd7dae0),
            background: h(0x1b1d22),
            stripes: h(0x20232a),
            edited: h(0xf0a35e),
            saved: h(0x6fcf97),
            line_numbers: h(0x6d7480),
            ignored: h(0x7f8691),
            current_line: h(0x2a2e35),
            inactive_selection: h(0x2c3038),
            selection: h(0x2c3e5f),
            important_text: h(0xff6b6b),
            important_text_background: h(0x7a3436),
            important_line_background: h(0x3d2425),
            unimportant_text: h(0x7aa7f5),
            unimportant_text_background: h(0x33507f),
            unimportant_line_background: h(0x22304a),
            replacement: h(0xd197f0),
            orphan: h(0x35284a),
            output: h(0xd7dae0),
            left_change: h(0x1f3a2a),
            right_change: h(0x22304a),
            conflict: h(0x3d2425),
            find_highlight: h(0x5a4a12),
            gutter: h(0x20232a),
            whitespace: h(0x4a505a),
            current_line_only_over_under: false,
            use_stripes: false,
            use_system_selection: false,
            fallback: [
                0xd197f0, 0x6fcf97, 0x8b929c, 0xf0a35e, 0x7aa7f5, 0xf07aa0, 0x6cc3d5, 0xd9c45a,
            ]
            .into_iter()
            .map(Colour::hex)
            .collect(),
            syntax: syntax([
                style(0x8b929c, false, true),
                style(0x6fcf97, false, false),
                style(0x6fcf97, false, false),
                style(0xd197f0, true, false),
                style(0xd7dae0, false, false),
                style(0xf0a35e, false, false),
                style(0x6cc3d5, false, false),
                style(0x7aa7f5, false, false),
            ]),
        },
        picture: PictureColours {
            background: h(0x1b1d22),
            same: h(0x6fcf97),
            similar: h(0xd9c45a),
            different: h(0xff6b6b),
            checker_light: h(0x3a3f48),
            checker_dark: h(0x2c3038),
        },
    }
}

/// A theme rule without an OPT id.
const fn plain(pattern: &'static str, kind: ValueKind) -> KindRule {
    KindRule {
        pattern,
        kind,
        opt: None,
        priority: None,
    }
}

/// Kinds of the keys of `theme.toml`.
pub(crate) const SELECTION_RULES: &[KindRule] = &[
    plain(
        "fonts.*.size",
        ValueKind::Float {
            min: 6.0,
            max: 72.0,
        },
    ),
    plain("fonts.*.family", ValueKind::String),
    plain("fonts.folder.use_system", ValueKind::Bool),
    plain("light_scheme", ValueKind::String),
    plain("dark_scheme", ValueKind::String),
];

/// Kinds of the keys of `themes/<name>.toml`.
pub(crate) const SCHEME_RULES: &[KindRule] = &[
    plain("base", ValueKind::Enum(SchemeBase::ALL)),
    plain("folder.use_system_selection", ValueKind::Bool),
    plain("folder.use_system_log_colors", ValueKind::Bool),
    plain("folder.use_stripes", ValueKind::Bool),
    plain("folder.color_folders_by_content", ValueKind::Bool),
    plain("folder.gradient_center_background", ValueKind::Bool),
    plain("folder.*", ValueKind::Colour),
    plain("file.current_line_only_over_under", ValueKind::Bool),
    plain("file.use_stripes", ValueKind::Bool),
    plain("file.use_system_selection", ValueKind::Bool),
    plain("file.fallback", ValueKind::List),
    plain("file.syntax.*.color", ValueKind::Colour),
    plain("file.syntax.*.bold", ValueKind::Bool),
    plain("file.syntax.*.italic", ValueKind::Bool),
    plain("file.syntax.*", ValueKind::Table),
    plain("file.syntax", ValueKind::Table),
    plain("file.*", ValueKind::Colour),
    plain("picture.*", ValueKind::Colour),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_and_print_lower_case() {
        let c = Colour::parse("#AABBCC").unwrap();
        assert_eq!(c.to_string(), "#aabbcc");
        assert_eq!(Colour::parse("#11223344").unwrap().to_string(), "#11223344");
        for bad in ["aabbcc", "#abc", "#gg0000", "#1122334", ""] {
            assert!(Colour::parse(bad).is_err(), "{bad}");
        }
        assert!((Colour::hex(0).contrast(&Colour::hex(0xffffff)) - 21.0).abs() < 1e-9);
    }

    #[test]
    fn builtins_have_enough_contrast() {
        for base in [SchemeBase::Light, SchemeBase::Dark] {
            let scheme = Scheme::builtin(base);
            for (element, ratio) in scheme.text_on_backgrounds() {
                assert!(ratio >= 4.5, "{base:?} {element}: {ratio:.2}");
            }
            assert_eq!(scheme.file.fallback.len(), 8);
            assert_eq!(scheme.file.syntax.len(), 8);
        }
    }

    #[test]
    fn resolution_overlays_and_bases() {
        let file: SchemeFile = toml::from_str(
            "base = \"dark\"\n[folder]\nbackground = \"#010203\"\n[file.syntax.keyword]\nbold = false\n\
             [file.syntax.label]\ncolor = \"#ff0000\"\n",
        )
        .unwrap();
        let custom = Scheme::resolve("mine", Some(&file));
        let dark = Scheme::builtin(SchemeBase::Dark);
        assert_eq!(custom.folder.background, Colour::hex(0x010203));
        assert_eq!(custom.folder.default_text, dark.folder.default_text);
        assert!(!custom.file.syntax["keyword"].bold);
        assert_eq!(
            custom.file.syntax["keyword"].color,
            dark.file.syntax["keyword"].color
        );
        assert_eq!(custom.syntax_colour("label", 3), Colour::hex(0xff0000));
        assert_eq!(custom.syntax_colour("other", 9), dark.file.fallback[0]);
        // A built-in name ignores `base`.
        let light = Scheme::resolve("light", Some(&file));
        assert_eq!(
            light.folder.default_text,
            Scheme::builtin(SchemeBase::Light).folder.default_text
        );
        // The full file of a scheme resolves to the same scheme.
        assert_eq!(
            Scheme::resolve("x", Some(&dark.to_file(Some(SchemeBase::Dark)))).file,
            dark.file
        );
        let selection = ThemeSelection::default();
        assert_eq!(selection.scheme_for(ThemeMode::FollowOs, true), "dark");
        assert_eq!(selection.scheme_for(ThemeMode::Light, true), "light");
        let bad: SchemeFile = toml::from_str("[file]\nfallback = [\"#000000\"]\n").unwrap();
        assert_eq!(bad.validate()[0].key, "file.fallback");
    }
}
