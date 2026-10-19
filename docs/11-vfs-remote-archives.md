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

## 3. Content fingerprints and CRC

- **VFS-020** (P0): `crc32` is the "CRC comparison" primitive (compatibility with
  archives and snapshots). BLAKE3 is used internally for caches (per-file content
  identity across sessions, P1) and for "binary same" shortcuts when both sides already
  have a known hash.

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
