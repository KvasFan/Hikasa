//! The `view` and `display` tables of a session as a view fills them (M2 W4b; D-108, D-120, SES-010). The tables
//! stay opaque to the session file (D-81); the GUI names no TOML type, so the conversions live here.

use hikasa_folder::DisplayOptions;
use hikasa_text::ViewOptions;
use toml::Value;

use crate::settings::state::LayoutValue;

/// An opaque session table (`view`, `display`, the `view` of a workspace tab).
pub type OpaqueTable = toml::Table;

/// The `view` table of a view's layout keys.
pub fn layout_table(keys: &[(&str, LayoutValue)]) -> OpaqueTable {
    keys.iter()
        .map(|(key, value)| {
            let value = match value {
                LayoutValue::Bool(b) => Value::Boolean(*b),
                LayoutValue::Float(f) => Value::Float(*f),
            };
            ((*key).to_owned(), value)
        })
        .collect()
}

/// A boolean value of a `view` table.
pub fn view_bool(value: &Value) -> Option<bool> {
    value.as_bool()
}

/// The `display` table of a Text Compare view: its filter and ignore-unimportant (gap bars are not kept).
pub fn text_display_table(options: &ViewOptions) -> OpaqueTable {
    let plain = ViewOptions {
        expanded: Vec::new(),
        ..options.clone()
    };
    let mut table = OpaqueTable::try_from(&plain).unwrap_or_default();
    table.remove("expanded");
    table
}

/// Applies a `display` table to a Text Compare view's options; an empty or unreadable table changes nothing.
pub fn apply_text_display(table: &OpaqueTable, options: &mut ViewOptions) {
    if let Ok(read) = table.clone().try_into::<ViewOptions>() {
        options.filter = read.filter;
        options.ignore_unimportant = read.ignore_unimportant;
    }
}

/// The `display` table of a Folder Compare view.
pub fn folder_display_table(options: &DisplayOptions) -> OpaqueTable {
    OpaqueTable::try_from(options).unwrap_or_default()
}

/// Applies a `display` table to a Folder Compare view's options; an empty or unreadable table changes nothing.
pub fn apply_folder_display(table: &OpaqueTable, options: &mut DisplayOptions) {
    if let Ok(read) = table.clone().try_into::<DisplayOptions>() {
        *options = read;
    }
}

#[cfg(test)]
mod tests {
    use hikasa_folder::{FolderDisplayFilter, FolderMode};
    use hikasa_text::TextDisplayFilter;

    use super::*;

    #[test]
    fn display_tables_round_trip() {
        let mut text = ViewOptions {
            filter: TextDisplayFilter::Context { lines: 3 },
            ignore_unimportant: true,
            expanded: vec![1..2, 5..6],
        };
        let table = text_display_table(&text);
        assert!(!table.contains_key("expanded"));
        let wanted = text.clone();
        text.filter = TextDisplayFilter::All;
        text.ignore_unimportant = false;
        apply_text_display(&table, &mut text);
        assert_eq!(text, wanted);
        apply_text_display(&OpaqueTable::new(), &mut text);
        assert_eq!(text, wanted, "an empty table changes nothing");

        let mut folder = DisplayOptions {
            filter: FolderDisplayFilter::Toggles { bits: 5 },
            folders: FolderMode::OnlyFiles,
            ignore_unimportant: true,
            suppress_filters: false,
        };
        let table = folder_display_table(&folder);
        let wanted = folder;
        folder.filter = FolderDisplayFilter::All;
        folder.folders = FolderMode::AlwaysShowFolders;
        apply_folder_display(&table, &mut folder);
        assert_eq!(folder, wanted);
    }

    #[test]
    fn layout_tables_hold_the_keys() {
        let table = layout_table(&[
            ("toolbar", LayoutValue::Bool(false)),
            ("font_size", LayoutValue::Float(15.0)),
        ]);
        assert_eq!(table["toolbar"], Value::Boolean(false));
        assert_eq!(table["font_size"], Value::Float(15.0));
    }
}
