//! The list pages of the Options dialog (M2 W4c; OPT-010, OPT-012, OPT-020): archive masks, Open With applications
//! and name filter presets. Every check is the schema's `validate` (T11): an item is taken into the working copy
//! only when the list still validates.

use hikasa_session::settings::archives::ArchiveType;
use hikasa_session::settings::{
    FilterPreset, FormValue, OpenWith, OpenWithApp, SettingsSchema, Shortcut, set_form_value,
    split_words,
};

use super::OptionsDialog;

/// The Open With page: the selection and the item being edited.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenWithPage {
    /// The selected row.
    pub selected: Option<usize>,
    /// The item editor: New and Clone have no index, Edit the row it replaces.
    pub editing: Option<OpenWithEdit>,
    /// The shortcut field waits for a key (§1.4.13).
    pub capturing: bool,
}

/// The item editor of the Open With page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenWithEdit {
    /// The row it replaces (`None`: a new row).
    pub index: Option<usize>,
    /// The item.
    pub app: OpenWithApp,
    /// Why OK did nothing.
    pub error: Option<String>,
}

impl OptionsDialog {
    /// Sets the mask of an archive type (a bad mask is a problem of the page, D-128).
    pub fn set_archive_mask(&mut self, kind: ArchiveType, mask: &str) {
        let path = format!("types.{kind}");
        if set_form_value(&mut self.archives, &path, &FormValue::Text(mask.to_owned())).is_ok() {
            self.edited();
        }
    }

    /// New: an empty item.
    pub fn open_with_new(&mut self) {
        self.open_with.editing = Some(OpenWithEdit::default());
    }

    /// Edit: the selected item.
    pub fn open_with_edit(&mut self) {
        let Some(index) = self.open_with.selected else {
            return;
        };
        if let Some(app) = self.openwith.app.get(index) {
            self.open_with.editing = Some(OpenWithEdit {
                index: Some(index),
                app: app.clone(),
                error: None,
            });
        }
    }

    /// Clone: a new item with the selected one's values.
    pub fn open_with_clone(&mut self) {
        let Some(app) = self
            .open_with
            .selected
            .and_then(|i| self.openwith.app.get(i))
        else {
            return;
        };
        let mut app = app.clone();
        app.description = format!("{} (copy)", app.description);
        self.open_with.editing = Some(OpenWithEdit {
            index: None,
            app,
            error: None,
        });
    }

    /// Delete: the selected item.
    pub fn open_with_delete(&mut self) {
        if let Some(index) = self.open_with.selected.take()
            && index < self.openwith.app.len()
        {
            self.openwith.app.remove(index);
            self.edited();
        }
    }

    /// OK of the item editor: checked with `split_words` and `OpenWith::validate`; a bad item stays in the editor.
    pub fn open_with_commit(&mut self) {
        self.open_with.capturing = false;
        let Some(edit) = self.open_with.editing.as_mut() else {
            return;
        };
        let item = OpenWith {
            app: vec![edit.app.clone()],
        };
        let error = split_words(&edit.app.command)
            .err()
            .map(|e| format!("command: {e}"))
            .or_else(|| item.validate().into_iter().next().map(|p| p.message));
        if let Some(error) = error {
            edit.error = Some(error);
            return;
        }
        let Some(edit) = self.open_with.editing.take() else {
            return;
        };
        match edit.index.filter(|i| *i < self.openwith.app.len()) {
            Some(index) => self.openwith.app[index] = edit.app,
            None => {
                self.openwith.app.push(edit.app);
                self.open_with.selected = Some(self.openwith.app.len() - 1);
            }
        }
        self.edited();
    }

    /// Cancel of the item editor.
    pub fn open_with_cancel(&mut self) {
        self.open_with.editing = None;
        self.open_with.capturing = false;
    }

    /// The shortcut field of the item editor: the next key is the item's shortcut.
    pub fn open_with_begin_capture(&mut self) {
        self.open_with.capturing = self.open_with.editing.is_some();
    }

    /// The key the shortcut field waited for: Esc gives up, anything else is the shortcut.
    pub(super) fn open_with_capture(&mut self, key: &Shortcut) {
        self.open_with.capturing = false;
        let plain = !key.ctrl && !key.alt && !key.shift && !key.cmd;
        if let Some(edit) = self.open_with.editing.as_mut()
            && !(plain && key.key == "Esc")
        {
            edit.app.shortcut = key.to_string();
        }
    }

    /// Adds a name filter preset with a free name.
    pub fn preset_add(&mut self) {
        let mut n = self.filters.preset.len() + 1;
        let mut name = format!("Preset {n}");
        while self.filters.preset.iter().any(|p| p.name == name) {
            n += 1;
            name = format!("Preset {n}");
        }
        self.filters.preset.push(FilterPreset::new(&name, "*.*"));
        self.edited();
    }

    /// Removes a name filter preset.
    pub fn preset_delete(&mut self, index: usize) {
        if index < self.filters.preset.len() {
            self.filters.preset.remove(index);
            self.edited();
        }
    }
}
