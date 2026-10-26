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
  *M1 document notes (D-12, D-43; contract §1.5.2, §2.9):* lines end only at CR, LF and
  CRLF; the Unicode separators VT, FF, NEL, LS and PS are ordinary characters (Hikasa keeps
  its own line table next to the rope). A document holds at most 4 GiB − 1 bytes and at
  most 2,147,483,645 lines (`MAX_LINES`, the alignment engine's limit); larger inputs, and
  text that a legacy encoding would decode to more than 4 GiB − 1 bytes, are rejected
  with *too large* before anything is aligned. Content classified binary (§2.9 of the
  contract: a known signature, a NUL byte, more than 10 % control bytes) is still decoded
  and shown, with warning `binary_content`, but it never gets a rules-based verdict: the
  result is *binary same* (identical bytes) or *binary different* (exit 1 / 11), in Text
  Compare, in `hikasa quick --rules` and in folder content tests alike. Malformed byte
  sequences are replaced with U+FFFD and counted (warning `lossy_decode`). Encoding and
  BOM are never compared (D-04): a UTF-16 file and its UTF-8 copy are *rules same*. The
  Sorted conversion sorts lines by code point and makes the document read-only; an
  External conversion runs only when the caller provides a process runner, otherwise the
  load fails (never a silent unconverted comparison). The binary verdict of an externally
  converted document is taken from the converted bytes.
  *Wave-3 review notes:* a line split by the characters-per-line limit never compares
  equal to a line ended by a real line break (`abcdef` wrapped at 3 differs from
  `abc⏎def`), whether or not line endings are compared; the limit counts the pieces of
  wrapped lines against the line limit before anything is allocated. Loading, decoding
  (in 1 MiB steps), the Sorted conversion, the line split and the rope build all honour
  cancellation within 100 ms, and a cancelled load is always an error, never a document.
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
  *M1 engine notes (D-16, D-54; contract §1.3.3, §2.14):* Myers is imara-diff's
  heuristic O(ND) diff and is not guaranteed to find a minimal LCS: its preprocessing
  can prune a line that occurs often on the other side, which then never matches (Patience
  inherits this in its Myers fallback). Histogram, Myers and Patience run without a work
  budget: on adversarial input (such as random lines over a small alphabet) they are
  roughly quadratic but stay cancellable. Standard alignment is bounded: when its work
  budget runs out, the remaining ranges are aligned with Histogram while a second,
  deterministic fallback budget lasts; beyond it they are split at lines that are unique
  on both sides and the gaps between them are paired by position or similarity (warning
  `alignment_degraded`), so Standard stays O(n log n) on every input.
  *Wave-3 notes (D-36 as amended, D-54):* the default engine *Auto* is Standard for plain
  text ("Everything Else"); for a format with a grammar ("source code") it is Histogram
  while the estimated cost of the differing middle (left lines × right lines after
  stripping the common start and end) fits the same deterministic budget that bounds
  Standard's own Histogram fallback (128 × (left + right lines) + 32,000,000), and
  Standard otherwise. Small and typical edits of source files therefore get Histogram,
  while large or repetitive files with changes spread over the whole file get the bounded
  Standard engine (a 7 MiB file of repeated statements against another compares in a
  fraction of a second instead of seconds to minutes). The engine actually used is shown
  with the result. Explicitly chosen Histogram, Myers and Patience are never replaced.
  Long imara-diff calls (more than 4,096 lines on either side) run on a helper
  thread, so cancelling a comparison returns within 100 ms even inside one large call; the
  helper of a cancelled call finishes in the background (at most two such helpers run at a
  time). Line weights (TC-012) are computed only when the algorithm is Standard, the only
  engine that uses them (also when *Auto* picks Standard).
- **TC-011** (P0): *Skew tolerance* (lines to look ahead/behind for a match in
  standard alignment, default 300); *Use closeness matching* (pair remaining
  mismatched lines by similarity so they render as *changed* lines with in-line
  character highlights rather than delete+insert); *Never align differences* (always
  show changes as delete+insert blocks).
  *M1 notes:* the skew tolerance limits Standard's own anchor search only; ranges that
  Standard's budget fallback aligns (Histogram, or the unique-line split) are not
  limited by it. Closeness matching runs within a per-gap and a total cell budget;
  beyond them a gap is paired by position (warning `closeness_budget_exhausted`).
  Without a similarity scorer only identical lines pair. Myers has no closeness
  matching: its gaps are paired by position (warning `closeness_ignored`).
  *Wave-3 notes (D-17; contract §2.15 as amended):* closeness matching applies to the
  gaps of Standard, Histogram and Patience; Unaligned pairs by position. Text Compare
  scores two lines by their *important* text (the text the rules keep, case-folded where
  case does not matter), so lines that differ only in unimportant text always pair. The
  score is 999 ‰ when the two texts are equal apart from whitespace (`x  ` against `x`,
  `}` against `} `), the share of common characters when either text has fewer than three
  (`}` against `};` scores 666 ‰), and the share of common character pairs otherwise. Two
  lines pair only when the score reaches the threshold (default 500 ‰); dissimilar
  replacements (`x` against `y`) show as a deleted plus an inserted line, both important.
  The score looks at the first 256 characters of each line's important text, so one
  score costs the same however long the lines are, and the scoring of one comparison
  has a deterministic work budget (counted in characters compared, about a tenth of a
  second of work): when a huge rewritten region exhausts it, the rest of that region shows
  as deleted plus inserted lines and warning `closeness_budget_exhausted` is added.
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
  *M1 notes (contract §2.16):* inline ranges are computed on demand for each changed row.
  On a row with only unimportant differences every range is unimportant (blue), for
  example `Hello World` against `helloworld` under `--unimportant whitespace,case`. On an
  important row, a case-only change inside a case-insensitive element (an SQL keyword, a C
  number, or everything-else text under `--unimportant case`) is unimportant; otherwise
  each range is split by the importance of its characters; ranges are widened to whole
  grapheme clusters, so a combining mark is never split from its base character. Lines
  longer than 10,000 characters show their whole differing middle as one range; on lines
  longer than 256 KiB the syntax spans and the colouring cover the first 256 KiB (the rest
  is one important span), so a view of a multi-megabyte line stays responsive.
- **TC-015** (P0): Merge conflicts (Text Merge): *Only different change lines* or
  *Left change and right change separated by ≤ N lines* (default 2).
- **TC-016** (P1): Results must be identical in GUI, CLI quick-compare, scripts and
  reports (same engine, same rules).
  *M1 notes (D-12, D-15, D-25):* `hikasa quick --rules` and the rules-based folder
  content test run the same key building as Text Compare and return the same verdict
  (property-tested for every algorithm), with the same regular-expression limits and the
  same External conversion program: a format whose files are converted by a program is
  converted on every path, and without a program every path fails with the same error.
  When the key sequences are equal the verdict is *rules same* or *unimportant only*
  without aligning; when they differ and orphan lines are always important, the verdict is
  *important differences*, also without aligning, unless both files have the same number
  of lines and a line's key depends on the lines before it (a hex number inside and
  outside a block comment), in which case the full comparison decides.
  The folder content test reads files up to 128 MiB per file (`DEFAULT_RULES_MAX_BYTES`);
  larger files are compared by bytes with warning `rules_size_limit`, and formats of a
  non-text kind (pictures, media, executables) are always compared by bytes.

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
- *M1 importance notes (D-04, D-13, D-14, D-53; contract §2.13):*
  - **Defaults (D-04):** leading, embedded and trailing whitespace are important;
    "everything else" is important and case-sensitive; every grammar element is important
    unless the file format ships it unimportant (Python's trailing `Whitespace` element and
    COBOL's `Identification` columns 73–80); no unimportant-text patterns; orphan lines are
    always important; line endings are not compared; encoding and BOM are never compared.
    A plain comparison is therefore exact apart from line endings, encoding and BOM, for
    every element the format marks important and case-sensitive. Formats can declare
    elements case-insensitive (C numbers, SQL keywords: `0xFF` against `0xff` is an
    unimportant change) or ship them unimportant (see above); with such a format the
    identity fast path does not apply.
    Ignoring whitespace is an explicit opt-in: `--unimportant whitespace` (also
    `leading-ws`, `embedded-ws`, `trailing-ws`, `case`, `everything-else`, or an element
    name such as `Comment`; `element:<Name>` when the name equals one of those words).
    `Everything Else`, the reserved name of text outside grammar items, means
    `everything-else`; as an element name (`element:Everything Else`, or a key of the
    rules file's `elements` table) it is rejected.
  - **Whitespace classes (D-13)** apply only to text that no grammar token covers. A
    continued block comment does not include the line's leading blanks, and a token that
    runs to the end of the line does not include the trailing blanks, so re-indenting a
    block comment is unimportant when leading whitespace is; an element that a format
    defines explicitly (Python's `Indent`) keeps its own importance. A line of blanks only
    counts as leading whitespace.
  - **Line keys (D-14; contract §2.13.2 as amended in wave 3):** a line is compared by its
    important text: the characters the rules keep, case-folded where case does not matter.
    Where removed characters separate two word characters of different tokens and one of
    them is a grammar token, the key keeps a token boundary, so `int x` differs from
    `intx` even with whitespace unimportant; elsewhere text joins across removed
    characters, like `diff -w` (`a + b` equals `a+b`, and in plain text `a b` equals `ab`,
    when whitespace is unimportant). A line from which nothing is removed or folded is
    keyed by its raw text, whatever the lines around it. Two lines with equal keys are
    *same* when their raw text is equal (and their endings, when compared), else an
    *unimportant* change. Byte-identical lines are never an important difference: when
    their keys differ only because of their context (a line inside a block comment
    against the same line outside it, with a case-insensitive number in it), the pair is
    an *unimportant* change. A line whose key is empty (blank, or only unimportant text)
    is *trivial*: it never anchors an alignment, and as an orphan it is unimportant unless
    orphan lines are always important.
  - **Identity fast path (D-53):** when the rules make every character important and
    case-sensitive (the defaults, with a format that has no unimportant or
    case-insensitive element), a line's key is its raw text and no tokenisation runs; the
    syntax spans for display are computed lazily. The full key building gives the same
    keys for every grammar, block comments and multi-line strings included (a line from
    which nothing is removed is keyed by its raw text), so both paths give identical
    results; an unimportant-text pattern that never matches does not change a result.
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
  *M1 note:* the engine gives one cell per thumbnail bucket (a bucket covers an equal
  share of the rows) with the set of row classes inside it (same, unimportant, important,
  left orphan, right orphan); with *Ignore Unimportant Differences* unimportant rows count
  as same. With more buckets than rows some buckets are empty.
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
  beneath the editors (P2; optional).
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
  *M1 notes (contract §2.18; D-03):* a row is *same* for display when it is same, or
  unimportant while *Ignore Unimportant Differences* is on. *Show Context* keeps every row
  within N rows of a difference, so differences at most 2N rows apart share one block,
  exactly like the hunks of a unified diff (the patch reports use the same view). Every
  run of hidden rows becomes one gap item ("N filtered lines"); expanded gap bars stay
  visible whatever the filter. Display filters never change the verdict or the exit
  code. The toggles show the categories a filter shows in full or in part
  (`TextDisplayFilter::toggles`; *Show Context* with N > 0 shows all four, its context
  rows being same rows), and flipping one switches to toggle mode without hiding a row
  of any other category: from *Show Context* only switching *Same* off removes the
  context rows.

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

- *M1 note (D-15):* one effective format applies to both sides of a comparison: the
  forced format (`--format`), else the format detected from the first side that has a
  name (left, then right; standard input uses the `--stdin-name` hint), else "Everything
  Else". When both sides have names and their detections differ, warning
  `format_mismatch` says which one was used.
- Grammar (what a comment *is*) lives in the file format. Importance (whether comments
  *matter*) lives in the session rules. Defaults for the Text Compare session type are
  edited from Home ▸ New ▸ Text Compare ▸ Edit; per-view overrides use the scope selector;
  folder sessions can carry overrides for all child text sessions.
