//! The cell texts of a folder row (M2 W9a, FC-040, FC-041; D-237): every column of one side, from the core's values
//! and texts (`human::local_time_text` in the clock's zone, `view::extension`, `view::attributes_text`). Toolkit-free.

use hikasa_folder::{ColourClass, EntryView};
use hikasa_session::api::human::{TimeZone, local_time_text};

use super::columns::ColumnId;
use super::glyphs::GlyphView;

/// One side's cell of a tree row: a text per column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Name (the whole name, control characters escaped).
    pub name: String,
    /// `(r)`, `(s)`, `(h)` after the name while Ext is hidden (FC-041).
    pub marks: String,
    /// The name is not valid Unicode (a tooltip says so).
    pub lossy: bool,
    /// Extension without the dot.
    pub ext: String,
    /// Size text (folders: the total below once scanned).
    pub size: String,
    /// Modification time in the clock's zone.
    pub modified: String,
    /// DOS letters or the Unix mode string.
    pub attributes: String,
    /// CRC32 as eight lower-case hex digits; empty until known.
    pub crc: String,
    /// Numeric owner.
    pub owner: String,
    /// Numeric group.
    pub group: String,
    /// The folder below the base (flat comparisons).
    pub path: String,
    /// Colour class of the colour spot and the name (FC-031).
    pub colour: ColourClass,
    /// The glyph in front of the name (D-236).
    pub glyph: GlyphView,
}

impl Cell {
    /// The text of `column` (Name: with the marks unless Ext is shown).
    pub fn text(&self, column: ColumnId, ext_shown: bool) -> String {
        match column {
            ColumnId::Name if ext_shown || self.marks.is_empty() => self.name.clone(),
            ColumnId::Name => format!("{} {}", self.name, self.marks),
            ColumnId::Ext => self.ext.clone(),
            ColumnId::Size => self.size.clone(),
            ColumnId::Modified => self.modified.clone(),
            ColumnId::Attributes => self.attributes.clone(),
            ColumnId::Crc => self.crc.clone(),
            ColumnId::Owner => self.owner.clone(),
            ColumnId::Group => self.group.clone(),
            ColumnId::Path => self.path.clone(),
        }
    }
}

/// `(r)(s)(h)` of an entry (FC-041): DOS read-only, system, hidden; on Unix `(r)` when no write bit is set. Hidden
/// dotfiles get nothing (their name says it).
pub fn attr_marks(entry: &EntryView<'_>) -> String {
    let mut out = String::new();
    if let Some(dos) = entry.attrs.dos() {
        use hikasa_folder::DosAttrs;
        for (flag, mark) in [
            (DosAttrs::READONLY, "(r)"),
            (DosAttrs::SYSTEM, "(s)"),
            (DosAttrs::HIDDEN, "(h)"),
        ] {
            if dos.contains(flag) {
                out.push_str(mark);
            }
        }
    } else if let Some(unix) = entry.attrs.unix()
        && unix.mode & 0o222 == 0
    {
        out.push_str("(r)");
    }
    out
}

/// `1 234 567` (thin grouping, bytes).
pub fn size_text(bytes: u64) -> String {
    let digits = bytes.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k).is_multiple_of(3) {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

/// What a cell needs besides its entry.
pub struct CellInput<'a> {
    /// The row is a folder.
    pub folder: bool,
    /// A folder whose listing is in (its total means something).
    pub scanned: bool,
    /// The bytes below a folder on this side.
    pub folder_bytes: u64,
    /// The zone of the Service's clock (D-237).
    pub zone: &'a TimeZone,
    /// The colour class.
    pub colour: ColourClass,
    /// The glyph.
    pub glyph: GlyphView,
}

/// The cell of one side's entry.
pub fn cell_of(entry: &EntryView<'_>, input: &CellInput<'_>) -> Cell {
    let unix = entry.attrs.unix();
    Cell {
        name: hikasa_report::escape_name(entry.name).into_owned(),
        marks: attr_marks(entry),
        lossy: entry.name_lossy,
        ext: if input.folder {
            String::new()
        } else {
            hikasa_folder::view::extension(entry.name)
                .map(|e| hikasa_report::escape_name(e).into_owned())
                .unwrap_or_default()
        },
        size: if input.folder && !input.scanned {
            String::new()
        } else if input.folder {
            size_text(input.folder_bytes)
        } else {
            size_text(entry.size)
        },
        modified: entry
            .mtime
            .map(|t| local_time_text(t, input.zone))
            .unwrap_or_default(),
        attributes: hikasa_folder::view::attributes_text(entry),
        crc: entry.crc32.map(|c| format!("{c:08x}")).unwrap_or_default(),
        owner: unix.map(|u| u.uid.to_string()).unwrap_or_default(),
        group: unix.map(|u| u.gid.to_string()).unwrap_or_default(),
        path: entry
            .dir
            .map(|d| hikasa_report::escape_name(d).into_owned())
            .unwrap_or_default(),
        colour: input.colour,
        glyph: input.glyph,
    }
}
