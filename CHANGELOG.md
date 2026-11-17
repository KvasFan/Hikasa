# Changelog

## Unreleased

### Added (W11 GUI text views, part a: editor and save)
- **Editing in Text Compare**: both panes are editors with a caret and a selection (keys, Shift, mouse drag, double and
  triple click, the line numbers), typed text and input methods (the composition shown at the caret), Overwrite Mode
  (Ins), Word Wrap, Undo/Redo per pane (Ctrl+Z/Y), Cut/Copy/Paste (Copy without a selection takes the line; on
  Linux the primary selection and middle-click paste), Delete Line (Shift+Ctrl+K, new), the delete, insert-line and
  indent commands, Select Section. The comparison follows every key and is redone exactly a moment after typing stops.
- **Copy to Right/Left** (Ctrl+R/L), Copy Line to Right/Left and to the other side; arrows between the panes copy a
  difference section with one click; `[next_difference] go_to_next_difference_after_copy` is honoured.
- **Saving in the window**: Save (Ctrl+S), Save All Files (new), Save File As… (F12), Save File on File System As…; the
  format's save options as one undo step; the save's questions as prompts (with *Save as UTF-8*); keys typed during a
  save are applied after it.
- **Unsaved edits** are asked about, one file at a time (Save, Discard, Save All, Discard All, Cancel), by every close
  of a view, a window or the program and by Reload.
- **File Info header** above each pane (View ▸ File Info): path (Enter opens a typed path in that pane), encoding
  (reopen with another, save with another, BOM), line endings (convert), format (pick another: the comparison is redone,
  the edits stay), *Read-only*, *Editing disabled* (*Edit anyway* for binary files), *Converted*, *Modified*.
- **Status bar** of a text view: line and column (display column for tabs), INS/OVR, the row's state, the element under
  the caret, encoding, line ending, *Recomparing…*.
- Help topics `dialog-unsaved-edits`, `dialog-save-questions`; `view-text-compare` describes the editor.
- `gui close` and `gui quit` (IPC `CloseView`, `Shutdown`) ask about unsaved edits first; the protocol has an additive
  `force` (Hello feature `close.force`) that drops them.

### Fixed (W11 part a, after its review)
- A close or a Reload whose *Save* followed an earlier save no longer goes on before its own save has asked and
  written (the edits could be lost); a Save As answer that cannot be used no longer leaves the close waiting forever.
- Keys, clicks and commands given while a pane is being saved (or its comparison redone) are applied in the order
  given, at the places they name; Cut during a save deletes the text it copied.
- Cut deletes the text only once the clipboard holds it.
- Save All asks where to save a modified pane that has no file (`clipboard://`, `-`) instead of skipping it.
- Save File on File System As… saved nothing (every file was refused); a path into an archive is refused.
- Clicks, Up/Down, Ctrl+Left/Right and End never put the caret inside a flag, an emoji sequence or a letter with an
  accent mark; a click in the other pane ends an input method's composition.
- Picking *Detected* in the format menu restores the detected format's tab size and line limit; reopening a pane with
  another file renames the tab; a reopen cancelled at its prompt is forgotten.
- Copy to Other Side and Copy Line to Other Side are in the Edit menu.
- The first key after a comparison no longer copies the syntax colouring's per-line states of the whole file (G8: the
  key's bookkeeping 0.07–0.3 ms instead of up to 2 ms on a million lines).
### Added (W12 hex and picture)
- **The Hex Compare engine** (W12a; the subcommands and the view follow in W12c and W13): two files of any size are
  opened lazily (a file up to 16 MiB is read whole and released at once, larger ones read in 64 KiB pages; archive
  members from the temp cache) and aligned **None** (byte n with byte n), **Fast** (content-defined blocks, small gaps
  byte by byte; 1 GiB in seconds) or **Complete** (every gap byte by byte with Myers windows within a budget), with
  exact statistics, sections (runs of differing bytes, fewer than 8 same bytes apart merged), a 4,096-bucket
  thumbnail, rows for any layout (bytes per line, groups, ASCII/Latin-1 characters, hex or decimal addresses) under
  the All, Differences (with context) and Same filters, navigation and Go To (`0x1F`, `1Fh`, `$1F`, `#31`, `31d`,
  `+n`, `end`), find of text in four encodings or hex bytes with `??` wildcards, and little/big-endian value
  interpretation. The verdict is the binary one (exit 1 or 11, never 12) for every alignment mode.

### Changed (W12 hex and picture)
- The binary quick compare, the CRC and the hex passes share one **chunk visitor** in fixed 8 MiB chunks (mapped,
  streamed or in memory, any number of threads, the same results); `quick --binary`'s answers are unchanged. A file
  that ends while it is compared or hashed is reported as `unexpected_eof` by the visitor and compared again by
  streaming (the M1 answer), never read as zeros.

### Changed (W10 live recompare, G8 follow-up)
- **Pasting, inserting or removing lines in a large edited pair stays incremental**: an edit that shifts the lines
  after it (Enter, deleting a line, pasting 100 lines, Copy Section) no longer makes the live recompare re-run a large
  nested part or fall back to the full compare. On the 1,000,000-line pair Enter, delete and copy are recomputed in
  3–17 ms (were up to 1.3 s on some places) and a paste of 100 lines in 22–30 ms at the median (was 1.4 s); the result
  is still exactly that of comparing the two documents from scratch (docs/02).

### Added (Table Compare in the window, phase 3)
- **The Table Compare view**: two files of a table format (CSV, workbooks) open in a grid, from the command line,
  `table compare --gui`, a drop, Home, Open With ▸ Table Compare or a folder row's Open. The grids show each side's row
  numbers and column heads, cells coloured by importance, orphan rows hatched on the other side, sheet tabs coloured by
  their state, the thumbnail and the Text Details panel; side by side or over-under. Display filters and toggles,
  Ignore Unimportant Differences, next and previous difference row (Ctrl+N/P), cell (Shift+Ctrl+N/P) and sheet, Go To
  (Ctrl+G), Hide Same Columns, Resize Columns to Fit, resizable and hideable columns, Copy of a cell or a row, the
  status bar's address and state, Table Compare Info, Table Compare Report…, Reload, Swap Sides, Compare Parent
  Folders.
- **Sessions**: `table-compare` sessions open in tabs and keep the sheet shown, the hidden columns and the widths in
  `[view]`, the display in `[display]`; the Session Settings dialog has the Sheets, Columns (per-column rules and the
  default), Rows and Parsing tabs, which compare the view again on OK; *Update session defaults* applies to new views.

### Fixed (Table Compare in the window, phase-3 review)
- The session tabs write back only the fields you change: a scope of *Update session defaults* no longer copies the
  view's own keys or column rules into the defaults, list entries keep their order, keys are one a line, and a space
  or tab separator is kept (shown as `space`, `tab`).
- `table compare --gui` without words runs what `table compare` runs (the session defaults apply only to views opened
  without words); two tables given a Text Compare option open in Text Compare and say why, one encoding for both
  sides is the tables' encoding; `--format` with `--left-format`/`--right-format` and a wrong `--cells` are refused
  before the window is contacted.
- A table view that closes or is replaced stops its comparison.
- Copy places a cell's whole text; a copied row quotes cells holding tabs or line breaks and leaves hidden columns out.
- The report dialog writes a file named without its folder to the last report's folder (else the home folder) and
  shows the whole path, asks before replacing a file, refuses `-` and `clipboard://`, and reports the view as it
  shows (its rows and *Ignore Unimportant Differences*).
- Two columns of the same name are hidden and resized one by one (`name#2` in the session); navigation never lands on
  a hidden column nor shows it again; sheet tabs scroll and their spots follow *Ignore Unimportant Differences*; a
  folder row honours the folder's format overrides.
- Memory and speed: text tables keep one offset per record after the comparison and re-read a row when it is shown
  (the 1,000,000 × 10 pair: 333 MiB in the window instead of 581 MiB, a comparison's peak 410 MiB instead of 617 MiB); next and
  previous difference take microseconds under any display filter; a 16,384-column sheet draws only its columns on
  screen.

### Added (Table Compare: xlsx and other tables, phases 1 and 2)
- **`hikasa table compare <left> <right>`** compares two tables cell by cell: every sheet of two workbooks (xlsx,
  xlsm, xlsb, xls, ods, read through `calamine` and recognised by content), two delimited or fixed-width text files,
  or one of each (an export against its source). Cells compare by typed values (`1.50` = `1.5`; a date cell = its CSV
  date; workbook numbers at the 15 digits a workbook shows, `--exact-numbers` for the stored value; a column with
  leading zeros or long numbers is text, `--number` makes it numeric). Sheets pair by name, then case, then position
  (`--sheets`, `--sheet <L>=<R>`); columns by header or number (`--columns`, `--align-column`); rows by `--key` as a
  database join (duplicates counted) or aligned like text (`--algorithm`, `--skew`, closeness, `--never-align`,
  `--sort-rows`). Rules: `--unimportant`, `--text`, `--number`, `--ignore-case`, `--ignore-whitespace`,
  `--tolerance [<cols>=]<n>[%]`, `--date-tolerance`, `--formulas`, `--ignore-hidden-rows`/`--ignore-hidden-columns`,
  `--hide`; they apply to every sheet, and a column a sheet lacks is left out there (`rule_not_applied`). Parser
  flags replace the formats' settings: `--type`, `--delimiter`, `--qualifier`, `--consecutive`, `--trim`,
  `--widths`, `--first-line`, `--regional system`, `--decimal`, `--thousands`, `--date-order`, `--date-sep`,
  `--encoding`, `--ctrl-z`. Exit codes 1, 2, 12 (2 with `--ignore-unimportant`), 13 and 100; `--gui` opens the
  comparison in the window.
- **Output**: a line per sheet pair, the differing rows (`row 3/3: price: 2 → 2.5`, at most 20 per sheet unless
  `--display all`) and a summary line; `--json` prints `hikasa.table_compare.v1` (44 schemas; JSON Schema in
  `docs/schemas/table_compare.v1.json`); `--report summary|side-by-side|interleaved|xml|json` writes reports as text or
  HTML to every REPORT target, and scripts write them with `table-report` (`data-report`).
- **Sessions**: `table-compare` sessions keep the flags' words (`[rules]`, `[rules.parse]`, per-sheet
  `[[rules.sheet]]`); `session run` runs them.
- **Folders and quick**: the rules-based content test of `folder compare` and `quick --rules` compare table files
  through the table engine; a table that does not load is that pair's error.
- **Formats**: the *Spreadsheet Workbooks* preset (`*.xls;*.xlsx;*.xlsm;*.xlsb;*.ods`, type `spreadsheet`); the
  `[table]` settings gain `kind`, `delimiters`, `trim_whitespace`, `widths`, `first_line`, `[table.regional]` and
  `ctrl_z_eof`; the built-in *Spreadsheet text* conversion lets Text Compare read workbooks as tab-separated text.
- Help topics `table-compare` and `table-formats`.
- **Review of phase 2** (contract §10.6, T-43…T-50): every parser flag also per side (`--left-decimal ,`,
  `--right-date-order mdy`, …; sessions `[rules.left_parse]`, `[rules.right_parse]`, and `[specs] left_encoding` and
  `right_encoding` each for its side); a `.csv` the comma does not split is read with `;`, tab or `|` when one of them
  does, and a given `--delimiter` that leaves every row one field warns `delimiter_mismatch`; `--regional system`
  reads Windows' and macOS' regional settings when no locale variable is set; `--max-rows`; output flags the output
  does not use are refused, `--cells` reaches `--report json`; per-sheet session rules no longer fail on a key they
  inherit (`keys_not_applied` names a sheet compared unkeyed); `session edit --set rules.*` sets every table rule;
  `--type fixed` needs widths. The human output honours `--hide` and `--ignore-unimportant` (`same (unimportant
  differences)`, the summary); a workbook a text view cannot convert keeps its own error (`unsupported`,
  `invalid_data`); identical files are `binary_same` in a folder before they are read; JSON `side.format` is null for a
  format of another kind; messages show sheets as written.

### Added (W10 text engine, part a: editing and recompare)
- **Editable documents** (core, rendered by W11): every text document keeps its lines in copy-on-write blocks, so an
  edit costs its own size and a snapshot shares every untouched block. Edits follow fixed line-ending rules: pasted
  breaks take the document's most frequent terminator (`[text_editing] default_line_ending`, `platform` by default,
  for a document without one), a joined line keeps the later line's terminator, a copy from the other pane keeps its
  own, wrapped lines are wrapped again. Undo and redo are unbounded per pane (typing groups within a second, at most a
  word a step); lines are marked *edited* and *saved edit*; bookmarks 0–9 follow their lines. The editing commands
  (Enter with auto indent, Tab and Shift+Tab, indent, Backspace that unindents, the Delete Line/Word family, Insert Line
  Before/After, overwrite, smart Home, word steps, Select Section, Copy to Right/Left and Copy Line) are defined in
  core.
- **Editability**: a pane is *Read-only* (read-only file or file system, a gz/bz2/xz/zst member, a Sorted or External
  conversion without a save program, a cut document) or *Editing disabled* (`--ro`, `--ro1`, `--ro2`, the session, a
  read-only parent folder side, binary content until the user allows it); writes of such a pane are refused.
- **Align With and Isolate** (TC-013): `hikasa text compare` (also `report text`, `session run` of a text session)
  takes `--align <L>:<R>` (lines or ranges, 1-based, repeatable) and `--isolate left|right:<a>[-<b>]`; a pair is a
  segment of its own, isolated lines align with nothing; crossing or overlapping pairs are dropped with warning
  `anchor_dropped`; `hikasa.text_compare.v1` gains `anchors`. Without anchors every result is unchanged.
- **Live comparison**: an edit moves the rows at once, then the comparison is recomputed off the window, re-running only
  the part of the alignment the edit can change; the result is always exactly that of comparing the two documents from
  scratch. After the W10a review the recompare keeps every line's key as an interned id, follows edits that move a
  part's common prefix or suffix, decides the skew window exactly per diagonal run, splices only the changed rows and
  shares the unchanged parts of its record: typing or deleting a line in a 1,000,000-line pair is recomputed in a few
  milliseconds (docs/02); work that would cost more than an eighth of the lines takes the full compare instead.

### Fixed (W10a review)
- *Auto* is decided again when an edit moves the first or last difference (a compare could stay on Standard where a
  fresh one picks Histogram); a format whose item starts at the first line re-keys the old and new first line.
- Two recompare inexactnesses found by the strengthened tests: a block reused although its extension's stop changed,
  and a window check reading the previous part's lost pairs.
- Undo and redo refuse a blocked pane like any edit; an editor wraps at its own characters-per-line limit from the
  start, so undo restores the loaded lines; Copy to Right/Left adds the target pane's line ending; the inline
  differences of a just-edited row come from the live lines; editing inside one very large difference (two unrelated
  files) no longer re-derives it per keystroke.
- `align_traced` reports that Myers ignores closeness on every input, as `align` does.

### Added (W10 text engine, part b: save, conversions, formats)
- **Save** (TC-001, TC-003, TC-005; core and Service, rendered by W11): an edited document is written so that every
  unchanged line keeps its exact bytes in every encoding (BOM, every line-ending kind, malformed bytes, ISO-2022-JP, a
  Ctrl-Z tail, lines wrapped at the characters-per-line limit); only edited lines are encoded. The whole text is
  encoded only on an encoding change, without a source, over a file changed on disk, or for an edited ISO-2022-JP
  document (warning `save_reencoded`). A save is a plan of the new VFS action `write`: the content is streamed in 1 MiB
  steps (a 48 MB save needs about 1 MiB of extra memory), the source it splices from is checked by CRC-32 at the end of
  its read, and the target by the stamp the plan saw. `load_document` records each file's size, time and CRC-32 while
  reading it (G1 unchanged).
- **Save questions** through W6's question machine (`Reply`, `ConfirmMode`, `Declined` now live in `hikasa_core::confirm`
  with an `Ask` trait, so one prompt drives both): *changed on disk* (only the time changed: warning `source_touched`,
  no question), *read-only target*, *unmappable characters* (written as `?` when replaced, never as numeric
  references; warning `unmappable_replaced`), *malformed bytes rewritten* (`lossy_line_rewritten`), *replace existing*,
  *overwrite backup*. Targets: a symbolic link is followed and stays, special files are refused, Save As moves the
  document, `--savetarget` redirects the right pane's save and is the next save's source after the first one, a
  document without a location saves as. Backups: `[backups] back_up_before_save` with `naming`/`folder`, one generation
  (the backup the last save made is replaced without asking), `--nobackups`; a member of a zip or tar archive (also
  nested) is saved by rebuilding its archive once, with one backup of the archive.
- **Format save options** (`save_trim_trailing_ws`, `save_leading_spaces_to_tabs`) change edited lines only, as one
  undo step before the save.
- **Conversions** (TC-063): Trim Trailing Whitespace, Leading Spaces to Tabs, Tabs to Spaces and Line Endings are one
  undo step each over the file or a selection's lines, counting the lines changed (*Converted N lines*); tab stops are
  display columns; converting every line's endings also changes the terminator new lines get. A request of many edits
  (a conversion, later Replace All) is one splice of the line table: trimming every second line of a 1,000,000-line
  file takes 0.2 s instead of 18 s, and lines it leaves unchanged keep their marks.
- **`hikasa text convert <FILE>…`**: `--trim-trailing`, `--tabs-to-spaces`/`--spaces-to-tabs` (`--tab-size`),
  `--line-endings lf|crlf|cr`, `--encoding`, `--bom`/`--no-bom`, `--lines`, applied in the fixed order trim, tabs, line
  endings, encoding; `--format`, `--encoding-in`; output to stdout (default), `--in-place` (one plan for all files,
  backups unless `--no-backup`) or `--output`; files with nothing to change are skipped (`no_change`) and never
  rewritten; `--yes`, `--force`, `--confirm`, `--unmappable refuse|replace`; without a terminal open questions exit 100
  with their counts; `--dry-run`; JSON `hikasa.text_convert.v1` (42 schemas); exit 0 or 100. Help topic
  `text-convert`.
- **A format per pane** (FF-003): `hikasa text compare --left-format`/`--right-format <id|name>` (also on `report
  text`; `[specs] left_format`/`right_format`): each side is tokenized and classified with its own grammar and the
  rules' elements are matched by name; `--format` with either is refused (exit 100); `hikasa.text_compare.v1` gains
  `right_format`.
- **Ignored lines** (TC-024): `--ignored left|right:<a>[-<b>]` on `text compare`, `report text` and `session run`;
  ignored rows count as same for the verdict, the exit code, statistics, display filters, the thumbnail, section
  navigation and every report, the alignment unchanged; marks follow edits in a live comparison and are kept by
  sessions (`[specs] ignored_left/right` with `ignored_check_left/right`); a changed file drops them with warning
  `ignored_lines_dropped`. `hikasa.text_compare.v1` gains `ignored`, rows' and sections' `ignored` and
  `stats.ignored_rows`.
- **`gui open` / `hikasa-gui` switches** (CLI-003): `--edit` opens each file in a Text Edit view (two files, two
  views; `--wait` waits for every view; with `--fv` or `--sync` the last wins; the window answers `unsupported_view`
  until W11 builds the view); `--savetarget=<file>` is sent as an absolute path; `--ro`, `--ro1`, `--ro2` block those
  panes of text views; `--nobackups` covers saves.

### Fixed (W10b gate)
- The live recompare could pair lines of a repeated key as if each were unique when an edit widened the part's window
  (found by the heavy property run; latent since W10a): the result now always equals a fresh compare.

### Fixed (W10b review)
- **A save keeps the file it replaces**: other hard links, the owner and group, POSIX ACLs and extended attributes
  (SELinux label, `user.*`) stay with the saved file. Where a rename would lose them (another hard link, another owner,
  a folder Hikasa cannot write) the file is written in place after its backup was copied, and a failure puts the old
  content back; a writable file in a read-only folder can now be saved. Windows keeps the hidden, system and archive
  attributes (what it cannot keep is in docs/11).
- Saving an archive member keeps its place in the archive and its header (owner, group, mode, zip attributes).
- `hikasa text convert` holds a file once: the conversions run line by line while the result is written (a 51 MB,
  1,000,000-line file: 414 MiB → 120 MiB at the peak).
- `text convert` of a file with nothing to change writes it to stdout as it is (it wrote nothing) and copies it to
  `--output`; only `--in-place` skips it. Stdout holds up to 64 MiB until the file's check at the end of its read, so
  a file changed meanwhile writes nothing; `--in-place` refuses a FIFO or device before reading it (it could block).
- A pane with `--savetarget` is editable when the target is writable, also when its own file is a read-only temporary
  file.
- GB18030 saves ask about U+E5E5 (which it cannot write) instead of writing `?` silently; ISO-2022-JP saves ask about
  half-width katakana (written full-width before); a save counts what the encoder replaced, so nothing is replaced
  without the question or `--unmappable replace`.

### Changed (W10b)
- D-15 revised: one effective format still applies to both sides unless a pane has its own; `format_mismatch` is not
  given when a pane has its own format.
- `--edit` and `--savetarget` no longer warn `switch_ignored`.
- Editing: the pieces of a line wrapped at the characters-per-line limit keep their marks together, and a Line Endings
  conversion changes only the lines whose terminator differs, so lines an edit leaves unchanged are saved byte for
  byte.

### Added (W10 text engine, part c: search and navigation)
- **Find and replace** (core, rendered by W11): plain text or regular expressions (backreferences and look-around too),
  case on or off with Unicode folding, whole words by the format's word characters (`+=` is a word as itself), Find
  Next and Previous with wrap, Select All with a limit, a comparison searched by row with the left side first;
  replacements with `$1`, `${1}`, `${name}` and `$$`, Replace All as one undo step. A search never crosses a line end
  but finds a word cut by the characters-per-line limit; a pattern that runs out of its backtracking budget on a line
  counts as no match there with one warning.
- **Find in Files** (core): files and folders searched through the VFS in name order, with name masks, recursion,
  hidden items, archives opened as folders, detected encodings, binary and too-large files skipped, limits of hits per
  search and per file, previews around each hit; files are read in parallel and reported in the same order whatever the
  number of threads, and memory does not grow with the hits. New warning `find_limit_reached`.
- **Navigation and details data** (core): next and previous difference section, inline difference (a long changed
  line range by range), edited lines and find hit, each with wrap; the next line of a side over gaps; Go To `line`,
  `line:column` or `:column` on the gutter's line numbers; Hex Details of a line in the save encoding (unwritable
  characters marked), Alignment Details with the panes' colours, the ruler.
- **Session link and Text Edit** (core): Next and Previous Difference Files over the parent folder comparison (closed
  folders included; the parent session compared again when its view is closed), Copy File and Open Next Difference as a
  list of steps (save the source's edits, copy the file as `folder op copy`, open the next pair), Text Edit documents
  and `text-edit` sessions, two texts compared without files (Compare Selection to Clipboard).
- The `$` replacement of folder renames and text replace is one shared rule.
- **`hikasa text find <pattern> <path>…`**: Find in Files on the command line. Hits stream as
  `<file>:<line>:<column>: <text>` while files are searched in parallel (the order never depends on the threads);
  `--regex`, `--case`, `--word`, `--recursive`, `--mask` with excluded folders, `--include-hidden`, `--archives
  opened`, `--encoding`, `--format`, `--convert`, `--max-matches`, `--max-matches-per-file`, `--max-file-size`;
  `--files-with-matches` and `--count`; skipped files with `--verbose`; `--json` prints the new schema
  `hikasa.text_find.v1` (43 schemas). A reader that goes away (`| head`) stops the search. Exit 0 with or without hits.
- **Replace in files**: `hikasa text find … --replace <text>` replaces every occurrence in the files found as one save
  plan (lines without a hit byte for byte, backups by `[backups] back_up_before_save`, `--backup`/`--no-backup`),
  after the question *Replace N occurrences in M files?* (`--yes`, `--confirm`; without a terminal exit 100, nothing
  written); `--dry-run` shows the plan; a file changed since the plan fails its item; `--unmappable refuse|replace`.
- Help topic `text-find`; `exit-codes`, `json-output`, `cli` and `index` updated.

### Added (W9 Folder Compare and Sync in the window, part a: the view)
- **Columns.** Name, Ext, Size, Modified, Attributes, CRC, Owner, Group, and Path in a flat comparison; both sides show
  the same columns in the same order and widths, kept per view type in `[layout.folder-compare]` (`columns`,
  `widths`, `sort`, `sort_side`, `sort_descending`) and in a session's `[view]`. A header click sorts by that column on
  that side, a second click the other way, a third back to the tree order (remembered too); folders stay first and only
  siblings move. A side wider than its half scrolls horizontally, both sides together; Name keeps at least 80 points.
- **Glyphs.** A hollow folder for one not scanned (while comparing, after a stop, an orphan folder left unscanned), a
  filled one in the new scheme colour `[folder] unopened_folder` for a folder not opened because subfolders are not
  scanned automatically, a red X where a listing failed, an arrow on links, the zipper on archives; the centre column
  shows a content result, the newer side's arrow or the state, an Ignored badge and the Space mark.
- **Selection.** Many rows on either side: click a side or the centre, Ctrl and Shift, a swipe down a side (both sides
  once it crosses the centre), Shift+arrows, Ctrl+Space, Space marks an item, Shift+Left/Right keep one side; the
  status bar counts the selected items and bytes per side (each byte once; a folder with its shown rows). Edit ▸ Select All Files, Select Newer and Select Orphans
  (left, right, both) and Invert Selection select what the same `--select` words select; Search ▸ Next/Previous Item on
  This Side skip over the gaps of the focused side; View ▸ Ignore Folder Structure.
- **View ▸ Columns** and the header's context menu show or hide each column; drag a header to move it, a separator to
  resize it, double-click a separator to fit it. With the CRC column shown, the CRCs of the rows on screen are computed
  once the list has been still for 200 ms.
- **Base folder fields** above the panes: completion while typing (folders and archives first), the recent folders of
  each side, Accept and Cancel, a missing folder created after a question naming it in full, a **Browse…** button.
  `-` and `clipboard://` are refused there and in the browse dialog (a window never reads standard input or the
  clipboard for them); names that are not valid Unicode are shown, marked, and cannot be picked.
- **The browse dialog**, Hikasa's own: places (home, the current folder, `/` or the drives, the view's bases, the recent
  folders), Up (also out of an archive), folders first, archives opened as folders, *Show hidden items*, a filter;
  folder, file and Save As modes. It serves the base fields (Session ▸ Browse for Folder), the Home panel, Session
  Settings, the Options path keys, Run Script and Export Settings.
- **Find Filename** (Ctrl+F): a panel above the tree with match case, whole word, regular expression, wrap around,
  search up and the sides; a hit inside a closed folder opens it; F3 and Shift+F3 repeat the search.
- **View ▸ Legend** (Ctrl+Alt+L): a panel that explains the colours, glyphs and centre marks in the theme's colours.
  **Session ▸ Folder Compare Info** (Ctrl+I): the comparison's statistics, with Copy.
- **Base commands**: Session ▸ Up One Level (Left Side, Right Side, the new Both Sides), Actions ▸ Set as Base Folders
  and Set as Base on Other Side, Session ▸ Compare Parent Folders (also in the Text Compare view), Swap Sides (folder
  and text views) and Clear Session. They, and an Accept of new bases, wait while an operation of the view runs.
- Help topics `dialog-browse`, `dialog-folder-info`, `panel-legend`, `panel-find-filename`.
- Core: `hikasa_folder::view` (the sort), `SyncOptions::swapped`, `api::paths` (`path_completions`, `browse_places`,
  `archive_format_of`), `vfs_op_plan`/`vfs_op_execute`, `human::local_time_text`.

### Added (W9 part b: operations and sync)
- **Every file operation of the Actions menu**, on the selection: Move to Right and Left, Copy and Move to Side…
  (which ask the side), Copy and Move to Other Side, Exchange, Copy and Move to Folder…, Rename (inline for one row, a
  DOS mask or regular expressions for more), Touch…, New Folder… (Ins). Each shows its plan before it runs, line by line
  as `hikasa folder op … --dry-run` prints it, with the counts, the warnings and the backups; a dialog plans again on every
  change of its options; *Include hidden items* everywhere; the plan that runs is the plan shown. Read-only sides are
  off, with a tooltip. The dialogs remember their last choices.
- **Ignored** (the selection's marks, kept in the session, nothing runs again), **Exclude** (the masks shown, then the
  view runs again), **Compare Contents** (CRC, Binary or Rules-based, a question with the pairs and bytes when
  `confirm_content_compare` is on, a results dialog), **Quick Compare** (one result per pair, Open), **Compare To…**
  (F7: two items, one and the Space mark, or one and an item you browse to), **Open With** ▸ (a Text Compare view, a
  Folder Compare view of two folders, the Open With applications, the associated application).
- **Synchronize** ▸ Update Right, Update Left, Update Both, Mirror to Right, Mirror to Left: a preview of the sync on the
  view as it stands (*Just selection*, the recycle bin, the confirmation mode), then Run.
- **The Folder Sync view**: the Operations preset, the plan's counts and Run Sync above the tree, the planned action of
  each row in the centre column (overrides and conflicts marked, a folder's summary as its tooltip), the overrides (Leave
  Alone, Copy Left to Right, Copy Right to Left, Delete Left, Delete Right), Next/Previous Conflict, Info with the plan's
  counts, Swap Sides mirroring the options, Sync Base Folders and Compare Base Folders. It opens from Home, New Session,
  a folder-sync session or workspace, `hikasa gui open … --sync` and `hikasa folder sync … --gui`; with `--wait` the
  command exits with the code of the view's last Run.
- `beep_after_long_operations`: an operation, a sync or a content compare of 10 s or more beeps at its end.
- Help topics `view-folder-sync`, `dialog-copy-to-folder`, `dialog-rename`, `dialog-touch`, `dialog-new-folder`,
  `dialog-exclude`, `dialog-compare-contents`, `dialog-quick-compare`, `dialog-sync-preview`.
- Core: `Service::folder_op_preview` and `folder_sync_preview` (`OpPreview`, `SyncPreview`: the dry run's lines over a
  borrowed plan, at most 16 bytes an item), `api::vocab::parse_touch_time`, `ViewOutcome::Sync`,
  `ViewKind::is_served`, `SyncPlan::dropped_overrides` and `unused_overrides`; `Session::swap_sides` mirrors a
  folder-sync session's options; `AppLauncher::start`; `TreeRead::spec_below`; `EntryView::ident`.

### Added (W9 part c: drag and drop, menus, rules)
- **Drag and drop.** Files and folders dropped from a file manager on a path edit or a pane's header become that side's
  base; dropped on a pane's tree they go into the folder under the pointer (or the base) after a prompt that shows the
  plan of a copy and of a move (`vfs cp`/`vfs mv --dry-run` lines) with an overwrite choice; the plan shown runs, and the
  folder is refreshed. Rows dragged to the other pane are copied (Shift: moved) through the operation's preview, onto a
  folder row of the other pane through Copy/Move to Folder. Drops from other programs need X11, Windows or macOS (not
  Wayland); nothing can be dragged out of the window.
- **Clipboard file lists.** Edit ▸ Copy (Ctrl+C) puts the selected items as files a file manager pastes (items inside
  archives are left out, with a message), Paste (Ctrl+V) drops a file manager's copy (or text lines that are paths) on
  the pane's folder, Select All (Ctrl+A) selects every row shown; a focused text field keeps these keys. Actions ▸ Copy
  Filename copies the selected paths as text.
- **Pane context menus** (right-click: the row is selected first; the entries of that side), the column header's menu
  with *Reset Columns*, and **File Manager** ▸ *Open Folder in File Manager*, *Show in File Manager* (Explorer, Finder,
  or the desktop's file manager over D-Bus).
- **Session Settings tabs for folder sessions:** Comparison, Name Filters, Handling, Misc (file formats per session,
  Ignored items) and, for Folder Sync, Sync (preset, matrix, options, overrides); *Disable editing* per side on Specs.
  Session ▸ Rules… and Name Filters… open them; core checks every change and shows a refusal on its tab.
- **The Filters edit** in the folder views' toolbar: the view's name filter, Enter applies it, the presets and recent
  filters in its drop-down, *Add To Presets…* (also on the Name Filters tab) writes `filters.toml`.
- Core: `ClipboardProvider::write_files`, `read_files` (the Linux clipboard helper keeps a file list too),
  `AppLauncher::reveal`, `host::reveal_command`, `Service::reveal_in_file_manager`, `open_folder_in_file_manager`,
  `api::file_manager_targets`, `Service::check_folder_rules`.

### Fixed (W9c review)
- A paste or drop never moves items out of a side with *Disable editing* on (or `--ro`): *Move* is off with the reason
  and checked again when pressed. Pane context menu entries are off exactly when their command would refuse.
- Session Settings with *Update session defaults* or a parent scope writes only what the dialog changed, no longer the
  view's own values (an earlier *this view only* change, Ignored items, sync overrides); a tab holding a refusal is
  marked and OK's tip names it.
- Ctrl+V pastes when the clipboard holds only files; Shift+Del on Windows is Delete Permanently, not Cut; Shift+F10 and
  the Menu key open a pane's context menu.
- A drop from another program whose place the window cannot tell (no pointer position during the drag) asks which
  pane's folder it goes into and offers *Open as new comparison* for one or two paths, instead of using a stale position;
  a drop on a file row goes into its folder; Shift at the drop shows the move first.
- Rows dragged onto a folder whose name ends in a space land in that folder; a name that is not valid Unicode is refused.
- Pasted text gives only absolute paths and `file://` addresses of this computer (on Windows with `\` separators); Copy
  and Copy Filename write in the order asked; the Linux clipboard helper is reaped; `localhost` is taken off file
  addresses read on Linux.
- *Show in File Manager* and *Open Folder in File Manager* start in the home (Windows: system) folder, never the
  item's nor the program's own, and on Windows *Show in File Manager* checks the item and takes `\\?\` off before
  selecting it.
- The Filters edit applies and records a filter only when the view is free, *Add To Presets* leaves the field as typed,
  and neither a new comparison nor Session Settings' OK replaces text being typed.

### Changed (W9b)
- `folder sync --gui` and `gui open … --sync` open the Folder Sync view; the GUI no longer answers `unsupported_view`.

### Fixed (W9b review)
- A sync whose view ends (the tab closed, Session Settings applied, Quit) ends its current item, is logged with one
  notice, and Quit waits for it; a waited Folder Sync view closed meanwhile exits with that sync's code. A Run that fails
  exits 100; a Folder Sync view that cannot load its folders is `load_failed`.
- An override that names no row of the comparison (a file not there yet, a folder not scanned, a filter) is kept, with a
  warning; only one that no row can take is removed, and only from a complete comparison.
- Run Sync never runs the plan of options changed a moment before; the rows and Info show no plan of an older
  comparison; the Synchronize preview offers *Plan again* when the comparison changed under it.
- Open With and the associated application ask before they open more than ten items, can be stopped, and stop at the
  first item that cannot be opened (an error notice names it). Compare To, Quick Compare and Open With ▸ Text/Folder
  Compare keep names that are not valid Unicode. `refresh_when_finished` refreshes the right folders after a re-run.
- A relative Copy/Move to Folder target is the source side's base folder's, shown in full before Start.
- Compare Contents beeps after a long run (`beep_after_long_operations`).
- Faster plans of large operations: the plan compares the file identities the scan recorded instead of reading each
  file's metadata again (the G15 preview's plan of 100,000 replacing copies: 0.8 s → 0.3 s).
- Faster script checks: the lexer scans bytes and a command name is a binary search (a 16 MiB script checks in a
  debug build in about half the time; same results).

### Changed (W9a)
- The Modified column shows local times (the zone of the clock), no longer UTC.
- Copy to Right, Copy to Left, Delete and Delete Permanently act on the whole selection.

### Added (W8 scripts, part a: the language and headless runs)
- **`hikasa script run` and `hikasa script check`.** A script is a text file of commands over one invisible folder
  comparison: load a session or two folders, set criteria and filters, expand, select, compare, copy, move, copy or move
  to a folder or a new archive, delete, rename, touch, set attributes, synchronize, write text and folder reports, log,
  beep. `%1`…`%9`, `%0`, `%NAME%`, `%date%`, `%time%` and `%fn_time%` expand per word, just before a command runs;
  `lt`/`rt` mean left and right where a side is expected; `"` quotes, `#` comments and a trailing `&` continues a line.
  `script check` reports every error with its line and column (exit 106) and runs nothing; a script that cannot be found
  or read is 105, a base that cannot be loaded ends the script with 107, a failed command makes it 100. Flags:
  `--confirm`, `--stop-on-error`, `--silent`, `--dry-run`, `--json` (`hikasa.script.v1`). Every command is
  the matching `folder op`, `folder sync` or `report` command, with the same plans and results.
- **`hikasa folder op attrib --change <CHANGE>…`** sets or clears the attributes a, s, h, r of the selected rows
  themselves (on Unix modes `r` is the write bits; `a`, `s`, `h` are skipped `unsupported` with one warning).
- The warnings `nothing_selected`, `attrib_unsupported` and `log_not_written`, and the help topic `script`.
- `[tweaks.scripts] beep_when_finished` beeps once at the end of a run that is not silent.

### Added (W8 scripts, part b: the window and the command line of the window)
- **Scripts in the window.** Tools ▸ Run Script… asks for a script, its arguments, *Stop on error* and the confirm
  mode, checks the script first (its errors with their positions, nothing runs) and runs it with a status window:
  the command that runs, the progress of what it does, the log lines, the end line, *Stop* while it runs and *Close*
  after. Questions are the confirmation dialogs of file operations over it, and stop on error asks Yes or No. At the end
  a notice (with *Show* on a failure) and the log panel's entries; `[tweaks.scripts] close_when_finished` closes it after
  exit 0, `beep_when_finished` asks for attention. One script runs at a time. Help topic `dialog-script`.
- **`hikasa gui script <file> [<arg>…]`** and **`hikasa script run … --gui`**: the script is found and checked in the
  terminal (105, 106 before the window is asked), then runs in the window with the terminal's current folder and the
  environment variables it names; `--wait` exits with the script's code, `--closescript` closes the status window after
  any run, `--dry-run` runs it in the window as a dry run (a window too old for that refuses it before anything is
  sent), `--json` prints `hikasa.gui_script.v1` (41 schemas). A window that runs a script refuses a second one;
  an older window says to restart it.
- **`@script` on the command line of `hikasa gui open` and `hikasa-gui`**: `hikasa-gui @s.txt a b` runs the script in
  the window (or sends it to the window that runs), `hikasa gui open @s.txt …` is `gui script s.txt …`. With `--silent`
  a script never goes through a window: it runs headless where it was started (Ctrl-C as for `hikasa script run`).
- IPC: `RunScript` carries the current folder, the referenced variables, `wait`, stop on error, the confirm mode, the
  close choice and the dry run; `ScriptFinished` answers a waited run; the error `busy`; the `Hello` reply lists the
  server's features (`run_script.dry_run`).

### Changed (W8b)
- `--silent` and `--closescript` of `hikasa gui open` and `hikasa-gui` apply to an `@script` (`--silent` also to a
  settings package, as before). Without a script `--closescript` warns that it applies to a script only, and `--silent`
  (not for a package) is `switch_ignored` as before; a view switch given with a script (`--ro`, `--sync`,
  `--nobackups`, …) is `switch_ignored`.

### Fixed (W8b review)
- A script's question answered in a second window reached the first window with a status window and the run waited
  for ever; each question and notice now names the status window it belongs to.
- `script run --gui --dry-run` ran the script for real in the window; it is a dry run there now.
- Quitting the window while a script copied returned at once and could cut the copy mid-file; it now stops the script
  and waits for the command that runs to end (at most 30 s), and a waiting `gui script --wait` hears the script's own
  end. A status window that goes with its window keeps the slot of the one script until its copy ended.
- `hikasa-gui @s.txt --silent` ignored Ctrl-C's cancel; it stops as `hikasa script run --silent` does.
- The window read and checked a script sent by `gui script` on the connection's thread; a worker does it now, and a
  relative script path is refused as the protocol says.

### Fixed (W8a review)
- Any failure of a script's `load` (standard input, the clipboard or an empty word as a base, an argument that
  makes a bad base, `create:` that cannot create) ends the script with 107; before, some were exit 100 and the next
  lines ran on the comparison loaded before. A load also drops the sync preset of an earlier Folder Sync session, so a
  bare `sync` after `load <left> <right>` fails instead of mirroring the new pair.
- `script check` reports every error: a lexical error no longer hides the others. It also refuses, at their position,
  words the run would refuse: a base or target `-`, `""` or `clipboard://`, `compare none`, a bad regular expression,
  `attrib +r -r`.
- A dangling link named as the script is a script that cannot be read (105), not absent.
- Base names that are not UTF-8 are shown escaped in every line of the log; a `%%` in an argument is shown as it is.
- `attrib` on a Windows disk changes `r` and skips `a`, `s`, `h` with a warning instead of failing every item; an
  item's change on Unix modes is shown `+r`/`-r`, not the mode.
- A rename onto a free name never leaves two names on a file system without a no-replace rename; a linked report
  runs at most one pair per thread at a time.

## 0.1.93 (2026-11-03, pre-release)

### Added
- **Reports, the core of the next release.** The report library renders the new layouts: Interleaved (one column, the
  left line above the right, as text, ANSI or HTML, with each side's changes struck out on request), XML dumps of text
  and folder comparisons (schema `docs/schemas/hikasa-report-1.xsd`, namespace `urn:hikasa:report:1`), HTML wrapping
  (`wrap-none`, `wrap-word`) and a print block (page orientation, a print palette), and folder side-by-side HTML with
  links to a report per differing file pair. Reports stream: memory stays flat however many rows they have, and a
  cancel stops within a MiB. Without the new words every existing report is byte for byte as before.
- **`hikasa report text|folder`.** Compares and writes a report, side-by-side by default, to standard output, a file, the
  clipboard (`--output-to clipboard`: an HTML report with a plain-text alternative; on Linux a small helper keeps it
  after `hikasa` ends), or the browser (`browser`, and `printer` with a print block), through a new folder in the cache
  that is cleaned after seven days. `--session <ref>` reports a saved session; `-` and `clipboard://` work as sources.
  The exit code is the comparison's.
- **One set of report flags** on `text compare`, `folder compare`, `session run` and both `report` subcommands:
  `--layout` (`--report`), `--options` and `--output-options` with the report vocabulary (`display-*`,
  `ignore-unimportant`, `line-numbers`, `no-line-numbers`, `strikeout-*`, `patch-*`, `column-*`, `include-file-links`,
  `html-*`, `print-*`, `wrap-*`), `--output-to`, `--title`, `--lines` (a range of lines), `--select` and `--path`
  (part of a folder tree), and `--dry-run`, which says what would be written. Two words of one slot are refused, a word
  the layout does not use gives one warning, and every word is checked before anything is compared.
- **Links to file reports.** `--options include-file-links` with folder side-by-side HTML writes a report per differing
  file pair into `<name>_files` beside the report and links them both ways. A later report to the same file replaces
  that folder whole (a marker says it is the report's); a folder of that name that is not the report's is never touched,
  and the report uses `<name>_files-2`. A failure or Ctrl-C leaves the old report and folder as they were.
- **Identical output.** `SOURCE_DATE_EPOCH` fixes the report time, so one comparison and one request give the same bytes
  through `report`, the compare subcommands, a file, standard output, the clipboard and the printer's page.
- **`hikasa.report.v1`** with `--json` (39 schemas), and the warnings `report_option_unused`, `report_folder_renamed`,
  `report_old_folder_kept`, `file_report_failed` and `source_date_epoch_invalid`.
- **Remembered report choices** for the coming report dialog, in `state.toml` (`[dialogs."report-text"]`,
  `[dialogs."report-folder"]`); the command line never reads them.

### Changed
- **A report combination that cannot be made is refused before the comparison** (`--report patch --html`, links to
  standard output): exit 100 as before, but nothing is compared first.
- `session run` takes the report flags of the compare subcommands; its `--dry-run` is the report's for a comparison and
  the sync's for a Folder Sync session, as before.

### Fixed
- **Review of the report writer** (before release): `printer` and `browser` with `include-file-links` always failed
  "not found" and now write the report with its folder of file reports; two reports in the same second no longer share
  a print folder; a report file that is a symbolic link to another folder gets its file reports beside the file it
  names, so the links work; a folder named as the report file is refused by `--dry-run` as by the run; links into an
  archive are refused before anything is compared; a clipboard that cannot be written says "cannot write the
  clipboard" (`io`), and no display is `unsupported`; on Linux the clipboard's helper runs detached from the folder and
  the terminal it was started in, and `hikasa-gui` serves it too; M1's flags (`--wrap`, `--patch-style`,
  `--no-line-numbers`, `--columns`) no longer warn "does not use" on a layout that ignores them; `session run` of a
  folder-sync session names the report flags it ignores in one warning. On Linux and macOS a rename onto a free name
  (folder operations, the report's folders) can no longer replace what another program put there in between.
- **A test of folder sync no longer fails now and then**: it ran too few cases for the share of rare situations it
  checks.
- **`folder op copyto` and `moveto` of an aligned folder pair.** With `--structure base` or `relative`, a pair whose
  folders differ only in case (under `--case-insensitive`) or in NFC/NFD form now lands in one folder at the target, the
  one the target already holds by either name, else a new one named as the left spells it (the right's spelling when only
  the right is written). Before, onto a case-sensitive target, the right side's files went to a folder of their own
  spelling that was never made, and failed `not_found`. A file and a folder whose names differ only in case, under
  `--case-insensitive`, now both go to a case-sensitive target, whichever side holds the file; before, the second was
  skipped as `duplicate_target`.

## 0.1.92 (2026-11-02, pre-release)

### Added
- **Copy and delete in the Folder Compare view.** Select one row, then Actions ▸ Copy to Right (Ctrl+R), Copy to Left
  (Ctrl+L), Delete (Del) or Delete Permanently (Shift+Del). The Delete dialog chooses the sides and the recycle bin; a
  confirmation dialog asks the operation's questions one at a time (Yes, No, Yes to All, No to All, Cancel, and "Do not
  ask again"); both name the row and its sides. The job can be stopped; its summary and failures go to the log, a notice
  sums it up (*Show log* when an item failed), and the rows it wrote are read again, with the open folders and the
  selected row kept, unless you selected another row or closed a folder meanwhile. The rest of the Actions menu, and
  selecting more than one row, come later.
- **The Folder Compare view reads less again.** Open on a closed archive opens it in place instead of comparing
  everything again, and the session remembers it once it opened (one that cannot be read gives a warning); a folder that
  was not compared yet (`--no-auto-scan`) is read when it is opened, after an operation that runs; `expand_on_load` and
  `expand_differences_only` open folders when a comparison ends; a Text Compare opened from a row gives its result back
  to the row.
- **`folder compare` flags.** `--attrib-filter` and `--unixtype-filter` hide items by DOS attribute and by Unix type;
  `--no-auto-scan` scans only the bases, and the orphan folders in a base with their subtrees; `--expand-on-load` and
  `--expand-differences` say what a view opens; `--ignored <PATH>` (with `!<PATH>` for an exception) marks rows whose
  differences count as same, while errors stay; `--format-on` and `--format-off` turn a file format on or off for one
  comparison. Rules files take the same keys, and old ones read as before.
- **`hikasa folder op`.** Compares, then copies, moves, exchanges, deletes, renames, touches, makes folders and archives, copies
  or moves out of the comparison, runs content tests and CRCs, adds to a session's exclude lists and Ignored marks, or opens
  rows with an application. A selection is a mask (`left.diff.files`, `newer`, `all`) or a path; nothing selected acts on
  nothing. `--dry-run` prints the plan. Ctrl-C at a question ends it at once. Schemas `hikasa.folder_op.v1`,
  `hikasa.folder_contents.v1`.
- **Rename masks.** DOS masks and regular expressions over the whole name; a chain is ordered and a cycle goes through a
  hidden name.
- **`hikasa folder sync`.** Update Left, Right or Both, Mirror to Left or Right, or a custom matrix of eight situations ×
  five actions (`--custom mirror-right,different=leave`); `--orphans deleted` reads an item on one side as deleted on the
  other; per-path `--override`s; `--create-empty`, `--within-archives`, `--recycle`, `--just-selection`, `--visible`,
  `--all-rows`. `--dry-run` prints each row's decision and every item. Schema `hikasa.folder_sync.v1`. Syncs settle: a
  second run of a preset plans nothing new (a copied link keeps its time; under `--case-insensitive --filename-case` the
  target takes the source's spelling; two source names that align to one are left as `ambiguous_name` and counted).
- **Folder-sync sessions.** `session new --type folder-sync` keeps the sync in `[rules.sync]`; `session run` runs it, with
  `--dry-run`, `--yes`, `--confirm`, `--allow-empty-source`; `session edit --set rules.sync.overrides."<path>"=<action>`
  sets a per-row override. A folder-sync file of an earlier version reads as before.
- **`--sync` and `--nobackups`** of `gui open` and `hikasa-gui` are understood. `--nobackups` turns backups off for the
  operations of that view, its duplicates and its tab in a saved workspace (a session does not keep it); `folder sync --gui` sends the sync to the GUI, which says it cannot show a Folder Sync view yet
  (exit 100).

### Data safety
- **Nothing is written before you confirm.** An operation is planned first, and its questions are asked before anything
  runs: read-only and system targets, a newer target, a move that replaces, an item with no recycle bin, a folder that
  replaces a file, a change in a read-only archive, a backup over a file the plan did not make. What lies in a folder
  answered no stays with it. `folder op delete` asks whatever `confirm_delete` says; without a terminal or `--yes` nothing
  is touched and the command exits 100. In the window, a dialog closed with a question open is a Cancel, and "Do not ask
  again" counts only with a yes. A question is about the row it names: if you select another row while the plan is made,
  or while the Delete dialog is open, nothing is asked or written.
- **A plan keeps what it saw.** A file changed since the comparison is not overwritten or deleted, and the rest go on. Nothing
  below a followed link, and nothing outside the base folders, is written or deleted; a row whose comparison failed is
  never touched. A rename never replaces. A folder is removed or sent to the recycle bin whole only when nothing below it
  is left out, and only as the plan saw it. A copy reads no more than the plan saw: a source swapped for a FIFO, or for a
  file of another size, fails "changed since the plan was made" and leaves nothing behind. In the window, a plan whose
  comparison changed before it ran does not run.
- **Disable editing** per side (`--ro`, `--ro1`, `--ro2`, `--read-only`, a session's `read_only_left`, `read_only_right`)
  is enforced before anything is planned, also for a `copyto` or `moveto` whose target leads into that side, by `..` or
  through a link. In the window the commands that would write such a side are off, `Del` included, and the Delete dialog
  greys it. It stays so through Session Settings, Save, Save As, Duplicate Tab and workspaces, and a session saved from
  the window keeps `read_only_left`, `read_only_right`: a side becomes writable only when you change that key.
- **The recycle bin is what you chose.** A failing recycle bin fails the item unless `--recycle-fallback` (in the window,
  *If the recycle bin fails, delete permanently*) was given, and then deletes only what the plan saw: a file that came
  into a folder since, or changed, stays, and so does its folder. The Delete dialog never chooses both sides for you, nor
  one side of a row that is on both, even when the other side is read-only.
- **An operation that is cut short is still reported.** Closing its tab, loading a workspace, applying Session Settings
  or quitting while an operation runs stops it after its current item; its summary and failures still go to the log,
  with a notice naming the tab. Quitting waits for it, so no temporary file is left behind.
- **`--path` never means the whole tree.** An empty `--path` (a shell variable that came out empty), `.`, `/` or `left:` is an
  error, and a `--path` that is not on its side is `not_found`; the whole tree takes `--select all`.
- **Filters win in folders the comparison did not list.** Such a folder is listed when the plan is made, through the same
  filters: what they hide is not touched, a folder that keeps something stays, and its files are stamped and asked about
  as rows are, so a file edited while a question is open is not deleted.
- **What would undo itself is refused when planned:** a move of a folder onto itself, a rename of a folder into itself, a
  `copyto` of an archive into itself; a refused `moveto` leaves no new folder, and a move of an archive opened on both
  sides replaces the file whole. Open With gives a name that starts with `-` as `./-…`, so a program never reads it as an
  option.
- **A plan runs what it shows.** An item that does not run, with no cancel, is no success (exit 100, a warning notice in the
  window), and a summary says how many did not run; a folder that cannot go no longer stops its neighbours.
- **A sync never deletes on the strength of what it could not see.** A sync that would delete while the other side holds no
  file (a mount that failed, only empty folders, only files a filter hides) is refused unless `--allow-empty-source`, which
  `--yes` does not imply; a source folder with no file whose counterpart's files a sync deletes is shown as *emptied
  entirely*. A comparison that did not finish deletes nothing; a row that failed, and everything below a folder that could
  not be read, is left and the sync exits 100; a side a filter hides is left, and so is what lies below a folder a filter
  hides on either side; a read-only side keeps its rows; a followed link that a mirror removes goes as the link, never what
  it leads to. `folder sync --session` of a folder-compare session needs a preset flag.
- **Update never removes.** A sync replaces only a row's own counterpart: a folder copied by *Update* does not replace a file
  of its name, nor `b.txt` a `B.txt` that a case-sensitive comparison keeps apart on a file system that folds.
- **A session's unknown keys are said.** Each one is a warning when the session runs, naming the nearest known key; in a
  folder-sync session one in `[specs]` or `[rules]` stops the run, as it may be what protects a side.

### Changed
- **`include_hidden_by_default` is now false.** Operations honour the filters unless a dialog or `--include-hidden` says
  otherwise. An explicit value in `options.toml` keeps working. It is a preference of folder operations only: a sync leaves
  a side a filter hides unless its own `--include-hidden` says otherwise.
- **A new copy no longer syncs to disk** (`fsync`). `vfs cp`, and every copy that creates its target, writes a file with its
  final time and mode under a temporary name and renames it, with two system calls fewer per file than before. Copying
  10,000 small files took 26 times a plain copy, and now takes about 1.4 to 1.5 times. A power cut just after a copy can
  lose files the copy created. It does not lose a file that existed: a copy that replaces still syncs the file before the
  rename, and a move syncs the file, its folder and the name of every folder it made before the source goes.
- **Full Refresh keeps the open folders and the selected row** of the Folder Compare view, found again by their paths; a
  row you select while it compares stays selected.

### Fixed
- **A path with `..` no longer hangs.** `vfs cp -r` and `vfs mv` of a folder to a target spelt with `..` (`../copy`,
  `a/../copy`) never ended and could not be cancelled; this goes back to 0.1.90. A `..` keeps the meaning the OS gives it
  through links, so a folder still cannot be copied below itself by spelling.
- **A copy never replaces a file that appears while it runs.** A new file takes its name by a rename that cannot replace,
  so a program that creates the target during a `vfs cp` keeps its file and the copy fails `already_exists`. This holds
  where the file system has that rename or hard links.
- **Windows: times can be set on a read-only file.** `vfs touch` of a file with `R`, and a folder operation that sets the
  time of one (Touch, or the time of the other side), failed `permission_denied`: the modification time was set through an
  open for writing, which `R` refuses. All times are now set through one handle that asks only to write attributes, on
  files, folders and links alike.

### Internal
- `ops::Plan` of kind `batch`, stamps, `exchange`, the fence of the base folders and backups are in place for the folder
  operations, with subtree refresh, contents on demand and `ArchiveVfs::extract_to_cache`.
- `open` 5.4.5 for the associated application; `hikasa-folder` gains `hikasa-formats`, for one regular expression policy.
- The window's dialogs of file operations are toolkit-free models, tested without a window; the commands
  `CopyRight`, `CopyLeft`, `Delete` and `PermanentDelete` are implemented in the catalogue.

## 0.1.91 (2026-11-01, pre-release)

### Added
- **Ctrl-C in the command line.** The first Ctrl-C cancels a running comparison or `vfs` operation: an archive being
  rewritten stays as it was, its temporary and lock files go, the file of `--output` keeps what it had, and the command
  exits 100. A second Ctrl-C exits 100 at once. Any other command ends at the first Ctrl-C, as before; SIGTERM too. A
  command started with Ctrl-C ignored, as `cmd &` in a script is, stays that way.
- **Named fonts.** The four font families of `theme.toml` and the Fonts page are looked up among the system's fonts, off
  the window's thread and only when a family is named. The built-in fonts stay behind them for the glyphs a font lacks,
  then the system's fallback, so icons and CJK keep working. A family that is not found keeps the built-in font, with one
  notice, and so does a font file that is broken: it is tried on a sample off the window's thread first. Listing and hex
  fonts load and wait for their views.
- **Writes inside archives.** `hikasa vfs cp`, `mv`, `rm`, `mkdir`, `touch` and `--output` work on members of zip and the
  tar family, nested ones too: `vfs cp -p notes.txt a.zip/docs/`, `vfs mv a.zip/old a.zip/new`. An archive is rewritten
  once per operation, to a temporary file next to it, which replaces it in one step. `vfs mkdir new.zip` makes an empty
  archive for each writable mask. Single gz, bz2, xz and zst streams stay read-only.
- **Archives.** zip, tar, `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst` and single gz, bz2, xz and zst streams work
  wherever a file does: `hikasa vfs ls a.zip`, `vfs cat a.zip/inner/x.txt`, `quick a.zip/x b/x`, `text compare`,
  `folder compare a.zip b/`, nested as `a.zip/b.tar.gz/c.txt`, and copies out with `vfs cp -r a.zip/ out/`. Masks and limits
  are in `archives.toml` (`hikasa help archives`).
- **Archives inside folder comparisons.** `--archives files|opened|folders` and `--open-archive <PATH>`; the default
  leaves results as they were. `--content crc` uses a zip's stored CRC without decoding.
- The window shows a zipper icon on archive rows; Open on a closed one opens it as a folder; two archives open a Folder
  Compare.

### Safety
- **A rewrite cannot lose an archive.** A read error, a full disk, Cancel or a failed rename leave it byte for byte as it
  was, and a move into an archive removes its sources only after the rewrite. An archive that changed since the
  operation was planned is left alone (`changed since the plan was made`). A member cannot be written over the archive
  that holds it, nor a file moved into itself.
- **Two writers cannot overlap.** A rewrite holds a lock next to the archive, so a second one, in any process on this
  machine, fails at once with `another operation is changing this archive`, which names the lock file, instead of losing
  the first one's change. It takes a file system with locks: on one without, and between machines on some network mounts,
  the check that the file did not change since the plan is all there is.
- **A rewrite keeps what it did not touch** as bytes, in order: encrypted members and methods it cannot decode, refused and
  link members, devices, sparse files, unknown extra fields, comments and raw names. A member hidden by a later one of the
  same name stays with it: it moves with it, also to another archive of any format, and goes when that member is removed
  or replaced, with a warning, so what was overwritten does not live on. A plain file holds one version, so a move out of
  the archive ends the older ones, and says so.
- **The temporary copy of an archive, and of any file Hikasa saves, is private** until it replaces the file. A kill during a
  rewrite leaves it next to the archive, and the next rewrite of that archive removes it.
- Archives are treated as hostile input: bad names are refused and listed, links are never followed or written to disk,
  damage keeps what lies before it, encrypted members are reported, and bombs stop at the limits (`entries_max`,
  `paths_max_mib`, `depth_max`, `stream_max_gib`, `cache_max_gib`, `ratio_max`, `nesting_max`).
- A member copied out never brings `setuid`, `setgid` or sticky bits, nor more than the umask gives.
- `--json` escapes DEL, C1 controls and bidirectional controls, so a hostile name cannot act on a terminal.

### Data safety
- **A settings file this version cannot read is never overwritten from the Options dialog.** A newer `schema_version` or
  broken TOML used to show defaults, and Overwrite replaced the file with them. The page now says why, the prompt holds
  only Reload and Cancel, and `settings_save` refuses such a file even with `force` (new outcome `Unreadable`). One such
  file also no longer turns the other settings files into defaults at start
  (`Service::settings_snapshot_partial`).
- **Overwrite keeps the unknown keys on disk.** It read them when the dialog opened, so keys added since were lost; it
  now reads the file again first.
- **The window's close button asks** when the Options dialog has unapplied edits, as Esc does.
- **`hikasa settings unset` removes a key kept without being known**, such as a view or command of a newer version, which
  it used to answer `not_found`; `settings get` of a key inside a kept command override shows its value.

### Fixed
- First run: no more "File formats reloaded" notices for the program's own format files.
- Writes to a zip on Windows keep the Unix mode of a member, and the same input gives the same archive on every OS.
- Folder views: a large folder font widens the size and time columns, which ran over each other from about size 16.
- Home follows a session deleted or changed by the command line at the next 2 s poll, without a focus change.
- Options search: the page follows the search, and no page shows when nothing matches.
- Dialogs: buttons sit at the bottom of the wizards, lists take the free height, and a dialog moves by its title bar.
- A notice raised while a dialog was open could not be clicked if an earlier notice was on screen, and the help of a
  dialog (F1) could be neither scrolled nor closed: both now stand above the dialog.
- Menu marks sit on the line of their label; the Menu column of the shortcuts page is straight; the log panel says
  "1 entry"; the help of Text Compare puts the thumbnail on the left.

## 0.1.90 (2026-10-31, pre-release)

A pre-release of 0.2.0 with the first part of M2: waves W0–W4 and the packages. 0.2.0 stays reserved for M2 complete;
W5–W14 are not in this version.

### Windows and macOS port of M2
- **Session and workspace lookup:** on a case-insensitive file system (Windows, macOS) `SessionStore::find` and
  `WorkspaceStore::find` answered with the case the user typed; they answer with the name on disk.
- **`LocalVfs` on Windows, as the contract says on every platform:** a path below a file is `not_a_directory` (it was
  `not_found`); `rename` with `replace` replaces an empty folder and a link to a folder, which Windows refuses
  to rename onto (the entry is removed first and put back if the rename fails, so that step is not atomic there);
  `create_symlink` stores `\` for `/` in the target, because a Windows link holding `/` resolves to nothing.
- **Tests:** platform-aware paths and golden normalisation; scenarios that need a read-only folder or a `..` in a spec
  are Unix only, each with its reason.

M2 is planned in `docs/design/m2-plan.md` (the maintainer's answers in §4.1) and specified wave by wave in
`docs/design/m2-contracts.md`. Entries per wave, newest first.

### Brand and packaging
- **Brand:** a logo for light and dark backgrounds (`assets/brand/`), with generated icons; the window now shows it.
- **Packages** (`packaging/`, local scripts, `packaging/README.md`): Linux `.tar.gz`, `.deb`, AppImage and an Arch
  `PKGBUILD`, Windows portable zips and MSI installers for x64 and x86, with `SHA256SUMS` and smoke tests. Every package
  carries `THIRD_PARTY_LICENSES.txt`, generated from the crates compiled into it. Desktop entry, AppStream metainfo and
  the `application/x-hikasa-pkg` MIME type for `.hikasa-pkg` files.
- **Windows:** `hikasa-gui.exe` is a windows-subsystem program in release builds and no longer opens a console;
  external conversions started by the GUI do not open console windows either.

### Wave 4c — Settings dialogs
- **Options dialog** (OPT pages, D-128): Tools ▸ Options…: a searchable page tree (title, label, key path, OPT id),
  every P0 key of `options.toml` on exactly one page, Archive Types, Open With and Name Filter Presets; live checks with
  each file's schema, Apply / OK / Cancel; only changed files are written, sparsely, as `hikasa settings set` writes
  them, keeping unknown and hidden keys. A file changed on disk meanwhile asks Overwrite / Reload / Cancel, and a notice
  offers Reload or Keep my edits as soon as it is seen.
- **Toolbars, Shortcuts, Menus** (UI-003, OPT-011, D-129): per view type: Menu and Toolbar check boxes, shortcuts by
  capture (Add, Remove, Default), display filter mode, large buttons, show-all flags, Reset View; conflicts shown live,
  never blocking; overrides of commands not implemented yet are kept. "Toolbar Options…" opens it from the toolbar's
  right-click menu.
- **Theme editor** (UI-020…023, FC-031, D-130): theme mode, the light and dark scheme, every element of `[folder]`,
  `[file]` (syntax elements and the fallback cycle) and `[picture]`, the flags and the four fonts, with Reset, New, Clone,
  Delete and Reset All; every change shows at once in every window, Cancel restores; contrast warnings below 4.5:1.
- **Wizards** (SES-060…062, D-131): Export, Import (per-item actions, locked sessions, delete choices, a summary of the
  exact choices) and Restore Factory Defaults, over the same calls as `hikasa settings`.
- **Packages on the command line** (D-127): `hikasa-gui x.hikasa-pkg` and `hikasa gui open x.hikasa-pkg` show the import
  wizard (new IPC request `ImportPackage`; forwarded to a running GUI; exit 0); `--silent` still imports without a window.
- **Core:** `CommandOverrides::{set_shortcuts, set_in_menu, set_on_toolbar, view_mut, reset_view, prune_view}`,
  `settings::form` (one key of a typed schema as a form field), scheme elements by name, `theme::selected_scheme`,
  `State::{dialog_text, set_dialog_text}`, `Service::gui_import_package`. `ProgramOptions`, `Export`, `Import` and
  `Restore` are implemented.
- **Dialogs:** a drop while a dialog is open no longer reaches the view under it; text fields stand out from the
  dialog's background.
- **Review fixes.** `commands.toml`: a view or command id this build does not know, and the override of a planned
  command with an unreadable shortcut, are kept in the file with a warning, in every load mode and through `settings
  set` (they used to be errors or dropped). The Options dialog checks only the files it would write; OK closes only when
  every file of the run succeeded; a failed read-back after the writes no longer reads as a failed save
  (`SettingsSaveDto::reread_error`); resetting the last element of a built-in keeps a file with unknown keys; Esc leaves
  open popups first and asks before discarding; Apply takes or refuses an open Open With item, whose shortcut is a capture
  field. The Import wizard shows its package on every step and never ignores a second one; wizard paths must be
  absolute. The OS close button and Exit wait for a running import, restore or Apply, and an exit nobody can refuse
  waits for the write. `text_scroll` and `two_windows` report the median of three runs after idle frames.

### Wave 4b — Sessions in the window
- **Sessions of views** (SES-001…006, D-120, D-121): every comparison view has a session: Save, Save As, Locked,
  close prompts ("Save changes to session?", "Save changes to locked session?") on every close path (tab, window,
  exit, Go to Home, the Sessions drop-down, Load Workspace; Cancel stops them), auto-save on close, at exit and every
  300 s while the view changes. `hikasa gui close` and `gui quit` never ask. A pair opened from a saved folder session
  is its child (TC-052).
- **Session Settings dialog** (SES-020…022, D-122): the Specs tab (inputs with recent paths, description, format,
  encodings), the scope selector and the tab registry the rule tabs of later waves join.
- **Home view** (SES-030, SES-031, SES-093, D-124): type buttons as drop targets, search, the sessions tree (New,
  folders, favorites, four auto-saved groups) with its context menu (13 new `Tree*` catalogue commands) and the
  management panel. `HomeCompare` accepts `clipboard://`.
- **Workspaces** (SES-040…042, D-125, D-126): Save Workspace As, Load Workspace, Manage Workspaces (rename, delete,
  shortcuts with conflicts shown), `[startup] load_workspace` and `on_exit_save_workspace_as`, lazy restore: a tab
  starts when first activated, and `gui list` shows the pending ones.
- **Command line** (CLI-002, SES-042, D-127): `hikasa-gui <session>` and `hikasa-gui <workspace>`, forwarded as
  `LoadSession`/`LoadWorkspace` to a running GUI.
- **Keeping changes safe:** Yes at a close holds the tab until the save returned; a failed save keeps it open with the
  reason and stops the close, Exit included. Renames, moves and deletes in Home reach open views and pending tabs; a
  Save refuses a file that is no longer the session the view loaded. Auto-save keeps a parent's overrides, works in an
  idle or minimized window, and asks at the close when its file is locked.
- **Locked sessions in folders** (D-91): `hikasa session rmdir --recursive` takes `--force`, and refuses a folder
  holding locked sessions without it; deleting or renaming such a folder in Home names them first.
- **Service calls** (§1.4.15): `session_load`, `session_load_autosaved`, `session_defaults`, `session_save`,
  `session_save_defaults`, `session_autosave`, `session_delete_autosaved`, `session_for_view`, `session_view_of`,
  `session_resolve_parent`, `session_apply_scope`, `session_rename_folder`, `session_folder_contents`, `session_prune_autosaved`, `workspace_load`, `inline_table`.

### Wave 4a — GUI shell
- **Windows and tabs** (UI-001, SES-090, SES-091, D-106…D-108): several windows in one process (Session ▸ New
  Window); the tab context menu (New Tab, Duplicate Tab, Move Tab to New Window, Move Tab to Window ▸, Close Other
  Tabs, Close Tab); tabs drag to reorder and, where the system reports window positions, into another window or
  out into a new one; the `[tabs]` options (where opens go, new tab placement, closing the last tab, hiding a single
  tab's bar, the warnings before closing several tabs or windows). `state.toml` remembers the last window's size
  and, per view type, `[layout.<view type>]` (toolbar, log panel and its height, font size, thumbnail).
- **Commands from core** (UI-002…UI-007, D-109…D-111): `hikasa_session::commands::effective` merges the catalogue
  with `commands.toml`; menus (grouped by section, View ▸ Display Filter), toolbars (in-house vector icons, large
  buttons, the display filter as a drop-down, favourites or toggles; right-click menu), hints with the shortcut
  and their timing options, and shortcut dispatch by focus area all follow it. New catalogue commands
  `TabDuplicate`, `TabMoveToNewWindow`, `TabMoveToWindow`, `TabCloseOthers` and a `toolbar` flag; `WindowNew`,
  `ShowToolbar` and `ShowLog` are implemented.
- **Dialogs** (D-112): modal inside their window (Esc, Enter, F1); prompts with "Do not ask again"; About is a
  dialog.
- **Theme** (UI-020…022, FC-031, D-113): the colours come from the scheme of `theme.toml` (light/dark, following
  the system), re-applied when a theme file changes; the editor and folder font sizes apply.
- **Status bar, jobs, messages, log** (UI-030, FC-100, D-115, D-116): state items per view, a job indicator with
  Cancel per view, a message line per view, and one log book per process (OPT-018 filter, capacity, optional
  file) shown by the folder views' Log panel.
- **Changes on disk** (UI-031, SES-053, D-117…D-119): a watch thread polls the settings files every 2 s and on
  focus; changed options and format files reload the service's settings (`Service::reload_settings`; later
  comparisons use them), `commands.toml` and theme files re-apply, each with a notice; the inputs of open text
  views are watched (reload automatically or offer Reload; a removed input is reported).
- **Drag and drop** (UI-008): files dropped on a Home path field fill it; two dropped on a window are compared.
- Core: `Service::settings_snapshot`, `settings_save`, `reload_settings`, `state_update`, `log_append`,
  `input_stamp`; `Service::options()` and `registry()` return snapshots (`Arc`); `State::layout`;
  `hikasa_session::log` and `hikasa_session::watch`. Performance mode `HIKASA_GUI_BENCH=two_windows`.
  `Service::loaded_settings()` (options, ignored keys and registry as one snapshot; reloads are serialized);
  `Service::ignored_option_keys()` returns a `Vec<String>` refreshed by each reload; the spooled copy of standard
  input is not watched; a refining catalogue entry without a shortcut takes the shared one.

### Wave 3 — VFS mutations, clipboard, generic copy
- **Mutating VFS** (VFS-001, ARCH-030, D-92…D-99): every back-end gains single-item primitives — create a folder,
  remove, rename, set times, set attributes, create a symbolic link, move to the recycle bin, an optional fast
  copy — each behind a new capability (`mkdir`, `remove`, `rename`, `set_attrs`, `trash`, `create_symlinks`,
  `set_created`; `set_times` is now used). Writes never follow the final symbolic link; new error kind
  `not_empty`.
- **Generic copy, move, remove** (ARCH-031, D-95…D-98, D-103): `hikasa_vfs::ops` plans every operation completely
  (whole trees, in execution order) before the first write, then executes it item by item with per-item results
  (`planned`, `done`, `skipped`, `failed`, `not_run`), re-checking each target. Copies are atomic per file (local
  files up to 64 MiB in one call, `copy_file_range` on Linux), keep times and permissions, copy links as links and follow overwrite
  policies `fail` (default), `skip`, `always`, `newer`; moves rename where they can, else copy and remove per item
  with rollback; the recycle bin never falls back to a permanent delete. Review fixes: `.`/`..` names and roots
  (also through `..`) are never operated on; a failed replacement (a link or folder over a file) leaves the old
  item; nothing runs below a folder that failed or was swapped for a link after planning (`not_run`), and
  removals re-check folder identity; `newer` is re-checked at execution; a move keeps its copy when the source
  vanished; case-only `mv` renames; local copies of read-only files work; planning reports `scanning` progress
  and can be cancelled; a link on another file system than the recycle bin is refused.
- **`hikasa vfs cp|mv|rm|mkdir|touch`** (D-102): `--dry-run` prints the plan; `rm` needs `--yes`; schema
  `hikasa.vfs_op.v1` (35 schemas); failed items keep `ok: true` and exit 100.
- **Clipboard** (TC-002, TC-065 data, D-100, D-101): `clipboard://` reads the clipboard's text, or an image as an
  in-house PNG named `clipboard.png`, once per command and only when given; `HIKASA_CLIPBOARD_FILE` stands in for
  it (empty → `not_found`, above 1 GiB → `too_large`). `gui open clipboard://` and `--gui` now send it to the
  window, which reads its own clipboard. Writing to `-` or `clipboard://` is `unsupported`.
- New dependencies `trash` (recycle bin, `hikasa-vfs`) and `arboard` (clipboard, `hikasa-session`), each confined
  to one file by the architecture test.

### Wave 2 — sessions, workspaces, settings packages
- **Sessions** (SES-001…006, SES-008, SES-010, D-80…D-84): one TOML file per session (`sessions/<folder>/<name>.toml`,
  defaults per type in `sessions/_defaults/`, auto-saved views in `sessions/_autosaved/`), read and written by the
  settings engine. Rules are stored sparse against the type's defaults, so a later change of the defaults reaches
  every session that did not change that rule; Folder and Text Compare rules are typed (the TOML of
  `--rules-file`), the other types keep theirs as they are until their waves. Names are portable (D-80) and unique
  per folder in any case; `-` and `clipboard://` are never stored (warning `spec_not_saved`, a new code).
  Auto-save rotation with day groups and the `max_auto_saved_sessions` limit, the scope table of the session
  settings dialog (view, defaults, these files / all files within the parent) and the close-prompt table are pure
  core functions with table tests.
- **`hikasa session list|show|new|edit|delete|rename|lock|unlock|mkdir|rmdir|run`**: `session run` runs a Folder or
  Text Compare session through the same service call and output as `folder compare` / `text compare` (same JSON,
  same exit codes); schemas `hikasa.session_list.v1`, `hikasa.session_show.v1`, `hikasa.session_write.v1`.
- **Workspaces** (SES-040…042, D-85): `workspaces/<name>.toml` with windows, tabs (a named session or an inline
  one) and layouts; `hikasa workspace list|show|delete|rename` (`hikasa.workspace_list.v1`,
  `hikasa.workspace_show.v1`, `hikasa.workspace_write.v1`); a tab whose session is gone opens Home with warning
  `session_missing` (a new code).
- **Settings packages** (SES-060…062, D-86, D-87): `hikasa settings export` writes a `.hikasa-pkg` (a zip with
  `manifest.toml`; deterministic: same configuration and time, same bytes); `hikasa settings import` validates
  every entry (paths, sizes, schemas) before writing anything, then prints or executes the plan (`--dry-run`,
  `--yes`, `--silent`, `--delete-existing`); `settings restore-defaults --category` restores factory defaults by
  category. Schemas `hikasa.settings_export.v1`, `hikasa.settings_import.v1`. New dependency `zip` (pure-Rust
  deflate only).
- **The window opens sessions and workspaces over IPC** (ARCH-091, D-89): `LoadSession` and `LoadWorkspace` are
  answered by the window; `hikasa gui session <ref> [--wait]`, `hikasa gui workspace <name>`, and `hikasa gui
  open <name>` resolves one argument that is not an existing path as a session, then a workspace (CLI-002, D-88); a
  `.hikasa-pkg` with `--silent` is imported. A window of 0.1 is reported as "does not support sessions; restart it".
- **Explicit rules stay written** (D-90): a session file keeps every rule it already held, even one that now
  equals the defaults, so a later change of the defaults does not change an explicit choice; `hikasa session edit
  --unset <key>` removes a rule (also with `--defaults`). Unknown rule keys are kept. The folder size and date
  filters and the time offset can be written `false` ("off") in a session or a `--rules-file`.
- **Locked sessions are protected** (SES-004, D-91): `session delete` and `session rename` need `--force`;
  `settings import` does not replace or delete a locked session and `settings restore-defaults
  --delete-sessions` does not delete one unless `--force` (new flag on both; plan action `skip_locked`, new
  warning code `session_locked`); auto-save never writes a locked file.
- **Stricter package import**: every session's rules and text rules are built and their regular expressions
  compiled before anything is written; names are matched without regard to case (two entries differing only by
  case are refused; an existing file keeps its spelling); an unknown category in the manifest is refused.
- Help topics `hikasa session` and `hikasa workspace`; `hikasa settings` and `hikasa gui` extended.

### Wave 1 — settings store
- **One engine for every settings file** (D-70, T13): `options.toml`, `state.toml`, `commands.toml`,
  `theme.toml`, `themes/<name>.toml`, `filters.toml`, `openwith.toml` and `archives.toml` are typed
  (`hikasa_session::settings`), carry `schema_version` with a migration chain (older files are written back on
  the next save, newer ones refused), keep unknown keys, are written sparsely (only values that differ from the
  defaults) and atomically, and refuse to overwrite a file changed on disk since it was read (`--force`).
- **Every option of `docs/15`** (OPT-001…OPT-022, D-71): `options.toml` is read completely; a wrong type or a
  value out of range is an error naming the file and the key for every command except `hikasa settings`, which
  warns `invalid_config_value` (a new warning code) and uses the default so that `settings set` repairs the file.
  Entries of `[[app]]` and `[[preset]]` are never dropped or reordered by that repair.
- **`hikasa settings get|set|unset|list|restore-defaults`** (D-73, D-77) with keys `<file>.<path>`, suggestions
  for unknown keys, shortcut-conflict warnings, a `--dry-run` plan and `--yes` for deleting files; schemas
  `hikasa.settings_get.v1`, `hikasa.settings_set.v1`, `hikasa.settings_list.v1`, `hikasa.settings_restore.v1`.
- **`hikasa format new|edit|delete|import|export|enable|disable|move|restore`** (D-75): they write only the files
  they change; a built-in can be disabled or restored, not deleted; the id `order` is reserved; schemas `hikasa.format_write.v1`,
  `hikasa.format_export.v1`, `hikasa.format_restore.v1`.
- **Change detection** (D-74): `ConfigWatch` polls length, time and CRC-32 of every settings and format file.
- **Themes** (D-78): built-in `light` and `dark` schemes (the M1 palettes) with every element of UI-021…023,
  editable by override files, custom schemes with a `base`; contrast of default text ≥ 4.5:1 is tested.
- **Shortcuts, Open With, archive types, filter presets** in core: the shortcut spelling of the command
  reference (every catalogue shortcut parses), per-view customization and conflicts (UI-003); pure Open With
  command expansion with quoting (D-76); archive type masks (OPT-010); name filter presets (OPT-020).
- Help topic `hikasa settings`; `hikasa format` extended. The window takes the sections of its view types from
  `hikasa_session::commands::view_sections`.

### Wave 0 — infrastructure
- **Command catalogue in core** (D-65): every command of the reference (458 in 14 sections) is data in
  `core/hikasa-session/catalogue/commands.toml`, read by `hikasa_session::commands`, with a status
  `implemented` (the 60 the window executes) or `planned`. `docs/reference/commands.md` is generated from it by
  `cargo xtask commands-doc` and gains a Status column; without it the file is the M1 one (D-66).
- **Help topics** (D-67): 17 Markdown topics in `help/` (contents, command-line basics, exit codes, JSON output,
  one per subcommand, the main window and the three views). `hikasa help [topic…]`, `hikasa help --list`, schema
  `hikasa.help.v1` (appended to `hikasa version --json`); an unknown topic exits 100 with suggestions.
- **The window**: Help ▸ Contents and Help ▸ Context Sensitive Help (F1) open the topics in a resizable panel on
  the right; links open other topics; Esc closes it. Tests check that the window executes exactly the
  `implemented` catalogue entries, with the catalogue's shortcuts.
- **Tooling** (D-68, D-69): the `xtask` workspace member (`cargo xtask commands-doc [--check]`, `cargo xtask
  help-check`); CI runs `cargo nextest run` (latency tests one at a time, retried on CI) plus doc tests, and a
  `docs` job runs the two xtask checks; `bench.yml` runs the benches and the G1/G2 gates by hand.
- Decisions D-59…D-69 record the maintainer's answers (xz through the pure-Rust `lzma-rust2`; no `uzers`).

## 0.1.0 — 2026-10-26 (M1)

M1: the core engines, the `hikasa` command line and the GUI prototype (`docs/16-roadmap.md`). Entries per wave,
newest first.

### Windows and macOS port fixes
- Opening a folder as a file is `is_a_directory` on Windows too (`LocalVfs::open_read`, `--log`; Windows reports
  it as "access denied"), and the mapped binary compare and CRC fall back for a folder instead of failing.
- macOS (and every Unix without `/proc`): the effective uid is found also when several threads capture the host
  facts at once; a shared probe file name made all but one report the uid as unknown, and the IPC socket folder
  check then refused the endpoint (`InsecureEndpoint`).
- A path flag given twice (`--config-dir cfg` and `--config-dir <cwd>/cfg`) is the same path also when it does not
  exist yet behind a symbolic link (macOS `/var` → `/private/var`).
- IPC on Windows: a client's connect to a busy pipe waits at most the call's timeout (`IpcError::Timeout`, exit
  100) instead of forever, and a server shut down right after it started no longer leaves its accept thread
  blocked (the self-connect stays open until the accept thread ends; contract §1.8.7 item 13).
- The repository checks files out byte for byte (`.gitattributes`: `* -text`), so goldens with CRLF or LF line
  endings stay exact on Windows; CI runs `cargo test --workspace --no-fail-fast`. CLI goldens mask the
  platform's file metadata, case sensitivity and capability lists outside Linux (contract §6.1).

### M1 final review fixes
- `hikasa-gui` binds its IPC endpoint and opens the window before it writes the first-run format files
  (`FormatStore::materialize`, D-21); the files are written on a background thread after the first frame, so
  disk flushes on a slow or busy disk can no longer delay the window or make `gui open --wait` miss its launch
  budget (unit tests with a blocking fake task and for the startup path; `tests/cli_gui.rs` checks that the
  files appear after the launch).
- Window tests run on a virtual X display only: `env -u WAYLAND_DISPLAY xvfb-run -a …` (`docs/17` §3.5);
  `tests/cli_gui.rs` and `tests/perf_gate.rs` remove `WAYLAND_DISPLAY` from the processes they start whenever
  `DISPLAY` is set. `docs/02` §2: the long non-ASCII scroll run was on the real display, not under Xvfb.
- CI no longer installs GTK/WebKit packages (eframe needs none to build); the `--gui` help text no longer shows
  an internal decision id; README, `docs/00` and `docs/16` describe the implemented state and the open release
  items; crate descriptions say what each crate contains in M1; contract §0.4 rule 3 (`anyhow` only in the
  GUI binary) and §6.4, `docs/02` and `docs/17` (CI bench job, `cargo xtask commands-doc` and nextest are M2
  items); `docs/12` on `quick --size` and the folder report defaults.

### Wave 6b — the GUI on egui
- `hikasa-gui` moved from Dioxus 0.7 to **egui 0.37 with eframe** (D-56 as amended, ARCH-GUI-001): the Dioxus
  prototype met the first screen and the frame rate but not the ARCH-GUI-003 memory limit (598 MiB PSS, 456 MiB
  GUI RSS), so the maintainer moved the whole GUI. The view models, the command catalogue, the IPC handler and
  single instance, the D-23 close codes, Home's rules and FC-062 are unchanged; the page, its script and styles
  (`assets/`) and the page ↔ Rust request channel are gone.
- One native window, repainted on demand (the mailbox wakes it with `request_repaint`; no polling thread): menu
  bar, tabs, toolbar (with the display-filter and folder-mode lists, the toggles and Ignore unimportant) and
  status bar generated from the catalogue; light and dark colours follow the system. Keyboard shortcuts are
  taken at the window root, the focus-bound keys (Enter, Left, Right, arrows, pages) by the folder tree while it
  has the focus; Ctrl with `+`/`=`, `-` and `0` stand for the keypad keys of the font-size commands.
- *Text Compare*: the panes are custom-painted for the visible rows only (line numbers, important, unimportant
  and orphan backgrounds, hatched absent sides, inline highlights, syntax colours, gap bars that expand on a
  click, the cursor row), with a thumbnail (click or drag to jump), a scroll bar, wheel and keyboard scrolling;
  the scroll position is a row index, so million-row views scroll exactly. *Folder Compare*: the tree is painted
  the same way (colour spots, names, sizes, times, centre marks with tooltips, disclosure triangles, selection,
  hover), updates while scanning, and opens a pair with Enter or a double-click.
- ARCH-GUI-003 re-measured 2026-10-26 (desktop, Radeon RX 7900 XTX on Vulkan, 120 Hz display; `docs/02` §2): first
  screen 0.51 s after the process start (coloured 0.70 s); 119.6–119.9 fps in all three scroll phases (UI work
  per frame ≤ 5 ms, p95 ≤ 3.4 ms); memory 275 MiB PSS / 281 MiB RSS after scrolling (peak ≈ 365 MiB) against
  381.5 MiB — **all three items met** on this machine (not yet on a 2020-class laptop). An empty window takes
  76 MiB PSS (Dioxus: 168). Folder scan with every folder open: 100,020 files per side in 0.21 s with UI work
  ≤ 3.3 ms per frame; 400,080 files with a warm cache ≤ 11.8 ms per frame.
- Tests: 39 unit tests (the mailbox's notifier, egui key → catalogue shortcut, UTF-16 columns → layout
  sections replace the Dioxus request-query and ticker tests); the window test runs under Xvfb (with
  `env -u WAYLAND_DISPLAY`, see the final review fixes); the benchmark
  summary now reports per phase the frame work, the frame intervals and eframe's CPU time per frame.
- Review fixes (wave 6b verification):
  - The benchmark reads PSS and RSS from one `smaps_rollup` snapshot and only the peak from `VmHWM` (the RSS came
    from the approximate `VmRSS`, and the unit test that compared it with the PSS failed now and then); memory
    re-measured on the same desktop: 275 MiB PSS / 281 MiB RSS, peak ≈ 365 MiB (`docs/02` §2).
  - Text panes: the UTF-16 column table of a row is built in one pass, so long non-ASCII lines no longer cost
    quadratic time per frame (a 4,096-column regression test).
  - `hikasa-text` gains `TextComparison::next_section_from` and `TextDisplayFilter::toggles`/`toggled` (additive,
    contract §1.5.7); the GUI no longer re-derives the importance rule of section navigation or the filter →
    toggles table. Toggling a category in *Show Context* now keeps the context rows (only switching *Same* off
    removes them); *Same* shows as checked there.
  - Names with control or bidirectional characters are escaped in tab captions (`escape_name`, as `gui list`
    prints them), pane headers, the folder base strip and error strips (`escape_controls`). The text panes show
    C1 controls, bidirectional formatting characters, zero-width characters, LS, PS and BOM as `\u{…}` escapes.
  - A job wakes the window after its body returned and the window repaints at once until the result is
    published, so a finished comparison no longer waits for the 100 ms poll.
  - Closed views release their data (a unit test); the memory a long-running window keeps after many
    open/close cycles is glibc's retention of freed heap, bounded and documented in `docs/02` §2.
  - README, `docs/02`, `docs/05` and the contract no longer name Dioxus as the toolkit or an approved crate.

### Wave 6 — the GUI prototype (Dioxus, replaced in wave 6b)
- `hikasa-gui`: the desktop window on Dioxus 0.7 (D-56, ARCH-GUI-001) over the session service (contract §1.10,
  §3.7); it contains no comparison logic (ARCH-L3, architecture check 7). One window with tabs:
  - *Home*: two path fields and Compare, resolved like the command line (`resolve_open`, CLI-002) on a job
    thread; the paths are passed verbatim (spaces are legal in names), and `-` and `clipboard://` are refused
    with an inline message (the window never reads standard input).
  - *Folder Compare*: a virtualised tree (only the rows on screen exist in the page) with per-side colour spots
    and names (FC-031, D-55), size and time columns, centre state marks, open/close folders, Expand All and
    Collapse All, the 13 display filter presets, the six toggles and the three folder modes (FC-055, FC-056),
    Ignore Unimportant Differences and Suppress Filters (FC-057), next/previous difference (FC-061), Stop, Full
    Refresh, and Open (Enter or double-click) of a file pair in a Text Compare tab with the exact paths of both
    sides (FC-062). The tree is rebuilt from the live model while the scan runs (at most every 66 ms).
  - *Text Compare*: side-by-side panes drawn on a canvas (ARCH-GUI-002): the page asks for the slice of rows it
    is about to draw and the Rust side lays out only that slice (line numbers, important, unimportant and orphan
    colours, inline highlights, syntax colours per grammar element, gap bars that expand on click); a
    thumbnail (TC-032), next/previous difference section (TC-050), display filters with Show Context and the
    toggles (TC-040), Ignore Unimportant Differences, font size, Reload. The documents appear unaligned as soon
    as they are loaded (`CompareObserver::loaded`), the aligned comparison replaces them.
- Command catalogue (ARCH-070, UI-003): the ids of `docs/reference/commands.md` (plus `HomeCompare`), captions,
  hints, default shortcuts, menu and toolbar placement, radio groups and availability predicates; the menu bar,
  the toolbar and the keyboard are generated from it.
- IPC server and single instance (ARCH-090…092, §4.4): `OpenView` (text and folder views, `--wait`),
  `ListViews`, `CloseView`, `Ping`, `Shutdown`; a closed view reports its D-23 code through
  `ViewOutcome::close`; quitting (IPC, Exit, the window's close button) closes every view with reason
  `shutdown` and its real code, then shuts the server down; a second `hikasa-gui a b` forwards its request to
  the running instance and exits 0 (`--solo` runs alone).
- Tests: 32 headless unit tests drive the view models through the catalogue (the enabled state of every
  command, navigation, filters, slices, FC-062, Home, the D-23 outcomes) and the IPC handler with a real
  in-process server and client; `tests/cli_gui.rs` (ignored: it opens a window) runs the real `hikasa` against
  the real `hikasa-gui` (`gui open --wait` codes of a text and a folder view, `gui ping/list/close/quit`,
  second-instance forwarding); `tests/perf_gate.rs` (ignored) measures ARCH-GUI-003 and the folder scan through
  the scripted benchmark mode (`HIKASA_GUI_BENCH=scroll|folder`).
- ARCH-GUI-003 measured 2026-10-26 on a desktop (Ryzen 7 7800X3D, WebKitGTK 2.54, 35 visible rows), not on the
  2020-class laptop the requirement names (`docs/02` §2 "M1 wave 6"): first screen 0.43 s after the process
  start (aligned and coloured 0.60 s); scrolling a 1,000,000-line comparison at 60.2 fps in all three phases, at
  the page's 60 Hz frame-clock ceiling; memory 598 MiB PSS for the GUI and its WebKit processes and 456 MiB RSS
  for the GUI process alone, against 400 MB = 381.5 MiB, so the memory limit is **not met** (an empty Home view
  already takes 168 MiB PSS). Reported to the maintainer; the toolkit is not switched (17-agent-workflow §7.1).
  Folder scan with CRC and *Show All*, every folder opened: 100,020 files per side in 0.26 s (UI-thread work
  ≤ 2.5 ms), 400,080 in 0.68 s with 7 rebuilds while scanning (≤ 8.9 ms), 1,000,200 in 1.64 s with 18 rebuilds
  (up to 21 ms with 1.1 million rows open, so frames are missed at that size).
- Wave-6 review fixes: Home no longer blocks the window on `-` (it read standard input on the UI thread);
  Stop while a folder comparison prepares stops it (no scan starts); the folder-view verdict and close code come
  from the core (`ViewOutcome::for_folder`, `PreparedFolderCompare::view_verdict`); a view whose `OpenView`
  arrives while quitting reports `shutdown` with 100 (`ViewOutcome::close_unopened`); the Text Compare status bar
  is visible again and the canvases are sized in whole device pixels (no per-frame rescaling); plain Enter, Left
  and Right reach focused buttons, lists and check boxes (the tree takes them while it has the focus); the
  next/previous-difference state is memoised per rebuild and selection; `gui list` shows the tab caption from the
  start; two empty files say so; the benchmark reports PSS and RSS in MiB against 381.5 MiB and the page frames
  while scanning; architecture check 7 no longer mistakes `&'static str` for a `static` item (contract §1.8.7
  item 37); `docs/reference/commands.md` lists `HomeCompare` and the Enter/Right/Left defaults.
- `docs/02-tech-stack.md`: the D-56 approval and the ARCH-GUI-003 measurements.

### Wave 5 — the `hikasa` command line
- `hikasa-cli`: the `hikasa` binary over the session service (contract §1.9): `quick`, `text compare`,
  `folder compare`, `format list|show|detect|tokens`, `vfs ls|stat|cat`, `gui open|ping|list|close|quit`,
  `--gui` on the compare commands, `completions` (bash, elvish, fish, powershell, zsh) and `version`; the global
  flags `--json`, `--quiet`, `--log`, `--verbose`, `--config-dir`, `--no-color`.
- Exit codes per CLI-004 and contract §2.21: usage errors and caught panics exit 100 (never 2 or 101), a broken
  pipe keeps the computed code; `gui open --wait` returns the view's code, 102 when it cannot wait, 103 without
  a GUI executable (D-01, D-02, D-03, D-22, D-23, D-35).
- One JSON document per invocation on stdout (also on error, `hikasa.usage_error.v1` for a command line that does
  not parse); progress and logs on stderr; `--output` reports written atomically (D-29, D-32, D-40).
- Vocabulary words are parsed by the core crates only; `gui open` uses the shared legacy parser (D-51, D-57).
- Integration tests for every subcommand with goldens for every JSON schema. The launch and wait paths run in
  the `cli_gui_launch` test target (`harness = false`), whose own binary is the scripted GUI when
  `HIKASA_TEST_FAKE_GUI=1`; no test GUI is built or installed as a program.
- Gates G1 and G2 through the binary (`cargo test --release -p hikasa-cli --test perf_gates -- --ignored`;
  not run in CI). Measured 2026-10-25 (Ryzen 7 7800X3D, release): G1 (2 × 1M lines, `--report summary`)
  0.43 s auto, 0.45 s standard, 0.36 s identical, peak RSS 298 MiB (bound 427 MiB; threshold 3 s); G2 (100k files
  per side, `--content none`) 0.05 s warm (threshold 1.5 s) and 0.694 s cold with dropped caches, measured
  2026-10-26 on btrfs over NVMe (threshold 5 s; `docs/12` §4).
- Wave-5 review fixes: `gui open -- -x a.c` reaches the shared parser with its `--` (D-51); `--json` counts
  only before `--`, a legacy `-json` selects the usage-error document when the `gui open` arguments do not
  parse, and that document keeps clap's whole message; `--version` prints `hikasa <version>` after a subcommand
  too (with `--json` the version document); a global value flag repeated with different values at different
  positions exits 100; `quick --crc|--binary` on an unreadable file reports the error on stderr and with
  `ok: false` in JSON; one error line per failed stdout write; the completions golden checks the field order.
- `hikasa-session`: a GUI executable that exists but cannot be started exits 103 (`ApiError::GuiNotStartable`,
  was 100 "ipc I/O"); `find_gui_exe` resolves a relative `HIKASA_GUI_EXE` against the current folder and skips
  candidates without an execute bit.
- `hikasa-vfs`: writing to a symbolic link replaces the file it points to and keeps the link; a FIFO or a
  device (`/dev/null`) is written to directly and never replaced (`--output`; contract §1.2.7 item 12).
- `docs/12-cli-and-scripting.md`: the M1 flag set, the wave-5 decisions and the measured gates; contract
  §1.9.3 (accepted additions and deviations of wave 5), §1.9.2 and §6.1 amended; `docs/11` §1 local writes.

### Wave 4 — reports and the session service
- `hikasa-report`: text reports (the annotated unified diff, normal, context and unified patches, side-by-side
  text and self-contained HTML, the summary, the statistics CSV) and folder reports (status list, side-by-side
  text and HTML, summary, the `hikasa.folder_compare.v1` JSON document); patches are byte-exact in every style
  (checked with GNU `patch` and `git apply`), with BOM and encoding warnings, escaping, grapheme-aware column
  widths and RPT-004 headers.
- `hikasa-session`: host capture, configuration and cache folders, options, `FormatStore` (built-ins, user
  formats, `order.toml`), the service API shared by the CLI and the GUI (requests, jobs, the panic boundary),
  IPC protocol v1 with framing, client and server, one endpoint per user and configuration, and the
  architecture test (checks 1–7).
- Review fixes (78 findings from break, conformance and consumer reviews; 71 fixed with tests, 7 rejected with
  reasons, contract §1.8.7): the IPC client verifies the socket folder's owner and permissions, dropped view
  waiters close as `abandoned`, shutdown releases waiters, writes never block the GUI, per-call deadlines,
  bounded frames and queues; `--config-dir` and the endpoint override reach a launched GUI (`HIKASA_GUI_EXE`
  for tests); a private, symlink-safe standard-input spool; pool panics never abort; the effective
  ignore-unimportant setting reaches folder JSON and reports; adapter API for the request ↔ view mapping, lazy
  standard input, vocabulary parsers, the D-23 close rules and escaping helpers.
- `hikasa-core`: `WarningCode::PatchEncoding`. Workspace at the end of the wave: 983 tests.

### Wave 3 — text and folder comparison
- `hikasa-text`: `TextDocument` (chunked, cancellable decoding; encodings, line endings, size limits), text
  rules, the compare engine (an identity fast path equal to the general path for every grammar, bounded Auto
  alignment, closeness pairing with work budgets), importance classification of differences, sections,
  statistics, display filters, the thumbnail, a progressive observer, and quick compare by rules that agrees
  with the full comparison (external conversions on every path).
- `hikasa-folder`: folder rules and criteria, a parallel cancellable scanner with name-only prefilter and
  re-stat, name alignment, the live `FolderModel` with O(1) counters, states and aggregates, display and folder
  display filters, flatten mode, symbolic link loop handling, content tests within a byte budget through
  injected comparators, and verdicts.
- Adversarial and conformance reviews per crate; accepted deviations in contract §1.5.7 and §1.6.8. Gates G1,
  G2 (warm), G4 and G5 pass. Workspace at the end of the wave: 724 tests.

### Wave 2 — file access, alignment and file formats
- `hikasa-vfs`: entry model, the `Vfs` trait, `LocalVfs`, `MemoryVfs` with fault injection, standard-input and
  clipboard blobs, the router, name-only prefilter and re-stat, streaming and memory-mapped CRC32 and binary
  comparison with scoped parallel chunks (results independent of the thread count). The memory-map module is
  the only `unsafe` code in the libraries.
- `hikasa-diff`: interner, the Unaligned, Standard, Myers, Histogram and Patience alignment engines, a bounded
  Standard fallback (`alignment_degraded`), closeness matching with cell budgets, never-align, inline
  differences and helper-thread cancellation.
- `hikasa-formats`: the format model and its TOML form, the grammar tokenizer (linear on long lines,
  cancellable inside a line), regular-expression engine selection, the registry with ordering and shadowing,
  21 original presets, encoding and binary detection, Sorted and External conversions.
- `hikasa-core`: `WarningCode::AlignmentDegraded`. Every crate had an adversarial and a conformance review;
  accepted deviations are recorded in the contract. Workspace at the end of the wave: 519 tests.

### Wave 1 — `hikasa-core`
- Sides, spec parsing and canonical form (local, standard input, clipboard, remote and HTTP schemes), error
  kinds and classification, cancellation tokens, progress sinks with per-phase throttling, job ids,
  `FileTime` (strict RFC 3339), case folding, the file-mask engine and `NameFilter`, the content comparator
  contract, warnings and exit codes (ARCH-020, VFS-010, FC-050, FC-051, FC-021, CLI-004).
- The architecture test (checks 1, 2, 4, 5), property suites, golden tables and the `mask_match` benchmark.
  Review: 22 findings from two reviewers, 20 fixed with regression tests, 1 rejected per contract §1.1.3.

### Preparation
- Workspace dependencies, lints (`unsafe_code` denied, `missing_docs`), MSRV 1.97 and the CI workflow (fmt,
  clippy and tests on Linux, Windows and macOS, cargo-deny).
- `docs/design/m1-contracts.md`: the binding M1 API contract (crate APIs, normative algorithms, JSON and IPC
  shapes, concurrency and error model, test plan, wave acceptance criteria, decision register).

## 0.0.0 — 2026-10-19
- Initial specification and workspace skeleton. No functionality.
- Restructured workspace into three layers (`core/`, `cli/`, `gui/`); CLI-first rule:
  every function is a `hikasa` subcommand, the GUI is reached via `hikasa gui` (IPC).
