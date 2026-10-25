# Changelog

## Unreleased — M1 wave 5: the `hikasa` command line
- `hikasa-cli`: the `hikasa` binary over the session service (contract §1.9): `quick`, `text compare`,
  `folder compare`, `format list|show|detect|tokens`, `vfs ls|stat|cat`, `gui open|ping|list|close|quit`,
  `--gui` on the compare commands, `completions` (bash, elvish, fish, powershell, zsh) and `version`; the global
  flags `--json`, `--quiet`, `--log`, `--verbose`, `--config-dir`, `--no-color`.
- Exit codes per CLI-004 and contract §2.21: usage errors and caught panics exit 100 (never 2 or 101), a broken
  pipe keeps the computed code; `gui open --wait` returns the view's code, 102 when it cannot wait, 103 without
  a GUI executable (D-01, D-02, D-03, D-22, D-23, D-35).
- One JSON document per invocation on stdout (also on error, `hikasa.usage_error.v1` for a command line that does
  not parse); progress and logs on stderr; `--output` reports written atomically (D-29, D-32, D-40).
- Vocabulary words are parsed by the core crates only; `gui open` uses the shared legacy parser (D-51, D-57).
- Integration tests for every subcommand with goldens for every JSON schema. The launch and wait paths run in
  the `cli_gui_launch` test target (`harness = false`), whose own binary is the scripted GUI when
  `HIKASA_TEST_FAKE_GUI=1`; no test GUI is built or installed as a program.
- Gates G1 and G2 through the binary (`cargo test --release -p hikasa-cli --test perf_gates -- --ignored`;
  not run in CI). Measured 2026-10-25 (Ryzen 7 7800X3D, release): G1 (2 × 1M lines, `--report summary`)
  0.43 s auto, 0.45 s standard, 0.36 s identical, peak RSS 298 MiB (bound 427 MiB; threshold 3 s); G2 (100k files
  per side, warm, `--content none`) 0.05 s (threshold 1.5 s; cold not measured).
- Wave-5 review fixes: `gui open -- -x a.c` reaches the shared parser with its `--` (D-51); `--json` counts
  only before `--`, a legacy `-json` selects the usage-error document when the `gui open` arguments do not
  parse, and that document keeps clap's whole message; `--version` prints `hikasa <version>` after a subcommand
  too (with `--json` the version document); a global value flag repeated with different values at different
  positions exits 100; `quick --crc|--binary` on an unreadable file reports the error on stderr and with
  `ok: false` in JSON; one error line per failed stdout write; the completions golden checks the field order.
- `hikasa-session`: a GUI executable that exists but cannot be started exits 103 (`ApiError::GuiNotStartable`,
  was 100 "ipc I/O"); `find_gui_exe` resolves a relative `HIKASA_GUI_EXE` against the current folder and skips
  candidates without an execute bit.
- `hikasa-vfs`: writing to a symbolic link replaces the file it points to and keeps the link; a FIFO or a
  device (`/dev/null`) is written to directly and never replaced (`--output`; contract §1.2.7 item 12).
- `docs/12-cli-and-scripting.md`: the M1 flag set, the wave-5 decisions and the measured gates; contract
  §1.9.3 (accepted additions and deviations of wave 5), §1.9.2 and §6.1 amended; `docs/11` §1 local writes.

## 0.0.0 — 2026-10-19
- Initial specification and workspace skeleton. No functionality.
- Restructured workspace into three layers (`core/`, `cli/`, `gui/`); CLI-first rule:
  every function is a `hikasa` subcommand, the GUI is reached via `hikasa gui` (IPC).
