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

## 5. Built-in presets (initial set)

| Name | Kind | Mask | Grammar elements / notes |
|------|------|------|--------------------------|
| Bash Scripts | Text | `*.sh;*.bash;*.zsh` | Comment (`#`→EOL), String (`'…'`, `"…"` esc `\`, `$'…'`), EnvironmentVar (`$name`, `${…}`, `$?`…), Keyword list, Identifier, Number, Operator; tab 4 |
| C, C++, C#, ObjC Source | Text | `*.c;*.cc;*.cpp;*.cxx;*.cs;*.h;*.hpp;*.hh;*.m;*.mm` | Comment (`//`, `/* */`), String (`"…"` esc `\`, `'…'`, `@"…"`), Directive (`#…`), Keyword, Identifier, Number (dec/hex/float), Operator; tab 4 |
| COBOL Source | Text | `*.cbl;*.cob;*.cpy` | LineNum (columns 1–6 `^\d{6}`), Comment (`*` in col 7), column 73–80 ignored (Columns item), String, Keyword, Identifier, Number |
| Object Pascal Forms | Text | `*.dfm;*.fmx` | Keyword, Identifier, Number, String, Operator; line weight `^\s*object` priority 4; tab 2 spaces |
| Object Pascal Source | Text | `*.pas;*.dpr;*.dpk;*.inc` | Comment (`//`, `{ }`, `(* *)`), Directive (`{$…}`), String, Keyword (case-insensitive), Identifier, Number (`$hex`), Operator; weight `=\s*class\s*\(` |
| HTML | Text | `*.htm;*.html;*.xhtml;*.asp;*.aspx;*.ascx` | Comment (`<!-- -->`), String, Keyword (tags/attributes list), Operator, entities `&#\d+;` |
| Java Source | Text | `*.java;*.jav;*.kt` | as C |
| JavaScript Source | Text | `*.js;*.mjs;*.ts;*.jsx;*.tsx` | as C + regex literal `/…/flags`; tab 2 spaces |
| Perl Scripts | Text | `*.pl;*.pm` | Comment `#`, POD blocks (`^=head…`→`^=cut`), String, Keyword, Identifier, Number, Operator |
| Python Scripts | Text | `*.py;*.pyw` | Comment `#`, String (`'`, `"`, `'''`, `"""`), Indent (`^\s+`), Keyword, Identifier, Number, Operator, trailing whitespace item |
| Registry Dumps / INI | Text | `*.reg;*.ini` | Comment (`;`), String, Keyword (`[section]`, value names), Operator |
| SQL | Text | `*.sql;*.dsql` | Comment (`--`, `/* */`), String (`n'…'` with `''`), Keyword (case-insensitive), Identifier (`@`/`#` prefixes), Number, Operator |
| Visual Basic Source | Text | `*.bas;*.cls;*.frm;*.vb;*.vbs;*.ctl;*.vbp` | Comment (`'`, `rem`), String (`"…"` with `""`), Keyword (case-insensitive), Identifier, Number, Operator |
| XML | Text | `*.xml;*.xsd;*.xsl;*.xslt;*.svg;*.plist` | Comment, String, Operator |
| Rust Source | Text | `*.rs` | Comment (`//`, `/* */` nesting approximated), String (`"…"`, raw `r#"…"#`), Char, Keyword, Identifier, Number, Operator, Attribute (`#[…]`) |
| Go, TOML, YAML, JSON, Markdown, Shell-like configs | Text | respective masks | reasonable grammars (P1) |
| RTF | Text | `*.rtf` | conversion RTF → text |
| Word-processor documents | Text | `*.docx;*.docm;*.dotx;*.odt;*.ott;*.odp` | conversion → text |
| PDF Documents | Text | `*.pdf` | conversion → text (P1) |
| Sorted | Text | (empty mask) | conversion: sort lines |
| Comma Separated Values | Table | `*.csv;*.tsv;*.txt?` | delimiter `,` (tab for tsv), qualifier `"` |
| Spreadsheet Workbooks | Table | `*.xls;*.xlsx;*.xlsm;*.ods` | conversion: spreadsheet, all sheets |
| HTML Tables | Table | `*.htm;*.html` (disabled by default) | conversion: HTML tables (P1) |
| Picture Files | Picture | `*.png;*.jpg;*.jpeg;*.gif;*.bmp;*.ico;*.tif;*.tiff;*.webp;*.tga;*.pnm;*.qoi;*.exr;*.hdr;*.dds` | |
| Media Files | Media | `*.mp3;*.mp4;*.m4a;*.m4b;*.m4p;*.m4v;*.flac;*.wav;*.ogg;*.opus` | |
| Executables | Version | `*.exe;*.dll;*.ocx;*.sys;*.drv;*.scr;*.bpl;*.so;*.dylib` | |
| Registry Files | Registry | `*.reg` | |

- **FF-040** (P0): Keyword lists for the presets are maintained as data files under
  `core/hikasa-formats/presets/` (TOML), not in code.
- **FF-041** (P1): Additional formats can be imported from community packages
  (settings package containing only formats).
