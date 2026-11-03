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
    /// Unknown column ids are dropped; a width is clamped; a bad sort word leaves the tree order. The text of one
    /// `invalid_config_value` warning comes back when anything was dropped.
    pub fn from_values(
        value: &dyn Fn(&str) -> Option<LayoutValue>,
    ) -> (ColumnLayout, Option<String>) {
        let mut layout = ColumnLayout::default();
        let mut dropped: Vec<String> = Vec::new();
        if let Some(words) = value("columns").and_then(LayoutValue::as_texts) {
            let mut visible = vec![ColumnId::Name];
            for word in words {
                match ColumnId::parse(&word) {
                    Some(ColumnId::Name) => {}
                    Some(id) if !visible.contains(&id) => visible.push(id),
                    Some(_) => {}
                    None => dropped.push(word),
                }
            }
            layout.visible = visible;
        }
        if let Some(widths) = value("widths").and_then(LayoutValue::as_numbers) {
            for (word, width) in widths {
                match ColumnId::parse(&word) {
                    Some(ColumnId::Name) => {}
                    Some(id) => {
                        layout.widths.insert(id, clamp_width(id, width as f32));
                    }
                    None => dropped.push(word),
                }
            }
        }
        if let Some(word) = value("sort").and_then(LayoutValue::as_text) {
            match ColumnId::parse(&word) {
                Some(column) => {
                    let side = match value("sort_side").and_then(LayoutValue::as_text).as_deref() {
                        Some("right") => Side::Right,
                        _ => Side::Left,
                    };
                    let descending = value("sort_descending")
                        .and_then(LayoutValue::as_bool)
                        .unwrap_or(false);
                    layout.sort = Some(SortKey {
                        column,
                        side,
                        descending,
                    });
                }
                None => dropped.push(word),
            }
        }
        let warning = (!dropped.is_empty()).then(|| {
            format!(
                "[invalid_config_value] unknown folder column id(s) {} in the layout: dropped",
                dropped
                    .iter()
                    .map(|w| format!("`{w}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
        (layout, warning)
    }

    /// The keys of `[layout.<view>]` this layout owns: `columns`, `widths` (only those dragged), `sort`, `sort_side`,
    /// `sort_descending` (the sort keys only with a sort).
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
        if let Some(sort) = &self.sort {
            out.push(("sort", LayoutValue::Text(sort.column.as_str().to_owned())));
            out.push((
                "sort_side",
                LayoutValue::Text(
                    if sort.side == Side::Right {
                        "right"
                    } else {
                        "left"
                    }
                    .to_owned(),
                ),
            ));
            out.push(("sort_descending", LayoutValue::Bool(sort.descending)));
        }
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
