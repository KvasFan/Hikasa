# hikasa vfs

`hikasa vfs` reads any spec the comparisons read, through the same virtual file system.

| Command | Result |
|---|---|
| `hikasa vfs ls <spec>` | the items of a folder: kind, size, time, name |
| `hikasa vfs stat <spec>` | the metadata of one item |
| `hikasa vfs cat <spec>` | the bytes of a file on stdout (`--json`: base64) |

`ls` and `stat` report symbolic links themselves; `--follow-symlinks` reports their targets.

A spec is a local path, `-` for standard input or `clipboard://`. Archive members, snapshots and remote
specs parse but are not available yet. See [Using the command line](topic:cli).
