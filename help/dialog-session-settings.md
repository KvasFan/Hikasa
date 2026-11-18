# Session Settings

Session ▸ Session Settings… edits the session of the active view; Session Settings on a row of the
[Home view](topic:view-home) edits a saved session or the defaults of a type.

## Specs

The left and right inputs (each with its recent paths and **Browse…**, the [Browse](topic:dialog-browse) dialog), a
description, *Disable editing* per side (folder and text sessions; Text Edit: its file) and, for table, hex and
picture sessions, the format (*Detect* or a [file format](topic:format)) and the two encodings (empty detects them;
an unknown encoding is refused). A text session's format and encodings are on its Format tab (below). The defaults
of a type hold no inputs, so those fields are off there.

OK with changed inputs reopens the view with them; a named session then counts as changed, and closing the view
asks whether to save. A text view keeps its edits: a pane whose file or encoding changed is read again (asking
first when it has unsaved edits), and new rules or formats compare the documents again as they are. Cancel
discards.

## Picture tabs

A picture session has a **Picture** tab — the mode (tolerance, mismatch range or blend), the tolerance (0 to 255), the
blend percentage, *Ignore unimportant differences*, *Auto Scale*, the tone map of HDR and OpenEXR pictures and the
maximum megapixels — and a **Format** tab with each side's picture format (empty: by the name). The values are checked
as [hikasa picture compare](topic:picture-compare) checks them; a refused one is shown on the tab and turns OK off. OK
in a [Picture Compare view](topic:view-picture-compare) compares again without reading the files (a tone map, the
megapixels or a format reads them again).

## Scope

Folder and text sessions have rule tabs (below). When a rule tab has changed something, the buttons at the bottom say
where the change goes:

- *This view only*: the view's own session.
- *Update session defaults*: the defaults of the type, for every later session. Only what you changed in this dialog
  goes there; what the view holds on its own (an earlier *this view only* change, its Ignored items, its sync
  overrides) stays the view's. The same holds for the two choices below.
- *These files within parent* and *All files within parent*: for a view opened from a saved folder session, a rule
  for this pair or for every pair of that type, stored in the folder session.

See [hikasa session](topic:session) for the same files from the command line, and [Contents](topic:index).

## The tabs of a folder session

Folder Compare and Folder Sync sessions have these tabs after Specs (whose *Disable editing* keeps a side from being
written):

- **Comparison**: compare sizes; compare timestamps with a tolerance in seconds, *Ignore daylight saving difference*,
  *Ignore timezone differences*, and the hours a side's times are shifted by; compare file name case; the content test
  (none, CRC, binary, rules-based), *Skip if quick tests indicate files are the same*, *Override quick test results*.
- **Name Filters**: the files and folders to include and to exclude, one mask a line; *Clear*; the presets; *Add To
  Presets…*; the filter as the toolbar writes it.
- **Handling**: how names align (case, different extensions, Unicode normalization forms), which folders are read and
  opened, *Follow symbolic links*, *Ignore folder structure*, and how archives are treated.
- **Misc**: the file formats this comparison uses (**bold** where the session has its own setting; *Reset* gives it the
  global one back) and the Ignored items, each with *Remove*.
- **Sync** (Folder Sync): the operations preset, the action of each situation (an action the situation cannot take is
  greyed out; a change makes the preset *Custom*), how an item on one side only is read, *Sync empty folders*, *Sync
  within archives*, the recycle bin options, and the rows you set by hand, each with *Remove*.

Each change is checked as the comparison would take it: a refusal shows on its tab and OK stays off until it is fixed.
A tab that holds a refusal is marked `(!)` in the tab row, and OK's tip names it.
Settings these tabs do not show are kept as they are.

## The tabs of a text session

Text Compare sessions have three tabs after Specs; Text Edit sessions have the Format tab. Session ▸ Rules… opens the
dialog on Importance.

- **Alignment**: the engine (*Auto*, which names the engine it chose last; Standard, Unaligned, Myers, Patience,
  Histogram), the skew tolerance in lines (Standard), *Use closeness matching* and its threshold in ‰ (0–1,000),
  *Never align differences*.
- **Importance**: the *Unimportant text* list (a text or a [regular expression](topic:regex-helper), *Match case*;
  **Add**, **Edit**, **Remove**, **Move Up**, **Move Down**); every element of the panes' formats with *Important*
  and where its setting comes from: *format* (the format's default), *defaults* (the type's defaults) or *this
  session*. **Reset to format** gives an element its format's default back, also over a setting the defaults hold.
  Then the whitespace classes (leading, embedded, trailing), *Everything else* and its *Character case*, *Orphan
  lines are always important*, *Compare line endings*, and **Edit Grammar…** (the [format
  editor](topic:dialog-format-editor) of the format of the pane that has the focus).
- **Format**: the format of each side (*Detect*, or an enabled text format), or one for both (*Same format for both
  sides*), and each side's encoding (empty detects it). These are the view's own: they never go to the defaults.

Only the settings you change are written, as `hikasa session edit --set` writes them.
