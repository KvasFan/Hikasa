# The Find in Files panel

The bottom panel of a window shows the last [Find in Files](topic:dialog-find-in-files) search; it stays until the
next search or **Clear**.

- Each file with hits is a line with its number of hits; its hits follow as `line:column: preview`, the hit
  highlighted. Files not searched (binary, too large, unreadable) are under **Skipped (N)**; click it to open it.
- The summary line counts the files searched and matched and the hits, and says when a limit stopped the search.
  **Stop** ends a search at once; the hits so far stay.
- Enter or a double click opens a hit: the Text Compare tab of this window showing the file is activated, else a
  [Text Edit view](topic:view-text-edit) of the file opens (or its tab is activated) with the hit selected and the
  query in its Find panel. When the open file was edited since the search, the hit's text is looked for near its
  line; when it is gone, the caret goes to the line and the message panel says so.
- Ctrl+C copies the selected line as `<file>:<line>:<column>: <preview>` (a file's line: all its hits).
- **Replace in Files…** with **Replace with** rewrites every hit: the plan lists the files and their counts first, then
  asks *Replace N occurrences in M files?* and the save questions. Files open in a view with unsaved edits are left
  out (*open with unsaved edits*), also when the edits were typed while the questions were open (*has unsaved
  edits*); open views of the rewritten files without edits are read again.
