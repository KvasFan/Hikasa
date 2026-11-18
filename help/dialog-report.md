# Report Dialog

Session ▸ **<Type> Report…** writes a report of the view's comparison. It is the same dialog in a
[Text Compare](topic:view-text-compare), [Folder Compare](topic:view-folder-compare),
[Folder Sync](topic:view-folder-sync), [Table Compare](topic:view-table-compare),
[Hex Compare](topic:view-hex-compare) and [Picture Compare](topic:view-picture-compare) view; a Text Edit view has no
report. The report is the one the command line writes with the same words ([hikasa report](topic:report)): the
dialog's fields become the flags of `hikasa <kind> compare --report …` or `hikasa report text|folder`, and the bytes are
the same.

- **Layout**: the layouts of the kind (text: side by side, interleaved, summary, unified diff, patch, statistics, XML;
  folders: side by side, summary, list, XML, JSON; tables: side by side, interleaved, summary, XML, JSON; hex: side by
  side, interleaved, summary; pictures: side by side, summary), and the options of the layout: line numbers, the
  strike-out of a side's differences, the patch style, the folder columns and *Include links to file reports*, the
  table cells of JSON, the hex addresses and context rows, the picture difference mode, checkerboard and full-size
  images.
- **Show**: the rows shown (all, differences, same, none, differences with context and their number where the kind
  has them, the folder filters). It starts as the view shows its rows and is never remembered.
- **Folders** (folder reports): with the folder structure, always, or files only.
- **Ignore unimportant differences**: starts as the view's.
- **Just selection**: the focus pane's selected lines (text) or the selected rows (folders); available when the view
  has a selection and the layout has rows.
- **Output**: HTML in colour, HTML in monochrome, HTML with a stylesheet of yours (its path), or plain text. XML, JSON,
  patches, statistics, the folder list and the picture side-by-side layout have one form, and the field is greyed.
- **Wrap** (text side by side and interleaved): none, at a character, at a word.
- **Print Preview**: the palette (colour or monochrome) and the orientation (portrait or landscape) of the printed
  report.
- **Title**: the report's title; empty keeps the default. It is not remembered.
- **Write to**: a file or the clipboard. A file named without its folder goes to the folder of the last report (the
  home folder the first time); the line under the field shows the whole path. **Browse…** picks the file. `-` and
  `clipboard://` are not files here: choose **Clipboard** to copy the report.

When a combination is refused (two words of one choice, a form the layout does not have), the reason is shown under
the fields and the buttons are greyed.

**Write Report** writes the report; when the file exists the dialog says so and the button becomes **Replace**, which
writes over it. **View in Browser** writes the report below the cache's `print` folder and opens it with the program
your system opens HTML files with; **Print Preview** does the same with the print palette and orientation, for the
browser's printing. If no program can open it, the file stays and the dialog names it. **Stop** stops a report being
written (the progress of a folder report with file links shows how many pairs are done); **Close** closes the dialog.
The dialog stays open after each action with its result.

A Text Compare view with edits not compared yet waits for the comparison of the edits before it writes; a side with
unsaved edits is marked *(unsaved edits)* in the report's header. A Folder Compare view reports its tree as it is now,
after copies, deletes, refreshes and excludes.

The layout, the options, the output and the file are remembered per kind after each report.
