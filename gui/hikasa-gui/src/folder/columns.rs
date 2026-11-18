//! The folder views' columns and their layout memory (M2 W9a, FC-040, FC-041, FC-014; D-234): which columns show, in
//! which order and widths (both sides alike), and the sort key a header click sets. Toolkit-free: `ui/folder.rs` draws
//! the header from it and turns clicks and drags into these calls.

use std::collections::BTreeMap;

use hikasa_core::Side;
use hikasa_folder::view::SortKey;
use hikasa_session::settings::state::LayoutValue;

/// A column of a folder view (the core's sort column: one set of ids for the header, the sort and the layout files).
pub use hikasa_folder::view::SortColumn as ColumnId;

/// The widest a column may be dragged (points).
pub const MAX_WIDTH: f32 = 2000.0;

/// The default width (points) of a column; Name stretches.
pub fn default_width(id: ColumnId) -> f32 {
    match id {
        ColumnId::Name => 0.0,
        ColumnId::Ext => 56.0,
        ColumnId::Size => 92.0,
        ColumnId::Modified => 136.0,
        ColumnId::Attributes => 88.0,
        ColumnId::Crc => 76.0,
        ColumnId::Owner | ColumnId::Group => 64.0,
        ColumnId::Path => 200.0,
    }
}

/// The narrowest a column may be (points).
pub fn min_width(id: ColumnId) -> f32 {
    match id {
        ColumnId::Name => 80.0,
        ColumnId::Ext => 24.0,
        ColumnId::Size | ColumnId::Attributes | ColumnId::Crc => 40.0,
        ColumnId::Modified | ColumnId::Path => 60.0,
        ColumnId::Owner | ColumnId::Group => 32.0,
    }
}

/// The header title of a column.
pub fn title(id: ColumnId) -> &'static str {
    match id {
        ColumnId::Name => "Name",
        ColumnId::Ext => "Ext",
        ColumnId::Size => "Size",
        ColumnId::Modified => "Modified",
        ColumnId::Attributes => "Attributes",
        ColumnId::Crc => "CRC",
        ColumnId::Owner => "Owner",
        ColumnId::Group => "Group",
        ColumnId::Path => "Path",
    }
}

/// The columns shown by default: Name, Size, Modified (and Path while the comparison is flat).
pub const DEFAULT_COLUMNS: [ColumnId; 4] = [
    ColumnId::Name,
    ColumnId::Size,
    ColumnId::Modified,
    ColumnId::Path,
];

/// A layout key's value with the good entries of a list or a table kept, and the others named
/// (`State::layout_value_kept`).
pub type KeptValue<'a> = dyn Fn(&str) -> Option<(LayoutValue, Vec<String>)> + 'a;

/// The columns, their order and widths, and the sort (D-234, D-235).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnLayout {
    /// The visible columns in order, Name first. Path is listed but shown only while the comparison is flat.
    pub visible: Vec<ColumnId>,
    /// The widths the user dragged (the others have their default).
    pub widths: BTreeMap<ColumnId, f32>,
    /// The sort (`None`: the tree order).
    pub sort: Option<SortKey>,
}

impl Default for ColumnLayout {
    fn default() -> ColumnLayout {
        ColumnLayout {
            visible: DEFAULT_COLUMNS.to_vec(),
            widths: BTreeMap::new(),
            sort: None,
        }
    }
}

impl ColumnLayout {
    /// The layout of `[layout.<view>]` (with the session's `[view]` overlaid: D-108's precedence is the caller's).
    /// Values as read (no entry of a list or a table left out); see [`ColumnLayout::from_kept`] (tests).
    #[cfg(test)]
    pub fn from_values(
        value: &dyn Fn(&str) -> Option<LayoutValue>,
    ) -> (ColumnLayout, Option<String>) {
        ColumnLayout::from_kept(&|key| value(key).map(|v| (v, Vec::new())))
    }

    /// The layout of `[layout.<view>]` from values that keep the good entries of a list or a table and name the
    /// others (`State::layout_value_kept`). Unknown column ids, entries of the wrong type and a repeated id are
    /// dropped and the rest kept; a width is clamped; a sort on a column that is not shown, or a bad sort word,
    /// leaves the tree order (`sort = ""` is the tree order written explicitly). The text of one
    /// `invalid_config_value` warning comes back when anything was dropped.
    pub fn from_kept(value: &KeptValue<'_>) -> (ColumnLayout, Option<String>) {
        let mut layout = ColumnLayout::default();
        let mut dropped: Vec<String> = Vec::new();
        if let Some((words, bad)) = value("columns") {
            dropped.extend(bad.iter().map(|b| format!("`{b}` in columns")));
            match words.as_texts() {
                Some(words) => {
                    let mut visible = vec![ColumnId::Name];
                    for word in words {
                        match ColumnId::parse(&word) {
                            Some(ColumnId::Name) => {}
                            Some(id) if !visible.contains(&id) => visible.push(id),
                            Some(_) => dropped.push(format!("`{word}` again in columns")),
                            None => dropped.push(format!("`{word}`")),
                        }
                    }
                    layout.visible = visible;
                }
                None => dropped.push("columns (not a list)".to_owned()),
            }
        }
        if let Some((widths, bad)) = value("widths") {
            dropped.extend(bad.iter().map(|b| format!("`{b}` in widths")));
            match widths.as_numbers() {
                Some(widths) => {
                    for (word, width) in widths {
                        match ColumnId::parse(&word) {
                            Some(ColumnId::Name) => {}
                            Some(id) => {
                                layout.widths.insert(id, clamp_width(id, width as f32));
                            }
                            None => dropped.push(format!("`{word}`")),
                        }
                    }
                }
                None => dropped.push("widths (not a table)".to_owned()),
            }
        }
        let text = |key: &str| value(key).and_then(|(v, _)| v.as_text());
        if let Some(word) = text("sort").filter(|w| !w.is_empty()) {
            match ColumnId::parse(&word) {
                Some(column) if layout.visible.contains(&column) => {
                    let side = match text("sort_side").as_deref() {
                        Some("right") => Side::Right,
                        _ => Side::Left,
                    };
                    let descending = value("sort_descending")
                        .and_then(|(v, _)| v.as_bool())
                        .unwrap_or(false);
                    layout.sort = Some(SortKey {
                        column,
                        side,
                        descending,
                    });
                }
                Some(_) => dropped.push(format!("the sort on `{word}`, a column not shown")),
                None => dropped.push(format!("`{word}`")),
            }
        }
        let warning = (!dropped.is_empty()).then(|| {
            format!(
                "[invalid_config_value] folder layout: unknown column id(s) or bad entries {}: dropped",
                dropped.join(", ")
            )
        });
        (layout, warning)
    }

    /// The keys of `[layout.<view>]` this layout owns: `columns`, `widths` (only those dragged), `sort`, `sort_side`,
    /// `sort_descending`. The sort keys are always written, the tree order as `sort = ""` (left, ascending), so a
    /// tree order replaces a remembered sort and a session's tree order wins over the view type's sort (D-108 overlays
    /// key by key).
    pub fn to_layout(&self) -> Vec<(&'static str, LayoutValue)> {
        let mut out = vec![(
            "columns",
            LayoutValue::Texts(self.visible.iter().map(|c| c.as_str().to_owned()).collect()),
        )];
        if !self.widths.is_empty() {
            out.push((
                "widths",
                LayoutValue::Numbers(
                    self.widths
                        .iter()
                        .map(|(id, w)| (id.as_str().to_owned(), f64::from(*w)))
                        .collect(),
                ),
            ));
        }
        let (column, side, descending) = match &self.sort {
            Some(sort) => (sort.column.as_str(), sort.side, sort.descending),
            None => ("", Side::Left, false),
        };
        out.push(("sort", LayoutValue::Text(column.to_owned())));
        out.push((
            "sort_side",
            LayoutValue::Text(if side == Side::Right { "right" } else { "left" }.to_owned()),
        ));
        out.push(("sort_descending", LayoutValue::Bool(descending)));
        out
    }

    /// The columns shown: the visible ones, Path only while `flat`.
    pub fn shown(&self, flat: bool) -> Vec<ColumnId> {
        self.visible
            .iter()
            .copied()
            .filter(|&c| c != ColumnId::Path || flat)
            .collect()
    }

    /// True when `id` is visible (`ColList`'s check marks).
    pub fn is_visible(&self, id: ColumnId) -> bool {
        self.visible.contains(&id)
    }

    /// Shows or hides `id` (`ColList`, the header menu); Name always shows. A hidden sorted column ends the sort.
    pub fn toggle(&mut self, id: ColumnId) {
        if id == ColumnId::Name {
            return;
        }
        if let Some(at) = self.visible.iter().position(|&c| c == id) {
            self.visible.remove(at);
            if self.sort.is_some_and(|s| s.column == id) {
                self.sort = None;
            }
        } else {
            self.visible.push(id);
        }
    }

    /// Moves `id` to `index` among the visible columns (a header drag); Name stays first.
    pub fn move_to(&mut self, id: ColumnId, index: usize) {
        if id == ColumnId::Name {
            return;
        }
        let Some(at) = self.visible.iter().position(|&c| c == id) else {
            return;
        };
        self.visible.remove(at);
        let index = index.clamp(1, self.visible.len());
        self.visible.insert(index, id);
    }

    /// The width of `id` (points; Name: its minimum, it stretches).
    pub fn width(&self, id: ColumnId) -> f32 {
        if id == ColumnId::Name {
            return min_width(id);
        }
        self.widths
            .get(&id)
            .copied()
            .unwrap_or_else(|| default_width(id))
    }

    /// Resizes `id` (a separator drag), clamped to its minimum and [`MAX_WIDTH`]; Name stretches and keeps none.
    pub fn resize(&mut self, id: ColumnId, width: f32) {
        if id == ColumnId::Name || !width.is_finite() {
            return;
        }
        self.widths.insert(id, clamp_width(id, width));
    }

    /// Fits `id` to the widest rendered cell (a double-click on its separator).
    pub fn fit(&mut self, id: ColumnId, widest: f32) {
        self.resize(id, widest + 12.0);
    }

    /// A header click on `id` on `side` (D-235): ascending, then descending, then back to the tree order; another
    /// column or side starts ascending.
    pub fn header_click(&mut self, id: ColumnId, side: Side) {
        self.sort = match self.sort {
            Some(s) if s.column == id && s.side == side && !s.descending => Some(SortKey {
                descending: true,
                ..s
            }),
            Some(s) if s.column == id && s.side == side => None,
            _ => Some(SortKey {
                column: id,
                side,
                descending: false,
            }),
        };
    }
}

/// `width` within `id`'s minimum and [`MAX_WIDTH`].
fn clamp_width(id: ColumnId, width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(min_width(id), MAX_WIDTH)
    } else {
        default_width(id).max(min_width(id))
    }
}
