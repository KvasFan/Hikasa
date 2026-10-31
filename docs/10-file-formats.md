# File Formats and Grammars

A **file format** tells Hikasa how to treat a kind of file: which view opens it,
how to convert it before comparing/saving, how to tokenize it (grammar) and small
editing preferences. Formats are user-editable and exportable; built-in presets are
provided.

## 1. Model

```
FileFormat {
  id, name, description, enabled, kind: Text|Table|Hex|Picture|Media|Version|Registry|External,
  mask: "*.c;*.h",             // semicolon list; same mask syntax as name filters (names only)
  conversion: Conversion,      // §4
  // Text/Table:
  encoding: Auto|Fixed(codepage), ctrl_z_eof: bool, chars_per_line_limit: Option<u32>,
  save_trim_trailing_ws: bool, save_leading_spaces_to_tabs: bool,
  // Text:
  grammar: Vec<GrammarItem>,   // §3, ordered, first match wins
  line_weights: Vec<{ find_text, is_regexp, priority: 1..4 }>,
  tab_size: u32, tab_as_spaces: bool, lines_independent: bool, column_based: bool,
  element_case: Map<ElementName, bool>,   // case sensitivity per element
  // Table:
  table: { detect: bool, delimited: {delimiters, qualifier, consecutive_as_one, surrounding_ws_is_delimiter}, fixed: {widths}, first_line: Detect|Names|Data, regional: {...} }
  // External:
  external: { quick_compare_cmd, compare_view_cmd, quick_merge_cmd, merge_view_cmd }
}
```

- **FF-001** (P0): **File Formats** dialog (Tools): ordered list with enable
  checkboxes, Move Up/Down, New (per kind), Clone, Edit, Delete, Import/Export. The
  first *enabled* format (top to bottom) whose mask matches the file name wins. Items
  shadowed by an earlier item are shown in red; disabled in grey; empty mask = manual
  selection only. "Everything Else" is the implicit default text format.
- **FF-002** (P0): Folder sessions can override the enabled state of formats
  (Rules ▸ Misc ▸ Enabled file formats; bold when different from global).
- **FF-003** (P0): File views show the detected format ("Detected (C/C++ Source)") in
  the File Info header; the user can pick another format for a pane or all panes;
  the session's Format tab stores per-side overrides and encoding overrides.
- **FF-004** (P0): Formats are stored one per TOML file; built-ins are embedded in
  the binary and written to the config dir on first run (so they can be edited; "Restore
  Factory Defaults" restores them). Changes on disk are detected ("File Format(s)
  changed on disk. Reopen?").

### 1.1 Format files (TOML, M1)

Each format is one TOML file, `formats/<id>.toml` in the configuration folder, with
`schema_version = 1`. The built-in presets are the same files, embedded in the binary
(`core/hikasa-formats/presets/`, FF-040). A file written by a newer version
(`schema_version` greater than supported) is an error; an unknown key is a warning
(`unknown_config_key`), never silently dropped.

```toml
schema_version = 1
id = "c-cpp"                      # [a-z0-9][a-z0-9-]{0,63}; also the file stem
name = "C, C++, C#, ObjC Source"
kind = "text"                     # text | table | hex | picture | media | version | registry | external
mask = "*.c;*.cc;*.h"             # names only, always case-insensitive (D-10)

[conversion]
method = "none"                   # none | sorted | external (§4.1)

[encoding]
mode = "auto"                     # auto (optional fallback = "<label>") | fixed (label = "<label>")

[text]
tab_size = 4
word_chars = ""                   # extra word characters for List items (D-58)

[elements]                        # per-element settings; an unordered map
Number = { case_sensitive = false, important = true }

[[grammar]]                       # ordered: first match wins (FF-026)
element = "Comment"
kind = "delimited"
from = "/*"
to = "*/"

[[line_weight]]
pattern = '^\s*object'
regex = true
priority = 4
```

- **Masks (D-10).** File-format masks match the file **name** only (anything up to the
  last `/` or `\` is ignored) and are always case-insensitive, so `*.C` (C++ on Unix)
  and `MAIN.C` are both detected as C. A mask item with a separator is invalid in a
  format. Exclude items (`-test_*`) skip the format for matching names. Detection takes
  the first enabled format whose mask has a matching include item and no matching
  exclude item; no match gives "Everything Else".
- **Archive members (M2 W5).** A member is detected by its own name (`a.zip/src/x.c` is C).
- **Standard input and the clipboard (D-15, D-100).** `-` is detected by its `--stdin-name`
  when given, else by content. `clipboard://` text has no name and is detected by content
  (usually "Everything Else" text); a clipboard image is a PNG named `clipboard.png`, so it is
  detected by that name (the picture formats of W12).
- **Shadowing (FF-001).** A format is reported as shadowed (red) only when every include
  item of its mask is provably covered by an include item of an earlier enabled format
  that has no exclude items (`*` and `*.*` cover everything, `*.c` covers `test*.c`,
  `make*` covers `makefile`, identical items cover each other). The test is sound: a
  red format can never be detected; it may miss rare cases.
- **Element ids (D-41).** Elements are numbered densely: 0 is "Everything Else", then
  the element names in order of first use in the `[[grammar]]` array, then names that
  are declared in `[elements]` but never used, in lexicographic order. `[elements]` is
  an unordered settings map; its key order never matters. The name "Everything Else" is
  reserved.
- **Element settings.** `case_sensitive` (FF-025, default true) says whether case
  changes inside the element are important. `important` (default true) is the
  element's default importance; session rules override it by name. The built-ins ship
  two unimportant elements: Python `Whitespace` (trailing whitespace) and COBOL
  `Identification` (columns 73–80).
- **Word characters (D-58).** `word_chars` lists extra characters that count as word
  characters for List items (word boundaries and keyword lookup), in addition to
  letters, digits and `_`. COBOL uses `"-"`: `END-IF` is one keyword and `END-OF-FILE`
  one identifier.
- **Enabled flags and order.** The enabled flag is not part of a format file. The
  built-in default flags live with the presets; the user's order and flags live in
  `formats/order.toml` (`schema_version = 1`, `[[format]] id = "…" enabled = true`).
  A user file with a built-in's id replaces that built-in in place; new formats are
  appended sorted by id; ids listed in `order.toml` come first, in that order (unknown
  ids give a warning). Folder sessions can still override the flags (FF-002).
- **Validation.** Loading checks the shape; compiling a format checks the rest: tab
  size 1–32, columns ≥ 1 and `to_column ≥ from_column`, weight priorities 1–4, non-empty
  patterns and tokens, every regex, the mask, the encoding labels and the external
  command templates. A broken user format is skipped with a warning
  (`user_format_skipped`): a file that does not parse, whose id is not its file name, or
  whose mask is invalid never costs the built-ins or the other formats. A broken format
  that is selected reports its error instead of being silently replaced.
- **Changing formats from the command line (M2, FF-001; D-75 revises D-21).** The
  `hikasa format` write commands touch only the files they change, `formats/<id>.toml`
  and/or `formats/order.toml`, and never copy the other built-ins into the folder
  (the GUI's first start still materializes them, D-21):
  - `new <id> [--from <id|name>] [--name] [--mask] [--kind]` writes a new user format,
    a copy of `--from` (named `<name> (copy)` unless `--name` is given) or an empty
    format of `--kind` (default `text`), and appends it to `order.toml` as enabled
    (`order.toml` is first created from the current order). An id that a built-in or a
    user format already has, or whose file `formats/<id>.toml` exists although it could
    not be loaded, is `already_exists`; when `order.toml` cannot be written the new file
    is removed again. The id `order` (in any case) is reserved for `formats/order.toml`:
    `new`, `import` and `edit` refuse it (`invalid_input`).
  - `edit <id> (--from-file <path> | --set <key=value>…)`: `--from-file` copies the file
    (its `id` must be `<id>`); `--set` applies TOML key paths to the canonical TOML of
    the format (a value that is not TOML is a string: `mask=*.x;*.y`) and writes the
    canonical TOML. An unknown key, a result that does not parse or does not compile is
    refused (`invalid_input`) and nothing is written. Editing a built-in writes its
    override file.
  - `delete <id>` removes a user format's file and its `order.toml` entry. A built-in,
    or a user file overriding one, cannot be deleted: disable it, or `restore` it.
  - `import <path> [--replace]` validates a format file and copies it to
    `formats/<id>.toml`; an id that exists (or a file `formats/<id>.toml` that could not
    be loaded) needs `--replace`.
  - `export <id|name> [-o <path>]` prints the canonical TOML or writes it atomically.
  - `enable`, `disable` and `move (--up|--down|--top|--bottom|--before <id>|--after <id>)`
    write `order.toml` only. When it does not exist it is created from the current
    order; when it exists, ids it lists that no format has are dropped and the formats
    it does not list are appended in their current order, so the effective order does
    not change except for the move (*interpretation*).
  - `restore (<id>… | --all)` removes the user files that override the named built-ins
    (each id must be a built-in), or with `--all` every user format file and
    `order.toml`. Like `settings restore-defaults` it is a plan: `--dry-run` prints it,
    otherwise `--yes` is required.

## 2. Format kinds and tabs

| Kind | Tabs |
|------|------|
| Text | General (name, mask, description), Conversion, Grammar, Misc |
| Table | General, Conversion, Type, Regional |
| Hex | General, Conversion |
| Picture | General, Conversion |
| Media / Version / Registry | General |
| External | General (mask, quick compare/merge and view command lines, description) |

- **FF-010** (P1): **External formats** delegate to another program: *Quick compare
  command line* (exit 0 = same, 1 = different) used for rules-based folder tests,
  *Compare view command line*, *Quick merge*, *Merge view*. Variables as in Open With
  (`%f %n %p %x %b %F %P` with `1`/`2`/`3` suffixes).

## 3. Grammar items (Text formats)

Each item has an **element name** (free text; built-ins: Comment, String, Keyword,
Identifier, Number, Operator, Directive, EnvironmentVar, LineNum, Indent, Tag…) and a
category:

- **FF-020** (P0): **Basic** — *Text matching* string, match case, regular expression.
- **FF-021** (P0): **Delimited** — *Text from* / *to* delimiters (or *Stop at end of
  line*), *Escape character*, match case, regular expressions. Example: string literal
  `'` … `'` with escape `\`.
- **FF-022** (P0): **List** — a list of tokens (one per line), match case, regular
  expressions; tokens match whole words.
- **FF-023** (P1): **Columns** — *Text from column* / *to column* or *to end of line*
  (fixed-position data such as COBOL sequence numbers).
- **FF-024** (P1): **Lines** — *Text matching* a heading line and the *next N lines*,
  *or line 1* (repeating page headings in reports).
- **FF-025** (P0): Per item: *This element is case sensitive* (changes in case within
  this element are important).
- **FF-026** (P0): Tokenizer semantics: scan each line left to right; at each position
  try items in list order, first match wins, consume the match; delimited items may
  span lines unless Stop at EOL; text not covered by any item is "Everything else".
  Nested/context-sensitive grammars are **not** supported (simple by design; HTML/PHP
  embedded languages tokenize imperfectly).
- **FF-027** (P0): Tokens feed (a) syntax colouring (element → style from the theme,
  with syntect fallback when a format has no grammar), (b) importance classification
  per session rules, (c) the "grammar element under cursor" status.
- **FF-028** (P0): The regex insert helper offers: Tab `\t`, any char `.`, `\w`, `\W`,
  `\d`, `\D`, `\s`, `[ ]`, `[^ ]`, `^`, `$`, form feed `\xc`, `|`, `*`, `+`, `?`.

**Misc tab**: Insert spaces instead of tabs, Tab stop, *Lines are independent* (no
sections; Next Difference moves line by line — e.g. data files), *Column-based data*
(compare column by column).

**Line weights**: regex patterns with priority 1–4; higher-weight matching lines are
preferred anchors in alignment (e.g. `^\s*object` in form files, `=\s*class\s*\(`
in Pascal).

### 3.1 Tokenizer semantics (M1)

- A line is its content without the line terminator. At each position the items are
  tried in list order; the first one that matches wins and its token is consumed. A
  zero-length match counts as no match. Text not covered by any item is "Everything
  Else" (adjacent runs are merged).
- **Basic** matches at the scan position (literal, or a regex whose match starts
  there). **Delimited** runs from `from` to `to`; an `escape` character skips the next
  character, except that when `to` is a literal starting with the escape character
  and that character is not doubled, it closes (so `'it''s'` and `"a\"b"` both work);
  without `to`, or with *Stop at end of line*, it ends at the end of the line;
  otherwise it continues on the next lines. **List** tokens must match whole words
  (a token that starts or ends with a word character needs a word boundary there) and
  the longest matching token wins; plain word tokens are looked up in a hash set.
  **Columns** count characters (1-based, inclusive). **Lines** start at a line
  containing the heading (or at line 1 with *or line 1*) and cover the next N lines.
- **Columns items are fields.** The first column of a Columns item is a boundary that
  no other token crosses: every item sees the text before the boundary as the whole
  line, so a string, an identifier or a comment that would run past it ends there (an
  item that runs to the end of the line ends at the boundary, without the blanks before
  it), and the Columns item takes its own columns. In COBOL this keeps columns 73–80
  the (unimportant) Identification area even when a literal or a name reaches column 72.
  A multi-line construct cut at a boundary still continues on the next line.
- **Whitespace at the edges (D-13).** A token that continues a multi-line construct
  leaves the line's leading whitespace outside the token, and a token that ends because
  the line ends leaves the trailing whitespace outside: re-indenting a block comment
  stays a whitespace-only change.
- A line's tokens depend only on the line, its number and the state carried over from
  the previous line, so any line can be re-tokenised on its own from a stored state.
  Large documents are tokenised in parallel chunks of 8192 lines that start
  speculatively in the normal state and are corrected afterwards; the result is
  identical to a sequential run.
- **Regular expressions.** Patterns are compiled with the linear-time `regex` engine;
  only patterns it rejects (backreferences, look-around) use the backtracking
  `fancy-regex` engine, with a step budget (exceeding it counts as "no match" and is
  reported as warning `regex_runtime_error`). Patterns see the whole line, so `^`, `$`
  and `\b` refer to the real line. Grammar authors should prefer linear-time forms.
- **Long lines.** With the built-in grammars, tokenising a line takes time proportional
  to its length, also for lines that a pattern would otherwise re-examine from every
  position (a long run of blanks, an opening `${` or `#[` that is never closed, many
  strings that contain a comment keyword). A user pattern that must look far ahead
  before failing can still be slow on very long lines. A cancel is noticed inside a
  line too, so a single multi-megabyte line cannot delay it.
- **Grammars are simple by design.** Items are not context-sensitive: an HTML or XML
  attribute value that spans lines is not recognised as a string, a Bash here-document
  is not recognised (an apostrophe in it opens a string that runs to the next
  apostrophe), and embedded languages tokenize imperfectly. The token goldens of each
  preset show these limits.

## 4. Conversions

- **FF-030** (P0): Conversion methods: *None* (plain text); *External program* for
  loading (`%s` source, `%t` target, `%o` original; filename encoding Unicode/ANSI;
  success = exit 0 and non-empty target) and optionally for saving (when "Disable
  editing" is unchecked); built-in converters: **Spreadsheet text** (xlsx/xls/ods → TSV
  per sheet), **Word-processor text** (docx/odt/odp → plain text), **RTF**, **PDF
  text** (P1), **HTML tables** (P1), **Sorted** (sort lines before comparing),
  **Form text** (binary form resources → text, P2), **Trimmed/Normalized** (P2).
- **FF-031** (P0): Converted content is read-only unless a save conversion exists; the
  File Info header shows "Converted".
- **FF-032** (P0): Encoding: Auto (BOM, detection, fallback) or a fixed code page from
  the supported list (Western/Central European/Baltic/Cyrillic/Greek/Turkish/Hebrew/
  Arabic/Vietnamese/Thai Windows and ISO variants, DOS code pages, KOI8-R/U, Shift-JIS,
  EUC-JP, GB2312, GB18030, HZ, Big5, Korean, UTF-8, UTF-16 LE/BE, UTF-32, Current
  Locale).

### 4.1 Conversions in M1

M1 implements *None*, *Sorted* and *External program*; the built-in document
converters are later items.

- **Sorted (D-38).** Lines are sorted by Unicode code point (a stable sort). Every
  output line gets the document's dominant line ending, and the document ends with a
  line ending only if the original did. A sorted document is read-only.
- **External programs (D-39).** Command lines are split into arguments by Hikasa's own
  rules and the program runs **without a shell**: arguments are separated by unquoted
  spaces or tabs; `"…"` groups an argument (the quotes are removed, `""` inside quotes is
  a literal `"`); a backslash is an ordinary character, so Windows paths survive. `%s`
  (the temporary copy of the source), `%t` (the file the program must write) and `%o`
  (the original location, for display) are substituted after splitting, so a path with
  spaces or quotes is always one argument; `%%` is a literal `%`, any other `%x` is an
  error. The program gets no standard input and its output is discarded; success means
  exit status 0 **and** a non-empty `%t` file. The program is killed on cancellation or
  after `timeout_secs` (default 60, 0 = no limit), and the temporary files are removed
  in every case. Without a save command line the converted document is read-only
  (FF-031).
- **Temporary files.** Each conversion creates its own private folder in the temporary
  folder (exclusively, owner-only on Unix, with a hard-to-guess name), holding the
  source copy (`%s`) and the target (`%t`); nothing that already exists there is
  followed or reused. The folder is removed after the program has ended, on every path.
  Only the program itself is killed on a timeout or cancel; programs it started keep
  running until they end (they can no longer write the removed folder).

### 4.2 Encoding detection (D-42)

For each side, in this order:

1. the per-pane or command-line override (a byte-order mark of that same encoding is
   still stripped);
2. a byte-order mark (UTF-8, UTF-32LE, UTF-32BE, UTF-16LE, UTF-16BE; UTF-32LE is tested
   before UTF-16LE because their marks share a prefix);
3. the format's fixed encoding;
4. with Auto: a BOM-less UTF-16 or UTF-32 byte pattern in the first 4096 bytes (mostly
   zero bytes in alternate positions); then, for pure ASCII content, the format's
   fallback encoding, else the current locale's encoding, each only if it is
   ASCII-compatible, else UTF-8; then UTF-8 if the content is valid UTF-8; then UTF-8
   with replacement characters if the content is *mostly* valid UTF-8 (at least one
   valid multi-byte character, at most 16 malformed sequences, and at least 8 valid
   multi-byte characters per malformed one: a stray byte must not turn every accented
   letter of the file into mojibake); then the statistical detector's guess (it never
   guesses UTF-8 or ISO-2022-JP).

The BOM-less pattern test must come before the ASCII and UTF-8 tests because ASCII
text in UTF-16 or UTF-32 is also valid UTF-8. It recognises ASCII-range UTF-16/UTF-32
(zero bytes in alternate positions) and BOM-less UTF-16 text in one non-Latin alphabet
such as Cyrillic, Greek, Hebrew or Arabic (a constant high byte mixed with ASCII), and
only when the decoded sample is plausible text (at most 10 % control characters), so
arrays of small 16- or 32-bit numbers stay binary. BOM-less UTF-16 in scripts that
spread over many high bytes (Chinese, Japanese, Korean) is not recognised: such files
need a byte-order mark or an encoding override. A format's fallback encoding therefore
applies only to content without non-ASCII bytes. Malformed sequences decode to U+FFFD
and are counted (warning `lossy_decode`). Supported encodings are those of the WHATWG
Encoding Standard plus UTF-32LE/BE (`utf-32` without a suffix means UTF-32LE, as
`utf-16` means UTF-16LE); labels that cannot be decoded (for example `hz-gb-2312`, or
DOS code pages other than 866) are rejected as unknown.

### 4.3 Binary detection

Content is classified from its first 8192 bytes: empty content and content with a
byte-order mark are text; a known file signature at offset 0 is binary; a BOM-less
UTF-16/UTF-32 pattern of plausible text (the same test as encoding detection) is text; a NUL byte is
binary; more than 10 % control bytes (C0 except tab, line feed, vertical tab, form
feed, carriage return, backspace, Ctrl-Z and escape, plus DEL) is binary; anything else
is text. The signatures (an in-house table):

| Kind | Signature at offset 0 |
|------|-----------------------|
| PNG | `89 50 4E 47 0D 0A 1A 0A` |
| GIF | `GIF87a` or `GIF89a` |
| JPEG | `FF D8 FF` |
| PDF | `%PDF-` |
| ZIP (also docx, xlsx, jar, …) | `50 4B 03 04`, `50 4B 05 06` or `50 4B 07 08` |
| gzip | `1F 8B` |
| ELF | `7F 45 4C 46` |
| 7z | `37 7A BC AF 27 1C` |
| xz | `FD 37 7A 58 5A 00` |
| Zstandard | `28 B5 2F FD` |

Binary content does not stop a text comparison: the text is decoded (lossily if
needed) and the comparison reports it as binary.

## 5. Built-in presets (initial set)

| Name | Kind | Mask | Grammar elements / notes |
|------|------|------|--------------------------|
| Bash Scripts | Text | `*.sh;*.bash;*.zsh` | Comment (`#`→EOL), String (`'…'`, `"…"` esc `\`, `$'…'`), EnvironmentVar (`$name`, `${…}`, `$?`…), Keyword list, Identifier, Number, Operator; tab 4 |
| C, C++, C#, ObjC Source | Text | `*.c;*.cc;*.cpp;*.cxx;*.cs;*.h;*.hpp;*.hh;*.m;*.mm` | Comment (`//`, `/* */`), String (`"…"` esc `\`, `'…'`, `@"…"`), Directive (`#…`), Keyword, Identifier, Number (dec/hex/float), Operator; tab 4 |
| COBOL Source | Text | `*.cbl;*.cob;*.cpy` | LineNum (columns 1–6 `^\d{6}`), Comment (`*` in col 7, up to its last non-blank character), column 73–80 ignored (Columns item: a field no other token crosses), String, Keyword, Identifier, Number |
| Object Pascal Forms | Text | `*.dfm;*.fmx` | Keyword, Identifier, Number, String, Operator; line weight `^\s*object` priority 4; tab 2 spaces |
| Object Pascal Source | Text | `*.pas;*.dpr;*.dpk;*.inc` | Comment (`//`, `{ }`, `(* *)`), Directive (`{$…}`), String, Keyword (case-insensitive), Identifier, Number (`$hex`), Operator; weight `=\s*class\s*\(` |
| HTML | Text | `*.htm;*.html;*.xhtml;*.asp;*.aspx;*.ascx` | Comment (`<!-- -->`), String (attribute values after `=`), Keyword (tag names after `<`/`</`, attribute names before `=`; case-insensitive; words in running text are never keywords), Operator, entities `&#\d+;` |
| Java Source | Text | `*.java;*.jav;*.kt` | as C |
| JavaScript Source | Text | `*.js;*.mjs;*.ts;*.jsx;*.tsx` | as C + regex literal `/…/flags`; tab 2 spaces |
| Perl Scripts | Text | `*.pl;*.pm` | Comment `#`, POD blocks (`^=head…`→`^=cut`), String, Keyword, Identifier, Number, Operator |
| Python Scripts | Text | `*.py;*.pyw` | Comment `#` (up to its last non-blank character), String (`'`, `"`, `'''`, `"""`), Indent (`^\s+`), Keyword, Identifier, Number, Operator, trailing whitespace item (unimportant by default, also after a comment) |
| Registry Dumps / INI | Text | `*.reg;*.ini` | Comment (`;`), String, Keyword (`[section]`, value names), Operator |
| SQL | Text | `*.sql;*.dsql` | Comment (`--`, `/* */`), String (`n'…'` with `''`), Keyword (case-insensitive), Identifier (`@`/`#` prefixes), Number, Operator |
| Visual Basic Source | Text | `*.bas;*.cls;*.frm;*.vb;*.vbs;*.ctl;*.vbp` | Comment (`'`, `rem`), String (`"…"` with `""`), Keyword (case-insensitive), Identifier, Number, Operator |
| XML | Text | `*.xml;*.xsd;*.xsl;*.xslt;*.svg;*.plist` | Comment, CDATA, Directive (`<?…?>`, `<!…>`), String (attribute values after `=`), Entity, Operator |
| Rust Source | Text | `*.rs` | Comment (`//`, `/* */` nesting approximated), String (`"…"`, raw `r#"…"#`), Char, Keyword, Identifier, Number, Operator, Attribute (`#[…]`) |
| Go, TOML, YAML, JSON, Markdown, Shell-like configs | Text | respective masks | reasonable grammars (P1) |
| RTF | Text | `*.rtf` | conversion RTF → text |
| Word-processor documents | Text | `*.docx;*.docm;*.dotx;*.odt;*.ott;*.odp` | conversion → text |
| PDF Documents | Text | `*.pdf` | conversion → text (P1) |
| Sorted | Text | (empty mask) | conversion: sort lines |
| Comma Separated Values | Table | `*.csv;*.tsv` | delimiter `,` (tab for tsv), qualifier `"`; the former `*.txt?` entry was a typo and is dropped (D-26) |
| Spreadsheet Workbooks | Table | `*.xls;*.xlsx;*.xlsm;*.ods` | conversion: spreadsheet, all sheets |
| HTML Tables | Table | `*.htm;*.html` (disabled by default) | conversion: HTML tables (P1) |
| Picture Files | Picture | `*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.ico;*.tif;*.tiff;*.webp;*.tga;*.pnm;*.qoi;*.exr;*.hdr;*.dds` | |
| Media Files | Media | `*.mp3;*.mp4;*.m4a;*.m4b;*.m4p;*.m4v;*.flac;*.wav;*.ogg;*.opus` | |
| Executables | Version | `*.exe;*.dll;*.ocx;*.sys;*.drv;*.scr;*.bpl;*.so;*.dylib` | |
| Registry Files | Registry | `*.reg` | **disabled by default** (D-27): "Registry Dumps / INI" comes first with the same mask |

- **FF-040** (P0): Keyword lists for the presets are maintained as data files under
  `core/hikasa-formats/presets/` (TOML), not in code.
  Each preset file starts with a `# Sources:` comment naming the public language
  specifications its keyword lists were written from. M1 ships 21 presets, in this
  order: Bash Scripts, C/C++/C#/ObjC Source, COBOL Source, Object Pascal Forms, Object
  Pascal Source, HTML, Java Source, JavaScript Source, Perl Scripts, Python Scripts,
  Registry Dumps / INI, SQL, Visual Basic Source, XML, Rust Source, Sorted, Comma
  Separated Values, Picture Files, Media Files, Executables, Registry Files (disabled).
  The presets that need a document converter (RTF, word-processor documents, PDF,
  spreadsheets, HTML tables) and the P1 grammars come later. In M1, table formats are
  compared as plain text and picture, media, version and registry formats by content
  bytes. Each preset has at least two token goldens (a typical file and a tricky one)
  under `core/hikasa-formats/tests/fixtures/tokens/`; they document its behaviour,
  including the known limits of a simple grammar (for example quotes in running text).
- **FF-041** (P1): Additional formats can be imported from community packages
  (settings package containing only formats).
