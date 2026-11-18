# Grammar Item

One item of a text format's grammar, edited from the Grammar tab of the [format editor](topic:dialog-format-editor).

- **Element**: the name of what the item marks (*Comment*, *String*, *Keyword* …): type one, or pick one of the
  format's names. *Everything Else* is reserved for the text no item matches.
- **Category**:
  - *Basic*: a text found where the scan stands.
  - *Delimited*: from a start text to an end text (empty: the end of the line); *Stop at end of line* ends it there
    when the end text is missing; an *Escape character* skips the character after it.
  - *List*: whole words, one a line; the longest that matches wins.
  - *Columns*: from a column to a column, or *to end of line* (columns count from 1).
  - *Lines*: a heading line holding a text, and the next *N* lines; *or line 1* also starts it at the first line.
- **Match case**; **Regular expression** (the texts are regular expressions; not for Columns; the ▸ button beside a
  field is the [regex helper](topic:regex-helper)).
- **This element is case sensitive**: changes of letter case inside this element are differences. The setting is
  the element's, shared by all of its items.

OK checks the item in its place in the format; a refusal names it and the dialog stays open.

See [Contents](topic:index).
