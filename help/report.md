# hikasa report

`hikasa report text <left> <right>` and `hikasa report folder <left> <right>` compare and write a report: to standard
output, a file, the clipboard, or a page the browser shows or prints. The report is side-by-side by default; the
[exit code](topic:exit-codes) is the comparison's, as for [hikasa text compare](topic:text-compare) and
[hikasa folder compare](topic:folder-compare), whose rule flags both subcommands take.

`--session <ref>` reports a saved session instead of two specs: a Text Compare session for `report text`, a Folder
Compare or Folder Sync session for `report folder`; the flags given apply on top ([hikasa session](topic:session)).
`-` and `clipboard://` are sources, as for `text compare` ([Using the command line](topic:cli)).

## The report flags

`text compare`, `folder compare`, `session run`, `report text` and `report folder` share these flags:

| Flag | Meaning |
|---|---|
| `--layout <word>` (`--report`) | text: `side-by-side`, `summary`, `interleaved`, `patch`, `statistics`, `xml`, `unified`; folders: `side-by-side`, `summary`, `xml`, `list`, `json` |
| `--options <word,…>`, `--output-options <word,…>` | words of the vocabulary below, comma-separated, repeatable |
| `--output-to <target>` | `-` (standard output, the default), `clipboard`, `printer`, `browser`, or a file |
| `--output <file>` | as `--output-to <file>`; both together are a usage error |
| `--title <text>` | the report's title |
| `--display <word>`, `--context <n>` | what is shown when no `display-*` word says (as the compare subcommands) |
| `--folders <mode>`, `--suppress-filters` | folders: the folder mode of the display, the items filters hide |
| `--patch-style <s>`, `--wrap <mode>`, `--line-numbers`, `--no-line-numbers` | text: the words `patch-<s>`, `wrap-<mode>`, `line-numbers`, `no-line-numbers` |
| `--width <n>` | text: the width of side-by-side and interleaved text |
| `--columns <list>` | folders: a `column-<name>` word per name |
| `--html`, `--html-mono` | the words `html-color`, `html-mono` |
| `--lines [left:\|right:]<a>-<b>` | text: only the rows of a range of lines (side-by-side, interleaved, xml) |
| `--select <mask>…`, `--path <rel>…` | folders: only the selected rows and their folders (list, side-by-side, xml) |
| `--dry-run` | print what would be written; write nothing |

A flag only one kind takes is refused for the other (exit 100). With `--json`, `report` writes `hikasa.report.v1`
([JSON output](topic:json-output)), so the report needs a target other than standard output; the compare subcommands
keep their own document and write the report to its target.

## The vocabulary

Words are case-insensitive, and `_` equals `-`. A word fills one slot; two different words of one slot are refused
(exit 100), also when one comes from a flag (`--html-mono` with `html-color`). A word the layout does not use gives one
warning `report_option_unused` and changes nothing.

| Slot | Words | Used by |
|---|---|---|
| display | text: `display-all`, `display-mismatches`, `display-context`, `display-matches`, `display-none`; folders: `display-` and a display filter, `display-context` | text side-by-side, interleaved, xml; folder list, side-by-side, xml, json |
| ignore | `ignore-unimportant` (also for the comparison: verdict and exit code) | all |
| numbers | `line-numbers`, `no-line-numbers` | text side-by-side, interleaved |
| strike | `strikeout-left-diffs`, `strikeout-right-diffs` | interleaved |
| patch | `patch-normal`, `patch-context`, `patch-unified` | patch |
| columns | `column-size`, `column-timestamp`, `column-attributes`, `column-crc`, `column-owner`, `column-group`, `column-status`, `column-none` | folder side-by-side |
| links | `include-file-links` | folder side-by-side HTML |
| palette | `html-color`, `html-mono`, `html-custom:<css>` (last in a list), `text` | the layouts with an HTML form |
| print palette | `print-color`, `print-mono` | HTML, the printer, the browser |
| orientation | `print-portrait`, `print-landscape` | HTML, the printer |
| wrap | `wrap-none`, `wrap-character`, `wrap-word` | text side-by-side, interleaved |

HTML is available for side-by-side, summary and interleaved; `xml` is one document type for text and folder
comparisons, described by `docs/schemas/hikasa-report-1.xsd`. `html-custom:<css>` reads a stylesheet of at most 1 MiB
from a file.

## Links to file reports

`include-file-links` with folder side-by-side HTML writes a text report for every file pair the report shows that
differs, into a folder beside the main file, `<name>_files`, and links each row to it; each pair report links back.
The folder holds a marker, `.hikasa-report.toml`: a later report to the same file replaces the folder it made, and
nothing else. A folder of that name that is not this report's is left alone, and the report uses `<name>_files-2` (up
to `-99`), with the warning `report_folder_renamed`. A pair too large for a rules-based comparison or found binary is
skipped; a pair that fails gives the warning `file_report_failed`, and the folder report is still written. The pair
reports are written beside, then swapped in, so a failure or a Ctrl-C leaves the old report and folder as they were.

## Targets

- A file is written through a temporary file and a rename: on any error, or a Ctrl-C, it keeps what it had. A file
  named like a keyword is written `./printer`.
- `clipboard` places the report on the clipboard: an HTML report with a plain-text alternative, anything else as text,
  at most 128 MiB (`too_large`). On Linux a small helper process keeps the content until something else is copied,
  after `hikasa` has ended. `HIKASA_CLIPBOARD_FILE=<file>` writes the report to that file instead.
- `printer` writes the report as HTML with a print block (portrait unless `print-landscape`) to a new folder below the
  cache folder's `print/` and opens it with the associated application, which prints it; `browser` does the same
  without the print block. Folders there older than seven days are removed. No application to open it is exit 100;
  the file stays.

`SOURCE_DATE_EPOCH` fixes the report time ([Using the command line](topic:cli)), so a report is the same bytes on
every run.

## Dry run

`--dry-run` compares but writes nothing, and prints `write <file>` or `replace <file>`; with links `mkdir <folder>`,
`replace <folder> (an earlier report)` or `mkdir <folder> (instead of <wanted>, which is not this report's)`, a
`write <folder>/<file>  <path>` line per pair (`skip (<reason>) <path>` for one known to be skipped), then
`would write <n> files`.
