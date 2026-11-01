# Virtual File System: Local, Archives, Snapshots, Remote Services

Everything that "has a structure of folders and files" is a VFS back-end and is
usable wherever a folder or file spec is accepted (folder sessions, file sessions,
scripts, command line, Copy to Folder targets, reports).

## 1. Back-ends

| Back-end | Read | Write | Timestamps | Attrs/Mode | Notes | Priority |
|----------|------|-------|------------|------------|-------|----------|
| Local file system | ✓ | ✓ | ✓ | DOS attrs (Windows), Unix mode/owner/group (Unix) | symlinks, junctions, recycle bin/trash, creation dates | P0 |
| Network shares (SMB/NFS mounted) | ✓ | ✓ | ✓ | ✓ | treated as local paths (`\\server\share`, mounted paths); `smb://` URLs resolved via GIO/mount helper (P2) | P0/P2 |
| Archives | ✓ | ✓ (zip, tar.*, 7z) | ✓ (stored) | stored flags | nested archives; CRC32 provided by zip | P0 (zip/tar/gz/bz2/xz/zst), P1 (7z), P2 (rar read-only opt-in, cab, iso, deb, rpm, wim, chm) |
| Snapshot (`.hksnap`) | ✓ | ✗ | ✓ | ✓ | no content; optional CRC/version | P1 |
| FTP / FTPS | ✓ | ✓ | ✓ (MDTM/MFMT when supported) | chmod (SITE CHMOD) | listing parsers for Unix/DOS/VMS formats, MLSD | P1 |
| SFTP | ✓ | ✓ | ✓ | ✓ | key/agent auth | P1 |
| WebDAV(S) | ✓ | ✓ | partial | ✗ | PROPFIND depth 1 (or recursive option) | P1 |
| S3-compatible object storage | ✓ | ✓ | object mtime | ✗ | buckets as top level; prefixes as folders; AWS credential files/env | P1 |
| Cloud drives (OAuth: Dropbox, OneDrive, Google Drive) | ✓ | ✓ | partial | ✗ | profile-only | P2 |
| Subversion repository | ✓ | ✗ | ✓ | ✗ | `svn://`, `svn+http(s)://`, `svn+ssh://`, `@rev` | P2 |
| HTTP(S) single file | ✓ | ✗ | Last-Modified | ✗ | file sessions only | P1 |
| Clipboard | ✓ | ✗ | now | ✗ | text or image (an image as a PNG named `clipboard.png`); read once per command, only when a `clipboard://` spec is given; no clipboard → `unsupported`, empty → `not_found` (exit 100); `HIKASA_CLIPBOARD_FILE` stands in for it (D-100) | P0 |
| Stdin / pipes | ✓ | ✗ | now | ✗ | `-` on the command line | P0 |
| MTP devices | ✓ | ✓ | ✓ | ✗ | `mtp://Device/…` via libmtp bindings | P2 |

- **VFS-001** (P0): Every back-end implements the `Vfs` trait (`01-architecture.md`
  §3.2) and declares capabilities; operations not supported are greyed out and scripts
  log a warning.
- **VFS-002** (P0): Archive members are opened by extracting to a per-session temp
  cache on demand (streaming for tar/gz); writes rebuild the archive atomically
  (temp + rename) and update CRCs. "New Folder" with an archive extension creates an
  empty archive. Archive **type masks** are configurable in options (Archive Types page;
  empty mask disables a type). *M2 W5a (read; contract §1.5):* `ArchiveVfs` is stacked on a base `Vfs` and serves
  zip, tar, `tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst`, and the single streams gz, bz2, xz and zst (one member named
  after the file without its last extension). A format is chosen by mask, never by content. Listing and CRC tests
  never write to disk; a zip member up to 1 MiB decodes to memory, a larger one to a cache file; a plain tar member is
  a range of the file; the first read of a compressed tar spools its member and a second distinct read writes the
  whole stream to the cache once. *M2 W5b (write; §1.5.10):* zip and the tar family are written through a batch and a
  rebuild, below; a primitive on a single stream, or below one, is `unsupported`.

**Archives in user terms (M2 W5).**

| Format | Reported besides read, list, seek, modification time, case-sensitive names |
|---|---|
| zip | stored CRC, DOS attributes, Unix mode, symbolic links; write, mkdir, remove, rename, set times, set attributes (`Mode`, and `Dos` with the `R` bit only), create links |
| tar family | Unix mode and owner, symbolic links; the same writes (`Mode` only) |
| single streams | nothing more |

A member of a nested archive gets the bits of the innermost format, and the write bits only when every archive around
it can be written.

- *Names.* Members are checked on their raw bytes: NUL, absolute names, drive letters, `..` and names over 4,096 bytes
  are refused, left out of the tree and shown as `<refused-N>` rows (at most 1,000), kept by a rebuild. A zip reads
  `\` as `/`; a tar refuses it. A file that is also a parent, and anything below a link member, is refused whatever
  the order. The last of two equal paths wins; `A.txt` and `a.txt` are two members.
- *Kinds.* A link member is listed and never followed or materialized: a copy out of an archive skips it, whatever the
  link policy. A hard link becomes the earlier regular member it names, else it is refused. Devices and FIFOs are
  `other`; sparse members are listed and not read.
- *Damage.* A tar or stream keeps what lies before the damage and adds a `<damaged>` row. A zip without a central
  directory, or a mask match with other content, is `invalid_data` when entered; the file stays a normal file. A
  decoder panic is `invalid_data` too.
- *Limits* (`archives.limits.*`, OPT-010): 1,000,000 entries, folders that only exist through members included; 256 MiB of
  paths and link targets per index; 256 folders of depth; 64 GiB per decoding pass and over overlapping zip members; 8 GiB
  of cache; ratio 2,000:1 past 64 MiB; nesting 8; xz and zstd windows of 1 GiB. Listing needs no declared-size limit:
  limits bind what is decoded, and a plain tar decodes nothing. A limit fails the archive, never the process.
- *Modes.* Copied out, a member keeps its `rwx` bits within what the platform gives a new file; never `setuid`, `setgid` or
  sticky.
- *Encrypted members* are listed, without a CRC; reading one is `unsupported`. No password is asked or tried.
- *Times.* Zip: the NTFS field, then 0x5455, then the DOS wall time in the injected zone (a time in a gap moves forward,
  a fold takes the earlier instant). Tar: PAX `mtime`, then the header. Streams: gzip `MTIME` when set, else the file.
- *Cache.* `<cache>/archives/s<n>/`, a random number, numeric names only, `0700`; `cache_max` evicts the least recently used
  files no reader holds; a new stamp of the archive drops its files. Created by the first spool, removed at exit with any
  folder that leaves empty; a held lock marks a live session, so a later session sweeps what a crash left, without
  process ids or clocks. Concurrent sessions share the folder safely: a lock counts only for the file at its path, and
  each cache file is checked when opened. The cache root and its parent must belong to the user or the system.
- **VFS-003** (P1): Remote listings are cached per session; "Refresh" invalidates.
  Transfers show per-file progress and throughput; connection limits and bandwidth
  limits come from the profile.
- **VFS-004** (P1): Remote files opened in file views are downloaded to a temp cache
  and uploaded on save; the cache is cleared on exit.

**Rebuilds (M2 W5b, D-148, D-149).** A member write never touches the archive. Primitives change an overlay of the
member tree, which every reader of the back-end sees; a batch holds them for one operation and is finished once, so an
operation costs **one rebuild per archive**, and a rebuild costs one copy of the archive, however small the change. `ops::execute`
is the one place an operation ends: it opens a batch on each archive it touches, finishes target archives before
those that only lose sources, and removes the sources of a move into an archive only after the target's rebuild. A change
outside a batch rebuilds at once, so a plain `open_write` cannot be lost. A batch belongs to the thread that opened it:
another thread's change meets `another operation is changing this archive`. So does another process or back-end: a batch
holds an advisory lock on `.<name>.hikasa-lock` beside the file for as long as it runs, so two writers cannot overlap, and the
second fails at once rather than wait for an archive that will have changed under its plan (D-157). Readers neither take
nor mind it. Its holder removes it; one left by a crash goes with the next writer. A folder or file system that cannot have
it goes without, and then only the stamp below stands between two writers. A rebuild writes through the base's atomic
writer (a temporary sibling, the file a link names being the target, then a rename), so a read error, a full disk, a
cancel or a failed commit leaves the archive as it was; the done items of that archive then become failed (`not_run` when
cancelled). The archive must still have the length, time and, where known, identity it had when the batch began, at the
start of the rebuild and again after the sync, right before the rename, else the items fail "changed since the plan was
made". The temporary file is owner-only until the rename and then gets the archive's mode. Its permissions stay, as for
any local write, and for the same reason the folder decides what may be written: a read-only archive in a writable folder is
rewritten and stays read-only (on Windows that rename fails and it stays as it was). Owner, ACLs and extended attributes
are not carried over, as for any local write, and a second hard link to the archive keeps the old bytes, while a symbolic
link stays. Its modification time is new, which is what tells other sessions it changed. A nested archive is rebuilt
into the cache and enters the archive around it, so a chain commits only at the outermost file, however deep the change and
whether or not the levels between have one of their own. A kill during a rebuild leaves the archive as it was, and the
temporary file and the lock file next to it: there is no signal handler, and the next rebuild of that archive removes
what no live writer holds, and nothing else.

What a rebuild keeps is copied as bytes, never decoded: zip records (local header, data, descriptor; the central
header with its offset changed), so an encrypted member, a method this build does not know, unknown extra fields and raw
names survive; tar blocks by one forward pass, with whatever lies between members (global headers, volume labels) and
anything after the end of the archive. Refused members stay; one refused only for a path conflict reads again once that path
is free, and the operation says so. A shadowed duplicate shares the fate of the member that shadows it (D-156): it stays while
that member stays or is touched, moves with it, and goes when it is removed or replaced, with a warning that names
the count, so what was overwritten does not live on in an older record; `--dry-run` says so first. A renamed or
touched zip member has those fields edited in its headers, so its data is not decoded either, and a name in another code page
keeps the bytes the rename leaves alone; a renamed or touched tar member gets a copy of its header, keeping owner, type,
device numbers, link name and its other PAX records, and a touch leaves its name as bytes. A hard link stays one while the
member it names stays; otherwise it becomes a regular member at its own place, with its content and its header's owner.
Sparse members cannot be moved or touched, and a folder holding a name that does not decode cannot be moved: both are
refused, and so is a target that is the archive holding the source, or a level around it. A time the format cannot
hold is stored as the nearest it can, with a warning. A zip with a prefix and a damaged archive are refused, unchanged. A
signed `.jar` or `.apk` loses its signature's validity, and the signature record of a central directory goes, with a warning.
Limits bind what is written: a stream, ratio, entry, text or depth past them is refused, since nothing could read it back;
only a rebuild that adds content can fail on the ratio.

**Local writes (M1).** Files are written atomically: the content goes to a temporary file
in the target's folder, which is synced and renamed onto the target, so a failure leaves
the target untouched (SES-053; `--output` of the CLI). A target that is a symbolic link is
resolved first: the file the link points to is replaced (the temporary file is created
next to that file) and the link itself stays. A target that is not a regular file — a
FIFO, a character or block device such as `/dev/null` — is never replaced: it is written
to directly, which cannot be atomic.

**Mutations (M2 W3, D-92…D-98).** Every back-end implements single-item primitives —
`create_dir`, `remove` (a file, a link or an empty folder), `rename` (within one back-end),
`set_times` (modification time; creation time on Windows and macOS; never the access time),
`set_attrs` (Unix permission bits; on Windows the read-only attribute only), `create_symlink`,
`move_to_trash` — each behind a capability (`mkdir`, `remove`, `rename`, `set_times`,
`set_attrs`, `create_symlinks`, `set_created`, `trash`), plus an optional fast file copy.
Recursion, copies across back-ends, moves, plans and per-item results live once in
`hikasa_vfs::ops` (ARCH-031). Every operation is planned completely before the first write
(T12) and each item re-checks its target when it runs: an item planned without a target that
finds one fails ("changed since the plan was made"). Copies go through a temporary sibling and a
rename, so a failed or cancelled item leaves its target untouched; local files up to 64 MiB are
copied in one call (`copy_file_range` on Linux), larger ones are streamed so cancellation stays
prompt. Replacing a file by a link or a folder moves the file aside first and puts it back when
the replacement fails. Nothing runs below a folder that could not be created, and `.`/`..` names
and roots are never removed, moved or trashed. Times, then attributes, are
applied after the content; a folder's after its whole subtree; owner and group never.

- **Links are never written through** (D-94): `remove`, `rename`, `move_to_trash` and `set_times`
  act on a link itself; trees are never descended through links and links are copied as links;
  a copy onto a link replaces the link, not the file it points to; root arguments are followed
  only when asked (`--follow-symlinks`). Intermediate components are followed as by every OS call.
  Saving (`open_write` without `replace_link`, `--output`) keeps the behaviour above.
- **Rename race.** `rename` checks the target with `lstat` and then renames; std has no atomic
  "rename unless it exists" without `unsafe`, so a process that creates the target in between is
  replaced. Removing a tree re-checks each folder (kind and file id) before listing it and again
  before touching its contents at execution (also the source folders of copies and moves), which
  narrows, but cannot close, the race with a process that swaps a folder for a link; below a
  changed folder nothing runs.
  On Windows, which renames onto no folder entry, a `rename` that replaces an empty folder or a link to a folder
  removes that entry first and puts it back if the rename fails; that replacement is not atomic there.
- **Moves** rename where they can (same back-end and file system, the target absent or
  replaceable); otherwise they copy and remove item by item, keeping a source folder whose items
  could not all be moved, and removing a fresh copy again when the source cannot be removed (but
  keeping it when the source is already gone). A
  rename that fails across file systems at execution is expanded there (a dry run cannot know it).
- **Recycle bin** (FC-073, D-98): `LocalVfs` only; one call per named item (a folder as a whole,
  a link as a link). It **never falls back to a permanent delete**: when the item cannot go to the
  recycle bin (no trash on that file system, no permission) the item fails and stays. On Linux and
  the BSDs a link on another file system than the home trash is refused, because the trash could
  only receive a copy of what it points to.

## 2. Spec syntax (canonical string forms)

```
/home/user/x                      ~/x            C:\x            \\server\share\x
/path/archive.zip/inner/file.txt  (archive members by path concatenation)
snapshot.hksnap/inner/dir
ftp://user@host/path   ftp://user:pass@host/path   ftp://host//absolute/path
ftps://user@host/path  sftp://user@host:2222/path
dav://host/path  davs://host/path
s3://bucket/prefix/   (credentials from profile named "s3://bucket" or default)
svn://host/path@1234  svn+https://host/repo/trunk
profile:MyProfile              profile:MyProfile?folder/file.ext
http://www.example.com/page.html
clipboard://        -            mtp://MyPhone/DCIM
```

**Archive specs (M2 W5a, D-137, D-138).** `Spec::parse` is unchanged; resolution then maps the path to the item, using
only the `Vfs`: a real item wins, the deepest existing prefix decides, and only a regular file whose name matches a
mask opens a way in. `/x/a.zip/inner/x` is a member, `/x/a.zip/` (canonical, with the trailing `/`) the archive as a
folder, `/x/a.zip` the file; nesting wraps (`a.zip/b.tar.gz/c.txt`). A command that wants a folder (`vfs ls`, `folder
compare`, an input ending in a separator) gets the root. Masks ignore case, members never do; the prefix on disk
follows the file system; `..` never enters. On Windows the canonical form is `C:\x\a.zip/inner/x`, which parses back
to the local path and resolves to the same spec.

- **VFS-010** (P0): Spec parsing is platform-aware (drive letters, UNC, `~`), supports
  relative paths resolved against the current directory on the command line, and
  canonicalizes for display and persistence. Passwords in URLs are stripped from
  persisted forms and moved to the keychain.

**Parsing and canonical form (binding interpretation of ARCH-020/VFS-010; decisions D-05, D-06).** The parser
is `hikasa_core::spec`; the normative algorithm and its 30-row test table are in `design/m1-contracts.md` §2.1.
Parsing is style-explicit (Unix or Windows rules, so the Windows rules are tested on every platform) and takes
the current and home directories from values captured once by the adapter.

- **Order.** A NUL character is an error and blank input is empty; otherwise the input is **not** trimmed
  (leading and trailing spaces are legal in file names). `-` is standard input; `clipboard://` (any case) is
  the clipboard; a `profile:` prefix (any case) is a profile spec `profile:Name[?path]`; text matching
  `scheme://` is a URL; anything else is a local path.
- **Schemes (D-05).** The recognised but unsupported schemes — `ftp`, `ftps`, `sftp`, `dav`, `davs`, `s3`,
  `svn`, `svn+http`, `svn+https`, `svn+ssh`, `mtp`, `profile:` and `http(s)` — **parse successfully**, so specs
  written by later versions stay readable and round-trip; resolving them in M1 fails with "unsupported". An
  unknown `scheme://` fails to parse; it is never silently reinterpreted as a local path. On Windows a
  one-letter scheme is a drive letter (`C://x` is `C:\x`).
- **URLs.** `scheme://[user[:password]@]host[:port][/path]`: the user information is split at the last `@`,
  user and password at the first `:`; an IPv6 host keeps its brackets (`[::1]:22`); the port must be 1–65535;
  the host must not be empty. Scheme and host are lower-cased; the user and the path are kept verbatim (no
  percent-decoding in M1). The canonical form never contains the password, and error messages replace it with
  `***` (conservatively: when a `/` inside a password ends the authority early, everything from the first `:`
  after `://` to the last `@` is masked). An `http(s)` authority ends at the first `/`, `?` or `#` and is
  parsed the same way. Its user information is **kept** like that of the other schemes: the user name
  verbatim, the password as a secret that never appears in the canonical form, in debug output or in an
  error message. The canonical form is `http[s]://[user@]host[:port]rest`.
- **Local paths on Unix (D-06).** `~` and `~/…` expand to the home directory (`~user` is not supported); a
  relative path is resolved against the current directory; empty and `.` components are dropped; `..` is
  **kept**, because removing it lexically is unsound across symbolic links, except that `/..` collapses to
  `/`; there is no trailing `/` except for the root. A backslash is an ordinary character.
- **Local paths on Windows (D-06).** `/` becomes `\`; `\\?\…` (verbatim) is kept unchanged; `\\.\…`
  (device namespace) is rejected; UNC roots are `\\server\share` (a missing share is an error); drive letters
  are upper-cased; `C:foo` resolves against a current directory on drive `C:` (otherwise it is an error) and
  `\foo` against the root of the current directory; `..` is resolved lexically, as Windows itself does;
  there is no trailing `\` except at `C:\`. A relative, rooted or drive-relative input (and `~\…`) is
  normalised component by component also when the current or home directory is itself a verbatim path (as
  `std::fs::canonicalize` returns them): with the current directory `\\?\C:\w`, `..\x` is `\\?\C:\x`.
  Only an input that is verbatim itself stays unchanged.
- **Round trip.** For every input that parses, parsing the canonical form again gives the same spec without
  its password (ARCH-020; removing the secrets clears the password of both remote and `http(s)` URLs). A
  non-UTF-8 local path is accepted from the command line; its canonical form is lossy and such a spec is never
  persisted: serialising it is an error.

## 3. Content fingerprints and CRC

- **VFS-020** (P0): `crc32` is the "CRC comparison" primitive (compatibility with
  archives and snapshots). BLAKE3 is used internally for caches (per-file content
  identity across sessions, P1) and for "binary same" shortcuts when both sides already
  have a known hash.

**Memory-mapped comparison and CRC (M1).** Large local regular files are compared and hashed through
read-only memory maps (ARCH-081), one 8 MiB chunk at a time with a cancellation check per chunk. A call may
use several threads (the caller plus scoped helper threads, at most `StreamOptions::threads`; the session uses
up to 4): the threads claim chunks in increasing order, and each maps, pre-faults, compares or hashes, and
unmaps its own chunks. The results never depend on the number of threads: chunk CRCs are combined exactly in
offset order, and a comparison reports the lowest differing offset. Because another process can change a file
while it is mapped, both files are **re-stat'ed after** a mapped comparison or CRC: when the length and the
modification time are unchanged the mapped result is kept; when either changed, the mapped result is discarded
and the content is compared or hashed again by streaming. The mapped, streaming and naive whole-buffer
comparisons give the same result, including the offset of the first difference. On Linux each chunk is also
pre-faulted (`MADV_POPULATE_READ`, kernel 5.14 or later) before it is read: when a concurrent truncation has
cut the chunk off, pre-faulting reports an error instead of raising SIGBUS, and the comparison or CRC falls
back to streaming without touching the chunk. **Residual risk (accepted):** a truncation of an input file
between pre-faulting a chunk and reading it — on other systems, and on Linux kernels older than 5.14 (which do
not know the advice), at any time while a chunk is mapped — can terminate the process (SIGBUS on Unix); the
mmap policy "never" disables the mapped path. This is the only `unsafe` code in Hikasa's libraries and
programs (`02-tech-stack.md`, "mmap residual risk"; test-only allocation counters aside).

**Pseudo-files.** A reported size of at most 4096 bytes never decides "different" by itself: procfs reports 0
and sysfs 4096 for files of other lengths, so such files are read (cheap for small files). Unfollowed symlinks
report the byte length of their exact target, and a target that is not valid UTF-8 keeps its exact bytes next
to the displayed text.

## 4. Snapshot format

- **VFS-030** (P1): `.hksnap` = zstd frame containing a header (magic, version, source
  spec, creation time, options used) followed by a stream of entries (relative path,
  kind, size, mtime, ctime, attrs/mode, owner, group, link target, crc32?, version
  info?, archive children?). Readable without decompressing the whole file for large
  trees (indexed blocks). Scripts: `snapshot` command.

## 5. Profile settings

### FTP profile (also FTPS/SFTP)

- **Global** (`<default>` profile): anonymous login email; SSH private key file and
  "Create Keys"; SSL client certificate and "Create Certificate"; ASCII transfer
  extensions list.
- **Login**: Protocol (FTP, FTP over SSL explicit/implicit, SFTP over SSH), Port,
  Host, Username, Password, Save password, Description.
- **Server**: filenames are case sensitive; listing encoding; server time zone;
  custom login commands (not for SFTP).
- **Connection**: simultaneous connections (1–10), read timeout, passive transfers
  (PASV), port range for active transfers, send HOST before login, clear control
  channel (CCC), clear data channel, TLS version range.
- **Proxy**: use proxy; proxy type (SOCKS4/5, HTTP CONNECT, FTP SITE/USER forms);
  host, port, username, password.
- **Listing**: link resolution (Fast: extension ⇒ file; Simple: try CWD), use MLSD,
  show hidden (`-a`), force long format (`-l`), complete timestamps (`-T`), resolve
  links (`-L`), recursive (`-R`), hide VMS versions, fetch incomplete timestamps.
- **Transfer**: transfer type Auto/ASCII/Binary; copy timestamps on upload; copy Unix
  permissions on upload; compress transfers (MODE Z); limit download/upload Kbps;
  aggressive SFTP uploads.

### Other profiles

- **S3**: authentication type (saved keys / credentials file + profile name /
  environment variables), optional bucket, endpoint URL (for non-AWS), region, path
  style, description.
- **WebDAV**: URL, username, password, recursive listings, description.
- **Subversion**: repository URL, revision, description.
- **Cloud drives**: OAuth "Connect" button (browser flow + code paste for providers
  that require it), description.

- **VFS-040** (P1): Profiles dialog: list (type icon, name, description), New (type
  menu), Clone, Edit, Delete, Test Connection.
- **VFS-041** (P1): Quick Connect (unsaved credentials) from the Browse dialog.
