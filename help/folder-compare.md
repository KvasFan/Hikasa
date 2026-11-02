# hikasa folder compare

`hikasa folder compare <left> <right>` compares two folder trees and lists the items with their state. By
default it shows the differences with the folder structure; the [exit code](topic:exit-codes) is the verdict.

## What is compared

- `--filters <masks>` keeps or excludes names (`*.c;-*.bak;-obj\`).
- `--criteria <list>`, or the single switches `--size`, `--timestamps`, `--tolerance <secs>`, `--ignore-dst`,
  `--ignore-timezone`, `--filename-case`, `--attributes <letters>`, `--permissions`, `--owner`, `--group`.
- `--content none|crc|binary|rules` adds a content test; `--skip-if-quick-same` and `--no-override-quick`
  tune it; `--text-rules-file` and the `--unimportant…` flags apply to `rules`.
- The bases can be [archives](topic:archives); `--archives files|opened|folders` and `--open-archive <PATH>` say what
  an archive inside a tree is.
- `--rules-file <toml>` loads folder rules; `--follow-symlinks`, `--flatten`, `--no-scan-orphans`,
  `--case-sensitive`, `--case-insensitive`, `--cutoff`, `--size-filter`, `--include-protected`.

- `--attrib-filter <expr>` hides regular files by DOS attribute (`+rh-s`), `--unixtype-filter <expr>` hides anything but
  folders by Unix type, `b c l p r s` (`-lp`); `none` turns either off.
- `--no-auto-scan` scans only the bases, and the orphan folders in a base with their subtrees. A folder pair not scanned
  is unknown, so the verdict is incomplete. `--expand-on-load` and `--expand-differences` say what a view opens.
- `--ignored <path>` marks a row and all below it, `!<path>` is an exception (a name that itself starts with `!` is
  written `./!name`); the longest path decides. A difference there counts as same in the verdict, the exit code and the
  statistics; an error stays an error.
- `--format-on <id>` and `--format-off <id>` turn a file format on or off for this comparison.

## What is shown

- `--display <filter>` (default `differences`), `--folders always|structure|files-only`,
  `--suppress-filters`, `--expand-all`, `--ignore-unimportant` (`--iu`).
- `--report list|side-by-side|summary|json`, `--columns <list>`, `--html`, `--html-mono`, `--title`,
  `--output <file>`.

## In the window

`--gui`, `--wait` and `--no-launch` open the comparison in the window ([hikasa gui](topic:gui)). See
[The Folder Compare view](topic:view-folder-compare).
