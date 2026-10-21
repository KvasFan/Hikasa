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
| Clipboard | ✓ | ✗ | now | ✗ | text or image | P0 |
| Stdin / pipes | ✓ | ✗ | now | ✗ | `-` on the command line | P0 |
| MTP devices | ✓ | ✓ | ✓ | ✗ | `mtp://Device/…` via libmtp bindings | P2 |

- **VFS-001** (P0): Every back-end implements the `Vfs` trait (`01-architecture.md`
  §3.2) and declares capabilities; operations not supported are greyed out and scripts
  log a warning.
- **VFS-002** (P0): Archive members are opened by extracting to a per-session temp
  cache on demand (streaming for tar/gz); writes rebuild the archive atomically
  (temp + rename) and update CRCs. "New Folder" with an archive extension creates an
  empty archive. Archive **type masks** are configurable in options (Archive Types page;
  empty mask disables a type).
- **VFS-003** (P1): Remote listings are cached per session; "Refresh" invalidates.
  Transfers show per-file progress and throughput; connection limits and bandwidth
  limits come from the profile.
- **VFS-004** (P1): Remote files opened in file views are downloaded to a temp cache
  and uploaded on save; the cache is cleared on exit.

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
