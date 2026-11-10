//! The package directory of OOXML and ODF workbooks (review F11): the end-of-central-directory record and the
//! central directory headers, read from the bytes in place (zip64 included). Detection reads member names and the
//! ODF `mimetype` member instead of searching the bytes, so a truncated workbook is reported as damaged, a zip that
//! merely holds a workbook is no workbook, and an OpenDocument text is no spreadsheet. The declared sizes bound
//! what the reader may inflate (the `xl/sharedStrings.xml` budget check of F3).

/// One member of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Member {
    /// Name as stored (`/`-separated).
    pub name: String,
    /// Compression method (0 stored, 8 deflated).
    pub method: u16,
    /// Compressed size.
    pub compressed: u64,
    /// Declared uncompressed size.
    pub size: u64,
    /// Offset of the local header.
    pub offset: u64,
}

const EOCD: [u8; 4] = *b"PK\x05\x06";
const EOCD64_LOCATOR: [u8; 4] = *b"PK\x06\x07";
const EOCD64: [u8; 4] = *b"PK\x06\x06";
const CENTRAL: [u8; 4] = *b"PK\x01\x02";
const LOCAL: [u8; 4] = *b"PK\x03\x04";

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// True when the bytes start like a zip package.
pub(crate) fn looks_like_package(bytes: &[u8]) -> bool {
    bytes.starts_with(&LOCAL) || bytes.starts_with(&EOCD)
}

/// The members of a package, or why its directory cannot be read (a truncated or damaged file).
pub(crate) fn directory(bytes: &[u8]) -> Result<Vec<Member>, &'static str> {
    // The record is 22 bytes plus a comment of at most 65,535.
    let floor = bytes.len().saturating_sub(22 + 65_535);
    let mut at = None;
    let mut i = bytes
        .len()
        .checked_sub(22)
        .ok_or("too short for a zip directory")?;
    loop {
        if bytes[i..].starts_with(&EOCD) {
            at = Some(i);
            break;
        }
        if i == floor {
            break;
        }
        i -= 1;
    }
    let eocd = at.ok_or("no zip directory at the end (truncated?)")?;
    let mut entries = u64::from(u16_at(bytes, eocd + 10).ok_or("short directory record")?);
    let mut size = u64::from(u32_at(bytes, eocd + 12).ok_or("short directory record")?);
    let mut start = u64::from(u32_at(bytes, eocd + 16).ok_or("short directory record")?);
    if (entries == 0xFFFF || size == 0xFFFF_FFFF || start == 0xFFFF_FFFF)
        && eocd >= 20
        && bytes[eocd - 20..].starts_with(&EOCD64_LOCATOR)
    {
        let rec = u64_at(bytes, eocd - 20 + 8).ok_or("short zip64 locator")? as usize;
        if !bytes.get(rec..).is_some_and(|b| b.starts_with(&EOCD64)) {
            return Err("damaged zip64 directory record");
        }
        entries = u64_at(bytes, rec + 32).ok_or("short zip64 record")?;
        size = u64_at(bytes, rec + 40).ok_or("short zip64 record")?;
        start = u64_at(bytes, rec + 48).ok_or("short zip64 record")?;
    }
    let end = start.checked_add(size).ok_or("damaged zip directory")?;
    if end > bytes.len() as u64 {
        return Err("the zip directory lies past the end of the file (truncated?)");
    }
    // Every central header is at least 46 bytes.
    if entries > size / 46 + 1 {
        return Err("damaged zip directory");
    }
    let mut out = Vec::with_capacity(entries as usize);
    let mut p = start as usize;
    for _ in 0..entries {
        if !bytes.get(p..).is_some_and(|b| b.starts_with(&CENTRAL)) {
            return Err("damaged zip directory entry");
        }
        let short = "short zip directory entry";
        let method = u16_at(bytes, p + 10).ok_or(short)?;
        let mut compressed = u64::from(u32_at(bytes, p + 20).ok_or(short)?);
        let mut usize_ = u64::from(u32_at(bytes, p + 24).ok_or(short)?);
        let name_len = usize::from(u16_at(bytes, p + 28).ok_or(short)?);
        let extra_len = usize::from(u16_at(bytes, p + 30).ok_or(short)?);
        let comment_len = usize::from(u16_at(bytes, p + 32).ok_or(short)?);
        let mut offset = u64::from(u32_at(bytes, p + 42).ok_or(short)?);
        let name = bytes.get(p + 46..p + 46 + name_len).ok_or(short)?;
        let extra = bytes
            .get(p + 46 + name_len..p + 46 + name_len + extra_len)
            .ok_or(short)?;
        // Zip64 extra field: the values that are 0xFFFFFFFF, in order.
        let mut e = 0;
        while e + 4 <= extra.len() {
            let id = u16_at(extra, e).unwrap_or(0);
            let len = usize::from(u16_at(extra, e + 2).unwrap_or(0));
            if id == 1 {
                let mut q = e + 4;
                for field in [&mut usize_, &mut compressed, &mut offset] {
                    if *field == 0xFFFF_FFFF {
                        *field = u64_at(extra, q).ok_or(short)?;
                        q += 8;
                    }
                }
            }
            e += 4 + len;
        }
        out.push(Member {
            name: String::from_utf8_lossy(name).into_owned(),
            method,
            compressed,
            size: usize_,
            offset,
        });
        p += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// The data of a stored member (`None` for a compressed one or a damaged header).
pub(crate) fn stored<'a>(bytes: &'a [u8], m: &Member) -> Option<&'a [u8]> {
    if m.method != 0 {
        return None;
    }
    let at = usize::try_from(m.offset).ok()?;
    if !bytes.get(at..)?.starts_with(&LOCAL) {
        return None;
    }
    let name_len = usize::from(u16_at(bytes, at + 26)?);
    let extra_len = usize::from(u16_at(bytes, at + 28)?);
    let data = at + 30 + name_len + extra_len;
    bytes.get(data..data.checked_add(usize::try_from(m.size).ok()?)?)
}

/// The name of the first member from its local header (a package whose directory is lost still names its first
/// part).
pub(crate) fn first_name(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(&LOCAL) {
        return None;
    }
    let name_len = usize::from(u16_at(bytes, 26)?);
    bytes
        .get(30..30 + name_len)
        .map(|n| String::from_utf8_lossy(n).into_owned())
}

/// A member by name, ASCII case-insensitive (OOXML part names are case-insensitive).
pub(crate) fn find<'a>(members: &'a [Member], name: &str) -> Option<&'a Member> {
    members.iter().find(|m| m.name.eq_ignore_ascii_case(name))
}
