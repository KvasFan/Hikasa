# Table Compare Report

Session ▸ **Table Compare Report…** writes a report of the view's comparison, as `hikasa table compare --report` does
([hikasa table compare](topic:table-compare), [hikasa report](topic:report)).

- **Layout**: side by side (one table per sheet, left cells beside right cells), interleaved (the left row, then the
  right row of each changed pair), summary (the statistics per sheet), XML or JSON.
- **Rows**: all rows, the differences, the same rows or none (the summary ignores it).
- **Form**: HTML or plain text, with or without row numbers; XML and JSON have one form.
- **Title**: the report's title; empty keeps the default.
- **Write to**: a file (an existing one is replaced) or the clipboard.

**Write Report** writes it; **Stop** stops a report being written. The layout, the rows and the form are remembered
for the next report.
