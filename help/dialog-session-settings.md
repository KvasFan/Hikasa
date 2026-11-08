# Session Settings

Session ▸ Session Settings… edits the session of the active view; Session Settings on a row of the
[Home view](topic:view-home) edits a saved session or the defaults of a type.

## Specs

The left and right inputs (each with its recent paths and **Browse…**, the [Browse](topic:dialog-browse) dialog), a description and, for file sessions, the format (*Detect*
or a [file format](topic:format)) and the two encodings (empty detects them; an unknown encoding is refused). The
defaults of a type hold no inputs, so those fields are off there.

OK with changed inputs reopens the view with them; a named session then counts as changed, and closing the view
asks whether to save. Cancel discards.

## Scope

A folder session has rule tabs (below); the text views get theirs later. When a rule tab has changed something, the
buttons at the bottom say where the change goes:

- *This view only*: the view's own session.
- *Update session defaults*: the defaults of the type, for every later session.
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
Settings these tabs do not show are kept as they are.
