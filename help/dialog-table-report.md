# Table Compare Report

Session ▸ **Table Compare Report…** writes a report of the view's comparison, as `hikasa table compare --report` does
([hikasa table compare](topic:table-compare), [hikasa report](topic:report)).

- **Layout**: side by side (one table per sheet, left cells beside right cells), interleaved (the left row, then the
  right row of each changed pair), summary (the statistics per sheet), XML or JSON.
- **Rows**: all rows, the differences, the same rows or none (the summary ignores it); it starts as the view shows its
  rows, and the report follows the view's *Ignore Unimportant Differences*.
- **Form**: HTML or plain text, with or without row numbers; XML and JSON have one form.
- **Title**: the report's title; empty keeps the default.
- **Write to**: a file or the clipboard. A file named without its folder goes to the folder of the last report (the
  home folder the first time); the line under the field shows the whole path. `-` and `clipboard://` are not files
  here: choose **Clipboard** to copy the report.

**Write Report** writes it; when the file exists the dialog says so and the button becomes **Replace**, which writes
over it. **Stop** stops a report being written. The layout, the form and the folder are remembered for the next
report.
