# Changelog

## 0.0.0 — 2026-10-19
- Initial specification and workspace skeleton. No functionality.
- Restructured workspace into three layers (`core/`, `cli/`, `gui/`); CLI-first rule:
  every function is a `hikasa` subcommand, the GUI is reached via `hikasa gui` (IPC).
