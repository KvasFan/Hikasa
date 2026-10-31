//! Archive fixtures, generated in the tests (§6.5): a tree as data → the bytes of each format; hostile members as
//! raw bytes (tar headers built by hand with their checksum, because `tar::Builder` refuses them; zip records patched
//! after writing). Nothing is downloaded and nothing is committed as a binary.
#![allow(dead_code)]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::{CaseSensitivity, FileTime, LocalPath, Spec};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, LocalVfs, MemMeta,
    MemoryVfs, Vfs,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::host;

/// The masks of `archives.toml`'s defaults.
pub fn default_masks() -> ArchiveMasks {
    masks_with(|_| None)
}

/// The default masks with some replaced.
pub fn masks_with(change: impl Fn(ArchiveFormat) -> Option<&'static str>) -> ArchiveMasks {
    let default = |format| match format {
        ArchiveFormat::Zip => "*.zip;*.jar;*.war;*.ear;*.apk",
        ArchiveFormat::Tar => "*.tar",
        ArchiveFormat::TarGz => "*.tar.gz;*.tgz",
        ArchiveFormat::TarBz2 => "*.tar.bz2;*.tbz;*.tbz2",
        ArchiveFormat::TarXz => "*.tar.xz;*.txz",
        ArchiveFormat::TarZst => "*.tar.zst;*.tzst",
        ArchiveFormat::Gz => "*.gz",
        ArchiveFormat::Bz2 => "*.bz2",
        ArchiveFormat::Xz => "*.xz",
        ArchiveFormat::Zst => "*.zst",
    };
    ArchiveMasks::new(ArchiveFormat::ORDER.map(|f| (f, change(f).unwrap_or(default(f)).to_owned())))
        .unwrap()
}

/// A config with the default masks, the UTC clock and the given cache folder.
pub fn config(cache_root: &Path, limits: ArchiveLimits) -> ArchiveConfig {
    config_in(cache_root, limits, jiff::tz::TimeZone::UTC)
}

/// A config whose clock lies in `zone`.
pub fn config_in(
    cache_root: &Path,
    limits: ArchiveLimits,
    zone: jiff::tz::TimeZone,
) -> ArchiveConfig {
    ArchiveConfig {
        masks: default_masks(),
        limits,
        clock: Arc::new(move || jiff::Timestamp::UNIX_EPOCH.to_zoned(zone.clone())),
        cache_root: cache_root.to_path_buf(),
    }
}

/// `ArchiveVfs` over a `MemoryVfs`.
pub struct OnMemory {
    pub mem: Arc<MemoryVfs>,
    pub vfs: Arc<ArchiveVfs>,
    pub dir: tempfile::TempDir,
}

/// A fresh back-end over an empty memory tree.
pub fn on_memory(limits: ArchiveLimits) -> OnMemory {
    let mem = Arc::new(MemoryVfs::new(CaseSensitivity::Sensitive));
    let dir = tempfile::tempdir().unwrap();
    let vfs = Arc::new(ArchiveVfs::new(
        mem.clone() as Arc<dyn Vfs>,
        config(dir.path(), limits),
    ));
    OnMemory { mem, vfs, dir }
}

impl OnMemory {
    /// Puts a file in the tree (mtime 12:00 of a fixed day) and returns its spec.
    pub fn put(&self, name: &str, bytes: Vec<u8>) -> Spec {
        let path = host(&format!("/{name}"));
        let meta = MemMeta {
            mtime: FileTime::from_unix(1_792_415_520, 0),
            ..MemMeta::default()
        };
        self.mem.insert_file(&path, bytes, meta).unwrap();
        super::mspec(&format!("/{name}"))
    }

    /// The root of an archive put under `name`.
    pub fn root(&self, name: &str, bytes: Vec<u8>) -> Spec {
        Spec::archive_member(self.put(name, bytes), "")
    }
}

/// `ArchiveVfs` over `LocalVfs` in a tempdir.
pub struct OnDisk {
    pub vfs: Arc<ArchiveVfs>,
    pub dir: PathBuf,
    pub cache: PathBuf,
    pub _keep: tempfile::TempDir,
}

/// A fresh back-end over a tempdir (canonicalized: macOS' temp folder is a link).
pub fn on_disk(limits: ArchiveLimits) -> OnDisk {
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let dir = root.join("work");
    let cache = root.join("cache");
    std::fs::create_dir_all(&dir).unwrap();
    let vfs = Arc::new(ArchiveVfs::new(
        Arc::new(LocalVfs::default()) as Arc<dyn Vfs>,
        config(&cache, limits),
    ));
    OnDisk {
        vfs,
        dir,
        cache,
        _keep: keep,
    }
}

impl OnDisk {
    /// Writes a file and returns its spec.
    pub fn put(&self, name: &str, bytes: &[u8]) -> Spec {
        let path = self.dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        local(&path)
    }
}

/// `Spec::Local` of `path`.
pub fn local(path: &Path) -> Spec {
    Spec::Local(LocalPath::new(path.to_path_buf()).unwrap())
}

/// A tree as data: `(path, content)`; a path ending in `/` is a folder.
pub type TreeData = Vec<(String, Vec<u8>)>;

/// The generated tree of the format tests.
pub fn tree() -> TreeData {
    vec![
        ("readme.txt".into(), b"hello\n".to_vec()),
        ("empty/".into(), Vec::new()),
        ("dir/a.txt".into(), b"a".repeat(1000)),
        ("dir/sub/b.bin".into(), (0..=255u8).collect()),
        ("dir/sub/zero".into(), Vec::new()),
        ("spaces in name.txt".into(), b"x".to_vec()),
        ("ünï.txt".into(), b"utf-8 name".to_vec()),
    ]
}

/// A zip of `tree` with one method.
pub fn zip_with(tree: &TreeData, method: CompressionMethod) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(0o644);
    for (path, content) in tree {
        if let Some(dir) = path.strip_suffix('/') {
            zip.add_directory(dir, opts).unwrap();
        } else {
            zip.start_file(path, opts).unwrap();
            zip.write_all(content).unwrap();
        }
    }
    zip.finish().unwrap().into_inner()
}

/// A deflated zip of `tree`.
pub fn zip_bytes(tree: &TreeData) -> Vec<u8> {
    zip_with(tree, CompressionMethod::Deflated)
}

/// A tar of `tree`, mtime `1_792_415_520`, mode 0644.
pub fn tar_bytes(tree: &TreeData) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, content) in tree {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_mtime(1_792_415_520);
        header.set_uid(1000);
        header.set_gid(100);
        if path.ends_with('/') {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            builder
                .append_data(&mut header, path, std::io::empty())
                .unwrap();
        } else {
            header.set_size(content.len() as u64);
            builder
                .append_data(&mut header, path, content.as_slice())
                .unwrap();
        }
    }
    builder.into_inner().unwrap()
}

/// `data` compressed for a stream format.
pub fn compress(format: ArchiveFormat, data: &[u8]) -> Vec<u8> {
    match format {
        ArchiveFormat::TarGz | ArchiveFormat::Gz => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        }
        ArchiveFormat::TarBz2 | ArchiveFormat::Bz2 => {
            let mut encoder =
                bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        }
        ArchiveFormat::TarXz | ArchiveFormat::Xz => {
            let mut encoder =
                lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1))
                    .unwrap();
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap()
        }
        ArchiveFormat::TarZst | ArchiveFormat::Zst => {
            zstd::stream::encode_all(Cursor::new(data), 3).unwrap()
        }
        ArchiveFormat::Zip | ArchiveFormat::Tar => data.to_vec(),
    }
}

/// A file name of each format.
pub fn name_of(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "a.zip",
        ArchiveFormat::Tar => "a.tar",
        ArchiveFormat::TarGz => "a.tar.gz",
        ArchiveFormat::TarBz2 => "a.tar.bz2",
        ArchiveFormat::TarXz => "a.tar.xz",
        ArchiveFormat::TarZst => "a.tar.zst",
        ArchiveFormat::Gz => "a.txt.gz",
        ArchiveFormat::Bz2 => "a.txt.bz2",
        ArchiveFormat::Xz => "a.txt.xz",
        ArchiveFormat::Zst => "a.txt.zst",
    }
}

/// The member of the four single streams in [`name_of`].
pub const STREAM_MEMBER: &str = "a.txt";
/// The content of the four single streams of [`bytes_of`].
pub const STREAM_CONTENT: &[u8] = b"one stream, one member\n";

/// `tree` as bytes of `format` (for a single stream, [`STREAM_CONTENT`]).
pub fn bytes_of(format: ArchiveFormat, tree: &TreeData) -> Vec<u8> {
    match format {
        ArchiveFormat::Zip => zip_bytes(tree),
        ArchiveFormat::Tar => tar_bytes(tree),
        ArchiveFormat::TarGz
        | ArchiveFormat::TarBz2
        | ArchiveFormat::TarXz
        | ArchiveFormat::TarZst => compress(format, &tar_bytes(tree)),
        ArchiveFormat::Gz | ArchiveFormat::Bz2 | ArchiveFormat::Xz | ArchiveFormat::Zst => {
            compress(format, STREAM_CONTENT)
        }
    }
}

/// One raw tar record: a header block written by hand with its checksum.
#[derive(Clone)]
pub struct RawTar {
    pub name: Vec<u8>,
    pub type_flag: u8,
    pub link: Vec<u8>,
    pub data: Vec<u8>,
    /// The size field; `None` = the length of `data`.
    pub size: Option<u64>,
    pub mtime: u64,
    /// The mode field.
    pub mode: u32,
    /// `ustar\0` + `00` when true, nothing (an old V7 header) otherwise.
    pub ustar: bool,
}

impl RawTar {
    pub fn file(name: &[u8], data: &[u8]) -> RawTar {
        RawTar {
            name: name.to_vec(),
            type_flag: b'0',
            link: Vec::new(),
            data: data.to_vec(),
            size: None,
            mtime: 1_792_415_520,
            mode: 0o644,
            ustar: true,
        }
    }

    pub fn typed(name: &[u8], type_flag: u8) -> RawTar {
        RawTar {
            type_flag,
            ..RawTar::file(name, b"")
        }
    }

    pub fn link(name: &[u8], type_flag: u8, target: &[u8]) -> RawTar {
        RawTar {
            link: target.to_vec(),
            ..RawTar::typed(name, type_flag)
        }
    }

    /// The header block.
    pub fn header(&self) -> [u8; 512] {
        let mut block = [0u8; 512];
        let name = &self.name[..self.name.len().min(100)];
        block[..name.len()].copy_from_slice(name);
        block[100..108].copy_from_slice(format!("{:07o}\0", self.mode).as_bytes());
        block[108..116].copy_from_slice(b"0000000\0");
        block[116..124].copy_from_slice(b"0000000\0");
        let size = self.size.unwrap_or(self.data.len() as u64);
        block[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        block[136..148].copy_from_slice(format!("{:011o}\0", self.mtime).as_bytes());
        block[156] = self.type_flag;
        let link = &self.link[..self.link.len().min(100)];
        block[157..157 + link.len()].copy_from_slice(link);
        if self.ustar {
            block[257..263].copy_from_slice(b"ustar\0");
            block[263..265].copy_from_slice(b"00");
        }
        seal(&mut block);
        block
    }

    /// Header, data, padding.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = self.header().to_vec();
        out.extend_from_slice(&self.data);
        pad(&mut out);
        out
    }
}

/// Writes the checksum of a header block.
pub fn seal(block: &mut [u8; 512]) {
    block[148..156].fill(b' ');
    let sum: u32 = block.iter().map(|&b| u32::from(b)).sum();
    block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
}

/// Pads to a whole number of blocks.
pub fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(512) {
        out.push(0);
    }
}

/// A PAX `x` header whose records are `(key, value)`.
pub fn pax(records: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (key, value) in records {
        let base = key.len() + value.len() + 3;
        let mut len = base + base.to_string().len();
        if len.to_string().len() + base != len {
            len = base + len.to_string().len();
        }
        body.extend_from_slice(format!("{len} {key}=").as_bytes());
        body.extend_from_slice(value);
        body.push(b'\n');
    }
    RawTar {
        data: body,
        ..RawTar::typed(b"PaxHeaders/x", b'x')
    }
    .bytes()
}

/// A raw tar: records and the two end blocks.
pub fn raw_tar(records: &[RawTar]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        out.extend(record.bytes());
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// A zip of stored files, whose names and metadata are then rewritten by `patch`, with names of any bytes: the writer
/// is not asked to accept them. Each record is written by hand.
pub struct RawZip {
    entries: Vec<RawEntry>,
}

/// One entry of a [`RawZip`].
#[derive(Clone)]
pub struct RawEntry {
    pub name: Vec<u8>,
    pub data: Vec<u8>,
    pub flags: u16,
    pub method: u16,
    pub version_made_by: u16,
    pub external: u32,
    pub dos_time: u16,
    pub dos_date: u16,
    pub crc32: Option<u32>,
    /// The uncompressed size field; `None` = the length of `data`.
    pub size: Option<u32>,
    pub extra: Vec<u8>,
    /// The comment of the central record.
    pub comment: Vec<u8>,
    /// The local header offset of the central record; `None` = where this entry was written.
    pub at: Option<u32>,
}

impl RawEntry {
    pub fn new(name: &[u8], data: &[u8]) -> RawEntry {
        RawEntry {
            name: name.to_vec(),
            data: data.to_vec(),
            flags: 0,
            method: 0,
            version_made_by: 20,
            external: 0,
            dos_time: 0x6000,
            dos_date: 0x5cef,
            crc32: None,
            size: None,
            extra: Vec::new(),
            comment: Vec::new(),
            at: None,
        }
    }

    /// With a Unix mode.
    pub fn unix(self, mode: u32) -> RawEntry {
        RawEntry {
            version_made_by: 0x0300 | 20,
            external: mode << 16,
            ..self
        }
    }
}

/// Bytes of a zip of `entries`, in that central-directory order.
pub fn raw_zip(entries: &[RawEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for entry in entries {
        let offset = out.len() as u32;
        let crc = entry.crc32.unwrap_or_else(|| crc32fast::hash(&entry.data));
        let compressed = entry.data.len() as u32;
        let size = entry.size.unwrap_or(compressed);
        let common = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&entry.flags.to_le_bytes());
            out.extend_from_slice(&entry.method.to_le_bytes());
            out.extend_from_slice(&entry.dos_time.to_le_bytes());
            out.extend_from_slice(&entry.dos_date.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&compressed.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(entry.extra.len() as u16).to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04");
        common(&mut out);
        out.extend_from_slice(&entry.name);
        out.extend_from_slice(&entry.extra);
        out.extend_from_slice(&entry.data);

        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&entry.version_made_by.to_le_bytes());
        common(&mut central);
        central.extend_from_slice(&(entry.comment.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&entry.external.to_le_bytes());
        central.extend_from_slice(&entry.at.unwrap_or(offset).to_le_bytes());
        central.extend_from_slice(&entry.name);
        central.extend_from_slice(&entry.extra);
        central.extend_from_slice(&entry.comment);
    }
    let start = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0u8; 4]);
    let count = entries.len() as u16;
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0u8; 2]);
    out
}

// ---- a base that cancels ---------------------------------------------------------------------------------------

/// A `MemoryVfs` whose writers cancel `token` after `after` bytes: cancellation at a byte of a rebuild. Used as the
/// base of an `ArchiveVfs`, so only what the rebuild writes counts.
pub struct CancellingBase {
    pub mem: Arc<MemoryVfs>,
    pub token: hikasa_core::CancelToken,
    pub after: u64,
}

struct CancellingWriter {
    inner: Box<dyn hikasa_vfs::AtomicWrite>,
    token: hikasa_core::CancelToken,
    left: u64,
}

impl Write for CancellingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        match self.left.checked_sub(n as u64) {
            Some(left) => self.left = left,
            None => self.token.cancel(),
        }
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl std::io::Seek for CancellingWriter {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

impl hikasa_vfs::AtomicWrite for CancellingWriter {
    fn commit(self: Box<Self>) -> Result<(), hikasa_vfs::VfsError> {
        self.inner.commit()
    }
}

impl Vfs for CancellingBase {
    fn capabilities(&self, at: &Spec) -> Result<hikasa_vfs::Capabilities, hikasa_vfs::VfsError> {
        self.mem.capabilities(at)
    }
    fn stat(
        &self,
        spec: &Spec,
        follow: hikasa_vfs::FollowLinks,
    ) -> Result<hikasa_vfs::Entry, hikasa_vfs::VfsError> {
        self.mem.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &hikasa_vfs::ListOptions<'_>,
        cancel: &hikasa_core::CancelToken,
    ) -> Result<hikasa_vfs::Listing, hikasa_vfs::VfsError> {
        self.mem.list(dir, opts, cancel)
    }
    fn open_read(
        &self,
        spec: &Spec,
    ) -> Result<Box<dyn hikasa_vfs::ReadSeek>, hikasa_vfs::VfsError> {
        self.mem.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &hikasa_vfs::WriteOptions,
    ) -> Result<Box<dyn hikasa_vfs::AtomicWrite>, hikasa_vfs::VfsError> {
        Ok(Box::new(CancellingWriter {
            inner: self.mem.open_write(spec, opts)?,
            token: self.token.clone(),
            left: self.after,
        }))
    }
    fn child(&self, dir: &Spec, name: &hikasa_vfs::FileName) -> Result<Spec, hikasa_vfs::VfsError> {
        self.mem.child(dir, name)
    }
}

// ---- reading a rebuilt archive back -----------------------------------------------------------------------------

/// What a member is, as every reader must agree: `None` content for a folder, the target for a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    Dir,
    File(Vec<u8>),
    Link(String),
}

/// Everything an `ArchiveVfs` shows below `root`, recursively. A member that cannot be read shows its error's text.
pub fn walk(vfs: &dyn Vfs, root: &Spec) -> std::collections::BTreeMap<String, Seen> {
    fn visit(
        vfs: &dyn Vfs,
        dir: &Spec,
        prefix: &str,
        out: &mut std::collections::BTreeMap<String, Seen>,
    ) {
        let listing = vfs
            .list(
                dir,
                &hikasa_vfs::ListOptions::default(),
                &hikasa_core::CancelToken::never(),
            )
            .unwrap();
        for entry in listing.entries {
            let path = format!("{prefix}{}", entry.name.as_str());
            let child = vfs.child(dir, &entry.name).unwrap();
            match entry.kind {
                hikasa_vfs::EntryKind::Dir => {
                    out.insert(path.clone(), Seen::Dir);
                    visit(vfs, &child, &format!("{path}/"), out);
                }
                hikasa_vfs::EntryKind::Symlink => {
                    out.insert(path, Seen::Link(entry.link_target.unwrap_or_default()));
                }
                _ => {
                    let bytes = vfs.open_read(&child).map(super::read_to_vec);
                    let seen = match bytes {
                        Ok(bytes) => Seen::File(bytes),
                        Err(error) => Seen::Link(format!("<{error}>")),
                    };
                    out.insert(path, seen);
                }
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    visit(vfs, root, "", &mut out);
    out
}

/// The decoded stream of a compressed tar, or `bytes` for a plain one.
pub fn decompress(format: ArchiveFormat, bytes: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    match format {
        ArchiveFormat::TarGz => {
            flate2::read::MultiGzDecoder::new(bytes)
                .read_to_end(&mut out)
                .unwrap();
        }
        ArchiveFormat::TarBz2 => {
            bzip2::read::MultiBzDecoder::new(bytes)
                .read_to_end(&mut out)
                .unwrap();
        }
        ArchiveFormat::TarXz => {
            lzma_rust2::XzReader::new(bytes, true)
                .read_to_end(&mut out)
                .unwrap();
        }
        ArchiveFormat::TarZst => {
            zstd::stream::read::Decoder::new(bytes)
                .unwrap()
                .read_to_end(&mut out)
                .unwrap();
        }
        _ => out = bytes.to_vec(),
    }
    out
}

/// `bytes` as the plain `zip` and `tar` crates read them, with none of the code under test: the portability check of
/// a rebuild. Folders only through their own records are added, as a tree shows them.
pub fn read_plain(format: ArchiveFormat, bytes: &[u8]) -> std::collections::BTreeMap<String, Seen> {
    use std::collections::BTreeMap;
    use std::io::Read;
    let mut out = BTreeMap::new();
    if format == ArchiveFormat::Zip {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        for at in 0..archive.len() {
            let mut file = archive.by_index(at).unwrap();
            let name = file.name().unwrap().trim_end_matches('/').to_owned();
            if file.is_dir() {
                out.insert(name, Seen::Dir);
            } else if file.is_symlink() {
                let mut target = String::new();
                file.read_to_string(&mut target).unwrap();
                out.insert(name, Seen::Link(target));
            } else {
                let mut content = Vec::new();
                file.read_to_end(&mut content).unwrap();
                out.insert(name, Seen::File(content));
            }
        }
    } else {
        let stream = decompress(format, bytes);
        let mut archive = tar::Archive::new(stream.as_slice());
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap();
            let name = path.to_string_lossy().trim_end_matches('/').to_owned();
            let kind = entry.header().entry_type();
            if kind.is_dir() {
                out.insert(name, Seen::Dir);
            } else if kind.is_symlink() {
                let target = entry.link_name().unwrap().unwrap();
                let target = target.to_string_lossy().into_owned();
                out.insert(name, Seen::Link(target));
            } else {
                let mut content = Vec::new();
                entry.read_to_end(&mut content).unwrap();
                out.insert(name, Seen::File(content));
            }
        }
    }
    for path in out.keys().cloned().collect::<Vec<_>>() {
        let mut dir = path.as_str();
        while let Some((parent, _)) = dir.rsplit_once('/') {
            out.entry(parent.to_owned()).or_insert(Seen::Dir);
            dir = parent;
        }
    }
    out
}
