//! The theme editor (M2 W4c, D-130; UI-020…023, FC-031): the Appearance pages edit working copies of `theme.toml`
//! and the scheme files. Every edit marks the preview, which the app core applies at once to every window; Cancel
//! restores the applied theme; Apply writes the files sparsely (D-70, D-78).

use hikasa_session::settings::keys::is_scheme_name;
use std::collections::BTreeSet;

use hikasa_session::settings::theme::{
    BUILTIN_SCHEMES, FONT_SIZES, MIN_CONTRAST, SchemeBase, SchemeGroup, selected_scheme,
};
use hikasa_session::settings::{Colour, Scheme, SchemeFile, SyntaxStyle, ThemeMode};

use super::OptionsDialog;

/// The state of the Appearance pages.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemePage {
    /// The scheme being edited.
    pub scheme: String,
    /// The edited scheme with every element (rebuilt by an edit, never per frame).
    pub resolved: Scheme,
    /// The name for New and Clone.
    pub new_name: String,
    /// The base for New.
    pub new_base: SchemeBase,
    /// Why New, Clone or Delete did nothing.
    pub error: Option<String>,
    /// The built-ins *Reset all* was pressed for: only that deletes an override file with unknown keys (D-130).
    pub reset_all: BTreeSet<String>,
}

impl Default for ThemePage {
    fn default() -> ThemePage {
        ThemePage {
            scheme: "light".to_owned(),
            resolved: Scheme::builtin(SchemeBase::Light),
            new_name: String::new(),
            new_base: SchemeBase::Light,
            error: None,
            reset_all: BTreeSet::new(),
        }
    }
}

/// A font of `theme.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontSlot {
    /// Text editors.
    Editor,
    /// Listings.
    Listing,
    /// Hex Compare.
    Hex,
    /// Folder views.
    Folder,
}

impl FontSlot {
    /// Every slot, in `theme.toml`'s order; a slot's value is its index here.
    pub const ALL: [FontSlot; 4] = [
        FontSlot::Editor,
        FontSlot::Listing,
        FontSlot::Hex,
        FontSlot::Folder,
    ];
}

impl OptionsDialog {
    /// The scheme files by name, the working copies.
    pub fn scheme_names(&self) -> Vec<String> {
        self.schemes.keys().cloned().collect()
    }

    /// The working file of the edited scheme.
    fn scheme_file(&self) -> Option<&SchemeFile> {
        self.schemes.get(&self.theme_page.scheme)
    }

    /// True when the edited scheme is built in (no Delete; Reset all deletes its override file).
    pub fn edits_builtin(&self) -> bool {
        BUILTIN_SCHEMES.contains(&self.theme_page.scheme.as_str())
    }

    /// The working theme mode.
    pub fn theme_mode(&self) -> ThemeMode {
        self.options.appearance.theme_mode
    }

    /// The scheme the window shows with the working copies, and its base (the preview, D-130).
    pub fn working_scheme(&self, os_dark: bool) -> (Scheme, SchemeBase) {
        let scheme = selected_scheme(&self.theme, self.theme_mode(), os_dark, |name| {
            self.schemes.get(name)
        });
        let base = Scheme::base_of(&scheme.name, self.schemes.get(&scheme.name));
        (scheme, base)
    }

    /// Rebuilds the resolved copy of the edited scheme.
    pub(super) fn refresh_scheme(&mut self) {
        if !self.schemes.contains_key(&self.theme_page.scheme) {
            "light".clone_into(&mut self.theme_page.scheme);
        }
        self.theme_page.resolved = Scheme::resolve(&self.theme_page.scheme, self.scheme_file());
    }

    fn scheme_edited(&mut self) {
        self.theme_page.error = None;
        let scheme = self.theme_page.scheme.clone();
        self.theme_page.reset_all.remove(&scheme);
        self.refresh_scheme();
        self.theme_edited = true;
        self.edited();
    }

    /// Chooses the scheme to edit.
    pub fn select_scheme(&mut self, name: &str) {
        if self.schemes.contains_key(name) {
            name.clone_into(&mut self.theme_page.scheme);
            self.theme_page.error = None;
            self.refresh_scheme();
        }
    }

    /// The scheme of light mode.
    pub fn set_light_scheme(&mut self, name: &str) {
        name.clone_into(&mut self.theme.light_scheme);
        self.scheme_edited();
    }

    /// The scheme of dark mode.
    pub fn set_dark_scheme(&mut self, name: &str) {
        name.clone_into(&mut self.theme.dark_scheme);
        self.scheme_edited();
    }

    /// The built-in the edited scheme is based on, with every element.
    fn base_scheme(&self) -> Scheme {
        let name = &self.theme_page.scheme;
        Scheme::builtin(Scheme::base_of(name, self.scheme_file()))
    }

    /// Sets a colour element of the edited scheme; a value equal to the base is not set (the file stays sparse).
    pub fn set_element_colour(&mut self, group: SchemeGroup, name: &str, value: Colour) {
        let base = self
            .base_scheme()
            .colours(group)
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| c);
        let stored = (base != Some(value)).then_some(value);
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme)
            && file.set_colour(group, name, stored)
        {
            self.scheme_edited();
        }
    }

    /// Sets a flag of the edited scheme; a value equal to the base is not set.
    pub fn set_element_flag(&mut self, group: SchemeGroup, name: &str, value: bool) {
        let base = self
            .base_scheme()
            .flags(group)
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v);
        let stored = (base != Some(value)).then_some(value);
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme)
            && file.set_flag(group, name, stored)
        {
            self.scheme_edited();
        }
    }

    /// True when the file sets the element.
    pub fn element_set(&self, group: SchemeGroup, name: &str) -> bool {
        self.scheme_file()
            .is_some_and(|f| f.colour(group, name).is_some() || f.flag(group, name).is_some())
    }

    /// *Reset*: the element goes back to the base value (removed from the file).
    pub fn reset_element(&mut self, group: SchemeGroup, name: &str) {
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme) {
            let colour = file.set_colour(group, name, None);
            let flag = file.set_flag(group, name, None);
            if colour || flag {
                self.scheme_edited();
            }
        }
    }

    /// Sets the style of a syntax element.
    pub fn set_syntax(&mut self, name: &str, style: SyntaxStyle) {
        let scheme = self.theme_page.scheme.clone();
        let base = self.base_scheme().file.syntax.get(name).copied();
        if let Some(file) = self.schemes.get_mut(&scheme) {
            let entry = file.file.syntax.entry(name.to_owned()).or_default();
            entry.color = (base.map(|b| b.color) != Some(style.color)).then_some(style.color);
            entry.bold = (base.map(|b| b.bold) != Some(style.bold)).then_some(style.bold);
            entry.italic = (base.map(|b| b.italic) != Some(style.italic)).then_some(style.italic);
            if *entry == Default::default() {
                file.file.syntax.remove(name);
            }
            self.scheme_edited();
        }
    }

    /// *Reset* of a syntax element.
    pub fn reset_syntax(&mut self, name: &str) {
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme)
            && file.file.syntax.remove(name).is_some()
        {
            self.scheme_edited();
        }
    }

    /// True when the file sets a syntax element.
    pub fn syntax_set(&self, name: &str) -> bool {
        self.scheme_file()
            .is_some_and(|f| f.file.syntax.contains_key(name))
    }

    /// Sets a colour of the fallback cycle (all eight are stored together, D-78).
    pub fn set_fallback(&mut self, slot: usize, value: Colour) {
        let mut cycle = self.theme_page.resolved.file.fallback.clone();
        let Some(entry) = cycle.get_mut(slot) else {
            return;
        };
        *entry = value;
        let same = cycle == self.base_scheme().file.fallback;
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme) {
            file.file.fallback = (!same).then_some(cycle);
            self.scheme_edited();
        }
    }

    /// *Reset* of the fallback cycle.
    pub fn reset_fallback(&mut self) {
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme) {
            file.file.fallback = None;
            self.scheme_edited();
        }
    }

    fn free_name(&self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if !is_scheme_name(name) {
            return Err(format!(
                "`{name}` is not a scheme name (letters, digits, `_` and `-`)"
            ));
        }
        if self.schemes.keys().any(|n| n.eq_ignore_ascii_case(name)) {
            return Err(format!("A scheme `{name}` exists."));
        }
        Ok(name.to_owned())
    }

    /// New: an empty scheme on `theme_page.new_base`, named `theme_page.new_name`; it becomes the edited one.
    pub fn new_scheme(&mut self) {
        match self.free_name(&self.theme_page.new_name.clone()) {
            Ok(name) => {
                let file = SchemeFile {
                    base: Some(self.theme_page.new_base),
                    ..SchemeFile::default()
                };
                self.schemes.insert(name.clone(), file);
                self.theme_page.scheme = name;
                self.theme_page.new_name.clear();
                self.scheme_edited();
            }
            Err(e) => self.theme_page.error = Some(e),
        }
    }

    /// Clone: a new scheme with the edited one's elements, named `theme_page.new_name`.
    pub fn clone_scheme(&mut self) {
        match self.free_name(&self.theme_page.new_name.clone()) {
            Ok(name) => {
                let mut file = self.scheme_file().cloned().unwrap_or_default();
                file.base = Some(Scheme::base_of(&self.theme_page.scheme, self.scheme_file()));
                self.schemes.insert(name.clone(), file);
                self.theme_page.scheme = name;
                self.theme_page.new_name.clear();
                self.scheme_edited();
            }
            Err(e) => self.theme_page.error = Some(e),
        }
    }

    /// Delete: not for a built-in; a mode that selected it falls back to its built-in.
    pub fn delete_scheme(&mut self) {
        if self.edits_builtin() {
            self.theme_page.error = Some("A built-in scheme cannot be deleted.".to_owned());
            return;
        }
        let name = std::mem::take(&mut self.theme_page.scheme);
        self.schemes.remove(&name);
        if self.theme.light_scheme == name {
            "light".clone_into(&mut self.theme.light_scheme);
        }
        if self.theme.dark_scheme == name {
            "dark".clone_into(&mut self.theme.dark_scheme);
        }
        "light".clone_into(&mut self.theme_page.scheme);
        self.scheme_edited();
    }

    /// *Reset all*: every element of the edited scheme back to its base (on a built-in: its override file goes).
    pub fn reset_all(&mut self) {
        let scheme = self.theme_page.scheme.clone();
        if let Some(file) = self.schemes.get_mut(&scheme) {
            let builtin = BUILTIN_SCHEMES.contains(&scheme.as_str());
            *file = SchemeFile {
                // A built-in's base is fixed: a stray `base` key does not keep its override file.
                base: file.base.filter(|_| !builtin),
                ..SchemeFile::default()
            };
            self.scheme_edited();
            if builtin {
                self.theme_page.reset_all.insert(scheme);
                self.edited();
            }
        }
    }

    /// The contrast warnings of the edited scheme: the default text on a background below 4.5:1 (never an error).
    pub fn contrast_warnings(&self) -> Vec<String> {
        self.theme_page
            .resolved
            .text_on_backgrounds()
            .into_iter()
            .filter(|(_, ratio)| *ratio < MIN_CONTRAST)
            .map(|(element, ratio)| {
                format!("Default text on {element}: contrast {ratio:.1}:1 (below 4.5:1)")
            })
            .collect()
    }

    /// The family of a font.
    pub fn set_font_family(&mut self, slot: FontSlot, family: &str) {
        let fonts = &mut self.theme.fonts;
        let target = match slot {
            FontSlot::Editor => &mut fonts.editor.family,
            FontSlot::Listing => &mut fonts.listing.family,
            FontSlot::Hex => &mut fonts.hex.family,
            FontSlot::Folder => &mut fonts.folder.family,
        };
        family.clone_into(target);
        self.scheme_edited();
    }

    /// The size of a font, clamped by the schema's range.
    pub fn set_font_size(&mut self, slot: FontSlot, size: f64) {
        let size = size.clamp(FONT_SIZES.0, FONT_SIZES.1);
        let fonts = &mut self.theme.fonts;
        match slot {
            FontSlot::Editor => fonts.editor.size = size,
            FontSlot::Listing => fonts.listing.size = size,
            FontSlot::Hex => fonts.hex.size = size,
            FontSlot::Folder => fonts.folder.size = size,
        }
        self.scheme_edited();
    }

    /// "Use the system font" of the folder views.
    pub fn set_folder_system_font(&mut self, on: bool) {
        self.theme.fonts.folder.use_system = on;
        self.scheme_edited();
    }
}
