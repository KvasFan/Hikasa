# Hikasa

**Hikasa** is a planned cross-platform desktop application, written in Rust, for
comparing, merging and synchronizing folders and files: folder trees (local, inside
archives, on remote services, or saved snapshots), text (2-way compare, 3-way merge,
edit, patch), tables, hex, pictures, media tags, executable version info and registry
files, plus a command-line interface, a script language and reports.

## Status

**Version 0.0.0 — specification only. There is no implementation yet.**

This repository currently contains:

- `docs/` — the complete functional and technical specification, written as
  instructions for AI coding agents and human contributors. Start with
  [`docs/00-overview.md`](docs/00-overview.md).
- `core/`, `cli/`, `gui/` — the Cargo workspace skeleton (empty crates) in three
  layers: **core** libraries hold all logic; the **`hikasa` CLI** exposes every
  function headless and connects to the GUI over IPC; the **GUI** is a thin desktop
  front-end over the same core (see [`docs/01-architecture.md`](docs/01-architecture.md)).
- `deny.toml` — the dependency license policy.

## Principles

- Rust only, open-source dependencies only (MIT / Apache-2.0 / BSD / MPL-2.0 / CC0 …).
- Pure, testable comparison engines; UI-agnostic core; **CLI-first**: every tool is a
  `hikasa` subcommand before it is a GUI view; a GUI built with Dioxus
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
