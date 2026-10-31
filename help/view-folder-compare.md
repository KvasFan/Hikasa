# The Folder Compare view

The Folder Compare view shows two folder trees side by side, aligned by name. Each item's colour tells its
state: same, different, newer or older, orphan (on one side only), filtered, or not compared.

## Navigating

- Up and Down move the selection; Right and Left open and close a folder while the tree has the focus.
- Enter opens the selected file pair in a [Text Compare view](topic:view-text-compare).
- Ctrl+N and Ctrl+P go to the next and the previous difference.
- Edit ▸ Expand All and Collapse All; Actions ▸ Open Subfolders and Close Subfolders.
- Ctrl+F5 compares everything again; Stop ends a running comparison.

## Display filters

- View ▸ Display Filter: Show All, Show Differences, Show Same, Show No Orphans, Show Orphans, the newer filters
  and Show None choose which items are listed; the toggles combine categories. The toolbar shows them as a
  drop-down with toggles, as favourites or as toggles (right-click the toolbar to choose).
- The folder modes: Always Show Folders, Compare Files and Folder Structure, Only Compare Files.
- Ignore Unimportant Differences treats unimportant differences as same; Suppress Filters temporarily shows
  filtered items.

## Log

View ▸ Log shows the log panel at the bottom: finished comparisons, warnings (with their codes) and errors of
every window, settings reloads and notices. `[tweaks.log_panel]` options choose what is kept, how many entries
and an optional file the entries are also written to. Clear empties it.

The same comparison is available as [hikasa folder compare](topic:folder-compare).
