# Hikasa M2 — Binding Contracts

> **Status: binding** for milestone M2 (version 0.2, `docs/16-roadmap.md`), branch `feat/m2`. It **extends**
> `docs/design/m1-contracts.md`: every tenet (T1–T11), decision (D-01…D-58), contract and test rule of M1 stays
> in force unless a decision below revises it with a new id. The plan, the scope and the maintainer's answers are
> in `docs/design/m2-plan.md` (§4.1); where the plan and this document disagree, this document wins.
>
> The contract is written **wave by wave** (plan §2): each wave's section is completed before the wave starts, so
> its implementers code against settled types. Sections of later waves are outlines until then (plan §6).

## Table of contents

- §0 Ground rules: scope, new tenets, decision register D-59…, dependencies, documentation updates
- §1 Wave contracts — §1.0 W0 Infrastructure (complete) · §1.1 W1 Settings store · §1.2–§1.14 W2–W14 (outline in plan §6, written
  before each wave)
- §6 Tests and acceptance per wave

---

## 0. Ground rules

### 0.1 Scope

The P0 ids of plan §1, minus the deferrals of Q1 (plan §4.1). Release: 0.2.0 after W14.

### 0.2 Tenets added in M2

| Id | Tenet | Enforced by |
|---|---|---|
| T12 | **Plan, confirm, execute.** Every mutation (file operation, sync, archive rebuild, save, settings import) is a serializable plan computed by core, confirmed by the adapter and executed by core with per-item results; `--dry-run` prints the plan that would be executed. | W3, W5, W6, W10 contracts; property tests |
| T13 | **Persisted state is versioned and program-owned.** Every settings file has `schema_version`, a migration chain, unknown-key preservation (Q11) and an atomic write. | W1 contract; round-trip properties |
| T14 | **Generated references are never edited by hand.** `docs/reference/commands.md` is the output of `cargo xtask commands-doc` over `core/hikasa-session/catalogue/commands.toml`; CI fails when it is stale. | CI `docs` job (D-66) |

### 0.3 Decision register (M2)

| ID | Decision | Rationale | Docs |
|---|---|---|---|
| D-59 | Q1 = a: the deferrals of plan §4 Q1 move to M3 (merge parts, snapshot/remote/profile parts, built-in document converters, spreadsheet/HTML table extraction, DDS, ACL and 8.3 options, taskbar previews, "update available", the third-party highlighter fallback). | Maintainer, 2026-10-28. | 16 |
| D-60 | Q2 = a: Folder Sync P0 (FS-001/002/004/005) is in M2: engine W6, view W9. | Maintainer. | 16 |
| D-61 | Q3 = a: `docs/15` options get ids `OPT-001…`, one per page or setting group, each with a priority. | A1: OPT had no ids. | 15 |
| D-62 | Q4 = a, Q5 = a, Q6 = a: configuration changes are detected by polling (window focus and every 2 s); browse dialogs are an in-house VFS browser; Print = the HTML report in the default browser. No `notify`, no `rfd`. | Maintainer. | 03, 13, 14 |
| D-63 | Q8 = a, Q9 = **b**, Q10 = b: archives are read in every P0 format and written for zip and the tar family by full rebuild; xz uses the pure-Rust `lzma-rust2` (no `liblzma`, no C code for LZMA); `thread-priority` is approved, `uzers` is not (owner and group stay numeric). Approved crates: `zip`, `tar`, `flate2`, `bzip2`, `lzma-rust2`, `zstd`, `image`, `arboard`, `trash`, `open`, `ctrlc`, `thread-priority`. Each is added in the wave that first uses it, with its `docs/02` line and fixed features. | Maintainer, 2026-10-28. | 02 |
| D-64 | Q11 = a, Q12 = a, Q13 = a: program-owned TOML keeps unknown keys, not comments; CI uses nextest and the docs checks, benches run in a manual job (25 % tolerance); help topics are Markdown files in `help/`, one per CLI subcommand, view and dialog; the localization table (ARCH-006) stays P1. | Maintainer. | 02, 17 |
| D-65 | **Command catalogue in core** (Q7 = a). The data of every command of `reference/commands.md` (458 rows, 14 sections) lives in `core/hikasa-session/catalogue/commands.toml`, embedded and parsed by `hikasa_session::commands`. Entry: `id`, `menu`, `caption`, `shortcut` (reference spelling), `description`, `status` (`implemented`/`planned`); section: `key`, `title`, `help` topic, optional `note`. The GUI keeps presentation and availability (toolbar labels, radio groups, predicates) next to its view models; its tests require that every command it executes is an `implemented` entry of its view type's sections with the same default shortcut, and that every `implemented` entry of those sections is executed by it. | ARCH-070 "shared with the CLI"; core holds data only, no GUI type. | 01 §3.6, 14 |
| D-66 | **Generated reference.** `docs/reference/commands.md` = `Catalogue::render_markdown(true)` (the M1 table plus a Status column). `cargo xtask commands-doc --check` and `cargo xtask help-check` run in the CI `docs` job. Without the Status column the generated file equals the M1 hand-maintained file apart from its final blank line (checked when W0 was written). | docs/17 §4.2; T14. | 17 |
| D-67 | **Help topics.** `help/<id>.md`, first line `# <title>`, links `[label](topic:<id>)`, embedded by `hikasa_session::help`. `hikasa help [words…]` joins the words with `-` and falls back to the leading words (`format list` → `format`); an unknown topic exits 100 with up to five suggestions (substring or edit distance ≤ 2). `hikasa help --list`. `hikasa.help.v1` = `{topic: {id, title, markdown, links} or null, topics: [{id, title}]}`. `help` reads no configuration. In the GUI, `ContextHelp` (F1) shows the topic of the active view type (its first catalogue section's `help`), `HelpContents` the index, in a resizable right-hand panel; links open topics; Esc closes it. Every topic is linked from another one (the index excepted). | DoD "every new command has a help topic"; UI-006. | 12, 14, 17 |
| D-68 | **Repository tasks.** `xtask/` is a workspace member (binary, `publish = false`) whose only dependency is `hikasa-session`; alias `cargo xtask` in `.cargo/config.toml`. The architecture test covers it (check 1 scans `xtask/`, check 3 lists it). | docs/16 "moved to M2". | 02 |
| D-69 | **Test runner.** CI runs `cargo nextest run --workspace --profile ci` plus `cargo test --workspace --doc`. `.config/nextest.toml` runs the wall-clock latency tests (names matching `cancel`, `latency`, `budget`, `within_`, `timeout`, `abandoned`) one at a time; the `ci` profile retries them twice (a retried pass is reported as flaky). `bench.yml` is manual (`workflow_dispatch`). | Q12 = a; the M1 macOS/Windows timing failures; under load (load average 30) `a_cancel_interrupts_one_long_line` failed once with every test in parallel and passed five times alone. | 02, 17 |

| D-70 | **One settings-file engine** (T13). Every program-owned settings file (`options`, `state`, `commands`, `theme`, `themes/<name>`, `filters`, `openwith`, `archives`) is loaded and saved by `SettingsFile<T>` (§1.1.2). Writes are **sparse**: a key equal to its default is not written, so a later change of a default reaches every user; unknown keys and sections are kept (Q11), comments and layout are not. | One owner of versioning, migration, preservation and atomic writes. | 03 §5 |
| D-71 | **Revises D-31.** `options.toml` is read completely through the typed model of `docs/15` ("Keys", OPT-001…OPT-022). Unknown keys inside a known section are still ignored silently (logged with `--verbose`) and kept on save; an unknown top-level section still warns `unknown_config_key`. A wrong type or a value out of range is an error for every command **except** the `settings` subcommands, which load leniently (the bad value is reported as warning `invalid_config_value` and replaced by its default in memory) so that `settings set` can repair the file. The M1 `ProgramOptions` (three keys) stays as a view computed from the full model. | The M1 subset grew to the whole page set; repair must stay possible from the CLI. | 03 §5.1, 15 |
| D-72 | **One file per list** (SES-051 wins over the page layout of `docs/15`). Archive type masks live in `archives.toml`, command customizations in `commands.toml`, Open With applications in `openwith.toml`, name-filter presets in `filters.toml`, colours and fonts in `theme.toml` and `themes/<name>.toml`. The sections `archive_types`, `commands` and `open_with` of `options.toml` stay reserved (known, empty; their unknown keys are kept). `[appearance]` of `options.toml` holds only `theme_mode`. | `docs/15` lists pages, `docs/03` lists files; each datum needs exactly one home. | 03, 15 |
| D-73 | **Settings keys** (A15). `hikasa settings` addresses every value as `<file>.<path>`: `<file>` is `options`, `state`, `commands`, `theme`, `filters`, `openwith`, `archives` or `themes.<scheme>`; `<path>` is the TOML key path, segments separated by `.`, a segment that is not a bare key written in double quotes, an array index as a decimal segment (`openwith.app.0.command`). A key that names a table addresses the whole subtree (get/list); `set` takes a leaf or a whole array. | One grammar for every file; the CLI is complete before the GUI dialogs (W4) use the same Service calls. | 12 |
| D-74 | **Change detection** (Q4 = a). A `ChangeStamp` is `(length, modification time in ns or none, CRC-32 of the bytes)`; a missing file has the stamp `Absent`. `ConfigWatch::poll` re-reads the stamps of the files it tracks and reports `Changed`, `Created` or `Removed`; it holds no timer (the adapters call it on window focus and every 2 s, D-62). A save first compares the stamp on disk with the stamp of the load: a difference is `SessionError::Conflict` unless the caller passes `Overwrite::Force`. | Two writers (CLI and GUI) without `notify`; the CRC catches same-size same-second edits. | 03 §5 |
| D-75 | **Revises D-21.** The `format` write commands (`new`, `edit`, `delete`, `import`, `enable`, `disable`, `move`, `restore`) write only the files they touch: `formats/<id>.toml` and/or `formats/order.toml`. They never materialize the other built-ins. A built-in cannot be deleted (it can be disabled; `restore` removes a user file that overrides it). | FF-001 operations available headless; D-21's intent (no surprise files) kept. | 10 §1 |
| D-76 | **Open With expansion is pure** (OPT-012). The command line is split into words first (whitespace separates words; `"…"` groups, `\"` is a quote inside a group), then variables are expanded inside each word, so a substituted path is always exactly one argument whatever it contains. `%%` is a literal `%`; an unknown variable, a `%…2` variable without a second item and a dangling `%` are errors. | No shell, no quoting bugs, testable without processes. | 15 |
| D-77 | **Restore defaults** (CLI part of SES-062 for single files): `hikasa settings restore-defaults` deletes the chosen settings files (the next load sees defaults). It is a T12 plan: `--dry-run` prints it; without `--dry-run` it needs `--yes`, else exit 100 (`invalid_input`). Sessions, workspaces and formats are not touched (formats: `format restore`; sessions and the wizard: W2). | T12; destructive commands never run by accident in scripts. | 03 §6, 12 |
| D-78 | **Themes** (UI-020…023). `theme.toml` picks the scheme for light and for dark mode (`light_scheme`, `dark_scheme`) and holds the four fonts; a scheme is `themes/<name>.toml`. The built-in schemes `light` and `dark` are embedded; a file `themes/light.toml` or `themes/dark.toml` overrides elements of the built-in (sparse); any other scheme names a `base` (`light` or `dark`) whose values fill the elements it does not set. Colours are `#rrggbb` or `#rrggbbaa`, written in lower case. Syntax colours are keyed by grammar element name (lower case); an element without an entry uses the eight-colour `fallback` cycle by element id, as in M1. | Editable built-ins without materializing them; every element has a value. | 14 §2 |
| D-79 | **Advisory exceptions.** `deny.toml` ignores the "unmaintained" notices RUSTSEC-2026-0192 (`ttf-parser`), -0370 (`zvariant`), -0371 (`zbus_names`) and -0372 (`zvariant_derive`): GUI transitive dependencies pinned by eframe 0.37 (winit → sctk-adwaita; accesskit_unix → zbus 5). Each entry is removed when an upgrade drops the crate; a security (non-unmaintained) advisory is never ignored this way. | Maintainer, 2026-10-28 (option a); no fix reachable without leaving eframe 0.37. | 02 |

### 0.4 Dependencies

W0 and W1 add no third-party crate (the change stamp uses `hikasa_vfs::crc32_reader`). Member manifests (architecture check 3): + `xtask` = `[hikasa-session]`. The
approved crates of D-63 enter the table of the wave that first uses them.

### 0.5 Documentation updates of W0

`docs/01` §3.6 (catalogue in core), `docs/02` §4 (tooling), `docs/12` §1 and §4 (`help`), `docs/14` (F1 panel),
`docs/16` (M1 leftovers, M2 progress), `docs/17` §3–§4, `CHANGELOG.md`. The OPT ids (D-61) and the M3 list of
D-59 are written in W1, where the options are implemented.

---

## 1. Wave contracts

### 1.0 W0 — Infrastructure (complete)

**Files.** `core/hikasa-session/catalogue/commands.toml` (data, D-65); `core/hikasa-session/src/commands.rs`;
`core/hikasa-session/src/help.rs`; `help/*.md` (17 topics: `index`, `cli`, `exit-codes`, `json-output`, one per
M1 subcommand and `help`, `main-window`, `view-home`, `view-folder-compare`, `view-text-compare`);
`cli/hikasa-cli/src/cmd/help.rs`; `gui/hikasa-gui/src/ui/help.rs`; `xtask/src/main.rs`; `.cargo/config.toml`;
`.config/nextest.toml`; `.github/workflows/ci.yml` and `bench.yml`.

**`hikasa_session::commands`.**

```rust
pub const CATALOGUE_SCHEMA_VERSION: u32 = 1;
pub enum CommandStatus { Implemented, Planned }          // serde snake_case; as_str()
pub struct CommandEntry { pub id: String, pub menu: String, pub caption: String, pub shortcut: String,
                          pub description: String, pub status: CommandStatus }   // deny_unknown_fields
pub struct CatalogueSection { pub key: String, pub title: String, pub help: String, pub note: String,
                              pub commands: Vec<CommandEntry> }
pub struct Catalogue { pub schema_version: u32, pub title: String, pub intro: String,
                       pub sections: Vec<CatalogueSection> }
pub struct CatalogueError(pub String);
pub fn catalogue() -> &'static Catalogue;                // parsed once; empty and logged if invalid (T9)
pub fn try_catalogue() -> Result<Catalogue, CatalogueError>;   // parse + problems()
pub fn parse_catalogue(text: &str) -> Result<Catalogue, CatalogueError>;
impl Catalogue {
    pub fn section(&self, key: &str) -> Option<&CatalogueSection>;
    pub fn find(&self, keys: &[&str], id: &str) -> Option<(&CatalogueSection, &CommandEntry)>;
    pub fn entries(&self) -> impl Iterator<Item = (&CatalogueSection, &CommandEntry)>;
    pub fn problems(&self) -> Vec<String>;
    pub fn render_markdown(&self, status_column: bool) -> String;
}
```

Problems: a section key not `[A-Za-z0-9-]+` or used twice; an id not `[A-Za-z0-9]+` or listed twice in its
section; a `help` topic that does not exist; a menu, caption, shortcut or description with `|`, a line break or
surrounding spaces.

**`hikasa_session::help`.**

```rust
pub struct HelpTopic { pub id: &'static str, pub source: &'static str }
pub const INDEX_TOPIC: &str = "index";
pub fn topics() -> &'static [HelpTopic];
pub fn topic(id: &str) -> Option<&'static HelpTopic>;
pub struct UnknownTopic { pub word: String, pub suggestions: Vec<&'static str> }   // Error
pub fn resolve(words: &[String]) -> Result<&'static HelpTopic, UnknownTopic>;
pub fn suggestions(word: &str) -> Vec<&'static str>;
pub enum Inline { Text(String), Code(String), Link { label: String, topic: String } }
pub enum Block { Heading { level: u8, text: Vec<Inline> }, Paragraph(Vec<Inline>), Item(Vec<Inline>),
                 Verbatim(String) }
pub fn parse_inline(line: &str) -> Vec<Inline>;
pub fn parse_blocks(source: &str) -> Vec<Block>;
pub fn plain(inline: &[Inline]) -> String;
impl HelpTopic { pub fn title(&self) -> String; pub fn blocks(&self) -> Vec<Block>;
                 pub fn links(&self) -> Vec<String>; pub fn render_text(&self) -> String; }
pub fn problems() -> Vec<String>;
```

Terminal text: the title underlined with `=`, one blank line between blocks (none between consecutive list
items), items as `- …`, verbatim blocks indented by four spaces, links as their label, code in backquotes.

**CLI and JSON.** `hikasa help [TOPIC…] [--list]` (`--list` conflicts with a topic); exit 0, an unknown topic 100
with `error.kind = "invalid_input"`. `HELP_SCHEMA = "hikasa.help.v1"` is appended to `SCHEMAS` (19 schemas).
`help` runs without the service, like `version` and `completions`. DTOs: `HelpDto { topic: Option<HelpTopicDto>,
topics: Vec<HelpListItemDto> }`, `HelpTopicDto { id, title, markdown, links }`, `HelpListItemDto { id, title }`.

**GUI.** Catalogue entries `HelpContents` and `ContextHelp` (F1, global scope). `AppCore::{help, show_help,
close_help}`; `Scope::{catalogue_sections, help_topic}`. `ui::help::show` draws the panel and only reports
`Open(id)` or `Close`.

### 1.1 W1 — Settings store

**Ids.** SES-051, SES-052, SES-053, FF-001 (CLI), FF-004 (write paths, restore, detection), UI-003 (persistence),
UI-020…023 (scheme files), SES-041 and SES-091 (options), OPT-001…OPT-022 (`docs/15` "Keys"). No GUI change in
W1 except keeping it compiling (the GUI reads the new model in W4).

**Files.** `core/hikasa-session/src/settings/` (`mod.rs`, `file.rs`, `stamp.rs`, `keys.rs`, `options.rs`,
`state.rs`, `commands.rs`, `theme.rs`, `filters.rs`, `openwith.rs`, `archives.rs`), `core/hikasa-session/src/
formats.rs` (write paths), `core/hikasa-session/src/api/settings.rs` (Service calls and DTOs),
`cli/hikasa-cli/src/cmd/settings.rs`, `cli/hikasa-cli/src/cmd/format.rs`, `help/settings.md`, `help/format.md`
(extended), golden files. `options.rs` at the crate root keeps its public items (`ProgramOptions`,
`OptionsLoad`, `load_options`, `parse_options`, `OPTION_SECTIONS`, `OPTIONS_FILE`, `OPTIONS_SCHEMA_VERSION`) as thin
wrappers over `settings::options`, so M1 callers do not change.

#### 1.1.1 Module map

```rust
pub mod settings {
    pub use file::{SettingsFile, SettingsSchema, Overwrite, LoadMode, Loaded, Problem};
    pub use stamp::{ChangeStamp, ConfigWatch, ConfigChange, ChangeKind};
    pub use keys::{SettingsKey, KeyInfo, ValueKind, FileId, parse_key};
    pub use options::Options;            // every OPT key; `Options::program_options() -> ProgramOptions`
    pub use state::State;
    pub use commands::{CommandOverrides, ViewCustomization, CommandOverride, Shortcut, ShortcutConflict};
    pub use theme::{ThemeSelection, Fonts, FontSpec, Scheme, Colour, SyntaxStyle, ThemeMode};
    pub use filters::{FilterPresets, FilterPreset};
    pub use openwith::{OpenWith, OpenWithApp, WorkingFolder, ExpandContext, ExpandItem, ExpandError,
                       split_words, expand_command, InstanceMode};
    pub use archives::{ArchiveTypes, ArchiveType};
    pub struct SettingsStore { /* ConfigLocation */ }
}
```

`SettingsStore::new(&ConfigLocation)`; `store.options(mode)`, `store.state(mode)`, `store.commands(mode)`,
`store.theme(mode)`, `store.scheme(name, mode)`, `store.filters(mode)`, `store.openwith(mode)`,
`store.archives(mode)` return `Result<Loaded<T>, SessionError>`; `store.path(FileId) -> PathBuf`;
`store.watch() -> ConfigWatch` over every settings file plus `formats/` (each `*.toml` and `order.toml`).

#### 1.1.2 `SettingsFile<T>` (D-70, T13)

```rust
pub trait SettingsSchema: Serialize + DeserializeOwned + Default + Clone + PartialEq {
    const FILE: FileId;                 // which file (FileId::Themes(name) for schemes)
    const VERSION: u32;                 // current schema_version (all files: 1 in W1)
    /// Rewrites a raw table of `from` (< VERSION) into the shape of `from + 1`. Called once per step.
    fn migrate(from: u32, table: &mut toml::Table) -> Result<(), String> { Err(format!("no migration from {from}")) }
    /// Range, enum, mask, reference checks after deserialization; each Problem names a dotted key.
    fn validate(&self) -> Vec<Problem>;
    /// Repairs (LoadMode::Lenient): replaces each invalid value by its default.
    fn repair(&mut self, problems: &[Problem]);
    /// Top-level keys that are not sections of this schema but are allowed (none by default).
    fn known_sections() -> &'static [&'static str];
}
pub enum LoadMode { Strict, Lenient }
pub enum Overwrite { IfUnchanged, Force }
pub struct Problem { pub key: String, pub message: String }
pub struct Loaded<T> { pub value: T, pub stamp: ChangeStamp, pub migrated_from: Option<u32>,
                       pub warnings: Vec<Warning>, pub ignored_keys: Vec<String> }
pub struct SettingsFile<T> { path: PathBuf, raw: toml::Table, loaded: Loaded<T> }
impl<T: SettingsSchema> SettingsFile<T> {
    pub fn load(path: &Path, mode: LoadMode) -> Result<SettingsFile<T>, SessionError>;
    pub fn parse(text: &str, origin: &str, mode: LoadMode) -> Result<SettingsFile<T>, SessionError>; // pure core
    pub fn value(&self) -> &T;   pub fn value_mut(&mut self) -> &mut T;
    pub fn stamp(&self) -> &ChangeStamp;
    pub fn to_toml(&self) -> String;                          // the bytes save() would write
    pub fn save(&mut self, overwrite: Overwrite) -> Result<bool, SessionError>;   // false: nothing to write
    pub fn changed_on_disk(&self) -> Result<bool, SessionError>;
}
```

Load algorithm (normative; a test per step):

1. Missing file → `T::default()`, stamp `Absent`, `raw` empty. Other read errors → `SessionError::Io`. Bytes that
   are not UTF-8 → `SessionError::Config`.
2. Parse to `toml::Table` (syntax error → `Config`, message on one line as in M1).
3. `schema_version`: absent → `T::VERSION` (M1 files without it stay valid); negative or not an integer →
   `Config`; newer than `T::VERSION` → `SessionError::Schema` (SES-052), in **both** modes.
4. Older: apply `T::migrate(v, …)` for `v = found … VERSION-1` on the raw table; record `migrated_from`. A
   migration error → `Config`. The migrated file is not written at load (SES-052: written back on next save).
5. Unknown top-level keys (not a field of `T`, not in `known_sections()`, not `schema_version`) → warning
   `unknown_config_key`, kept in `raw`; unknown keys inside known tables → `ignored_keys`, kept in `raw`.
6. Deserialize `T` from the table **with unknown keys removed** (serde must not reject them; types use
   `#[serde(default)]`). A type error: Strict → `Config` naming the dotted key; Lenient → the key is replaced
   by its default value in the table, warning `invalid_config_value`, retry.
7. `validate()`: Strict → the first problem is `Config` (message `<origin>: <key>: <message>`); Lenient →
   `repair()`, one `invalid_config_value` warning per problem.

Save algorithm (normative):

1. If `overwrite = IfUnchanged` and the current disk stamp differs from `loaded.stamp` → `SessionError::Conflict
   { path }` (new variant; classifies as `invalid_data`; message `<path>: changed on disk since it was read`).
   The CLI never forces except `settings set --force`.
2. Document = `raw` with every known key path of `T` removed, then the **sparse** serialization of `value`
   merged in: a leaf (or a whole array) equal to the same path of `T::default()` is omitted; an empty known table
   is omitted. Arrays of tables (`[[app]]`, `[[preset]]`) are whole values; each element type keeps its unknown
   keys in a `#[serde(flatten)] extra: toml::Table` field, so they survive too.
3. `schema_version = VERSION` is always the first line. Sections in the order of the type's fields, then unknown
   sections in their original order. `toml::to_string` formatting; no comments.
4. If the document equals the file's current bytes → return `Ok(false)`, no write. If the document holds only
   `schema_version` and the file is absent → `Ok(false)` (never create a file of defaults).
5. Create the parent folder (`create_dir_all`), `write_atomic`, refresh `loaded.stamp`, return `Ok(true)`.

#### 1.1.3 Change stamps (D-74)

```rust
pub enum ChangeStamp { Absent, Present { len: u64, modified_ns: Option<i128>, crc32: u32 } }
impl ChangeStamp { pub fn of(path: &Path) -> Result<ChangeStamp, SessionError>; pub fn of_bytes(bytes: &[u8], modified_ns: Option<i128>) -> ChangeStamp; }
pub enum ChangeKind { Created, Changed, Removed }
pub struct ConfigChange { pub path: PathBuf, pub kind: ChangeKind }
pub struct ConfigWatch { /* path → stamp */ }
impl ConfigWatch {
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> ConfigWatch;   // stamps taken now
    pub fn track(&mut self, path: PathBuf);                               // stamp taken now
    pub fn poll(&mut self) -> Vec<ConfigChange>;                          // sorted by path; updates stamps
    pub fn acknowledge(&mut self, path: &Path);                           // after our own save
}
```

`poll` compares `len` and `modified_ns` first and computes the CRC only when they are equal to the stored values
**and** the file was modified less than 2 s before the previous poll (same-second edits), else when they differ;
an unreadable file is reported once as `Changed` and keeps its old stamp. The `formats/` folder is tracked by
listing it at each poll (new and removed `*.toml` files are `Created`/`Removed`). No thread, no timer (D-62).

#### 1.1.4 Schemas

`options.toml`: `docs/15` "Keys" (binding; OPT-001…OPT-022; one Rust field per key, nested structs per table).
`archives.toml`, `commands.toml`, `openwith.toml`, `filters.toml`: `docs/15` "Keys". Validation beyond types:
masks parse with `hikasa_core::mask::MaskList` (OPT-010, OPT-020); filter preset names unique and non-empty;
Open With `description` and `command` non-empty and the command splits (D-76); a `commands.toml` view key is one
of the eight view names, a command id exists in one of the catalogue sections of that view (mapping below), a
shortcut parses. Unknown ids or views are **problems**, not unknown keys (a typo must not pass silently).

View → catalogue sections (moves from the GUI's `Scope::catalogue_sections` into
`hikasa_session::commands::view_sections(view) -> &'static [&'static str]`; the GUI calls it):
`home` → `home, global`; `folder-compare` → `folder-compare, folder-shared, session, global`; `folder-sync` →
`folder-sync, folder-shared, session, global`; `text-compare` and `text-edit` → `text-compare, file-shared,
session, global`; `table-compare` → `table-compare, file-shared, session, global`; `hex-compare` →
`hex-compare, file-shared, session, global`; `picture-compare` → `picture-compare, session, global`.

**Shortcuts (UI-003).** `Shortcut::parse(&str)`: modifiers `Shift+`, `Ctrl+`, `Alt+`, `Cmd+` in any order
(case-insensitive), then one key: a letter, a digit, `F1`…`F24`, `Num 0`…`Num 9`, `Num +`, `Num -`, `Num *`,
`Num /`, or one of `Enter Tab Esc Space BkSp Del Ins Home End PgUp PgDn Up Down Left Right` or a single
punctuation character. A trailing context in parentheses (`Enter (in a path field)`) is kept as text and
ignored for conflicts. `Display` writes the reference spelling: `Shift+Ctrl+Alt+Cmd+Key`. Every non-empty
`shortcut` of the catalogue must parse (catalogue problem from W1). Effective shortcuts of a view = catalogue
defaults of its sections, replaced per command by an override. `CommandOverrides::conflicts(view) ->
Vec<ShortcutConflict { shortcut, commands: Vec<String> }>`: two commands of the same view with the same
shortcut (context-less) — reported by `settings list` as warnings and by the W4 editor; not a load problem
(the catalogue itself has view-dependent duplicates that the GUI resolves by focus).

`state.toml` (SES-051; write to `docs/03` §5.2):

```toml
schema_version = 1
last_session = ""                        # name of the session last used (W2)
[mru]                                    # each list newest first, capped at max_mru_items on push
left = []  ;  right = []  ;  center = []  ;  folders = []  ;  files = []
find = []  ;  replace = []  ;  name_filters = []  ;  workspaces = []
[window]                                 # last main window (a workspace stores its own, W2)
width = 1200  ;  height = 800  ;  x = -1  ;  y = -1  ;  maximized = false     # x/y −1 = let the platform place it (A17)
[flags]
first_run_done = false                   # set by the GUI first start (D-21 materialization)
hints_seen = []                          # ids of one-time hints
[dialogs]                                # free tables per dialog id (RPT-003 "remembers last choices"); kept as-is
```

`State::push_mru(list, value, max)`: removes an equal entry (exact string), inserts at the front, truncates to
`max`. Window sizes `200..=32767`, positions `-1..=32767`.

`theme.toml` and `themes/<name>.toml` (UI-020…023; write to `docs/14` §2.1):

```toml
# theme.toml
schema_version = 1
light_scheme = "light"
dark_scheme = "dark"
[fonts.editor]   family = ""  size = 13.0     # "" = the built-in monospace; size 6.0..=72.0
[fonts.listing]  family = ""  size = 13.0
[fonts.hex]      family = ""  size = 13.0
[fonts.folder]   family = ""  size = 13.0  use_system = true

# themes/<name>.toml
schema_version = 1
base = "light"                                # light | dark; ignored for the built-in names
[folder]   # UI-021
default_text, background, folder_info, selection, inactive_selection_background, stripes, log_panel,
filtered_out, unknown, same, newer, older, orphan, different, left_change, right_change, mergeable, conflict
use_system_selection = false, use_system_log_colors = false, use_stripes = true,
color_folders_by_content = true, gradient_center_background = false
[file]     # UI-022
default_text, background, stripes, edited, saved, line_numbers, ignored, current_line, inactive_selection,
selection, important_text, important_text_background, important_line_background, unimportant_text,
unimportant_text_background, unimportant_line_background, replacement, orphan, output, left_change,
right_change, conflict, find_highlight, gutter, whitespace
current_line_only_over_under = false, use_stripes = false, use_system_selection = false
fallback = ["#…", …]                          # exactly 8 colours (syntax colours by element id, M1 slots 1…8)
[file.syntax.<element name, lower case>]      color = "#…", bold = false, italic = false
                                              # built-ins define: comment, string, character, keyword,
                                              # identifier, number, operator, directive
[picture]  # UI-023
background, same, similar, different, checker_light, checker_dark
```

Every colour element is required in a **resolved** `Scheme` (`Scheme::resolve(name, file: Option<&SchemeFile>)
-> Scheme`: built-in values, overlaid by the file's set elements; a non-built-in scheme overlays its `base`).
Built-in values: the M1 `gui/hikasa-gui/src/ui/theme.rs` palettes where an element exists there (folder
colour classes, text backgrounds, gutter, syntax slots → `fallback`), otherwise chosen by the implementer;
default text on background and on each line background has a contrast ratio ≥ 4.5:1 (UI-009; unit test).
`ThemeSelection::scheme_for(mode: ThemeMode, os_dark: bool) -> &str`. A selected scheme that does not exist
is a problem (lenient: falls back to the built-in of that mode).

**Open With (D-76).** `split_words(&str) -> Result<Vec<String>, ExpandError>`; `expand_command(template,
&ExpandContext) -> Result<Vec<String>, ExpandError>` where `ExpandContext { first: ExpandItem, second:
Option<ExpandItem>, line: Option<u64>, working_folder: PathBuf, path_delimiter: Option<char> }` and
`ExpandItem { path: PathBuf, base: PathBuf }` (absolute). `%x` of a name without extension is `""`; `%b` of
`.bashrc` is `.bashrc`; `%F`/`%P` are relative paths computed lexically (no file-system access); `%l` without a
line is `1`; the delimiter replaces both `/` and `\` in substituted paths only. `InstanceMode::of(template)` =
`PerPair` if any `%<v>2` occurs, else `PerItem`. A table test with ≥ 25 rows covers every variable, both
suffixes, quoting, `%%`, the errors and the Windows-style delimiter.

#### 1.1.5 Settings keys and Service calls (D-73)

```rust
pub enum FileId { Options, State, Commands, Theme, Themes(String), Filters, OpenWith, Archives }
pub struct SettingsKey { pub file: FileId, pub path: Vec<KeySegment> }   // KeySegment::{Name(String), Index(usize)}
pub fn parse_key(text: &str) -> Result<SettingsKey, ApiError>;          // invalid_input on bad syntax
pub enum ValueKind { Bool, Int { min: i64, max: i64 }, Float { min: f64, max: f64 }, Enum(&'static [&'static str]),
                     String, Path, Mask, Colour, Shortcut, List, Table }
pub struct KeyInfo { pub key: String, pub kind: ValueKind, pub default: toml::Value, pub opt: Option<&'static str>,
                     pub priority: Option<&'static str> }
pub fn key_infos(file: &FileId) -> Vec<KeyInfo>;    // every leaf key of the file's schema (from T::default())
```

`Service` gains (all read the configuration fresh; no caching across calls):

| Call | Behaviour |
|---|---|
| `settings_get(&SettingsKey) -> Result<SettingsValueDto, ApiError>` | effective value (file or default) of a leaf or subtree; unknown path → `not_found` with up to five suggestions (edit distance ≤ 2 over `key_infos`) |
| `settings_set(&SettingsKey, value_text: &str, force: bool) -> Result<SettingsSetDto, ApiError>` | parses `value_text` as a TOML value; if that fails and the key's kind is String/Path/Mask/Colour/Shortcut/Enum, the raw text is the string; type/range/validate errors → `invalid_input`; lenient load, then save (`Overwrite::IfUnchanged` unless `force`); setting a value equal to the default removes the key (sparse) |
| `settings_unset(&SettingsKey)` | removes the key (back to default); same DTO |
| `settings_list(prefix: Option<&SettingsKey>, changed_only: bool) -> SettingsListDto` | every known leaf (plus unknown kept keys, flagged) in file order then key order; lenient load; warnings and shortcut conflicts attached |
| `settings_restore_defaults(files: &[FileId], dry_run: bool) -> Result<SettingsRestoreDto, ApiError>` | D-77: the plan (each file: `path`, `exists`) and, unless dry run, deletion; `FileId::Themes` with no name = every `themes/*.toml` |
| `format_new(id, from: Option<&str>, name: Option<&str>, mask: Option<&str>, kind: Option<FormatKind>)` | D-75: `formats/<id>.toml` from a copy of `from` (clone) or an empty format of `kind` (default text); id taken (built-in or user) → `already_exists`; appended to `order.toml` (created from the current order when absent) as enabled |
| `format_edit(id, set: &[(String, String)] or file: Option<&Path>)` | replaces the format with the parsed `file` (its `id` must equal `id`), or applies dotted `key=value` edits to the canonical TOML; the result must parse and compile, else `invalid_input` and nothing is written; editing a built-in writes its override file |
| `format_delete(id)` | user-only format: file removed, `order.toml` entry removed; built-in → `invalid_input` ("disable it instead"); unknown → `not_found` |
| `format_import(path, replace: bool)` | validates and copies to `formats/<id>.toml`; existing id without `replace` → `already_exists` |
| `format_export(id, out: Option<&Path>)` | canonical TOML to `out` (atomic) or returned for stdout |
| `format_enable(id, bool)`, `format_move(id, to: MoveTo)` | write `order.toml` only (`MoveTo::{Up, Down, Top, Bottom, Before(id), After(id)}`); first write creates `order.toml` from the current registry order |
| `format_restore(ids: &[String] or all)` | removes user override files of built-ins (and, with all, every user file and `order.toml`); a plan with `dry_run` like D-77 |

Every write call uses `write_atomic` and returns the touched paths. Formats keep their M1 parser and
canonical writer (`parse_format`, `to_toml`); `FormatOrder` gains the mutation helpers needed above.

#### 1.1.6 CLI and JSON

```
hikasa settings get <KEY>
hikasa settings set <KEY> <VALUE> [--force]
hikasa settings unset <KEY>
hikasa settings list [PREFIX] [--changed]
hikasa settings restore-defaults (--all | --file <FILE>…) (--dry-run | --yes)
hikasa format new <ID> [--from <ID>] [--name <TEXT>] [--mask <MASK>] [--kind <KIND>]
hikasa format edit <ID> (--from-file <PATH> | --set <KEY=VALUE>…)
hikasa format delete <ID>
hikasa format import <PATH> [--replace]
hikasa format export <ID> [-o <PATH>]
hikasa format enable <ID> | format disable <ID>
hikasa format move <ID> (--up | --down | --top | --bottom | --before <ID> | --after <ID>)
hikasa format restore (<ID>… | --all) [--dry-run | --yes]
```

Exit 0 on success, 100 on any error (kinds as in the table above; a usage error stays 100 with
`hikasa.usage_error.v1`). Human output: `get` prints the TOML value (a subtree as TOML), `set`/`unset` print
`<key> = <value>` and `(file unchanged)` when nothing was written, `list` prints `key = value` lines with
`  # default: …` when changed, `restore-defaults` and `format restore` print one line per file
(`delete <path>` / `absent <path>`), format writes print `wrote <path>`/`removed <path>`.

New schemas (appended to `SCHEMAS`, 19 → 26, each with a golden):

| Schema | `data` |
|---|---|
| `hikasa.settings_get.v1` | `{key, file, path, value, default, is_default, kind, opt}` (value/default as JSON; a subtree as an object) |
| `hikasa.settings_set.v1` | `{key, file, path, old, new, written}` (`unset` uses the same schema) |
| `hikasa.settings_list.v1` | `{entries: [{key, value, default, is_default, kind, opt, unknown}], conflicts: [{view, shortcut, commands}]}`; warnings in the envelope |
| `hikasa.settings_restore.v1` | `{dry_run, files: [{file, path, existed, deleted}]}` |
| `hikasa.format_write.v1` | `{action, id, written: [path], removed: [path]}` (new, edit, delete, import, enable, disable, move) |
| `hikasa.format_export.v1` | `{id, path or null, toml}` |
| `hikasa.format_restore.v1` | `{dry_run, ids, removed: [path]}` |

TOML values in JSON: string, integer, float, boolean as JSON scalars; arrays and tables as JSON arrays and
objects; datetimes (unused) as strings.

#### 1.1.7 Documentation updates of W1

`docs/03` §5.2 (new: `state.toml`, writing rules D-70, change detection D-74, `settings` restore D-77), §5.1
(D-31 → D-71 note); `docs/10` §1 (format write commands, D-75); `docs/12` §1 and the command table (`settings`,
the new `format` forms; `export|import` stay W2); `docs/14` §2.1 (new: theme and scheme files, D-78); `docs/15`
"Keys" (written with this contract) and the M1 section's pointer to D-71; `docs/16` M2 progress;
`help/settings.md` (new), `help/format.md`, `help/index.md` link; `CHANGELOG.md`.

#### 1.1.8 W1 notes (implementation)

*W1 note:* where the implementation had to interpret or extend the text above:

- **Order of kept keys.** `toml` is built without `preserve_order` (features are fixed at workspace level), so a
  table read from a file is sorted: kept unknown keys are written in sorted order after the known keys of their
  table, and unknown sections in sorted order after the known ones (save step 3). Known keys keep the declaration
  order of the schema (taken from its serde serialization).
- **Change stamps without a clock.** Core code may not read the clock (T1), so `ConfigWatch::poll` computes the
  CRC whenever length and time are unchanged (instead of only for files modified less than 2 s before the
  previous poll); settings files are small. `ConfigWatch::track_folder` (additive) is how `formats/` and `themes/`
  are listed at each poll.
- **Conflicts in `settings list`.** The catalogue binds the Copy and Take commands of the file views to the same
  keys by design (20 pairs), so `settings_list` reports `CommandOverrides::user_conflicts`: the conflicts that
  involve a command whose `shortcuts` the file overrides. `conflicts(view)` still returns all (W4 editor). In human
  mode they are `warning:` lines on stderr; there is no warning code for them.
- **Sparse entries.** Entries of maps and arrays of tables are also written sparsely against the defaults of a new
  entry (`[view.<v>]`, `[[app]]`, `[[preset]]`). A scheme file is sparse against the built-in of its base on every
  save (`SettingsSchema::save_defaults`; additive, default `T::default()`), whatever produced the value: a change
  of `base` keeps only the elements the file set, and colours compare as colours (`#FFFFFF` = `#ffffff`). Every
  W1 schema reads `schema_version = 0` as 1 (an identity migration; an M1 test loads such a file).
- **Arrays of tables in lenient loading.** Lenient loading never drops or reorders the elements of an array, so
  the positions in memory (`openwith.app.1`) are the positions in the file. A leaf of the wrong type inside an
  element is located by its full key (`app.1.wait_for_previous`) and repaired to the default of that leaf, like a
  scalar. A check that fails for a whole element (an empty required field, a duplicate preset name) names the
  element (`app.1`): the element is kept as it is, with warning `invalid_config_value` "(the entry is kept as it
  is)", and written back unchanged. `settings set` refuses only a problem the change adds, so the other entries
  (and the bad entry itself) stay editable.
- **Known keys.** A kind rule whose last segment is a wildcard (`tabs.*`) gives the kind of the keys a table has;
  it does not make another name a key, so a misspelt key is `not_found` with suggestions (D-73) and an unknown
  key kept in the file is shown by `get`/`list` as `unknown`. An array position past the end, a view that is not
  a view type and a command id that is not in the sections of its view are `not_found` for `get`, `set` and
  `unset` (`commands.view.home.command.NoSuch.menu` included).
- **Change stamps** of a load and of a later poll share one spelling of the modification time (a time before
  1970 is a negative number of nanoseconds).
- **API shapes.** `SettingsSchema::repair` has a default (on the serialized tree); `FILE` of the scheme schema is
  `FileId::Themes(String::new())`. Additive: `SettingsFile::{new_default, loaded, unknown, path}`,
  `SettingsStore::{file, scheme_file, resolved_scheme, scheme_names, scheme_files, dir}`, `FileId::{every, parse,
  relative_path, name}`, `FolderFont` (the folder font has `use_system`), `MruList`, `ServiceInit::options_mode`
  (`LoadMode::Lenient` for the `settings` commands; the service reads `options.toml` at construction),
  `ApiError::{NotFound, AlreadyExists}`, `Service::settings_store`, `parse_format_kind`. `store.scheme` returns
  `Loaded<SchemeFile>`; `settings_list` returns a `Result` (a newer or unparsable file is an error);
  `format_edit(id, set, file)` and `format_restore(ids, all, dry_run)` take both alternatives; `format_new --from`
  accepts an id or a name and names the copy `<name> (copy)`. The DTOs carry the load warnings (and the TOML
  value for human output) in fields the JSON skips.
- **Format ids.** `order` (in any case) is reserved: `format new`, `import` and `edit` refuse it with
  `invalid_input`, since `formats/order.toml` is the order file. `format new <id>` and `format import` without
  `--replace` are `already_exists` also when `formats/<id>.toml` exists but was skipped at load (it does not
  parse). `format new` writes `<id>.toml`, then `order.toml`; when the second write fails the first file is
  removed. The clap usage of `settings restore-defaults` and `format restore` names both alternatives
  (`<--all|--file <FILE>>`, `<ID|--all>`).
- **Human `format restore`** prints `delete <path>` for removed files and `absent <path>` for named ids without a
  file; `format edit --set` refuses unknown keys (the canonical writer would drop them); `--from-file` copies the
  file as it is (like `import`).

### 1.2–1.14 W2–W14

Outlines in plan §6; each section is written here before its wave starts.

---

## 6. Tests and acceptance

### 6.0 W0 acceptance (met)

- A0.1 `cargo xtask commands-doc --check` and `cargo xtask help-check` pass; the generated reference without the
  Status column equals the M1 file (458 rows; 60 `implemented`).
- A0.2 GUI tests `the_window_matches_the_core_catalogue` (both directions), `every_view_type_has_a_help_topic`,
  `help_commands_show_topics`.
- A0.3 CLI tests `cli_help.rs`: goldens of a text topic, a JSON topic and the list; an unknown topic exits 100
  with a suggestion; every M1 subcommand resolves to a topic titled `hikasa <subcommand>`; `help` works with a
  configuration that does not load.
- A0.4 Unit tests of the catalogue (validity, lookups, problems, Markdown) and of the help reader (topic set,
  resolution, inline, blocks).
- A0.5 The architecture test covers `xtask`; the workspace is green under `cargo nextest run --workspace`,
  `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --check`, `cargo doc` and `cargo deny`.

### 6.1 W1 acceptance

- A1.1 **Round trip (T5, P-SET).** For every schema `T`: proptest over generated values (within ranges):
  `parse(to_toml(v)) == v`; `to_toml` of `T::default()` is exactly `schema_version = 1\n`; saving twice writes
  once (`save` returns `false` the second time).
- A1.2 **Preservation (Q11).** A fixture with unknown top-level sections, unknown keys inside known tables and
  unknown keys inside `[[app]]`/`[[preset]]` entries: after `settings set` of one key, every unknown key is
  present with an equal value (compared as `toml::Value`), and only the set key changed.
- A1.3 **Migration (SES-052).** A test-only schema with `VERSION = 3` and migrations 1→2→3 loads hand-written v1
  and v2 fixtures to the same value, sets `migrated_from`, does not write at load, writes v3 on save. A newer
  `schema_version` is `Schema` in Strict and Lenient mode; negative or non-integer is `Config`.
- A1.4 **Strict vs lenient (D-71).** Wrong type and out-of-range values: every non-`settings` command exits 100
  naming file and key; `settings list` and `settings set` work and warn `invalid_config_value`; `settings set`
  of the bad key repairs the file. M1 option fixtures (`tests` of `options.rs`) still pass.
- A1.5 **Atomic write and conflict.** Fault injection (a read-only folder, a target that is a directory): the
  target is untouched and no temporary file is left. An external edit between load and save → `Conflict`
  (exit 100); `--force` overwrites.
- A1.6 **Change detection (D-74).** `ConfigWatch` reports `Created`, `Changed` (including same length and same
  mtime with different bytes, by setting the mtime back), `Removed`, new and removed format files; nothing after
  `acknowledge`; nothing on an unchanged poll.
- A1.7 **Every key.** A test parses the "Keys" table of `docs/15` and checks that each key exists in
  `key_infos(Options)` with the same type, range or enum values and default, and vice versa; each range is
  tested at both bounds and one step outside.
- A1.8 **Schemas' extra rules.** Masks, preset names, Open With entries, `commands.toml` views, ids and
  shortcuts (every catalogue shortcut parses and round-trips its spelling), shortcut conflicts, scheme
  resolution (built-in overlay, `base`, missing scheme falls back), contrast ≥ 4.5:1 in both built-ins.
- A1.9 **Open With.** The D-76 table test (≥ 25 rows) and `InstanceMode`.
- A1.10 **CLI goldens.** `settings get|set|unset|list|restore-defaults` and each `format` write form in text and
  JSON, in a temporary `--config-dir`; read-only commands still create nothing (D-21); exit codes 0/100;
  `help settings` resolves; `cargo xtask help-check` and `commands-doc --check` pass.
- A1.11 **Format writes (D-75).** `new` (clone and empty), `edit` (file and `--set`), `delete` (user, built-in
  refused), `import` (`--replace`), `export`, `enable`/`disable`, `move` (all six forms), `restore` (one id,
  `--all`, dry run): after each, `format list --json` shows the expected registry and only the touched files
  exist.
- A1.12 The whole workspace is green (nextest, doc tests, fmt, clippy `-D warnings`, doc, deny, architecture
  test, xtask checks); the GUI builds and its tests pass, using `commands::view_sections`.
