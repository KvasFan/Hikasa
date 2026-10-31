# Archives

Hikasa reads inside archives wherever it reads files: `hikasa vfs`, `quick`, `text compare`, `folder compare` and
the window take a path into an archive as a spec, and `hikasa vfs` writes there too: zip and the tar family can be
[changed](#changes-rebuilds); a single gz, bz2, xz or zst stream stays read-only.

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

## Changes (rebuilds)

`vfs cp`, `mv`, `rm`, `mkdir`, `touch` and `--output` work on members as on files:

```text
hikasa vfs cp -p notes.txt a.zip/docs/      # copies into the archive; -p makes the missing folder
hikasa vfs cp -r photos/ b.tar.gz/          # a tree, into a compressed tar
hikasa vfs mv a.zip/old.txt a.zip/new.txt   # a rename inside the archive
hikasa vfs rm --yes a.zip/old/              # members, for good: a member has no recycle bin
hikasa vfs mkdir new.zip                    # an empty archive
```

**An archive is rewritten, once per operation.** The items only change what Hikasa sees; at the end the whole archive is
written to a temporary file next to it, which then replaces it in one step. So a change costs one copy of the archive,
however small the change, and it needs that much free space beside it. Anything that goes wrong on the way (a full
disk, a read error, Cancel) leaves the archive as it was, and the items of that archive report `failed` (`not_run` when
cancelled). A move into an archive removes its sources only after the rewrite. Nested archives are rewritten from
the inside out, as one step.

**What a rewrite keeps.** Members the operation did not touch stay the same bytes, in their order, with their metadata:
also members it could not read itself, such as encrypted ones or methods it does not know, refused and link members,
devices and sparse files. A zip keeps its comment and its members' comments and extra fields. The permissions of the
archive file stay; it gets a new modification time. A renamed or touched member keeps its data.

**What a rewrite does not keep.**

- Members hidden by a later one of the same name go, with an `archive_duplicate_member` warning.
- A signed `.jar` or `.apk` is no longer validly signed.
- A gzip header's stored name and time go: rewrites are repeatable, byte for byte.
- Its compression becomes the format's usual level (deflate 6, gzip 6, bzip2 9, xz 6, zstd 3) for the tar family;
  kept zip members are not recompressed.

**What is refused**, with the archive unchanged:

- the archive changed since the operation was planned (`changed since the plan was made`);
- a damaged archive (`invalid_data`), since what lies past the damage would be lost;
- a zip with a prefix, such as a self-extractor (`unsupported`);
- moving or touching a sparse member, or moving a folder that holds a member whose name does not decode
  (`unsupported`): neither could be written back as it was;
- a result that would pass the limits below, which nothing could read back (`too_large`);
- `--trash` on a member (`unsupported`).

Another operation on the same archive at the same moment fails with `another operation is changing this archive`. A
hard link inside a tar stays one while the member it names stays; when that member goes or moves, the link becomes a
regular member with the content it had. New members are stored as Unix mode `0644`, folders `0755`, with the current
time; a copy then applies the source's time and mode as for any target. `vfs mkdir` makes an empty archive from any
writable mask, `x.gz` stays a plain folder, and in `mkdir -p new.zip/sub` only the last name counts.

## Folder comparisons

Both sides of `folder compare` can be archives, a folder inside one, or one of each. Inside the trees,
`--archives` says what an archive is:

| Mode | An archive in a tree |
|---|---|
| `opened` (default) | a file, until its path is given with `--open-archive <PATH>` (repeatable; relative to the bases, `/`-separated, `a.zip/b.tar.gz` for a nested one) |
| `files` | always a file |
| `folders` | always a folder |

An archive that cannot be read is an error row and the comparison goes on. A scan opens at most 5,000,000 entries
inside archives: an archive that would pass that stays a file (`archive_scan_limit`). The rules keys are `handling.archives` and
`open_archives`. In the window, Open on a closed archive row adds it and compares again.

With `--content crc`, a zip's stored CRC is trusted as it is: nothing is decoded, so a damaged member with a good stored
CRC, or a member changed together with its CRC, is "same" for the CRC test. Use `--content binary` to compare the
content itself. The other formats get their CRC when they are indexed.

## Safety

Archives are treated as hostile input.

- **Names.** Absolute names, drive letters, `..`, NUL and names over 4,096 bytes are refused: they are left out of the
  tree and listed as `<refused-N>` rows (at most 1,000) with one `archive_member_refused` warning. A file that is
  also a parent of another member, anything below a link member, and a file named `.` are refused too. When a path appears
  twice, the last one wins (`archive_duplicate_member`). Names that only look equal because bytes could not be decoded are
  two members: the last stays and the others are refused.
- **Modes.** A member copied out keeps its `rwx` bits within what the platform gives a new file, and never `setuid`, `setgid`
  or sticky. On Windows, names that are no file names there (`CON`, `aux.txt`, a trailing dot) fail as items.
- **Links.** A link member is listed with its target and never followed; a copy out of an archive skips it
  (`archive_symlink_skipped`). Devices, FIFOs and sparse members are listed and not read.
- **Damage.** A tar or a stream keeps the members before the damage and adds a `<damaged>` row. A zip with no central
  directory, or a mask match with other content, is `invalid_data` when opened; the file itself stays a normal file. NUL
  bytes after a gzip or bzip2 stream are padding: ignored, with an `archive_trailing_zeros` warning.
- **Encrypted members** are listed with their name, size and time; reading one is `unsupported`
  (`archive_encrypted`). No password is asked or tried.

## Limits

| Key | Default | Range | Bounds |
|---|---|---|---|
| `archives.limits.entries_max` | 1000000 | 1000…100000000 | members of one archive, and the folders that only exist through them |
| `archives.limits.stream_max_gib` | 64 | 1…4096 | decoded bytes of one pass, and of zip members whose data overlap |
| `archives.limits.cache_max_gib` | 8 | 1…1024 | the temp cache, and so the largest member that spools |
| `archives.limits.ratio_max` | 2000 | 0…1000000 | output / input past 64 MiB; 0 turns it off |
| `archives.limits.nesting_max` | 8 | 1…32 | archives around a spec |
| `archives.limits.paths_max_mib` | 256 | 1…65536 | the text of one index: paths, link targets, the paths of folders no member names |
| `archives.limits.depth_max` | 256 | 16…2048 | folders above a member |

A limit reached is `too_large` and names the limit. They bind what is written too: a new member that does not fit the
cache, or an archive that would pass a limit, is refused. Listing a large zip or plain tar needs no declared-size limit:
limits bind what is decoded, and a compressed tar is decoded to be listed. A zip whose entries declare more than
`ratio_max` times its size blocks reads until `ratio_max` is 0; it can still be listed.

## The cache

Members over 1 MiB are decoded to files under the cache folder, in `archives/s<n>/`, with numeric names only, so no
member name reaches a path. What an operation writes into an archive waits there until the rewrite. The oldest files that no reader holds make room. The folder goes at exit; one left by a
crash goes at the next session that extracts. Listing creates nothing. Several running Hikasa keep apart in one
cache folder, which must belong to the user.
