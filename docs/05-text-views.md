# Text Views: Text Compare, Text Merge, Text Edit, Text Patch

All four views share the text document model (`hikasa-text`), the alignment engines
(`hikasa-diff`) and the file-format/grammar engine (`hikasa-formats`).

## 1. Document model

- **TC-001** (P0): A document is a rope of Unicode text plus per-line metadata: original
  line ending (CRLF / LF / CR / none at EOF, "MIX" when inconsistent), edited flag,
  saved-edit flag, bookmarks. The source encoding (detected: BOM → `chardetng` →
  format default → "current locale"; overridable per pane) and line-ending style are
  preserved on save unless converted.
- **TC-002** (P0): Loading: specs may be local files, archive members, remote files,
  HTTP URLs, clipboard, stdin. Files are read through the file format's **conversion**
  (`10-file-formats.md` §4) before being shown; a converted document is read-only
  unless the format defines a save conversion.
- **TC-003** (P0): Binary content detection warns "contains binary data, edit anyway?".
  A *characters per line limit* (format setting) wraps very long lines artificially;
  the artificial breaks are removed when saving.
- **TC-004** (P0): Editing is disabled when: load error, comparison cancelled, the
  location is read-only (read-only archives, media), the conversion cannot save, the
  parent folder session disabled editing, the session disabled editing, or command-line
  read-only switches were given. The status bar shows "Read-only"/"Editing disabled".
- **TC-005** (P0): Save (Ctrl+S) writes back through the VFS with optional backup,
  preserving encoding/BOM and line endings; "Save File As…" (F12), "Save File on File
  System As…", "Save File Using Profile As…". Format "When saving" options: trim
  trailing whitespace, convert leading spaces to tabs. `--savetarget` overrides the
  output path.
- **TC-006** (P1): "Check for files changed on disk" → prompt to reload (or auto-reload
  when no edits) when the view regains focus or a watcher fires. **Reload Files** (F5)
  prompts to save edits; **Recompare Files** (Ctrl+F5) re-aligns the current (edited)
  content without reloading and clears undo history.

## 2. Alignment engines (Rules → Alignment tab)

- **TC-010** (P0): Engine choice: *Unaligned* (line n ↔ line n), *Standard
  alignment* (recursive: find the best anchor block, align it, recurse into the gaps;
  progressive output; supports closeness matching), *Myers O(ND)* (LCS via
  `imara-diff`; whole-file; no closeness), *Patience Diff* (unique-line anchors, falls
  back to Myers in gaps). **Histogram** is also offered (imara-diff) as the default
  for source code.
- **TC-011** (P0): *Skew tolerance* (lines to look ahead/behind for a match in
  standard alignment, default 300); *Use closeness matching* (pair remaining
  mismatched lines by similarity so they render as *changed* lines with in-line
  character highlights rather than delete+insert); *Never align differences* (always
  show changes as delete+insert blocks).
- **TC-012** (P0): Alignment operates on **important text** only when the rules say
  unimportant text is ignorable, i.e. lines differing only in unimportant tokens match
  as *unimportant difference* lines; **line weights** from the file format (regex
  patterns with priority) bias the anchor choice towards structurally significant lines
  (function headers, class declarations).
- **TC-013** (P0): Manual overrides: **Align With** (F7): select line(s), then click the
  counterpart on the other side; **Isolate**: force the selected lines to align with
  nothing. Overrides are kept across recompare and saved with the session as line-content
  anchors (P1).
- **TC-014** (P0): Inline (within-line) differences are computed per aligned pair at
  character level, grouped by grammar element, so that an unimportant token change inside
  an otherwise important line is coloured blue, not red.
- **TC-015** (P0): Merge conflicts (Text Merge): *Only different change lines* or
  *Left change and right change separated by ≤ N lines* (default 2).
- **TC-016** (P1): Results must be identical in GUI, CLI quick-compare, scripts and
  reports (same engine, same rules).

## 3. Importance (Rules → Importance tab)

- **TC-020** (P0): A list of **Unimportant text** items (regex/plain patterns whose
  matches are unimportant anywhere), then the grammar element checklist of the current
  file format(s): each element (Comment, String, Keyword, Identifier, Number, Operator,
  Directive, …) is important or not; plus *Leading whitespace*, *Embedded whitespace*,
  *Trailing whitespace*, *Everything else* (with *Character case* sensitivity for
  everything-else text; grammar elements carry their own case flag). "Edit Grammar…"
  opens the format editor.
- **TC-021** (P0): *Orphan lines are always important* (an inserted line containing
  only unimportant text still counts as important).
- **TC-022** (P0): *Compare line endings (PC/Mac/Unix)* — off by default (line ending
  differences are ignored).
- **TC-023** (P1): **Replacements** tab: items {text to find, replace with, match case,
  whole words, regular expression (replacement-style `$1`), side (left/right/either)};
  when the left text with the replacement applied equals the right text the difference
  is unimportant and painted with the *Replacement* colour. "Replacement…" command adds
  an item from the current difference. Navigation: Next/Previous Replacement.
- **TC-024** (P0): "Ignored" marks the selected lines/section as ignored (persisted in
  the session for named sessions); *Ignore Unimportant Differences* (view) treats
  unimportant as same for display, navigation, counts and reports.

## 4. Display

- **TC-030** (P0): Two editor panes (side-by-side or over-under), scrolled in lockstep
  by *alignment row*; gaps (missing lines) render as hatched empty rows; lines beyond
  EOF optionally cross-hatched. A gutter between/beside panes hosts per-section copy
  buttons ("Copy Section to Left/Right", "Copy Line(s)…") and conflict markers. Option
  "Right side gutter for left editor" puts both gutters in the middle.
- **TC-031** (P0): Colouring: important difference text (red) / unimportant (blue) /
  replacement; line background light red/blue when a line contains such a difference
  (even if scrolled off); orphan colour optional; syntax highlighting on non-difference
  lines (optionally also on difference lines); edited lines and saved-edits colours;
  ignored section colour; current line; selection (custom or system); find highlight;
  visible whitespace glyphs (space ·, tab →, CRLF/LF/CR marks when Visible Whitespace
  is on).
- **TC-032** (P0): **Thumbnail**: one pixel-row per alignment row at the left, coloured
  by state, with a rectangle for the viewport and a triangle for the current line;
  click to jump; modes *Compress to fit* / *Allow scrolling*; can be hidden.
- **TC-033** (P0): **Details panel** at the bottom: *Text Details* (editable full-width
  copy of the current line pair), *Hex Details* (read-only hex of the current lines),
  *Alignment Details* (editable character alignment view); *Ruler*; "Lock Visible
  Whitespace"; hide.
- **TC-034** (P0): Line numbers; Word Wrap (both panes wrap to width, alignment rows
  grow to the tallest side); Display Font / Increase / Decrease / Reset (Ctrl+Num+/−/0);
  Extra line spacing; column guide line at N; dim inactive pane by N%; single vertical
  scrollbar option; sticky splitter position; Zoom pane (F11) and incremental pane
  resize (Ctrl+Alt+Left/Right).
- **TC-035** (P0): **File Info** header per pane: path (editable spec with MRU and
  Browse), encoding and line-ending indicators (click to override), format selector,
  read-only flag, "Modified". A **Filtered gap bar** ("N FILTERED LINES") appears where
  display filters hide lines; it can be expanded/collapsed; "Show filtered line counts"
  option.
- **TC-036** (P1): **Webpages** panels: render the current files (HTML) as web pages
  beneath the editors (P2 on Dioxus via iframe; optional).
- **TC-037** (P0): Status bar: cursor line/column, Insert/Overwrite (Ins), comparison
  state of the current line (Same, Important Difference, Unimportant Difference, Ignored,
  Left/Right Orphan, …), grammar element under the cursor, encoding, line ending, and
  the load time.

## 5. Display filters

- **TC-040** (P0): Show All, Show Differences, Show Same, Show None, **Show Context**
  (differences with N context lines — program option "Number of context lines"); toggle
  mode with Same / Left Orphans / Right Orphans / Differences. Merge filters: Show All,
  Show Changes, Show Conflicts, Show Left Changes, Show Right Changes, Show Mergeable,
  Show Unchanged, Show None; *Ignore Same Changes*; *Favor Left/Right Changes*.

## 6. Navigation

- **TC-050** (P0): Next/Previous Difference Section (Ctrl+N/P: next unignored
  section), Next/Previous Difference (Shift+Ctrl+N/P: next difference *text* within
  long lines), Next/Previous Edit, Next/Previous Replacement, Next/Previous Existing
  Line (Alt+Down/Up, skip gaps), Next/Previous Conflict Section (Alt+N/P), Clear
  Conflict Section + Next, Next/Previous Left/Right Taken, Go To line:column (Ctrl+G),
  Bookmarks 0–9 (toggle Ctrl+Shift+digit, go to Ctrl+digit, Clear Bookmarks), browse
  mode selector for the toolbar arrows (by difference section / difference / edit /
  find / replacement / taken).
- **TC-051** (P0): Options: go to first difference on load; go to next difference after
  copying to the other side; wrap around at end; show message panel/beep when none
  found; limit "Next Difference Files" to the current folder.
- **TC-052** (P0): Child-session navigation: **Next/Previous Difference Files**
  (Ctrl+M / Shift+Ctrl+M) open the parent folder session's next pair with differences
  in this view; **Copy File to Right/Left and Open Next Difference** (Ctrl+Alt+R/L)
  with a confirmation listing the steps (save/discard edits, copy entire file, open next);
  Take Left/Right File and Open Next Difference in merge.
- **TC-053** (P0): Pane navigation: Go Left/Right (Alt+Left/Right), Go Next/Prev
  pane (F6/Shift+F6), Go Input/Output (Alt+PgUp/PgDn) in merge.

## 7. Editing

- **TC-060** (P0): Full editor in each pane: insert/overwrite, selection (keyboard,
  mouse, Shift+arrows, Ctrl+A), Cut/Copy/Paste, Delete, Undo/Redo (unbounded, per
  pane, including copy-section operations), Increase/Decrease Indent (Shift+Ctrl+I/U),
  Delete Line / to Start / to End of Line, Delete Word / to Start / to End of Word,
  Insert Line Before/After (Shift+Ctrl+Enter / Ctrl+Enter), Select Section (Ctrl+D),
  auto indent, backspace unindents, tab inserts tab or spaces per format, tab stop per
  format.
- **TC-061** (P0): Comparison re-runs incrementally after each edit (debounced); the
  edited lines are marked.
- **TC-062** (P0): Copy to Right/Left (Ctrl+R/L: selected lines or current section),
  Copy Line to Right/Left (Shift+Ctrl+R/L), dynamic "to Other Side".
- **TC-063** (P0): Convert File ▸ Trim Trailing Whitespace, Leading Spaces to Tabs,
  Tabs to Spaces, Line Endings ▸ To Windows/DOS, To Unix, To Macintosh (reported as
  "Converted N lines").
- **TC-064** (P0): Find (Ctrl+F, panel: text/regex with element inserter, match case,
  whole words, wrap, sides, close automatically, "Select All" matching lines, "initialize
  text to find with current word" option), Find Next/Previous (F3/Shift+F3), Replace
  (Ctrl+H; replace one/all with confirmation), **Find in Files** (Shift+Ctrl+F: folder,
  mask, subfolders → results list opening Text Edit views), Compare Selection to
  Clipboard (new view).
- **TC-065** (P0): Open Clipboard (Shift+Ctrl+V) loads clipboard text into the picked
  pane.

## 8. Text Merge specifics

- **TM-001** (P1): Up to four panes: left, center (ancestor, optional, can be hidden),
  right on top; editable **output** below (can be detached into its own window).
  Inputs are read-only.
- **TM-002** (P1): Line classes: Unchanged, Same change, Left change, Right change,
  Different change (both changed, mergeable because non-overlapping within the
  section), Conflict (overlapping different changes). Colours: teal left, magenta right,
  red conflict, yellow = edited output, orange mergeable.
- **TM-003** (P1): The output is built automatically on load: center (or left when no
  center) plus non-conflicting changes taken from the changed side; conflicts keep the
  center text and are flagged. `--favorleft/--favorright` take the favoured side for
  unimportant-only conflicts and suppress its highlighting.
- **TM-004** (P1): Take commands (toolbar, gutter buttons per section, context menu):
  Take Left / Center / Right (Ctrl+L / – / Ctrl+R), Take Left Then Right (Ctrl+B),
  Take Right Then Left (Shift+Ctrl+B), Take Left/Center/Right Line; **Conflict** toggles
  the conflict flag of a section. Editing the output marks the section as edited;
  re-taking reverts.
- **TM-005** (P1): Save output; "Compare to Output" (left/center/right vs output in a
  new Text Compare); "Merge Parent Folders"; **Text Merge Info** statistics by class.
- **TM-006** (P1): Headless merge (CLI `--automerge`, script, Folder Merge): same
  engine; conflicts → exit code 14/101; `--force` writes CVS-style conflict markers;
  `--reviewconflicts` opens the view when conflicts exist.

## 9. Text Edit

- **TE-001** (P0): Single-pane editor with all editing, search (incl. Find in Files),
  bookmarks, format/encoding/line-ending handling, syntax highlighting, word wrap,
  webpages panel; opened from Tools ▸ Edit Text File, Open With ▸ Text Edit, or the
  `--edit` switch.

## 10. Text Patch

- **TP-001** (P1): Opens a `.diff`/`.patch` (normal, context or unified formats; multi-
  file) and reconstructs the comparison: left = original, right = patched, per file
  (Next/Previous Difference Files navigates between files inside the patch). Read-only
  panes; standard navigation, thumbnail, details, bookmarks.
- **TP-002** (P1): **Apply Patch** applies the patch to the chosen base folder/file
  (strip-level prompt) and reports rejects; "Open With ▸ Text Edit" on the patch file.
- **TP-003** (P1): Text Compare report "Patch" layout generates normal/context/unified
  diffs consumable by this view (`13-reports.md`).

## 11. Rules vs. formats (precedence)

- Grammar (what a comment *is*) lives in the file format. Importance (whether comments
  *matter*) lives in the session rules. Defaults for the Text Compare session type are
  edited from Home ▸ New ▸ Text Compare ▸ Edit; per-view overrides use the scope selector;
  folder sessions can carry overrides for all child text sessions.
