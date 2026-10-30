# Session Settings

Session ▸ Session Settings… edits the session of the active view; Session Settings on a row of the
[Home view](topic:view-home) edits a saved session or the defaults of a type.

## Specs

The left and right inputs (each with its recent paths), a description and, for file sessions, the format (*Detect*
or a [file format](topic:format)) and the two encodings (empty detects them; an unknown encoding is refused). The
defaults of a type hold no inputs, so those fields are off there.

OK with changed inputs reopens the view with them; a named session then counts as changed, and closing the view
asks whether to save. Cancel discards.

## Scope

Rule tabs arrive with the editing views. When one has changed something, the buttons at the bottom say where the
change goes:

- *This view only*: the view's own session.
- *Update session defaults*: the defaults of the type, for every later session.
- *These files within parent* and *All files within parent*: for a view opened from a saved folder session, a rule
  for this pair or for every pair of that type, stored in the folder session.

See [hikasa session](topic:session) for the same files from the command line, and [Contents](topic:index).
