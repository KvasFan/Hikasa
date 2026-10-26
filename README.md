# Hikasa

**Hikasa** is a cross-platform desktop application (in development), written in Rust, for
comparing, merging and synchronizing folders and files: folder trees (local, inside
archives, on remote services, or saved snapshots), text (2-way compare, 3-way merge,
edit, patch), tables, hex, pictures, media tags, executable version info and registry
files, plus a command-line interface, a script language and reports.

## Status

**Version 0.1.0: milestone M1 (core engines, command line, GUI prototype).** CI
builds and tests it on Linux, Windows and macOS; the performance gates were measured
on Linux. See [`docs/16-roadmap.md`](docs/16-roadmap.md) for what is done and what is
still open.

What M1 contains:

- **Core libraries** (`core/`): specs, file masks and name filters; a virtual file
  system for local files, standard input and the clipboard; streamed CRC and binary
  comparison; alignment engines (standard, Myers, histogram, patience) with closeness
  matching and inline differences; 21 built-in file formats with grammar tokenizing
  and encoding detection; Text Compare (rules, important and unimportant differences,
  display filters); Folder Compare (rules, filters, parallel scan, live model); text
  and folder reports (plain text, HTML, patches, summaries, JSON); the session service
  shared by the CLI and the GUI, with IPC between them.
- **The `hikasa` command line** (`cli/hikasa-cli`): `quick`, `text compare`,
  `folder compare`, `format list|show|detect|tokens`, `vfs ls|stat|cat`,
  `gui open|ping|list|close|quit`, `completions` and `version`, with exit codes and
  `--json` output ([`docs/12-cli-and-scripting.md`](docs/12-cli-and-scripting.md)).
- **The GUI prototype `hikasa-gui`** (`gui/hikasa-gui`, egui/eframe): one window with
  tabs for Home, Folder Compare and Text Compare, menus, toolbar and shortcuts from
  the command catalogue, and the IPC endpoint that `hikasa gui …` talks to.

Not in M1 (planned for M2 and later, see the roadmap): sessions and workspaces,
editing, merging and synchronizing, file operations, archives and remote back-ends,
Table, Hex and Picture Compare, the script language, the options dialog and the user
manual.

### Build and run

```sh
cargo build --release
target/release/hikasa --help
target/release/hikasa text compare left.txt right.txt
target/release/hikasa folder compare left/ right/ --json
target/release/hikasa-gui                      # the window (Home)
target/release/hikasa-gui left/ right/         # or open a comparison directly
target/release/hikasa gui open a.txt b.txt --wait
```

A Rust toolchain 1.97 or newer is needed. On Linux the GUI needs an X11 or Wayland
session at run time; building needs only the toolchain and a C compiler (no GUI
development packages).

### Repository layout

- `docs/` — the functional and technical specification, written as instructions for
  AI coding agents and human contributors. Start with
  [`docs/00-overview.md`](docs/00-overview.md); the binding M1 API contract is
  [`docs/design/m1-contracts.md`](docs/design/m1-contracts.md).
- `core/`, `cli/`, `gui/` — the Cargo workspace in three layers: **core** libraries
  hold all logic; the **`hikasa` CLI** exposes every function headless and connects to
  the GUI over IPC; the **GUI** is a thin desktop front-end over the same core (see
  [`docs/01-architecture.md`](docs/01-architecture.md)). The crates for tables, hex,
  pictures, metadata and scripts are still empty.
- `deny.toml` — the dependency license policy.

## Principles

- Rust only, open-source dependencies only (MIT / Apache-2.0 / BSD / MPL-2.0 / CC0 …).
- Pure, testable comparison engines; UI-agnostic core; **CLI-first**: every tool is a
  `hikasa` subcommand before it is a GUI view; a GUI built with egui/eframe
  (see [`docs/02-tech-stack.md`](docs/02-tech-stack.md) for the evaluation and the
  performance gate).
- Linux first, then Windows and macOS.

## Reading order for implementers

1. `docs/00-overview.md` — vocabulary, quality bar, requirement notation
2. `docs/01-architecture.md` and `docs/02-tech-stack.md`
3. `docs/16-roadmap.md` — what to build first
4. `docs/17-agent-workflow.md` — the rules
5. The per-area specifications (`03` … `15`) and `docs/reference/commands.md`

## License

Dual-licensed under MIT or Apache-2.0, at your option. See `LICENSE-MIT` and
`LICENSE-APACHE`.
