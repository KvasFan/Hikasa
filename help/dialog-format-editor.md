# Format editor

The format editor opens from the [File Formats](topic:dialog-file-formats) manager (**Edit…**, **New**, **Clone**),
from a text view's toolbar button **Format** (the format of the pane that has the focus) and from **Edit Grammar…**
on the Importance tab of [Session Settings](topic:dialog-session-settings). Its tabs depend on the kind:

| Kind | Tabs |
|---|---|
| Text | General, Conversion, Grammar, Misc |
| Table | General, Conversion, Type, Regional |
| Hex, Picture | General, Conversion |
| Media, Version, Registry | General |

## General

The name, the id (fixed), the mask (name patterns separated by `;`: `*` any characters, `?` one, `[abc]` a set;
empty: the format is chosen by hand only) and a description.

## Conversion

*None (plain text)*, *Sorted* (the lines are sorted before comparing; the file cannot be edited), *External* (a
program converts the file when it is loaded, and back when it is saved: `%s` the input file, `%t` the output file,
`%o` the original name; a timeout; Unicode or ANSI file names; *Disable editing* when there is no save command) and
*Spreadsheet text* (every sheet of a workbook as tab-separated text). A converted file shows *Converted* in the
view's header.

## Grammar (Text)

The grammar items in order: the first item that matches at a place wins. **Add…**, **Edit…** (or a double click)
open the [Grammar Item](topic:dialog-grammar-item) dialog; **Clone**, **Delete**, **Move Up** and **Move Down** act
on the selected item. Below them, each element of the grammar with *Case sensitive* and *Important by default* (the
default a session's Importance tab starts from), and the line weights: lines that hold a text or a [regular
expression](topic:regex-helper) are preferred as alignment anchors, by priority 1–4.

## Misc (Text)

Tab size (1–32), *Insert spaces instead of tabs*, extra word characters, *Lines are independent*, *Column-based
data*, a characters-per-line limit, the encoding (*Auto* with a fallback label, or a fixed label), *Ctrl-Z ends the
file*, and what saving does: trim trailing whitespace, leading spaces to tabs.

## Type and Regional (Table)

The table's type (detect, delimited, fixed width, spreadsheet), its delimiters and text qualifier, consecutive
delimiters as one, trimming, the fixed widths (`10;20;8`), what the first line holds and Ctrl-Z; the decimal and
thousands separators, the date order and separator, or the system's settings. See [Table formats](topic:table-formats).

## Preview (Text)

A sample (type it, or **Load sample…** for the first 64 KiB of a file) shown in the colours of each element, as the
edited grammar reads it — so a grammar is tried before it is saved. It follows your edits after a short pause.

## OK and Cancel

OK checks the format first; a refusal names the item (*Grammar item 3: …*, *Line weight 1: …*) and nothing is
written. Then the format is written as `hikasa format edit` writes it: the format file of that id in the formats
folder (a built-in gets its override file). Keys of your file the editor does not know are kept; comments are not.
When the file changed on disk since the editor read it, you choose: **Overwrite**, **Reload** (your edits go) or
**Cancel**. Cancel closes without writing.

See [hikasa format](topic:format) and [Contents](topic:index).
