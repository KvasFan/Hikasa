# hikasa folder compare

`hikasa folder compare <left> <right>` compares two folder trees and lists the items with their state. By
default it shows the differences with the folder structure; the [exit code](topic:exit-codes) is the verdict.

## What is compared

- `--filters <masks>` keeps or excludes names (`*.c;-*.bak;-obj\`).
- `--criteria <list>`, or the single switches `--size`, `--timestamps`, `--tolerance <secs>`, `--ignore-dst`,
  `--ignore-timezone`, `--filename-case`, `--attributes <letters>`, `--permissions`, `--owner`, `--group`.
- `--content none|crc|binary|rules` adds a content test; `--skip-if-quick-same` and `--no-override-quick`
  tune it; `--text-rules-file` and the `--unimportant…` flags apply to `rules`.
- `--rules-file <toml>` loads folder rules; `--follow-symlinks`, `--flatten`, `--no-scan-orphans`,
  `--case-sensitive`, `--case-insensitive`, `--cutoff`, `--size-filter`, `--include-protected`.

## What is shown

- `--display <filter>` (default `differences`), `--folders always|structure|files-only`,
  `--suppress-filters`, `--expand-all`, `--ignore-unimportant` (`--iu`).
- `--report list|side-by-side|summary|json`, `--columns <list>`, `--html`, `--html-mono`, `--title`,
  `--output <file>`.

## In the window

`--gui`, `--wait` and `--no-launch` open the comparison in the window ([hikasa gui](topic:gui)). See
[The Folder Compare view](topic:view-folder-compare).
