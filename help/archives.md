# Archives

Hikasa reads inside archives wherever it reads files: `hikasa vfs`, `quick`, `text compare`, `folder compare` and
the window take a path into an archive as a spec. In this version archives are read-only: copying **out of** one
works, changing a member does not (`unsupported`).

## Formats

| Type | Default masks | Reading a member |
|---|---|---|
| `zip` | `*.zip;*.jar;*.war;*.ear;*.apk` | decodes that member |
| `tar` | `*.tar` | a range of the file |
| `tar_gz`, `tar_bz2`, `tar_xz`, `tar_zst` | `*.tar.gz;*.tgz`, `*.tar.bz2;*.tbz;*.tbz2`, `*.tar.xz;*.txz`, `*.tar.zst;*.tzst` | the first read decodes up to its member, a second distinct one the whole stream, once |
| `gz`, `bz2`, `xz`, `zst` | `*.gz`, `*.bz2`, `*.xz`, `*.zst` | one stream, one member |

A format is chosen by the name, never by the content, case-insensitively. The masks are the `archives.types.*`
[settings](topic:settings); an empty mask turns a type off. A single stream is an archive of one member named after
the file without its last extension (`x.txt.gz` holds `x.txt`; a name with no extension gets `.data`).

## Specs

- `a.zip/inner/x.txt` is a member, `a.zip/inner` a folder inside, `a.zip/b.tar.gz/c.txt` a member of a nested archive.
- `a.zip` is the file. `a.zip/` with the separator is the archive as a folder. `vfs ls a.zip` and `folder compare
  a.zip b/` want a folder, so they open it; `quick a.zip b.zip` compares the two files.
- A real item always wins: a real folder named `x.zip` stays a folder.
- Member names are case-sensitive on every platform; the masks are not.
- `..` never enters an archive.

## Folder comparisons

Both sides of `folder compare` can be archives, a folder inside one, or one of each. Inside the trees,
`--archives` says what an archive is:

| Mode | An archive in a tree |
|---|---|
| `opened` (default) | a file, until its path is given with `--open-archive <PATH>` (repeatable; relative to the bases, `/`-separated, `a.zip/b.tar.gz` for a nested one) |
| `files` | always a file |
| `folders` | always a folder |

An archive that cannot be read is an error row and the comparison goes on. After 5,000,000 entries listed inside
archives a scan opens no more of them (`archive_scan_limit`). The rules keys are `handling.archives` and
`open_archives`. In the window, Open on a closed archive row adds it and compares again.

With `--content crc`, a zip's stored CRC is used as it is: nothing is decoded, so a damaged member with a good stored
CRC is "same" for the CRC test. The other formats get their CRC when they are indexed.

## Safety

Archives are treated as hostile input.

- **Names.** Absolute names, drive letters, `..`, NUL and names over 4,096 bytes are refused: they are left out of the
  tree and listed as `<refused-N>` rows (at most 1,000) with one `archive_member_refused` warning. A file that is
  also a parent of another member, and anything below a link member, is refused too. When a path appears twice, the
  last one wins (`archive_duplicate_member`).
- **Links.** A link member is listed with its target and never followed; a copy out of an archive skips it
  (`archive_symlink_skipped`). Devices, FIFOs and sparse members are listed and not read.
- **Damage.** A tar or a stream keeps the members before the damage and adds a `<damaged>` row. A zip with no central
  directory, or a mask match with other content, is `invalid_data` when opened; the file itself stays a normal file.
- **Encrypted members** are listed with their name, size and time; reading one is `unsupported`
  (`archive_encrypted`). No password is asked or tried.

## Limits

| Key | Default | Range | Bounds |
|---|---|---|---|
| `archives.limits.entries_max` | 1000000 | 1000…100000000 | members of one archive |
| `archives.limits.stream_max_gib` | 64 | 1…4096 | decoded bytes of one pass and of all reads of one archive |
| `archives.limits.cache_max_gib` | 8 | 1…1024 | the temp cache, and so the largest member that spools |
| `archives.limits.ratio_max` | 2000 | 0…1000000 | output / input past 64 MiB; 0 turns it off |
| `archives.limits.nesting_max` | 8 | 1…32 | archives around a spec |

A limit reached is `too_large` and names the key. Listing a large archive needs no declared-size limit: limits bind
what is decoded.

## The cache

Members over 1 MiB are decoded to files under the cache folder, in `archives/s<n>/`, with numeric names only, so no
member name reaches a path. The oldest files that no reader holds make room. The folder goes at exit; one left by a
crash goes at the next session that extracts. Listing creates nothing.
