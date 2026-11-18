# Unsaved edits

When a Text Compare view with edits that are not saved would close — Close Tab, Close Other Tabs, closing the
window, Exit, Go to Home, loading a workspace, Reload of the files — Hikasa asks for each modified file, one at a
time: *Save changes to <name>?*

- **Save** saves the file (Save As for a file without a place, such as one from the clipboard). When the save fails,
  is declined or cancelled, nothing closes.
- **Discard** drops that file's edits.
- **Save All** and **Discard All** answer for every file waiting in the same close.
- **Cancel** closes nothing.

These questions come before the question about saving the session. An auto-saved session remembers the files and
the rules, never the edits: it reopens the files as they are on disk. See [The Text Compare
view](topic:view-text-compare).
