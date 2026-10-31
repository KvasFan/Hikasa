//! The window's colours (M2 W4a, D-113; UI-020…022, FC-031): [`Palette::from_scheme`] maps the elements of the
//! resolved `Scheme` (`theme.toml` and `themes/<name>.toml`) to the painters' colours through one data table
//! (`scheme_map!`); the window chrome (panels, borders, hover, error strips) takes the fixed colours of the
//! scheme's base (light or dark). egui's visuals start from the same base, with the selection colour of
//! `[folder] selection` unless `use_system_selection`.

use egui::Color32;
use hikasa_folder::{Category, ColourClass};
use hikasa_session::settings::Scheme;
use hikasa_session::settings::theme::{Colour, SchemeBase};

/// The fixed chrome colours of a base.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Chrome {
    bg: Color32,
    fg: Color32,
    muted: Color32,
    chrome: Color32,
    border: Color32,
    accent: Color32,
    hover: Color32,
    error: Color32,
    error_bg: Color32,
    warning: Color32,
    warning_bg: Color32,
    gap: Color32,
    gap_fg: Color32,
    hatch: Color32,
    thumb_view: Color32,
    knob: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const LIGHT_CHROME: Chrome = Chrome {
    bg: hex(0xffffff),
    fg: hex(0x1d2129),
    muted: hex(0x69707d),
    chrome: hex(0xf3f4f6),
    border: hex(0xd5d9e0),
    accent: hex(0x2f6fdb),
    hover: hex(0xeef1f5),
    error: hex(0xb42318),
    error_bg: hex(0xfdecea),
    warning: hex(0x8a5a00),
    warning_bg: hex(0xfff4d6),
    gap: hex(0xf0f2f5),
    gap_fg: hex(0x69707d),
    hatch: hex(0xe3e6eb),
    thumb_view: Color32::from_rgba_premultiplied(12, 28, 55, 64),
    knob: hex(0xc3c8d0),
};

const DARK_CHROME: Chrome = Chrome {
    bg: hex(0x1b1d22),
    fg: hex(0xd7dae0),
    muted: hex(0x8d94a0),
    chrome: hex(0x23262c),
    border: hex(0x3a3f48),
    accent: hex(0x5b8def),
    hover: hex(0x2a2e35),
    error: hex(0xff8a80),
    error_bg: hex(0x3b2222),
    warning: hex(0xf0c060),
    warning_bg: hex(0x3a3220),
    gap: hex(0x262a31),
    gap_fg: hex(0x8d94a0),
    hatch: hex(0x2c3038),
    thumb_view: Color32::from_rgba_premultiplied(27, 42, 72, 77),
    knob: hex(0x4a505a),
};

/// One colour of a scheme as egui's.
pub fn colour(c: Colour) -> Color32 {
    let [r, g, b] = c.rgb;
    match c.alpha {
        Some(a) => Color32::from_rgba_unmultiplied(r, g, b, a),
        None => Color32::from_rgb(r, g, b),
    }
}

/// The scheme elements and the palette fields they paint: `group.element => [fields]`. Every scheme field of
/// [`Palette`] is listed exactly once (test); an element may paint several fields.
macro_rules! scheme_map {
    ($($group:ident . $element:ident => [$($field:ident),+]),* $(,)?) => {
        /// Every colour the custom-painted views use: the chrome of the scheme's base and the scheme's elements.
        /// Some elements are painted by later waves' views (merge colours, stripes, find highlight).
        #[allow(dead_code)]
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct Palette {
            /// The base is dark.
            pub dark: bool,
            /// Chrome background (toolbar, panels).
            pub bg: Color32,
            /// Chrome text.
            pub fg: Color32,
            /// Secondary text.
            pub muted: Color32,
            /// Header and gutter strips, the menu and tab bars.
            pub chrome: Color32,
            /// Borders and dividers.
            pub border: Color32,
            /// Accent (cursor, focus).
            pub accent: Color32,
            /// Hovered row.
            pub hover: Color32,
            /// Error text.
            pub error: Color32,
            /// Error strip background.
            pub error_bg: Color32,
            /// Warning text.
            pub warning: Color32,
            /// Warning strip background.
            pub warning_bg: Color32,
            /// Gap bar.
            pub gap: Color32,
            /// Gap bar text.
            pub gap_fg: Color32,
            /// Hatching of an absent side.
            pub hatch: Color32,
            /// Thumbnail: the visible part.
            pub thumb_view: Color32,
            /// Scroll bar knob.
            pub knob: Color32,
            /// Syntax colours (slot 1…8; the scheme's fallback cycle; slot 0 is `t_text`).
            pub syntax: [Color32; 8],
            $($(
                #[doc = concat!("`[", stringify!($group), "] ", stringify!($element), "`.")]
                pub $field: Color32,
            )+)*
        }

        impl Palette {
            /// The palette of a resolved scheme (D-113); the chrome follows the scheme's base (the built-ins:
            /// their own; another scheme: light — the window passes the base of its file with
            /// [`Palette::from_scheme_base`]).
            #[cfg_attr(not(test), allow(dead_code))]
            pub fn from_scheme(scheme: &Scheme) -> Palette {
                Palette::from_scheme_base(scheme, Scheme::base_of(&scheme.name, None))
            }

            /// The palette of a resolved scheme with the chrome of `base`.
            pub fn from_scheme_base(scheme: &Scheme, base: SchemeBase) -> Palette {
                let dark = base == SchemeBase::Dark;
                let c = if dark { DARK_CHROME } else { LIGHT_CHROME };
                let mut syntax = [c.fg; 8];
                for (slot, colour_of) in syntax.iter_mut().zip(scheme.file.fallback.iter()) {
                    *slot = colour(*colour_of);
                }
                Palette {
                    dark,
                    bg: c.bg,
                    fg: c.fg,
                    muted: c.muted,
                    chrome: c.chrome,
                    border: c.border,
                    accent: c.accent,
                    hover: c.hover,
                    error: c.error,
                    error_bg: c.error_bg,
                    warning: c.warning,
                    warning_bg: c.warning_bg,
                    gap: c.gap,
                    gap_fg: c.gap_fg,
                    hatch: c.hatch,
                    thumb_view: c.thumb_view,
                    knob: c.knob,
                    syntax,
                    $($($field: colour(scheme.$group.$element),)+)*
                }
            }

            /// The scheme-mapped fields by name (tests).
            #[cfg(test)]
            fn scheme_fields(&self) -> Vec<(&'static str, Color32)> {
                vec![$($((stringify!($field), self.$field),)+)*]
            }
        }

        /// The table as data (tests): (group, element, fields).
        #[cfg(test)]
        const SCHEME_MAP: &[(&str, &str, &[&str])] = &[
            $((stringify!($group), stringify!($element), &[$(stringify!($field)),+]),)*
        ];
    };
}

scheme_map! {
    folder.default_text => [f_text],
    folder.background => [f_bg],
    folder.folder_info => [f_info],
    folder.selection => [select],
    folder.inactive_selection_background => [f_inactive_select],
    folder.stripes => [f_stripes],
    folder.log_panel => [log_bg],
    folder.filtered_out => [c_filtered],
    folder.unknown => [c_unknown],
    folder.same => [c_same],
    folder.newer => [c_newer],
    folder.older => [c_older],
    folder.orphan => [c_orphan, thumb_orph],
    folder.different => [c_different],
    file.default_text => [t_text],
    file.background => [t_bg],
    file.stripes => [t_stripes],
    file.line_numbers => [gutter_fg],
    file.gutter => [gutter],
    file.current_line => [current_line],
    file.selection => [t_select],
    file.inactive_selection => [t_inactive_select],
    file.important_text => [imp_fg, thumb_imp],
    file.important_text_background => [imp_hl],
    file.important_line_background => [imp_bg, orph_imp_bg],
    file.unimportant_text => [unimp_fg, thumb_unimp],
    file.unimportant_text_background => [unimp_hl],
    file.unimportant_line_background => [unimp_bg, orph_unimp_bg],
    file.orphan => [orph_bg],
    file.find_highlight => [find_hl],
    file.whitespace => [whitespace],
}

/// The built-in light palette (tests of the painters).
#[cfg(test)]
pub fn light() -> Palette {
    Palette::from_scheme(&Scheme::resolve("light", None))
}

impl Palette {
    /// The colour of a folder colour class (FC-031) for an item of `category`; `None` for an absent side.
    /// Newer and different, older and unknown share a class; the category tells them apart.
    pub fn folder(&self, class: ColourClass, category: Category) -> Option<Color32> {
        Some(match class {
            ColourClass::Same => self.c_same,
            ColourClass::NewerOrDifferent => match category {
                Category::Different => self.c_different,
                _ => self.c_newer,
            },
            ColourClass::Orphan => self.c_orphan,
            ColourClass::OlderOrUnknown => match category {
                Category::Unknown => self.c_unknown,
                _ => self.c_older,
            },
            ColourClass::Filtered => self.c_filtered,
            ColourClass::Error => self.error,
            ColourClass::Absent => return None,
        })
    }

    /// The colour of a syntax slot (0: default text).
    pub fn syntax_colour(&self, slot: u32) -> Color32 {
        match slot {
            0 => self.t_text,
            n => self.syntax[((n - 1) % 8) as usize],
        }
    }

    /// egui's visuals for this palette: the base's, with the selection of `[folder] selection` unless
    /// `use_system_selection` (D-113).
    pub fn visuals(&self, scheme: &Scheme) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        if !scheme.folder.use_system_selection {
            v.selection.bg_fill = self.select;
            v.selection.stroke.color = self.f_text;
        }
        v.panel_fill = self.bg;
        v.window_fill = self.bg;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_session::settings::SchemeFile;
    use std::collections::BTreeSet;

    fn custom(json: serde_json::Value) -> SchemeFile {
        serde_json::from_value(json).expect("a scheme file")
    }

    /// A4.5: every palette field of the table is mapped exactly once; changing one element changes exactly its
    /// field(s); built-ins and a custom scheme with a base.
    #[test]
    fn the_palette_follows_the_scheme_table() {
        let mut fields = BTreeSet::new();
        let mut elements = BTreeSet::new();
        for (group, element, mapped) in SCHEME_MAP {
            assert!(
                elements.insert((*group, *element)),
                "{group}.{element} twice"
            );
            for f in *mapped {
                assert!(fields.insert(*f), "{f} mapped twice");
            }
        }
        let light = Palette::from_scheme(&Scheme::resolve("light", None));
        let names: BTreeSet<&str> = light.scheme_fields().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, fields);
        assert!(!light.dark);
        let dark = Palette::from_scheme(&Scheme::resolve("dark", None));
        assert!(dark.dark && dark.bg != light.bg && dark.t_bg != light.t_bg);
        // A custom scheme takes its base's chrome.
        let mine = custom(serde_json::json!({ "base": "dark" }));
        let p = Palette::from_scheme_base(&Scheme::resolve("mine", Some(&mine)), SchemeBase::Dark);
        assert!(p.dark);
        assert_eq!(p.scheme_fields(), dark.scheme_fields());
        // One element at a time: exactly its fields change.
        let unique = "#010203";
        for (group, element, mapped) in SCHEME_MAP {
            let mut inner = serde_json::Map::new();
            inner.insert((*element).to_owned(), serde_json::Value::from(unique));
            let mut outer = serde_json::Map::new();
            outer.insert("base".to_owned(), serde_json::Value::from("light"));
            outer.insert((*group).to_owned(), serde_json::Value::Object(inner));
            let file = custom(serde_json::Value::Object(outer));
            let p = Palette::from_scheme(&Scheme::resolve("mine", Some(&file)));
            let changed: BTreeSet<&str> = p
                .scheme_fields()
                .into_iter()
                .zip(light.scheme_fields())
                .filter(|((_, a), (_, b))| a != b)
                .map(|((n, _), _)| n)
                .collect();
            let want: BTreeSet<&str> = mapped.iter().copied().collect();
            assert_eq!(changed, want, "{group}.{element}");
            for f in *mapped {
                let got = p
                    .scheme_fields()
                    .into_iter()
                    .find(|(n, _)| n == f)
                    .unwrap()
                    .1;
                assert_eq!(got, Color32::from_rgb(1, 2, 3), "{f}");
            }
            assert_eq!(p.bg, light.bg, "the chrome stays");
        }
        // The fallback cycle paints the syntax slots.
        let file = custom(serde_json::json!({ "file": { "fallback": [
            "#000001", "#000002", "#000003", "#000004", "#000005", "#000006", "#000007", "#000008"
        ] } }));
        let p = Palette::from_scheme(&Scheme::resolve("light", Some(&file)));
        assert_eq!(p.syntax_colour(1), Color32::from_rgb(0, 0, 1));
        assert_eq!(p.syntax_colour(9), Color32::from_rgb(0, 0, 1));
        assert_eq!(p.syntax_colour(0), p.t_text);
    }

    #[test]
    fn folder_classes_use_their_elements() {
        let p = Palette::from_scheme(&Scheme::resolve("light", None));
        assert_eq!(p.folder(ColourClass::Same, Category::Same), Some(p.c_same));
        assert_eq!(
            p.folder(ColourClass::NewerOrDifferent, Category::Different),
            Some(p.c_different)
        );
        assert_eq!(
            p.folder(ColourClass::NewerOrDifferent, Category::LeftNewer),
            Some(p.c_newer)
        );
        assert_eq!(
            p.folder(ColourClass::OlderOrUnknown, Category::Unknown),
            Some(p.c_unknown)
        );
        assert_eq!(p.folder(ColourClass::Absent, Category::Same), None);
        // The selection follows `[folder] selection` unless the system's is wanted.
        let scheme = Scheme::resolve("light", None);
        assert_eq!(p.visuals(&scheme).selection.bg_fill, p.select);
    }
}
