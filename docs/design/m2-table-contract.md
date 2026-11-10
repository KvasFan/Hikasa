# Hikasa M2 — Table Compare contract (W12 table part)

> **Status: binding** for the Table Compare engine (`core/hikasa-table`) and, as specified here, for its later
> integration (phase 2: Service, CLI, reports, sessions, folder content test; phase 3: the GUI grid of Wave 13). It
> **extends** `docs/design/m2-contracts.md` the way a wave section does (§0 ground rules, tenets T1–T14, decisions
> D-59…D-291 stay in force) and is merged into it as §1.12 when phase 2 starts; its ids are local: decisions **T-1…**,
> numbered notes **N1…**, acceptance **AT.1…**, gate **G9** (plan §5.2).
>
> **Why a separate file.** The maintainer asked on 2026-11-09 for *xlsx compare as soon as possible*. That revises
> D-59 / plan A6, A7 (spreadsheet extraction deferred to M3) and the "`calamine` not proposed for M2" line of plan §3
> for **TB-004** (T-1). The engine was built beside W10c, which owned `hikasa-session`, `cli/`, `docs/` and the main
> contract at that time, so phase 1 touched only `core/hikasa-table/`, the workspace manifest and lock (the
> dependency), `docs/02` and this file.

## 0. Scope

**Ids.** TB-001 (table formats: Type, Regional, encoding, Ctrl+Z; the *built-in spreadsheet extraction* conversion),
TB-002 (delimited and fixed parsing, sniffing), TB-003 (first line), TB-004 (all sheets of xlsx, xlsm, xlsb, xls, ods
through `calamine`; sheet state), TB-005 (typed cells), TB-010 (sheet alignment), TB-011, TB-012 (column alignment and
handling), TB-013 (rows: keys, unkeyed engines), TB-021, TB-022, TB-024 (the models of filters, navigation and the Info
counts; the view is phase 3), TB-025 (report layouts, phase 2), FF-030's *Spreadsheet text* converter (phase 2, N25),
the Table part of the D-11 revision (folder content test, phase 2), gate G9.

**Out of scope.** HTML-table extraction (TB-004's second half, P1 → M3), editing and saving tables (TB-023, P1 →
phase 3 or M3; the writer of §2.4 exists for it), spreadsheet write-back (P2), password-protected workbooks (refused
with a clear error, T-9), charts, pictures, styles and conditional formats (not cell values).

| Phase | Content | Files | State |
|---|---|---|---|
| **1 Engine** | dependency, parser, loader, compare engine, model, vocabulary parsers, `TableComparator`, tests, G9 | `core/hikasa-table/**`, `Cargo.toml`, `Cargo.lock`, `docs/02`, this file | **done** (§9) |
| **2 Integration** | Service, CLI `table compare`, JSON `hikasa.table_compare.v1`, reports, sessions, folder content test, format model, architecture test, help, docs | §6 file by file | after W10c lands |
| **3 GUI** | the Table Compare view (W13 grid), sheet tabs, Columns/Rows/Sheets dialogs, navigation, Info | §7 | Wave 13 |

## 1. Decisions

| ID | Decision | Rationale | Docs |
|---|---|---|---|
| T-1 | **TB-004 enters M2** (revises D-59 and plan A6/A7 for spreadsheets only): all sheets of xlsx, xlsm, xlsb, xls and ods are compared by the table engine; HTML tables stay M3. | Owner, 2026-11-09: "xlsx compare as soon as possible". | 06, 10, 16 |
| T-2 | **`calamine` 0.38.0** (MIT), `default-features = false` (no `dates`/`chrono`, no `picture`), named only under `core/hikasa-table/src/workbook/`. Its `zip` ^9 (deflate), `encoding_rs` and `serde` unify with the workspace's; new packages in `Cargo.lock`: `calamine`, `atoi_simd`, `byteorder`, `codepage`, `fast-float2`, `quick-xml` 0.42, `log` (if absent) and their own needs (§8). Dev only: **`rust_xlsxwriter` 0.102.0** (MIT OR Apache-2.0, no features) generates the xlsx fixtures of the tests. `cargo deny check licenses bans sources` passes without a new exception. | `docs/02` already names calamine for spreadsheets; pure Rust; the only maintained reader of all five formats. | 02 |
| T-3 | **One engine, two loaders.** Delimited and fixed text is parsed in-house (§2; one owner of every TB-002 option, plan §3 "`csv` not proposed"); workbooks are read through calamine (§3). Both produce the same `Workbook` model (§4), and the comparison never knows where a sheet came from: a CSV compares against an xlsx sheet. | T11 one owner; mixed pairs (an export against its source) are the common case. | 06 |
| T-4 | **Every read goes through the VFS.** `load_workbook(vfs, spec, format, opts, ctl)` stats, size-checks and reads the bytes with `open_read_with` (cancellable), then parses from memory; archive members, stdin and clipboard blobs work like local files. `load_bytes(bytes, name, format, opts, ctl)` is the pure entry the folder content test and the tests use. | VFS-001; T1 (no `std::fs` in the engine). | 11 |
| T-5 | **Compact cells.** A cell is 12 bytes (`kind`+flags, two `u32`): empty, number (`f64`), text (offset and length into the sheet's arena), bool, date-time (ms since 1970-01-01T00:00, civil, no zone), duration (ms), error (code). Delimited cells are text spans into the decoded file (zero copy; only fields with doubled qualifiers are unescaped into the arena's tail), so the display shows the file's own spelling; spreadsheet cells are typed by the reader. Rows are ragged (each stores cells up to its last non-empty one). Formula texts live in a side table. | G9: 1M × 10 per side in ≤ 4 × input bytes; the display must show `1.50`, not `1.5`. | 06 |
| T-6 | **Typed comparison** (TB-005). Per side and column the loader detects a column kind (number, date-time, bool, text, empty) over **all** rows. A comparison column's *General* type resolves to the kind both sides share; when they disagree, each text cell pair is parsed opportunistically (number, then date, then bool) and compared typed when both parse to one kind; *Text* (forced) compares display texts. Typed cells of a workbook keep their own kind. Equal values with different spellings (`1.0` / `1`, `2024-01-31` / `31/01/2024` under DMY) are **same**. A number written with a leading zero (other than `0` and `0.x`) or with more than 15 significant digits makes its column Text (an identifier: zip codes, account numbers), and the opportunistic parse keeps a leading-zero text text, so `007` ≠ `7` and `01234` ≠ `1234` are important differences (T-24); numbers past 15 significant digits compare by their exact decimal (T-23). | Per-column detection keeps `007` ≠ `7` in an id column of text; the opportunistic rule keeps one stray `n/a` from turning a numeric column into text on one side (both pinned by `t6_rationale_id_columns_and_stray_text`). | 06 |
| T-7 | **What is unimportant** (TB-012). A difference is *unimportant* when it is in an Unimportant column, or vanishes under the column's tolerances or text exceptions: numeric tolerance (absolute, or relative % of the larger magnitude), date tolerance, *Character case*, *Whitespace* (leading, trailing and runs of whitespace). Exactly equal typed values are *same*. **Open default:** a value within tolerance is an unimportant difference (blue; verdict 12, 2 with `--ignore-unimportant`), never silently same. | TB-012 lists tolerances beside the *text important except for* exceptions, which TC-021 already treats as unimportant; the user sees what the tolerance absorbed. | 06 |
| T-8 | **Rows** (TB-013). With key columns rows join by key (§5.4): keys compare typed and exact (tolerances never apply to keys; case and whitespace exceptions of a key column do), the output is in key order, duplicate keys pair in their order of appearance and are counted and reported (`duplicate_keys`, up to 10 examples per side). Without keys rows align through `hikasa_diff::align` on row tokens (Unaligned, Standard, Myers, Patience, Histogram; skew; closeness with a cell-equality scorer; never-align), optionally after sorting both sides. **Open default:** no keys, Standard, skew 300, closeness 500 ‰. | Same engines as text (T11); database-join semantics for keys. | 06 |
| T-9 | **Errors are clear and typed.** Password-protected (encrypted OOXML package in a compound file, or a BIFF `FILEPASS`) → `TableError::Encrypted`; not a workbook or damaged → `TableError::Corrupt` with the reader's message; too large → `TooLarge`; cancelled → `Cancelled`. Each classifies (`ErrorKind`): `unsupported`, `invalid_data`, `too_large`, `cancelled`. Exit 100. | CLI-004 "any error 100"; T9 no panics on hostile input. | 12 |
| T-10 | **Sheets** (TB-010): `by-name` (**default**: equal names, then case-insensitive names, then the remaining sheets by position), `left-name`, `right-name` (pairs by name only; the order follows that side; the rest are orphans), `position`, or an explicit list of pairs. A pair's differing names are informational (`renamed`), never a difference: a CSV (one sheet named after the file) pairs with the first sheet of a workbook. A sheet on one side only is an important difference unless it is empty. | "by name, then by position, with a manual map" (owner brief). | 06 |
| T-11 | **Columns** (TB-011): `auto` (**default**: by name when both sides have header names, else by number), `number`, `left-name`, `right-name`, `custom` (explicit pairs; unnamed columns follow by number). Name matching: exact, then case-insensitive and trimmed (a header difference of that pair is unimportant). With header rows aligned by number, differing header names are important differences (`header_differences`). | The header row is content of the file. | 06 |
| T-12 | **Column references** (vocabulary, parsed in core, check 7): a decimal `3` is the 1-based comparison column; `#3` too; `name:<text>` a header name; otherwise a header name (exact, then case-insensitive), then column letters `A`…`XFD` (1-based position). Lists are comma-separated (`--key id,date`); a `\,` is a literal comma. A reference that names nothing is `invalid_input` (exit 100), listing the columns. | One grammar for CLI, sessions and scripts; names win over letters so a header called `ID` is not column 238. | 12 |
| T-13 | **Formulas.** The cached value is compared. With `compare_formulas` a differing formula text is also an important difference; the formula texts are in the model either way (`Sheet::formula`). A formula without a cached value is an empty cell plus its text. | Spreadsheets from different tools may share values but not formulas; the default matches what a user sees. | 06 |
| T-14 | **Merged cells:** the value stays in the top-left cell, the covered cells are empty; the model keeps the regions per sheet (`SheetInfo::merged`, `Sheet::merged_region(r, c)`), so a covered cell is same as an empty cell on the other side. **Dates:** serials of the 1900 epoch (with its fictitious 1900-02-29 mapped to 1900-02-28) and of the 1904 epoch become civil date-times; a serial below 1 is a time of day on 1899-12-30 in both epochs (so a time cell equals a CSV `12:30`), **except serial 0 of the 1904 epoch, which is that epoch's first day, 1904-01-01** (the date a 1900 workbook stores as serial 1462; a 00:00 time-only cell of a 1904 workbook therefore reads as that date; review F10); durations stay durations. Errors keep their code (`#DIV/0!`, `#N/A`, `#NAME?`, `#NULL!`, `#NUM!`, `#REF!`, `#VALUE!`, `#DATA!`) and compare by code. | TB-004 cases the owner listed; dates compare across epochs and across file types. | 06, 10 |
| T-15 | **Memory is bounded and measured.** xlsx and xlsb stream cells (`worksheet_cells_reader`) into **sparse rows**: a gap inside a row costs nothing, so a sheet using A1 and XFD1048576 costs two cells and 1,048,576 row starts, and a row with A and XFD two cells (review F4; the sheet keeps the sparse form, 4 more bytes a cell, only when it is smaller than the dense one, and the comparison walks the stored cells of sparse rows). xls and ods go through calamine's dense range (one 32-byte `Data` per cell of the used rectangle), guarded by `SpreadsheetOptions::max_cells` (default 200,000,000: the used rectangle of a dense sheet, the stored cells of a streamed one; above it `TooLarge`). A streamed sheet spanning more than max(1,048,576, 4 × its stored cells) rows or max(16,384, its stored cells) columns is refused (`TooLarge` `rows`/`columns`) before anything is allocated for it. Text is bounded by T-26. calamine allocates a dense range before the guard can look at it: xls is bounded by its format (65,536 × 256), ods is not (open item §9.7). Every loader honours `max_bytes` (default and cap 4 GiB − 1: arena offsets are `u32`). | Hostile or huge workbooks must not exhaust memory (T9); G9 budget. | 02 |
| T-16 | **Verdict and exit codes** (§6 of the plan: the 1/2/11/12/13 family): bytes identical → `binary_same` (1); else no difference → `rules_same` (2); only unimportant → `unimportant_only` (12; 2 with `--ignore-unimportant`); any important → `important_differences` (13); any error → 100. A table comparison never answers 11. The engine computes the rules part; `bytes_identical` is set by the caller that read the bytes. | D-01's mapping (`ExitCode::for_kind`). | 12 |
| T-17 | **Parallel and cancellable.** Sheet pairs compare in parallel on the caller's rayon pool, results in sheet order (T2); inside a sheet, column detection, key encoding and sorting are parallel per side; every loop checks cancellation at least every 65,536 rows; a cancelled job returns `Cancelled`, never a partial result. Loading both sides runs in parallel (`rayon::join`). | G4 cancel ≤ 100 ms; G9. | 01 |
| T-18 | **Delimited sniffing** (TB-002 *Detect*, §2.3): candidates `,` `;` tab `|` `:` space over the first 64 records (quote-aware), the candidate with the most consistent non-zero count wins (ties in that order); a candidate must occur in the first record, `:` is none when it separates the parts of times, and records that are each one date-time value are one column (review F7); the qualifier is `"` unless fields open with `'` consistently; when no candidate is consistent and the sample lines have aligned blank columns, the format is **fixed** with the detected widths; else one column. *First line* Detect: a column-vote heuristic (§2.5). | TB-002 Detect; one documented algorithm. | 06 |
| T-19 | **Regional settings** (TB-001, A16): decimal separator, thousands separator (or none), date order (YMD, DMY, MDY), date separator (or any of `-` `/` `.`), all explicit; *use system* is resolved by the host from the locale (phase 2, the A16 table); the engine never reads the environment (T1). Defaults: `.`, `,`, YMD, any. Numbers: optional sign, digit groups of 3 when a thousands separator is present, one decimal separator, exponent, a trailing `%` divides by 100; leading and trailing blanks ignored. Dates: ISO 8601 (`2024-01-31`, `2024-01-31T12:30:00.250`, a space instead of `T`) always, plus the regional order with 2-digit years pivoting at 1930–2029 and an optional time with seconds, milliseconds and AM/PM. | A16 interpretation; locale-independent tests. | 06, 15 |
| T-20 | **JSON** `hikasa.table_compare.v1` (§6.3), built by `hikasa-report` (`table_compare_json`) from the model; filters from the display options; differing rows carry only their differing cells unless `cells: all`. | As `hikasa.text_compare.v1` (D-18). | 12, 13 |
| T-21 | **Folder content test** (D-11 revised for Table): with the Rules method, a pair whose format is of kind Table compares through `hikasa_table::TableComparator` (a `ContentComparator`); Hex and Picture kinds stay by bytes. A table that fails to parse gives `error` for that pair, never a fallback to bytes. | Plan §5.3 D-11 row. | 04 |
| T-22 | **Spreadsheet text conversion** (FF-030, revises D-59 for this converter): a Text-kind format with conversion `spreadsheet` turns every sheet into TSV text (`--- <sheet name> ---` line between sheets) through `hikasa_table::spreadsheet_to_text`, so Text Compare and Find in Files read workbooks too. | Same reader, no second implementation; phase 2 wires it into `hikasa-formats`' conversion enum. | 10 |
| T-23 | **Long numbers are exact** (review F1). Two numbers each of at most 15 significant digits compare through their `f64` (exact for them: such a decimal has one `f64`). A number with more (up to 38) compares by its normalized decimal (sign, digits without leading or trailing zeros, the exponent folded in), never through `f64`; past 38 digits a number is text. Keys and row tokens encode the same way (§5.4), so `12345678901234567` and `…568` are distinct keys, never duplicates. Tolerances use the approximations (within them: unimportant, never same). | An `f64` holds 15.95 digits: ids past 2^53 collided (identical files reported duplicate keys). | 06 |
| T-24 | **Leading zeros** (review F2). Detection makes a column with a leading-zero or longer-than-15-digit number Text (T-6). The column type **Number** (`ColumnType::Number`, beside General and Text) parses every number of the column, leading zeros included: `007` = `7`, `01234` = `1234`. | Identifiers lose their meaning when parsed; a user who knows the column is numeric says so. | 06, 12 |
| T-25 | **Workbook precision** (review F6). A number stored in a workbook compares at 15 significant digits, the precision the workbook shows: `0.1 + 0.2` (`0.30000000000000004`) equals a CSV `0.3`; integers below 2^53 stay exact whatever their length (16-digit ids). `ColumnRule::exact_numbers` (per column, `RulePatch`) compares the stored binary value (its shortest round-trip decimal) instead. Rounding is a deterministic function of the value, used alike by equality and the encodings. | An xlsx against its CSV export must not differ on floating-point noise; the exact mode keeps the noise visible for those who ask. | 06 |
| T-26 | **Text is bounded** (review F3). Each shared-string entry is copied into a sheet once and every cell using it points at that copy; inline strings repeated in a row (ods repeats) reuse the copy. `SpreadsheetOptions::max_text_bytes` (default `None`: 64 × the input's size, at least 1 GiB) bounds the text of the whole workbook (`TooLarge` `bytes of text`), and a declared `sharedStrings` part larger than it is refused before calamine inflates it (`TooLarge` `bytes of shared strings`; the zip reader caps a part at its declared size). | A 22 KB xlsx whose one 20 MB string filled 150 cells took 5.8 GB. | 02 |
| T-27 | **Out-of-order cells load** (review F8). A row's cells are buffered and sorted by column (a repeated position keeps its last cell); a row before the open one makes the loader read that sheet again, every cell first, then sorted by position. Never `Corrupt`. | Hand-written and tool-written xlsx do not always keep the order the format asks for. | 06 |
| T-28 | **Trailing Ctrl+Z** (review F12). Without `ctrl_z_eof`, a lone 0x1A at the very end of a text table is dropped (the old end-of-file mark), never a row of data; inside the data it stays data. | DOS-era exports end with it; a phantom last row would be a difference. | 06 |
| T-29 | **Hidden rows and columns** (review F13). `SheetInfo::hidden_rows` and `hidden_columns` (merged inclusive ranges in sheet coordinates) and `Sheet::row_hidden`, `column_hidden` model them. xlsx and xlsm give them (the `hidden` attributes of `<row>` and `<col>`, and `zeroHeight` sheets whose unlisted rows are hidden): calamine does not report them, so a second, parallel pass reads the worksheet parts (in-house inflate and tag scan; `SpreadsheetOptions::read_hidden_rows_columns`, default on). xlsb, xls and ods give none (calamine reads no row or column records of them), text tables have none. `SheetRules::ignore_hidden_rows` / `ignore_hidden_columns` (default off) leave them out of the comparison as if absent from their side; rows keep their sheet labels. | Owner: model them; comparing what a user sees is an option, not the default. | 06 |

## 2. Delimited and fixed text (TB-001, TB-002, TB-003)

### 2.1 Types

```rust
pub struct TableFormat {
    pub kind: TableKind,               // Detect | Delimited(DelimitedOptions) | Fixed(FixedOptions) | Spreadsheet
    pub first_line: FirstLine,         // Detect (default) | Names | Data
    pub regional: Regional,            // §T-19
    pub encoding: Option<&'static encoding_rs::Encoding>, // None = Auto: BOM, UTF-8, chardetng over the bytes
    pub ctrl_z_eof: bool,              // a 0x1A ends the data (default false)
}
pub struct DelimitedOptions {
    pub delimiters: Vec<char>,         // any of them separates fields (default [',']); never a qualifier, CR or LF
    pub qualifier: Option<char>,       // default Some('"'); None = no quoting
    pub consecutive_as_one: bool,      // a run of delimiters separates once; leading and trailing runs are ignored
    pub trim_whitespace: bool,         // "surrounding whitespace is part of the delimiter": spaces and tabs around
                                       // unquoted fields and around qualifiers are dropped
}
pub struct FixedOptions { pub widths: Vec<u32> }   // in characters; text past the last width is one more column
```

`TableFormat::from_extension(name)` gives the presets: `*.csv` comma, `*.tsv`/`*.tab` tab, `*.xls;*.xlsx;*.xlsm;*.xlsb;
*.ods` Spreadsheet, anything else Detect. Phase 2 maps `hikasa_formats::TableSettings` onto it (§6.1).

### 2.2 Parsing (normative)

1. **Decoding.** BOM (UTF-8, UTF-16 LE/BE) wins; else the format's encoding; else valid UTF-8; else chardetng's guess.
   Malformed sequences become U+FFFD and the warning `lossy_decode` (count). Over `u32::MAX` bytes decoded: `TooLarge`.
2. **Records** end at LF, CRLF or CR outside a qualified field; a final line end does not open a record; an empty
   line is a record with no fields (it stays a row, so row numbers follow lines). Each row keeps its 1-based first
   line number (`Sheet::row_line`).
3. **Fields.** With a qualifier, a field whose first character (after blanks when `trim_whitespace`) is the qualifier
   is *qualified*: it runs to the next qualifier not doubled; a doubled qualifier is one literal qualifier; delimiters,
   CR and LF inside are data; characters between the closing qualifier and the next delimiter are appended as they are
   (lenient, warning `stray_after_qualifier` once per sheet); an unterminated qualified field runs to the end of the
   data (warning `unterminated_qualifier` with its line). A qualifier inside an unqualified field is data.
4. **Consecutive delimiters as one:** separators are maximal runs of delimiter characters; runs at the start or the end
   of a record separate nothing. Without the option `a,,b` has three fields and `a,` two.
5. **Fixed widths:** a record is cut at the cumulative widths counted in Unicode scalar values; each field is trimmed of
   spaces and tabs; a remainder past the last width is one more field when not blank; qualifiers mean nothing.
6. **Ctrl+Z:** with `ctrl_z_eof` the first 0x1A ends the data; without it a lone 0x1A at the very end is dropped (T-28).

### 2.3 Sniffing (T-18)

Over the first 64 records of the decoded text (records split quote-aware with `"`): when at least 80 % of the records
after the first (all of them when there is one) are each one date-time value in any date order and hold no `,` `;` tab
or `|` (`2024-01-02 12:00:00`, `01/02/2024 1:05 PM`, `12:30`), the table is one column. Else for each candidate the
count of occurrences outside qualifiers per record; a candidate that does not occur in the first record is no candidate,
nor is `:` when in at least 80 % of the records that hold one every `:` has a digit before it and exactly two digits after
it (times); the score of a candidate is the number of records whose count equals the
modal non-zero count. The winner has the highest score ≥ ⌈0.8 · sampled records⌉ (records of one line only: score 1
suffices), ties in candidate order. Qualifier: `'` when at least half of the fields of the sample open with `'` and
none with `"`, else `"`. No winner: if at least two sample lines exist and some character positions are blank in every
non-empty line while the position before is non-blank in at least one, the format is Fixed with widths at those
boundaries (runs of blank columns collapse to one boundary at the run's end); else one column (Delimited on `\0`).
The sniffed options are returned (`Workbook::detected`), so the caller can show them (File Info) and save them.

### 2.4 Writing

`write_delimited(rows, &DelimitedOptions) -> Result<String, WriteLoss>` writes the first delimiter, quotes a field
when it contains a delimiter, the qualifier, CR or LF, starts or ends with a blank (when `trim_whitespace`), or is the
only and empty field of its row; qualifiers inside are doubled; records end with `\n`. Without a qualifier, a field
that would need quoting is `WriteLoss` ("data lost during editing because of format", TB-023's warning).
**P-DELIM** (proptest): for random rows of random strings and every option set with a qualifier,
`parse(write(rows)) == rows` (with `consecutive_as_one` only for rows without empty fields; with `trim_whitespace`
fields are compared trimmed).

### 2.5 First line (TB-003)

`Names` makes the first record the header; `Data` keeps it as data; `Detect`: with fewer than 2 records, data. Else
each column of the first record votes with a weight over up to 50 following records (review F5): a **name** (text that is
no number, date or bool) **+2** over a body of typed values (every non-empty body cell a number, date, duration or bool),
**+1** over texts of one length its own differs from, else 0; any header value that **occurs in its body column −2**; a
**year** (a whole number 1000–2999, typed or text) 0 over numbers, +1 over another typed kind; **another value** −2 over
values of its own kind, +1 over another typed kind, else 0; an error −1; a **blank** cell 0 (its column is named by its
letters). The first record is the header when the votes sum above 0: blank names (pandas' index column) and years
(`region,2023,2024`) are allowed when the other columns vote clearly. Header names never take part in type detection.

## 3. Workbooks (TB-004)

`load_spreadsheet(bytes, name, &SpreadsheetOptions, ctl) -> Result<Workbook, TableError>`:

| Format | Detection | Reader | Cells | Merged regions | Formulas |
|---|---|---|---|---|---|
| xlsx, xlsm | zip with `xl/workbook.xml` | `Xlsx`, streaming `worksheet_cells_reader` + `next_cell_with_formula` | streamed | `load_merged_regions` | in the same pass |
| xlsb | zip with `xl/workbook.bin` | `Xlsb`, streaming `next_cell` | streamed | — (calamine has none; empty) | `worksheet_formula` (second pass) |
| xls | compound file with a `Workbook`/`Book` stream | `Xls`, `worksheet_range` | dense, guarded (T-15) | `worksheet_merge_cells` | `worksheet_formula` |
| ods | zip with `content.xml` and the ODS mimetype | `Ods`, `worksheet_range` | dense, guarded | — | `worksheet_formula` |

Detection is by content (review F11): a compound file is xls, or an encrypted OOXML package when it has
`EncryptionInfo`/`EncryptedPackage` streams (T-9); a zip is read through its directory (the end record and the central
headers, zip64 included; in-house, `src/workbook/package.rs`): `xl/workbook.bin` is xlsb, `xl/workbook.xml` xlsx (xlsm
by the extension or a `xl/vbaProject.bin`), a stored `mimetype` member decides ODF (`…opendocument.spreadsheet` is ods;
any other, an `.odt` for instance, is no workbook), `content.xml` without one is ods by the extension. A zip whose
directory cannot be read is a **damaged or truncated workbook** (`Corrupt`) when its first part or its extension is a
workbook's; a zip without a workbook part (one that holds an `.xlsx`, say) is no workbook: `Corrupt` "not a workbook: a
zip archive of N members (…)" when a spreadsheet was asked for, else a text table.
Sheet order is the workbook's; hidden sheets are loaded and flagged (`SheetInfo::hidden`), chart sheets and macro sheets
are skipped with the warning `sheet_skipped`. Shared strings are resolved by the reader; rich text compares by its
plain text. A cell's position is absolute: `Sheet::origin()` is the used range's top-left, row and column labels are
the sheet's own (`A1` addresses in reports and JSON). Header detection (§2.5) and column-kind detection apply as for
text. `SpreadsheetOptions { max_cells, max_text_bytes (T-26), load_formulas (default true), load_hidden (default true;
false skips hidden sheets with the warning `sheet_skipped`), read_hidden_rows_columns (default true, T-29) }`.

`spreadsheet_to_text(&Workbook) -> String` (T-22) writes each sheet as TSV with display texts.

## 4. The model

```rust
pub struct Workbook { /* sheets, source kind, detected options, warnings */ }
impl Workbook {
    pub fn sheets(&self) -> &[Sheet];
    pub fn source(&self) -> SourceKind;            // Delimited | Fixed | Xlsx | Xlsm | Xlsb | Xls | Ods
    pub fn detected(&self) -> &TableFormat;         // the effective format after Detect
    pub fn warnings(&self) -> &[TableWarning];
}
pub struct Sheet { /* name, info, arena, cells, row starts, header, column kinds, formulas, merged */ }
impl Sheet {
    pub fn name(&self) -> &str;
    pub fn info(&self) -> &SheetInfo;               // hidden, origin (row, col), merged regions, date_1904,
                                                    // hidden_rows, hidden_columns (T-29)
    pub fn is_sparse(&self) -> bool;                // rows store their cells with their columns (T-15)
    pub fn row_hidden(&self, r: u32) -> bool;       // T-29
    pub fn column_hidden(&self, c: u32) -> bool;
    pub fn row_count(&self) -> u32;                 // data rows (header excluded)
    pub fn column_count(&self) -> u32;              // widest row, header included
    pub fn row(&self, r: u32) -> RowView<'_>;       // cells of one data row (ragged; dense or sparse: len, get,
                                                    // values with gaps as Empty, stored cells with their columns)
    pub fn cell(&self, r: u32, c: u32) -> Value<'_>;// Value::Empty past the row's end
    pub fn header(&self) -> Option<&[String]>;      // column names from the first line, when it is a header
    pub fn column_name(&self, c: u32) -> Cow<'_, str>; // header name or letters
    pub fn column_kind(&self, c: u32) -> ColumnKind;// detected: Empty | Number | DateTime | Bool | Text | Mixed
    pub fn formula(&self, r: u32, c: u32) -> Option<&str>;
    pub fn row_label(&self, r: u32) -> u64;         // 1-based line (text) or sheet row number (workbook)
    pub fn column_label(&self, c: u32) -> String;   // letters of the sheet's own column
}
pub enum Value<'a> { Empty, Number(f64), Text(&'a str), Bool(bool), DateTime(jiff::civil::DateTime),
                     Duration(jiff::SignedDuration), Error(CellError) }
impl Value<'_> { pub fn display(&self) -> Cow<'_, str>; }   // §4.1
```

### 4.1 Display texts

Numbers: Rust's shortest round-trip form (`5`, `0.1`, `1e-7`), `-0` as `0`; date-times `YYYY-MM-DD` at midnight, else
`YYYY-MM-DD HH:MM:SS` plus `.mmm` when milliseconds are not 0; durations `[-]H:MM:SS[.mmm]`; bools `TRUE`/`FALSE`;
errors their code. Text cells show their text. The display text is what forced-Text columns compare, what reports and
JSON print, and what `spreadsheet_to_text` writes.

## 5. The comparison

### 5.1 Options

```rust
pub struct TableCompareOptions {
    pub sheets: SheetAlignment,          // ByName | LeftName | RightName | Position | Custom(Vec<SheetPair>)
    pub default_sheet: SheetRules,       // for every sheet pair
    pub sheet_rules: Vec<(SheetSelector, SheetRules)>, // per pair, by left (or right) sheet name or 1-based index
    pub compare_formulas: bool,          // T-13
}
pub struct SheetRules {
    pub columns: ColumnAlignment,        // Auto | Number | LeftName | RightName | Custom(Vec<(ColumnRef?, ColumnRef?)>)
    pub keys: Vec<ColumnRef>,            // priority = order (TB-012 Key)
    pub default_rule: ColumnRule,        // TB-012 "Edit Default"
    pub column_rules: Vec<(ColumnRef, RulePatch)>, // patches on the default, in order; refs resolve after column alignment
    pub rows: RowAlignment,              // algorithm, skew, closeness ‰ (None = off), never_align, sort_rows
    pub hidden: Vec<ColumnRef>,          // display only (Hide Column)
}
pub struct ColumnRule {
    pub kind: ColumnType,                // General | Text | Number (T-24)
    pub unimportant: bool,
    pub ignore_case: bool,               // TB-012 "text important except for: Character case"
    pub ignore_whitespace: bool,         //   "... Whitespace"
    pub numeric_tolerance: Option<Tolerance>, // Absolute(f64) | Relative(percent f64)
    pub date_tolerance_ms: Option<i64>,
    pub exact_numbers: bool,             // T-25: workbook numbers by their binary value (default: 15 digits)
}
```

`SheetRules` also has `ignore_hidden_rows` and `ignore_hidden_columns` (T-29, default off).

### 5.2 Columns and the comparison column list

Column alignment (T-11) gives the comparison columns, each `ColumnPair { left: Option<u32>, right: Option<u32>, name,
rule, key: Option<u8> }`, in the left side's order with right-only columns after the left column they follow on the
right (by number alignment: in position order). Column references (T-12) of keys and rules resolve against this list:
a number or letters name a comparison column, a header name matches the left name, then the right name.

### 5.3 Cells (normative)

For a paired row and a comparison column, the left and right values are taken (a missing column, or a row shorter
than the column, is `Empty`). Let `rule` be the column's rule and `kind` the resolved kind (T-6):

1. Both display texts identical byte for byte and both cells of the same stored kind → **same** (fast path).
2. Forced Text: compare display texts; equal → same.
3. Otherwise interpret: typed cells keep their kind (a workbook number at 15 significant digits, T-25); a text cell is
   parsed under `kind` (Number, DateTime, Bool; the type Number parses leading zeros too) or, when the sides' kinds
   disagree, opportunistically (number unless it has a leading zero, date, bool). Empty and blank text are both *empty*.
   A number of more than 15 significant digits is a long number (T-23).
4. Numbers: equal (`-0 == 0`; a long number equal only to a long number of the same decimal) → same; `|a − b| ≤ abs` or `|a − b| ≤ pct/100 · max(|a|, |b|)` → unimportant;
   date-times and durations likewise with the date tolerance; bools and errors by value; mixed kinds compare display
   texts.
5. Texts: equal → same; equal after the column's exceptions (case: Unicode simple case folding of
   `hikasa_core::case`; whitespace: trimmed and runs collapsed to one space) → unimportant.
6. Anything else → **important**, or **unimportant** when the column is Unimportant.
7. With `compare_formulas`, a pair whose values are same but whose formula texts differ is important (unimportant in an
   Unimportant column). Cells covered by a merge on both sides are same.

### 5.4 Rows

**Keyed** (T-8). Each row's key is encoded order-preservingly into bytes (per key column: a kind byte, then numbers as
sign-flipped big-endian `f64` bits (of the approximation for a long number) followed by `0`, or by `1` and the
order-preserving exact decimal for a long number (T-23), date-times and durations as sign-flipped `i64`, bools one byte, texts folded per the
column's exceptions with `0x00` escaped and a `0x00 0x00` terminator, empty as its own kind byte), so byte order is
typed order (empty < bool < number < date-time < duration < text < error). Both sides are sorted stably by key (rayon,
in parallel), merged: equal keys pair one to one in order; the leftovers of a key are orphans. Output order: key order;
between equal keys left rows first in their order. Duplicates: every row whose key occurs more than once on its side is
counted (`duplicate_key_rows`), the first 10 such keys per side are kept as examples with their row labels, and the
warning `duplicate_keys` names the counts.

**Unkeyed.** Each row's token is a 64-bit hash (SipHash 1-3 with fixed keys, deterministic) of its comparison columns
that are not Unimportant: for each non-empty value in column order, the count of empty columns before it and the value
normalized as for keys (typed, case- and whitespace-folded per the exceptions), the same bytes for a dense and a sparse
row (T-15);
tokens are interned (`hikasa_diff::Interner`), all-empty rows are trivial. `hikasa_diff::align` runs with the sheet's
`RowAlignment`; the closeness scorer is the per-mille share of comparison columns whose cells are same or unimportant
(§5.3). A hash collision can only make two different rows a candidate pair: their cells are still compared (§5.3), so
a state is never wrong. `sort_rows` sorts both sides by their full typed row (key encoding over every column) first; the
output follows the sorted order. Diff warnings (`alignment_degraded`, `work_budget_exhausted`, `closeness_*`) become
table warnings. **After Standard degrades** (review F9; warning `alignment_degraded`), every range of non-equal runs goes
through a bounded Myers pass (at most 2,048 edits a range and Standard's fallback budget, 128·(n+m) + 32,000,000
operations, in all): its matched blocks pair as equal rows and the gaps between them align again with the sheet's options
(closeness, never-align); a range Myers cannot finish keeps Standard's rows. 200,000 rows of `1,{i%7}` against the same
shifted by one row: 199,999 same rows (positional pairing gave 0).

**Row states.** A paired row is `same`, or `changed` with importance `unimportant` or `important` (the maximum of its
cells); an orphan row (`left_only`, `right_only`) is important unless no non-empty cell of it lies in an important
column (then unimportant).

### 5.5 Result model

```rust
pub struct TableComparison {              // immutable, Send + Sync
    pub fn sheets(&self) -> &[SheetComparison];
    pub fn stats(&self) -> &TableStats;   // totals over the sheets plus sheet counts
    pub fn kind(&self, bytes_identical: bool, ignore_unimportant: bool) -> ContentKind;  // T-16
    pub fn warnings(&self) -> &[TableWarning];
    pub fn left(&self) -> &Arc<Workbook>; pub fn right(&self) -> &Arc<Workbook>;
}
pub struct SheetComparison {
    pub fn left(&self) -> Option<u32>; pub fn right(&self) -> Option<u32>;   // sheet indices
    pub fn name(&self) -> &str; pub fn renamed(&self) -> bool;
    pub fn columns(&self) -> &[ColumnPair];
    pub fn rows(&self) -> &[RowPair];     // 12 bytes each: left, right (u32::MAX = none), state, importance
    pub fn cell_state(&self, row: u32, col: u32) -> CellState;          // Same | Unimportant | Important
    pub fn differing_cells(&self, row: u32) -> &[CellDiff];             // sparse, sorted by column
    pub fn header_states(&self) -> &[CellState];
    pub fn stats(&self) -> &SheetStats;   // rows, cells, columns, duplicate keys, header differences
    pub fn state(&self) -> SheetState;    // the TB-004 tab spot: Same | Unimportant | Important | LeftOnly | RightOnly
    pub fn alignment(&self) -> RowAlignmentUsed;  // Keyed{keys} | Engine{algorithm} | Unaligned
    pub fn duplicates(&self) -> &DuplicateKeys;
}
```

`SheetStats`: `rows` (left, right, pairs), `same_rows`, `changed_important`, `changed_unimportant`,
`left_only_important`, `left_only_unimportant`, `right_only_*`, `cells_compared`, `cells_same`, `cells_unimportant`,
`cells_important`, `columns_paired`, `columns_left_only`, `columns_right_only`, `header_differences`,
`duplicate_key_rows` (left, right). `TableStats` adds `sheets_paired`, `sheets_left_only`, `sheets_right_only`,
`sheets_differing`.

**View and navigation data** (TB-021, TB-022): `RowFilter { show_same, show_changed, show_left_only, show_right_only,
ignore_unimportant }` with the four presets All, Differences, Same, None; `SheetComparison::visible(&RowFilter) ->
impl Iterator<Item = u32>`; `nav::next_difference_row(sheet, from, dir, ignore_unimportant)`,
`nav::next_difference_cell(sheet, (row, col), dir, …)`, `nav::next_difference_sheet(cmp, from, dir, …)`, wrap-free
(the caller wraps per OPT-007).

### 5.6 Job

`compare_workbooks(left: Arc<Workbook>, right: Arc<Workbook>, &TableCompareOptions, &Ctl) -> Result<TableComparison,
TableError>` (T-17). `compare_specs(vfs, left, right, &LoadRequest, &TableCompareOptions, &Ctl)` loads both through
the VFS in parallel and compares (`bytes_identical` returned beside the result). `TableComparator { formats, options }`
implements `ContentComparator` (method `rules`): it reads both sources (`read_all`, the comparator's `size_limit`,
default 512 MiB), loads with the format its resolver gives for the file name, compares and maps the kind (T-21).

### 5.7 Vocabulary (check 7: parsed in core only)

`ColumnRef::parse`, `parse_column_list`, `Tolerance::parse` (`0.01`, `0.5%`), `parse_date_tolerance` (`30s`, `5m`,
`2h`, `1d`, a bare number = seconds), `ColumnRuleSpec::parse` (`[<cols>=]<value>` for per-column flags), `SheetAlignment::parse`
(`by-name`, `left-name`, `right-name`, `position`), `ColumnAlignment::parse` (`auto`, `number`, `left-name`,
`right-name`), `SheetPair::parse` (`<L>=<R>`, `<L>=`, `=<R>`; a 1-based index or a name), `FirstLine::parse`,
`DateOrder::parse` (`ymd`, `dmy`, `mdy`), `TableKindWord::parse` (`detect`, `delimited`, `fixed`, `spreadsheet`),
`parse_widths` (`10;20;8` or `10,20,8`), `parse_delimiters` (`,`, `;`, `tab`, `\t`, `space`, `pipe`, any one character,
several separated by nothing: `,;`). Words are ASCII case-insensitive, `_` = `-` (as W7's vocabulary).

## 6. Phase 2 — integration (file by file)

Written so that a successor builds it without new decisions; every *Open default* of §1 applies.

### 6.1 `hikasa-formats`

- `model.rs`: `TableSettings` gains (all `#[serde(default)]`, additive): `kind` (`detect` | `delimited` | `fixed` |
  `spreadsheet`), `delimiters: String` (several characters; `delimiter` stays and is the first), `trim_whitespace`,
  `widths: Vec<u32>`, `first_line` (`detect` | `names` | `data`), `regional { decimal, thousands, date_order, date_sep,
  use_system }`, `ctrl_z_eof`. `Conversion` gains `Spreadsheet` (T-22) for Text formats. `toml_io.rs` `TABLE_KEYS`
  grows accordingly. Presets: *Comma Separated Values* (`*.csv;*.tsv`, delimiter `,`, tab for tsv: the preset gets two
  entries or a mask rule — keep D-26's single entry and let `TableFormat::from_extension` pick tab for `*.tsv`),
  *Spreadsheet Workbooks* (`*.xls;*.xlsx;*.xlsm;*.xlsb;*.ods`, kind `spreadsheet`), enabled. `hikasa-formats` does not
  depend on `hikasa-table`; the mapping `TableSettings → TableFormat` lives in `hikasa-session` (`api/table.rs`).

### 6.2 `hikasa-core`

`WarningCode` gains `DuplicateKeys`, `UnterminatedQualifier`, `StrayAfterQualifier`, `SheetSkipped`, `MergedCells`
(informational, only with `--verbose`), `TableAlignmentDegraded` is not needed (`AlignmentDegraded` exists).
`hikasa_table::TableWarningCode` maps one to one (as `DiffWarningCode`, D-… of W2). `JobKind` gains `TableCompare`.

### 6.3 `hikasa-report` — JSON and reports

- New `src/json/table.rs`: `table_compare_json(cmp, &TableViewOptions) -> TableCompareJson`, schema
  `hikasa.table_compare.v1`:
  `{left: SideJson, right: SideJson, format: {left, right: {kind, delimiters, qualifier, consecutive_as_one,
  trim_whitespace, widths, first_line, regional, encoding, detected: bool}}, options: {sheets, compare_formulas},
  ignore_unimportant, verdict: ContentResult, stats: TableStatsJson, sheets: [{name, left: {index, name} | null,
  right, renamed, state, alignment: {mode: "keyed"|"engine"|"unaligned", keys: [names], algorithm}, columns: [{index,
  name, left: {index, letter, name} | null, right, key: n | null, rule: {...}, kind_left, kind_right, hidden}], stats,
  duplicates: {left_rows, right_rows, left_examples: [{key: [text], rows: [label]}], right_examples},
  header: [{column, state, left, right}] | null, display: {filter, toggles, cells: "differing"|"all"},
  rows: [{row, state, importance, left: label | null, right: label | null, cells: [{column, state, left: CellJson |
  null, right: CellJson | null}]}], hidden_rows: n}], warnings}` with `CellJson = {value: display text, type: "number"
  | "text" | "date_time" | "duration" | "bool" | "error" | "empty", formula: text | null}`; `SideJson = {spec, size,
  mtime, source: "delimited"|"fixed"|"xlsx"|…, sheets: n, encoding | null}`. Arrays never `null`; numbers as JSON
  numbers where they are counts, cell values always as display texts (lossless for the file's spelling). Golden in
  `tests/fixtures/json/table_compare.json`; `docs/schemas/table_compare.v1.json` (JSON Schema) added.
- New `src/table/`: layouts **summary** (P0, A10 of the plan; text and HTML: the header of RPT-004, per sheet a block
  of the statistics and the sheet state, totals), **side-by-side** and **interleaved** (TB-025, P1, but cheap on the
  model: side-by-side = one table per sheet, left cells | right cells, row labels when *row numbers*; interleaved = left
  row then right row per changed pair; both honour the display filter, per-sheet sections, colours of RPT-010 per cell).
  `ReportKind::Table` in `request.rs` with the layout words `summary`, `side-by-side`, `interleaved` and the options
  `row-numbers`/`no-line-numbers`, `display-*` (TB-021 filters), `ignore-unimportant`; `render_report` dispatches. The
  script command `table-report` (alias `data-report`) of SCR-001 maps to this request (W8's checker gains the slots).

### 6.4 `hikasa-session`

- `api/table.rs` (new): `TableCompareRequest { left: Spec, right: Spec, format: FormatChoice (auto by name, by id,
  per side), table: TableFormatOverrides (every CLI parser flag), options: TableCompareOptions, ignore_unimportant,
  display: TableViewOptions }`; `Service::table_compare(&TableCompareRequest, &Ctl) -> Result<TableCompareOutcome,
  ServiceError>` with `{comparison: Arc<TableComparison>, bytes_identical, kind, exit: ExitCode, warnings,
  left_format, right_format}`; `Service::table_compare_json(…)`; the D-15 rule: one format for the pair unless per-side
  formats are asked (D-281). The mapping `TableSettings` → `TableFormat`, the A16 locale table (*use system*: the host
  captures `LC_NUMERIC`/`LC_TIME`/`LANG` in `host.rs`, the table maps `de_*` → `,` `.` DMY `.`, `fr_*` → `,` U+202F
  DMY `/`, `en_US` → `.` `,` MDY `/`, `en_GB` → `.` `,` DMY `/`, …, unknown → defaults).
- `api/dto.rs`, `api/human.rs`: the human output (§6.5).
- `session/` (the per-type structs of W2): session type `table-compare` with `[specs] left/right/left_format/
  right_format`, `[rules.table]` = `TableCompareOptions` in TOML (sheet pairs, per-sheet rules with columns, keys,
  rules by column reference, rows), `[view]` = display filter, hidden columns, sheet tab. `session run` executes it.
- Folder content test (T-21): `api/folder.rs` builds the rules comparator as a dispatcher: Table-kind formats →
  `hikasa_table::TableComparator`, else `RulesComparator` of hikasa-text.
- `catalogue/commands.toml`: the Table Compare section's commands stay `planned` until phase 3 flips them.
- **Architecture test** `tests/architecture.rs` (exact edits, verified on this branch: applied, `cargo nextest run -p
  hikasa-session --test architecture` 37/37 passed, reverted; §9.5):
  1. `STUB_CRATES` loses `"hikasa-table"` (now `[&str; 3]`).
  2. `DEPENDENCY_TABLE`: `("hikasa-table", &["hikasa-core", "hikasa-diff", "hikasa-vfs", "calamine", "jiff", "serde",
     "thiserror", "encoding_rs", "chardetng", "rayon", "memchr"], &["proptest", "pretty_assertions", "serde_json",
     "rust_xlsxwriter", "zip", "tempfile"])`; `hikasa-session` gains `hikasa-table` in phase 2, `hikasa-report` too.
  3. `check_3_the_lock_gains_no_package`: 586 → 594 (the eight packages of §9.1).
  3a. `check_4_refuses_exports_without_pub` plants its violation in another stub crate (`hikasa-picture`, with
     `picture_api`), since `hikasa-table` is no stub any more.
  4. Check 2 gains a confinement rule: the token `calamine` appears only under `core/hikasa-table/src/workbook/` (as
     `zip` under `archive/`), and `rust_xlsxwriter` only under `core/hikasa-table/tests/`.
  5. Check 7: `VOCABULARY`/slot parsers gain the §5.7 parsers (`ColumnRef::parse`, `parse_column_list`,
     `Tolerance::parse`, …): adapters never call them; only `hikasa-session`'s `api/` does.

### 6.5 `cli/hikasa-cli` — `hikasa table compare`

`hikasa table compare <left> <right>` with: `--format <id|name>`, `--left-format`, `--right-format`; parser flags
`--type detect|delimited|fixed|spreadsheet`, `--delimiter <d>` (repeatable or a set), `--qualifier <c>|none`,
`--consecutive`, `--trim`, `--widths <w;w;…>`, `--first-line detect|names|data`, `--decimal <c>`, `--thousands
<c>|none`, `--date-order ymd|dmy|mdy`, `--date-sep <c>|any`, `--encoding <label>`, `--ctrl-z`; sheets `--sheets
by-name|left-name|right-name|position`, `--sheet <L>=<R>` (repeatable; implies `custom` with only those pairs);
columns `--columns auto|number|left-name|right-name`, `--align-column <L>=<R>` (repeatable); rules `--key <cols>`,
`--unimportant <cols>`, `--text <cols>`, `--ignore-case [<cols>]`, `--ignore-whitespace [<cols>]`, `--tolerance
[<cols>=]<n>[%]` (repeatable), `--date-tolerance [<cols>=]<n>[s|m|h|d]`, `--hide <cols>`, `--formulas`; rows
`--algorithm unaligned|standard|myers|patience|histogram`, `--skew <n>`, `--closeness`/`--no-closeness`,
`--closeness-threshold <‰>`, `--never-align`, `--sort-rows`; output `--ignore-unimportant` (`--iu`), `--display
all|differences|same|none`, `--cells differing|all` (JSON), the W7 report group (`--report summary|side-by-side|
interleaved`, `--html`, `--output`, `--title`, …), `--json`, `--gui`/`--wait`/`--no-launch`. Rules flags apply to every
sheet; per-sheet rules come from sessions. Default human output: one line per sheet pair (`= Sheet1`, `≠ Data  12
rows differ (3 important cells…)`, `< OnlyLeft`), then the differing rows of each sheet as `row L/R: col: left → right`
lines (at most 20 per sheet unless `--display all`), then the summary line; exit codes T-16. `hikasa quick <a> <b>
--rules` on table files uses `TableComparator`. Help topic `help/table-compare.md`; `docs/12` §1 row and §4 rows;
`docs/reference/commands.md` regenerated. Errors: a column reference naming nothing, a sheet pair naming no sheet, a
bad word → 100 `invalid_input` before loading; an encrypted workbook → 100 `unsupported` with the message *the
workbook is password-protected; Hikasa does not decrypt workbooks*.

### 6.6 `gui/hikasa-gui` — phase 3 (Wave 13)

- `views/table/` (new): the grid over `SheetComparison` (virtualized rows, ARCH-GUI-002): row numbers (`row_label`),
  column headers (`ColumnPair` names, letters), cell colours from `cell_state`/`RowPair` (red important, blue
  unimportant, orphan, replacement), gutter spot per row, thumbnail from the row states, sheet tabs at the bottom with
  `SheetComparison::state` spots (TB-004), Hide Same Columns (`stats` per column: phase 3 adds
  `SheetComparison::column_differs(col)`, computed from `CellDiff`s), Resize Columns to Fit, Text Details (the current
  row's cells: `Sheet::row`), status bar (address `column_label`+`row_label`, `cell_state`, sheet name; Info =
  `stats`).
- Dialogs: Sheets tab (`SheetAlignment`, Custom pairs with Move Up/Down, Tidy), Columns tab (`ColumnAlignment`, the
  per-column rule dialog over `ColumnRule`, Edit Default), Rows tab (`RowAlignment`, keys) — each edits
  `TableCompareOptions` of the session and recompares through the Service job.
- Navigation commands → `nav::*`; display filters → `RowFilter`; Find in cells (phase 3: a `find` over display
  texts, `hikasa_formats::Pattern`).
- Gate: 1M × 10 grid scroll ≥ 60 fps, < 400 MB (plan W13; the engine's model for that input is measured in §9.4).

### 6.7 Documentation of phase 2

`docs/06` (TB-001…TB-013 *M2 notes* pointing here), `docs/10` (the Spreadsheet Workbooks preset, xlsb, the Spreadsheet
text conversion), `docs/12` (§1 `table compare` row; §4 every flag, `hikasa.table_compare.v1`, exit codes), `docs/13`
(Table Compare layouts), `docs/16` (TB-004 moved to M2; M2 progress), `docs/02` (§8 row exists; G9 figures exist),
`CHANGELOG.md`, the help topics `table-compare` and `table-formats`.

## 7. Acceptance

Rules: tests generate their inputs (seeded xorshift, `rust_xlsxwriter`, hand-built minimal zips); committed binaries
stay under 8 KiB each; nothing reads the clock or the environment; every property is checked by mutation (§9.3).

**Phase 1 (engine; met — §9).**

- AT.1 **Delimited parser** — table tests of every §2.2 rule (qualified fields with delimiters, CR, LF, doubled
  qualifiers, stray text after a qualifier, unterminated qualifier, consecutive delimiters with leading and trailing
  runs, trimming, CRLF/CR/LF/mixed, empty lines, Ctrl+Z, BOMs, UTF-16, windows-1252); **P-DELIM** round trip.
- AT.2 **Sniffing** — comma, semicolon with decimal commas, tab, pipe, space-aligned fixed widths, a single column,
  quotes with embedded delimiters; header detection both ways.
- AT.3 **Regional parsing** — numbers (`1,234.5`, `1.234,5` under `,`/`.`, `12%`, `-1e3`, rejected `1,23,4`), dates in
  YMD/DMY/MDY with each separator, 2-digit years, times, AM/PM, ISO with `T`.
- AT.4 **Workbooks** — xlsx generated with `rust_xlsxwriter`: several sheets, numbers, strings (shared), bools, dates
  (formatted serials), formulas with cached values, merged ranges, an error cell; a hand-built xlsx with the 1904 epoch
  and an inline string; ods hand-built (`content.xml`); xlsb/xls when a small valid file can be produced (§9.6);
  encrypted (a compound file with `EncryptedPackage`) and corrupt (truncated zip, random bytes) inputs give their
  errors; a member of a zip archive loads through `hikasa_vfs` (`ArchiveVfs`).
- AT.5 **Memory** — a sparse xlsx using A1 and XFD1048576 loads with two cells; `max_cells` refuses a dense range over
  the limit; the live bytes of a 200k × 20 sheet are reported (§9.4).
- AT.6 **Compare goldens** — sheet pairing in every mode, column alignment in every mode, keyed joins with duplicates,
  unkeyed alignment with inserted/deleted/changed rows, tolerances (absolute, relative, date), case and whitespace
  exceptions, unimportant columns, formulas, CSV against xlsx of the same data (same), stats and the verdict in every
  class; goldens as compact text dumps (`tests/fixtures/golden/*.txt`).
- AT.7 **Properties** — P-SELF `compare(a, a)` is all same (verdict rules-same) for random tables, keyed and unkeyed;
  P-MIRROR swapping sides mirrors every row (left ↔ right, orphans swap) and the statistics; P-KEYORDER a keyed join is
  independent of the input row order (with unique keys: identical results; with duplicates: identical multisets of row
  states); P-DELIM (AT.1).
- AT.8 **Cancellation** — a cancelled token stops load and compare with `Cancelled`, within 100 ms on 1M rows.
- AT.9 **Vocabulary** — each parser of §5.7 accepts its words and refuses others with a message listing the choices.
- AT.10 **Folder comparator** — `TableComparator` on two CSVs with different spellings of the same values gives
  `rules_same`, with a changed value `important_differences`, on a corrupt workbook `error`.
- AT.11 **G9** — §9.4.

**Phase 2.** AT.20 Service and CLI goldens of `hikasa table compare` (human, `--json` golden against the schema, every
exit code 1/2/12/13/100, `--report summary` text and HTML); AT.21 sessions round trip and `session run`; AT.22 folder
compare of a tree with CSV and xlsx files under the Rules method; AT.23 the architecture test with §6.4's edits; AT.24
help and docs checks.

**Phase 3.** AT.30 the grid's view-model tests (catalogue-driven), AT.31 the GUI gate.

## 8. Dependencies

| Crate | Version | Licence | Where | Features | New in `Cargo.lock` |
|---|---|---|---|---|---|
| `calamine` | 0.38.0 | MIT | `hikasa-table` (`src/workbook/` only) | none (`default-features = false`) | see §9.1 |
| `rust_xlsxwriter` | 0.102.0 | MIT OR Apache-2.0 | `hikasa-table` dev (tests only) | none | see §9.1 |

`hikasa-table`'s manifest: `hikasa-core`, `hikasa-diff`, `hikasa-vfs`, `calamine`, `jiff`, `serde`, `thiserror`,
`encoding_rs`, `chardetng`, `rayon`, `memchr`; dev: `proptest`, `pretty_assertions`, `serde_json`, `rust_xlsxwriter`,
`zip`, `tempfile`.

## 9. Phase 1 results

### 9.1 Commits and the lock

Branch `feat/m2-table` (from `feat/m2` 813a08b), commits in order: the dependency (`eaeefb7`), the engine (`66c97ae`),
parser and workbook tests, comparison tests and properties, the G9 harness, the mutation-driven tests, this contract with
`docs/02`, `spreadsheet_to_text`. New packages
in `Cargo.lock` (586 → 594): `calamine` 0.38.0, `atoi_simd`, `byteorder`, `codepage`, `debug_unsafe`, `fast-float2`,
`quick-xml` 0.42.0 (beside 0.41.0: one more `multiple-versions` warning of `cargo deny`), `rust_xlsxwriter` 0.102.0.
`cargo deny check licenses bans sources`: ok, no new exception.

### 9.2 Deviations from §1–§5 found while building

- Column rules are patches (`RulePatch`) on the sheet's default rule, so `--tolerance price=…` and `--ignore-case name`
  combine on one column (§5.1 amended).
- When only one side detected a header (*Detect*), the comparison uses one choice for both sides: a header on both when
  the other side's first row could be that header (no error; it shares a name with the detected header, or it is all
  names, blanks and years with at least one name; review F5), else none (`side_views`). The sheets keep their own
  detection; `SheetComparison::has_header(side)` tells what the comparison used.
- Orphan columns' header cells and a blank orphan row count as differences (header: important unless the column is
  Unimportant; blank row: unimportant).
- Leading columns empty in every row of a workbook sheet are dropped (the origin moves), so a sheet starting at column C
  aligns with a CSV by number.
- xlsb merged regions are not exposed by calamine (empty); ods neither.

### 9.3 Mutations

Each hand mutation was applied alone, the whole `hikasa-table` suite (default proptest cases) was run, and the file was
restored (`/var/tmp/hikasa-work/table/mut/run.py`).

| # | File | Mutation | Caught by |
|---|---|---|---|
| M1 | `delimited/parse.rs` | a doubled qualifier is not unescaped | P-DELIM, `qualified_fields_hold_delimiters_line_ends_and_doubled_qualifiers` |
| M2 | `delimited/parse.rs` | a trailing delimiter (`a,`) drops its empty field | **equivalent**: ragged rows never store a trailing empty cell, so no observable change |
| M3 | `delimited/write.rs` | the writer does not double qualifiers | P-DELIM |
| M4 | `delimited/parse.rs` | consecutive delimiters: a trailing run opens an empty field | **equivalent** (as M2) |
| M5 | `regional.rs` | the 2-digit year pivot moves from 30 to 20 | `regional::tests::dates_and_times` |
| M6 | `regional.rs` | DMY dates read as MDY | `dates_and_times`, `regional_settings_type_the_columns` |
| M7 | `workbook/mod.rs` | the 1904 epoch's base date is 1900-01-01 | `hand_built_xlsx_1904_epoch_inline_strings_and_errors` |
| M8 | `workbook/mod.rs` | the 1900 leap-bug boundary `d < 60` becomes `d < 61` | `hand_built_xlsx_1904_epoch_inline_strings_and_errors` (a first variant, `d == 60` → `d == 59`, was equivalent: both branches give 1900-02-28) |
| M9 | `compare/cells.rs` | a value within tolerance counts as same, not unimportant | `typed_equality_and_tolerances`, `tolerances_exceptions_and_unimportant_columns` |
| M10 | `compare/cells.rs` | the key encoding of numbers is not order-preserving | `encoding_preserves_order` |
| M11 | `compare/rows.rs` | the leftovers of a duplicate key group are dropped | `keyed_join_with_duplicates_in_any_order` |
| M12 | `compare/rows.rs` | an unstable key sort (duplicates out of order) | `duplicates_keep_their_order_whatever_the_interleaving` (added after it survived; the all-equal-key test missed it, pdqsort leaves equal runs in place) |
| M13 | `compare/sheet.rs` | values of a left-only column are ignored | `values_of_a_one_sided_column_are_differences` (added after it survived) |
| M14 | `compare/columns.rs` | the case-insensitive name pass is dropped | `sheets_pair_by_name_then_position`, `sheet_and_column_alignment_modes` |
| M15 | `compare/result.rs` | unimportant sheets are ignored by the verdict | `tolerances_exceptions_and_unimportant_columns` |
| M16 | `compare/sheet.rs` | a formula difference is never applied | `formulas_count_only_when_asked` |
| M17 | `header.rs` | the header vote of typed bodies is inverted | 14 tests (every golden with a header) |
| M18 | `compare/columns.rs` | header harmonization drops both headers | `one_detected_header_is_used_on_both_sides` (added after it survived) |
| M19 | `compare/cells.rs` | texts always compare case-insensitively | `keyed_join_with_duplicates_in_any_order` |
| M20 | `workbook/mod.rs` | streamed xlsx columns shift by one | `generated_xlsx_with_every_cell_kind`, `merged_regions_of_a_hand_built_sheet` |

18 effective mutations, all caught; 2 equivalent. The first campaign also exposed two faulty properties (P-MIRROR
asserted cell counts for unkeyed alignments, whose gap pairing prefers the left side; P-KEYORDER treated `3` and `3.0`
as different keys): both were corrected, their seeds stay in `tests/props.proptest-regressions`.

### 9.4 Gate G9 (engine)

Release build, `tests/gates.rs` under `systemd-run --user --scope -p MemoryMax=8G`, one process per row, peak RSS from
`VmHWM`; the machine had other builds running. Figures also in `docs/02`.

| Row | Input | Time | Peak RSS | Threshold |
|---|---|---|---|---|
| keyed | 1,000,000 × 10 CSV pair, 164 MB in all, key `id` | 0.664 s | 497 MiB = 3.1 × input | < 5 s, ≤ 4 × input: **met** |
| unkeyed | same pair, Standard | 1.313 s | 602 MiB = 3.8 × input | met |
| xlsx keyed | 200,000 × 20 pair, 58 MB in all, key `col0` | 2.513 s | 161 MiB | reported |
| xlsx unkeyed | same pair | 2.103 s | 167 MiB | reported |

### 9.5 Checks run on the branch

On `feat/m2-table` (worktree), 2026-11-09: `cargo fmt --all --check` ok; `cargo clippy -p hikasa-table --all-targets`
with `RUSTFLAGS='-D warnings'` ok, also `--target x86_64-pc-windows-gnu`; `cargo nextest run -p hikasa-table` 44 run, 44
passed, 5 skipped (the `#[ignore]` gate rows); `PROPTEST_CASES=4000 cargo test --release -p hikasa-table` ok; `cargo
deny check licenses bans sources` ok. The architecture test with the §6.4 edits applied (then reverted): 37/37; without
them it fails on checks 3 and 4 (`hikasa-table` exports items and has dependencies, the lock has 594 packages).  `cargo
clippy --workspace --all-targets` with `-D warnings`: ok on Linux and on `x86_64-pc-windows-gnu`. The other workspace
tests were not run: the lock change only adds packages, which no other crate uses.

### 9.6 xls and xlsb fixtures

No tool on the build machine writes xls or xlsb (no office suite, no Python writer), and a hand-built BIFF8 compound file
was not attempted in the time; the xls and xlsb paths are exercised only by their error cases (a compound file that is no
workbook, an encrypted package). Open item: a small committed xls/xlsb pair (< 8 KiB each) made with any office suite.

### 9.7 Open items

- xls/xlsb positive fixtures (§9.6).
- ods dense ranges are allocated by calamine before `max_cells` can refuse them.
- The architecture test (§6.4 edits; the diff is in the handover folder as `architecture.patch`) is not applied on this
  branch: it is a `hikasa-session` change, so `cargo nextest run -p hikasa-session --test architecture` fails here on
  checks 3 and 4 until phase 2 applies the edits; the check-2 confinement and check-7 vocabulary rules are still to write.
- Phase 2 and phase 3 (§6).
