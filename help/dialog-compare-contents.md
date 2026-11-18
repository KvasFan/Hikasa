# Compare Contents

Actions ▸ **Compare Contents** in the [Folder Compare view](topic:view-folder-compare) compares the contents of the
selected file pairs (the pairs below a selected folder too):

- **Method**: CRC, Binary, or Rules-based (the [text rules](topic:text-compare) of the session's formats). It starts as
  the session's content method.
- **Show results dialog**: after the compare, a list of every pair and its result, with the counts.

The dialog counts the pairs and their bytes; with `[file_operations] confirm_content_compare` a question names them first
(*Do not ask again* turns it off). The results enter the view: the centre column shows each result (same, different,
rules-based same, unimportant differences only, important differences). With `beep_after_long_operations` a compare of
10 seconds or more beeps. The method and the results choice are remembered. See [hikasa folder op](topic:folder-op)
`compare`.
