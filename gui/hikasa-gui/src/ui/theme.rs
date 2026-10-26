//! The window's colours: one palette for light and one for dark mode, picked from egui's visuals, which follow
//! the system theme. The folder colours are FC-031's classes, the text colours TC-031's states.

use egui::Color32;
use hikasa_folder::ColourClass;

/// Every colour the custom-painted views use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// View background.
    pub bg: Color32,
    /// Default text.
    pub fg: Color32,
    /// Secondary text.
    pub muted: Color32,
    /// Header and gutter strips.
    pub chrome: Color32,
    /// Borders and dividers.
    pub border: Color32,
    /// Accent (cursor, focus).
    pub accent: Color32,
    /// Selected row.
    pub select: Color32,
    /// Hovered row.
    pub hover: Color32,
    /// Error text.
    pub error: Color32,
    /// Error strip background.
    pub error_bg: Color32,
    /// Folder colours: same, newer or different, orphan, older or unknown, filtered, error.
    pub c_same: Color32,
    /// Newer or different.
    pub c_diff: Color32,
    /// Orphan.
    pub c_orphan: Color32,
    /// Older or unknown.
    pub c_old: Color32,
    /// Filtered.
    pub c_filtered: Color32,
    /// Error.
    pub c_error: Color32,
    /// Line-number gutter.
    pub gutter: Color32,
    /// Line numbers.
    pub gutter_fg: Color32,
    /// Changed row, important.
    pub imp_bg: Color32,
    /// Inline difference, important.
    pub imp_hl: Color32,
    /// Changed row, unimportant.
    pub unimp_bg: Color32,
    /// Inline difference, unimportant.
    pub unimp_hl: Color32,
    /// Orphan row, important.
    pub orph_imp_bg: Color32,
    /// Orphan row, unimportant.
    pub orph_unimp_bg: Color32,
    /// Gap bar.
    pub gap: Color32,
    /// Gap bar text.
    pub gap_fg: Color32,
    /// Hatching of an absent side.
    pub hatch: Color32,
    /// Thumbnail: important.
    pub thumb_imp: Color32,
    /// Thumbnail: unimportant.
    pub thumb_unimp: Color32,
    /// Thumbnail: orphans.
    pub thumb_orph: Color32,
    /// Thumbnail: the visible part.
    pub thumb_view: Color32,
    /// Scroll bar knob.
    pub knob: Color32,
    /// Syntax colours (slot 1…8; slot 0 is `fg`).
    pub syntax: [Color32; 8],
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The light palette.
pub const LIGHT: Palette = Palette {
    bg: hex(0xffffff),
    fg: hex(0x1d2129),
    muted: hex(0x69707d),
    chrome: hex(0xf3f4f6),
    border: hex(0xd5d9e0),
    accent: hex(0x2f6fdb),
    select: hex(0xdbe7fb),
    hover: hex(0xeef1f5),
    error: hex(0xb42318),
    error_bg: hex(0xfdecea),
    c_same: hex(0x1d2129),
    c_diff: hex(0xc62828),
    c_orphan: hex(0x7b3fb5),
    c_old: hex(0x8a8f98),
    c_filtered: hex(0x0f8b8d),
    c_error: hex(0xd9480f),
    gutter: hex(0xf6f7f9),
    gutter_fg: hex(0x9aa0aa),
    imp_bg: hex(0xfde7e7),
    imp_hl: hex(0xf6b3b3),
    unimp_bg: hex(0xe6effc),
    unimp_hl: hex(0xb8cff3),
    orph_imp_bg: hex(0xf3e6fb),
    orph_unimp_bg: hex(0xecf0f6),
    gap: hex(0xf0f2f5),
    gap_fg: hex(0x69707d),
    hatch: hex(0xe3e6eb),
    thumb_imp: hex(0xd64545),
    thumb_unimp: hex(0x5b8def),
    thumb_orph: hex(0x9a5cd0),
    thumb_view: Color32::from_rgba_premultiplied(12, 28, 55, 64),
    knob: hex(0xc3c8d0),
    syntax: [
        hex(0x8c2bb3),
        hex(0x0a7d45),
        hex(0x6b6f76),
        hex(0xb25e00),
        hex(0x1f5fbf),
        hex(0xa1264f),
        hex(0x3d7a8a),
        hex(0x7a6400),
    ],
};

/// The dark palette.
pub const DARK: Palette = Palette {
    bg: hex(0x1b1d22),
    fg: hex(0xd7dae0),
    muted: hex(0x8d94a0),
    chrome: hex(0x23262c),
    border: hex(0x3a3f48),
    accent: hex(0x5b8def),
    select: hex(0x2c3e5f),
    hover: hex(0x2a2e35),
    error: hex(0xff8a80),
    error_bg: hex(0x3b2222),
    c_same: hex(0xd7dae0),
    c_diff: hex(0xff6b6b),
    c_orphan: hex(0xc38fff),
    c_old: hex(0x7f8691),
    c_filtered: hex(0x3cc6c8),
    c_error: hex(0xff9e5e),
    gutter: hex(0x20232a),
    gutter_fg: hex(0x6d7480),
    imp_bg: hex(0x3d2425),
    imp_hl: hex(0x7a3436),
    unimp_bg: hex(0x22304a),
    unimp_hl: hex(0x33507f),
    orph_imp_bg: hex(0x35284a),
    orph_unimp_bg: hex(0x282d36),
    gap: hex(0x262a31),
    gap_fg: hex(0x8d94a0),
    hatch: hex(0x2c3038),
    thumb_imp: hex(0xff6b6b),
    thumb_unimp: hex(0x6f9bf5),
    thumb_orph: hex(0xb77bf0),
    thumb_view: Color32::from_rgba_premultiplied(27, 42, 72, 77),
    knob: hex(0x4a505a),
    syntax: [
        hex(0xd197f0),
        hex(0x6fcf97),
        hex(0x8b929c),
        hex(0xf0a35e),
        hex(0x7aa7f5),
        hex(0xf07aa0),
        hex(0x6cc3d5),
        hex(0xd9c45a),
    ],
};

impl Palette {
    /// The palette of the current visuals.
    pub fn of(visuals: &egui::Visuals) -> &'static Palette {
        if visuals.dark_mode { &DARK } else { &LIGHT }
    }

    /// The colour of a folder colour class (FC-031); `None` for an absent side.
    pub fn folder(&self, class: ColourClass) -> Option<Color32> {
        Some(match class {
            ColourClass::Same => self.c_same,
            ColourClass::NewerOrDifferent => self.c_diff,
            ColourClass::Orphan => self.c_orphan,
            ColourClass::OlderOrUnknown => self.c_old,
            ColourClass::Filtered => self.c_filtered,
            ColourClass::Error => self.c_error,
            ColourClass::Absent => return None,
        })
    }

    /// The colour of a syntax slot (0: default text).
    pub fn syntax_colour(&self, slot: u32) -> Color32 {
        match slot {
            0 => self.fg,
            n => self.syntax[((n - 1) % 8) as usize],
        }
    }
}
