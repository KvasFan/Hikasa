# Exclude

Actions ▸ **Exclude** in a [Folder Compare view](topic:view-folder-compare) or a [Folder Sync view](topic:view-folder-sync)
hides items by adding masks to the view's name filters:

- **Selected items**: the selected rows by their paths.
- **All files of these types**: every file with the extension of a selected file.

The dialog shows the masks for files and for folders before OK. OK adds them (each once) to the session's exclude
lists and the comparison runs again; the session is changed until it is saved. The choice is remembered. See
[hikasa folder op](topic:folder-op) `exclude`.
