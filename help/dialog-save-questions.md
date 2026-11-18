# Save questions

A save from a Text Compare view may need an answer before it writes, one question at a time, with the number of
questions left:

- **The file changed on disk** since it was read: Yes writes the whole text again (your edits win).
- **The file is read-only**: Yes writes it anyway.
- **Characters the encoding cannot write**: Yes writes them as `?`; **Save as UTF-8** changes the file's encoding to
  UTF-8 (one step you can undo) and saves.
- **Lines that were decoded with errors** were edited: Yes writes them as shown.
- **Save As onto an existing file**: Yes replaces it (with its backup).
- **The backup would replace a file** this file's saves did not make: Yes replaces it.

**Yes to All** and **No to All** answer the remaining questions; **Cancel** writes nothing, and the edits stay. The
command line has the same questions, answered by flags ([hikasa text convert](topic:text-convert)). See [The Text
Compare view](topic:view-text-compare).
