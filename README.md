<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/logo-dark.svg">
    <img src="assets/brand/logo-light.svg" alt="Hikasa logo" width="128" height="128">
  </picture>
</p>

<h1 align="center">Hikasa</h1>

<p align="center"><b>A folder and file compare tool for the desktop and the command line.</b></p>

<p align="center">
  Compare two folder trees, text files, tables, binary files or pictures, inside zip and tar archives too, see what changed down to the character,
  keep the setup as a session and run it again from the window or from a script. Written in Rust, with an egui window and
  a <code>hikasa</code> command that does the same comparisons without one. Edit text and synchronize folders in the same
  window; merging comes next: see <a href="#what-works-today">what works today</a>.
</p>

<p align="center">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-1e2a44?style=flat-square&logo=rust">
  <img alt="Runs on Linux · Windows" src="https://img.shields.io/badge/runs%20on-Linux%20%C2%B7%20Windows-1e2a44?style=flat-square&logo=linux&logoColor=white">
  <img alt="License MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-1e2a44?style=flat-square">
  <img alt="Status 0.2.0" src="https://img.shields.io/badge/status-0.2.0-6aa0ff?style=flat-square">
</p>

<p align="center">
  <img src="docs/images/text-compare.png" alt="Text Compare of two versions of a Rust source file: changed lines tinted red on both sides, the changed characters marked inside them, hatched gaps where the other side has extra lines, and syntax colouring">
  <br>
  <sub>Text Compare: changed lines tinted, the changed characters marked inside them, gaps where one side has more lines, syntax colouring from the file format. Project-made demo data.</sub>
</p>

<p align="center">
  <a href="#screenshots">Screenshots</a> ·
  <a href="#why-hikasa">Why Hikasa</a> ·
  <a href="#what-works-today">What works today</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#roadmap">Roadmap</a> ·
  <a href="#downloads">Downloads</a> ·
  <a href="#license-and-credits">License and credits</a>
</p>

## Screenshots

<table>
  <tr>
    <td width="50%"><img src="docs/images/folder-compare.png" alt="Folder Compare of two versions of a small source tree, every folder open: newer files in red, older ones in grey, files on one side only in purple with a hatched gap opposite, identical files in black"><br><sub>Folder Compare: red is newer, grey is older, purple is on one side only, black is the same.</sub></td>
    <td width="50%"><img src="docs/images/home.png" alt="Home view: a tree of named sessions in folders with an Auto-saved group below, and the description, left and right paths of the selected session"><br><sub>Home: sessions in folders, the auto-saved ones by day, the selected session ready to open.</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/images/options.png" alt="Options dialog over a Folder Compare: a page tree with a search field on the left, the Tabs page on the right, with Apply, OK and Cancel"><br><sub>Options: a searchable page tree over the settings files.</sub></td>
    <td width="50%"><img src="docs/images/theme.png" alt="Options, File Views page with a colour picker open on important line background: the changed lines in the Text Compare behind the dialog have turned green"><br><sub>The theme editor changes the views as you pick: here the changed lines behind the dialog turn green.</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/images/shortcuts.png" alt="Options, Toolbars, Shortcuts, Menus page for Folder Compare: commands with their shortcuts, Menu and Toolbar check boxes, and Add, Remove and Default for the selected command"><br><sub>Toolbars, Shortcuts, Menus: each view has its own commands and keys.</sub></td>
    <td width="50%"><img src="docs/images/import-wizard.png" alt="Import Settings wizard over a Text Compare: the package path, then program state and sessions with check boxes, each session marked Replace, and options for locked sessions and deleting existing items"><br><sub>The Import wizard: choose what a settings package brings in, item by item.</sub></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/images/dark-theme.png" alt="The dark theme on a Text Compare of the same Rust file, from the first line"><br><sub>The dark theme, on Text Compare.</sub></td>
    <td width="50%"><img src="docs/images/dark-folder.png" alt="The dark theme on a Folder Compare showing only the differences, in a second tab"><br><sub>The dark theme, on Folder Compare with only the differences shown.</sub></td>
  </tr>
</table>

## Why Hikasa

- **The command line comes first.** Every comparison is a `hikasa` subcommand before it is a view, with `--json` output and documented exit codes (1 or 2 same, 13 different, 12 only unimportant differences), so it fits scripts and CI jobs.
- **Differences that matter.** Each file format knows its grammar, so a rule can say what is unimportant: `--unimportant element:Comment` turns a changed comment into exit code 12 instead of 13. Whitespace, case, plain text and regular expressions work the same way. 21 file formats are built in, among them Rust, C and C++, Python, JavaScript, Java, SQL, HTML and XML.
- **Folders by your rules.** Compare by size, timestamps, permissions, CRC, byte by byte, or with the text rules, with name filters and display filters on top.
- **Built for big inputs.** Measured at 0.1.0 on one desktop (Ryzen 7 7800X3D, NVMe, Linux): a cold scan of 100,000 files per side, sizes and timestamps only, in 0.694 s, and a side-by-side view of two 1,000,000-line files that opens in 0.51 s and scrolls at the display's 120 Hz. Not yet measured on a 2020-class laptop.
- **Sessions keep your setup.** Named sessions in folders, auto-saved ones, locks, workspaces that restore windows and tabs, and settings packages that move all of it to another machine.
- **Plain settings.** TOML files you can read, change with `hikasa settings get|set` or in the Options dialog, and carry in a portable folder with `hikasa-portable.toml`.
- **Rust and open source only.** No proprietary SDK, open-source dependencies under a license policy that `cargo deny` checks ([`deny.toml`](deny.toml)).

## What works today

This page describes version 0.2.0, milestone M2: every P0 requirement of the folder, text, table, hex and picture views, sessions, the window and its options, archives, reports and scripts, on the command line and in the window. In the [command catalogue](docs/reference/commands.md), 350 of 518 commands are implemented; the rest belong to later milestones (merging, the media, version and registry views, editing of tables and bytes) and stay out of the menus.

| Area | Status |
|---|---|
| Folder Compare: parallel scan, quick tests, CRC, byte and rules-based content tests, columns, sort, display, name, attribute and Unix-type filters, Ignored marks, legend, Find Filename, Info | Works in the window and with `hikasa folder compare` |
| File operations: copy, move, exchange, delete (recycle bin or not), rename by mask, touch, new folder, open with, Exclude, drag and drop, clipboard file lists; each one a plan that is previewed, confirmed and executed with per-item results and backups | Works in the window, with `hikasa folder op` and `hikasa vfs cp\|mv\|rm\|mkdir\|touch`; the Attributes dialog is M3 (the `attrib` command and script command work) |
| Folder Sync: Update, Mirror and custom presets, per-row overrides, the Folder Sync view, folder-sync sessions | Works in the window and with `hikasa folder sync`; Sync Memory is M3 |
| Text Compare: alignment (standard, Myers, histogram, patience), inline differences, syntax colouring from 21 file formats, important and unimportant differences, display filters, layouts, details panel, thumbnail, bookmarks, Align With and Isolate | Works in the window and with `hikasa text compare` |
| Text editing: both panes and the Text Edit view, undo, a live recompare, saves that keep every unchanged line byte for byte, backups, find and replace, Find in Files with replace in files, conversions | Works in the window, with `hikasa text find` and `hikasa text convert` |
| File formats: the File Formats manager, the grammar editor with preview and regex helper, a format per pane | Works in the window and with `hikasa format` |
| Table Compare: CSV, TSV, fixed widths, every sheet of xlsx, xlsm, xlsb, xls and ods; keys, tolerances, regional settings | Works in the window's grid and with `hikasa table compare`; editing cells is M3 |
| Hex Compare: files of many gigabytes, None, Fast and Complete alignments, Go To, Find, value panel, Layout dialog | Works in the window and with `hikasa hex compare\|dump\|find`; editing bytes is M3 |
| Picture Compare: PNG, JPEG, GIF, BMP, ICO, TIFF, WebP, TGA, PNM, QOI, EXR, HDR; tolerance, range and blend, Auto Scale, offsets, rotations, flips, a difference image, zoom and synchronized panning | Works in the window and with `hikasa picture compare`; DDS and metadata comparison are M3 |
| Archives: zip, tar, `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst` and single gz, bz2, xz and zst streams, nested too, as folders and file sides; zip and the tar family written by rebuilds | Works everywhere a spec is taken; encrypted members are listed, not decrypted; single streams are read-only; no 7z (M3) |
| Clipboard and standard input as specs: `clipboard://` (text and images), `-` | Works on the command line and in the window |
| Reports: side-by-side, interleaved, XML, summaries, patches, statistics of text, folder, table, hex and picture comparisons; links to file reports; a file, standard output, the clipboard, the printer or the browser | Works with `hikasa report` and the report flags, and in the window's report dialog with View in Browser and Print Preview (the HTML report in the browser) |
| Sessions, auto-save, locks, session folders, defaults, workspaces, settings packages | Works in the window and with `hikasa session`, `workspace` and `settings export\|import` |
| Scripts: load, filter, expand, select, copy, move, delete, sync, compare, log, option, reports | Works with `hikasa script run\|check`, and in the window with its status window (`hikasa gui script`) |
| Window: several windows, tabs, menus, toolbars and shortcuts from the catalogue, Options, the theme editor, light and dark themes, the settings wizards, Home, status bar, log, notices, help on F1, Quick Compare at startup | Works under Xvfb and in tests; checks on a real desktop and a real GPU are open (see below) |
| Version control: `hikasa gui open a b --wait` as the diff tool of git, Mercurial, Subversion and Perforce | Works; settings in `hikasa help version-control` |
| Text Merge, Folder Merge, Sync Memory, remote back-ends, snapshots, profiles, 7z, document converters, media, version and registry views | Not yet (M3) |

CI runs clippy, the tests and the doc tests on Linux, Windows and macOS. The tests that open a window are not part of CI; they run locally on Linux under Xvfb with software rendering, where the window gates were measured for this release ([docs/02](docs/02-tech-stack.md), *M2 W14: the release run*). The window tests on Windows and macOS, the frame-rate run on a GPU and a run on a 2020-class laptop are still open, and the known limitations of 0.2.0 are listed in the [CHANGELOG](CHANGELOG.md).

## Quick start

You need Rust 1.97 or newer and a C compiler. On Linux the window needs an X11 or Wayland session at run time; building needs no GUI development packages.

```sh
git clone --branch v0.2.0 https://github.com/KvasFan/Hikasa.git
cd Hikasa
cargo build --release
target/release/hikasa --help
```

Compare from the command line:

```sh
target/release/hikasa text compare left.txt right.txt
target/release/hikasa text compare a.rs b.rs --unimportant element:Comment
target/release/hikasa text compare a.rs b.rs --report side-by-side --html > diff.html
target/release/hikasa folder compare left/ right/ --json
target/release/hikasa help text compare
```

Compare in the window:

```sh
target/release/hikasa-gui                      # the window (Home)
target/release/hikasa-gui left/ right/         # or open a comparison directly
target/release/hikasa gui open a.txt b.txt --wait
```

Keep a comparison as a session, then run it with or without the window:

```sh
target/release/hikasa session mkdir review
target/release/hikasa session new --type text-compare --left a.txt --right b.txt review/notes
target/release/hikasa session run review/notes
target/release/hikasa gui session review/notes
```

The exit code is the verdict; [help/exit-codes.md](help/exit-codes.md) lists them all and [docs/12](docs/12-cli-and-scripting.md) covers the command line. `hikasa completions <shell>` prints completions for bash, zsh, fish, elvish and PowerShell.

Settings live in `~/.config/hikasa` (or `$XDG_CONFIG_HOME/hikasa`), `%APPDATA%\Hikasa` on Windows and `~/Library/Application Support/Hikasa` on macOS. A file named `hikasa-portable.toml` next to the executable keeps them in that folder instead.

## Architecture

A Cargo workspace in three layers. The **core** crates hold all the logic and no UI: `hikasa-core`, `-vfs` (file access), `-diff` (alignment), `-formats`, `-text`, `-folder`, `-table`, `-hex`, `-picture`, `-report`, `-script` and `-session` (the service the other two layers call); `-metadata` is still empty (M3). **`hikasa`** (`cli/hikasa-cli`) exposes every function headless and talks to a running window over IPC. **`hikasa-gui`** (`gui/hikasa-gui`, egui and eframe) is a thin front-end over the same core; a second launch forwards its arguments to the running one. See [docs/01](docs/01-architecture.md).

Linux comes first, then Windows and macOS.

| Document | Contents |
|---|---|
| [docs/00-overview.md](docs/00-overview.md) | Vocabulary, quality bar, requirement notation: start here |
| [docs/01-architecture.md](docs/01-architecture.md) | Layers, crates, IPC |
| [docs/02-tech-stack.md](docs/02-tech-stack.md) | Dependencies, the GUI toolkit evaluation, performance gates and measurements |
| [docs/03](docs/03-sessions-settings-profiles.md) … [docs/15](docs/15-options-reference.md) | One specification per area: folder, text, table, hex and picture views, formats, VFS, CLI, reports, UI, options |
| [docs/16-roadmap.md](docs/16-roadmap.md) | Milestones and what is done |
| [docs/17-agent-workflow.md](docs/17-agent-workflow.md) | The rules for human and AI contributors |
| [docs/design/](docs/design/m2-plan.md) | The M2 plan and the binding contracts of M1 and M2 |
| [docs/reference/commands.md](docs/reference/commands.md) | The command catalogue, generated |
| [help/](help/index.md) | The help topics of `hikasa help` and F1 |
| [packaging/README.md](packaging/README.md) | How the packages are built, and the release steps |
| [CHANGELOG.md](CHANGELOG.md) | What changed: per release, and wave by wave within M2 |

The specification comes first and is written as instructions for AI coding agents and human contributors; the implementation follows it wave by wave. To work on the code, read docs/00, 01, 02, 16 and 17 in that order, then the area specification and the command catalogue.

### Development

```sh
cargo install --locked cargo-nextest cargo-deny
```

| Command | What it does |
|---|---|
| `cargo build --release` | `target/release/hikasa` and `hikasa-gui` |
| `cargo fmt --all --check` | formatting |
| `cargo clippy --workspace --all-targets -- -D warnings` | lints |
| `cargo nextest run --workspace` | the tests |
| `cargo test --workspace --doc` | the doc tests |
| `cargo deny check` | licenses, advisories, bans and sources |
| `cargo xtask commands-doc [--check]` | writes `docs/reference/commands.md` from the catalogue, or fails when it is stale |
| `cargo xtask help-check` | every subcommand, view and dialog has a help topic, and every link resolves |
| `env -u WAYLAND_DISPLAY xvfb-run -a cargo test -p hikasa-gui --test cli_gui -- --ignored` | the tests that open a window, on a virtual X display |
| `packaging/build-all.sh` | every package into `target/dist/`, with `SHA256SUMS` |
| `packaging/smoke-test.sh` | tests the packages; nothing is installed on the system |

The screenshots on this page are real captures of `hikasa-gui` on a virtual display with software rendering; [`docs/images/make-demo.sh`](docs/images/make-demo.sh) generates the demo data and records how they were made.

## Roadmap

0.0.0 was the specification and 0.1.0 (M1) brought the core engines, the command line and the first window. 0.2.0 (M2) is the usable tool: editing, file operations and sync, sessions and workspaces, archives, Table, Hex and Picture Compare, reports and scripts, in the window and on the command line (0.1.90–0.1.93 were its pre-releases). M3, for 1.0, adds Text Merge and Folder Merge, Sync Memory, remote back-ends (FTP, SFTP, WebDAV, S3), snapshots, profiles, 7z, document converters, shell integration, accessibility and the user manual. After 1.0 come cloud drives, a plugin API and more archive types. The details are in [docs/16-roadmap.md](docs/16-roadmap.md).

## Downloads

The packages of 0.2.0 are on its [release page](https://github.com/KvasFan/Hikasa/releases/tag/v0.2.0), with `SHA256SUMS`. They are built by hand from [`packaging/`](packaging/README.md) on the maintainer's machine.

| Format | File | Status |
|---|---|---|
| Source | `cargo build --release` | See [Quick start](#quick-start) |
| Linux x86-64 `.tar.gz` | `hikasa-0.2.0-x86_64-linux.tar.gz` | Smoke tested |
| Linux x86-64 AppImage | `Hikasa-0.2.0-x86_64.AppImage` | Smoke tested; needs FUSE, or `--appimage-extract-and-run`; `--cli` reaches the command line |
| Debian and Ubuntu `.deb` (amd64) | `hikasa_0.2.0-1_amd64.deb` | Smoke tested |
| Arch Linux package | `hikasa-0.2.0-1-x86_64.pkg.tar.zst` | `packaging/aur/PKGBUILD` builds the tagged source; not on the AUR yet; linked against the system's glibc |
| Windows x64 MSI | `hikasa-0.2.0-windows-x64.msi` | Tested under wine only, not on real Windows |
| Windows x86 MSI | `hikasa-0.2.0-windows-x86.msi` | Tested under wine only, not on real Windows |
| Windows x64 portable zip | `hikasa-0.2.0-windows-x64-portable.zip` | Tested under wine only, not on real Windows; settings stay in the zip's folder |
| Windows x86 portable zip | `hikasa-0.2.0-windows-x86-portable.zip` | Tested under wine only, not on real Windows; settings stay in the zip's folder |

The `.tar.gz`, AppImage and `.deb` are linked against glibc 2.31 (the newest symbol they use is `GLIBC_2.30`), so they should run on distributions of that age, but they have not been run on one yet. The x86 `hikasa.exe` sometimes hangs at exit under wine (one run in 160 in a stress test), and the cause is not known yet. There is no macOS package.

## License and credits

Dual-licensed under [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE), at your option. The logo and icons in `assets/brand/` are original works of the project.

Dependencies are open source under the licenses allowed in [`deny.toml`](deny.toml), mostly MIT and Apache-2.0, with the free font licenses of the fonts egui embeds. Every package carries `THIRD_PARTY_LICENSES.txt`, generated from the crates compiled into it. The interface is built with [egui and eframe](https://github.com/emilk/egui).
