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

/// Bits of a deflate stream, least significant first.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    n: u32,
}

impl Bits<'_> {
    fn refill(&mut self) {
        while self.n <= 56 {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.buf |= u64::from(byte) << self.n;
            self.pos += 1;
            self.n += 8;
        }
    }

    fn take(&mut self, k: u32) -> Option<u32> {
        if k == 0 {
            return Some(0);
        }
        if self.n < k {
            self.refill();
        }
        // Reading past the end yields zeros; a stream that needs them is damaged.
        if self.pos > self.data.len() + 8 {
            return None;
        }
        let v = (self.buf & ((1u64 << k) - 1)) as u32;
        self.buf >>= k;
        self.n -= k;
        Some(v)
    }

    fn align(&mut self) {
        let drop = self.n % 8;
        self.buf >>= drop;
        self.n -= drop;
    }
}

/// Bits of the fast decoding table.
const FAST: u32 = 10;

/// A canonical Huffman code: a table for codes of up to `FAST` bits, the canonical counts for longer ones.
struct Huffman {
    /// `symbol << 4 | length`, 0 = not in the table.
    table: Vec<u32>,
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Option<Huffman> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for len in 1..16 {
            offs[len] = offs[len - 1] + counts[len - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offs[usize::from(l)])] = sym as u16;
                offs[usize::from(l)] += 1;
            }
        }
        let mut table = vec![0u32; 1 << FAST];
        let mut code: u32 = 0;
        let mut next = [0u32; 16];
        for len in 1..16 {
            code = (code + u32::from(counts[len - 1])) << 1;
            next[len] = code;
        }
        for (sym, &l) in lengths.iter().enumerate() {
            let l = u32::from(l);
            if l == 0 || l > FAST {
                continue;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            let rev = c.reverse_bits() >> (32 - l);
            let mut i = rev;
            while i < (1 << FAST) {
                table[i as usize] = ((sym as u32) << 4) | l;
                i += 1 << l;
            }
        }
        Some(Huffman {
            table,
            counts,
            symbols,
        })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Option<u16> {
        if bits.n < 15 {
            bits.refill();
        }
        let e = self.table[(bits.buf & ((1 << FAST) - 1)) as usize];
        if e != 0 {
            let l = e & 15;
            bits.buf >>= l;
            bits.n -= l;
            return Some((e >> 4) as u16);
        }
        // Longer codes, bit by bit.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self.symbols.get((index + code - first) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Output kept for back references.
const WINDOW: usize = 32 * 1024;
/// Output handed to the sink at a time.
const CHUNK: usize = 1 << 20;

/// Inflates a raw deflate stream (RFC 1951), handing the output to `sink` in chunks; the sink returns `false` to
/// stop early. At most `limit` bytes come out. `None` for a damaged stream.
pub(crate) fn inflate(data: &[u8], limit: u64, sink: &mut dyn FnMut(&[u8]) -> bool) -> Option<()> {
    let mut bits = Bits {
        data,
        pos: 0,
        buf: 0,
        n: 0,
    };
    let mut out: Vec<u8> = Vec::with_capacity(CHUNK + WINDOW + 300);
    let mut flushed = 0usize; // bytes of `out` already handed to the sink
    let mut total: u64 = 0;
    let fixed_lit = {
        let mut l = [0u8; 288];
        l[..144].fill(8);
        l[144..256].fill(9);
        l[256..280].fill(7);
        l[280..].fill(8);
        Huffman::new(&l)?
    };
    let fixed_dist = Huffman::new(&[5u8; 30])?;
    loop {
        let last = bits.take(1)?;
        let kind = bits.take(2)?;
        match kind {
            0 => {
                bits.align();
                let len = bits.take(16)?;
                let nlen = bits.take(16)?;
                if len != !nlen & 0xFFFF {
                    return None;
                }
                for _ in 0..len {
                    out.push(bits.take(8)? as u8);
                }
            }
            1 | 2 => {
                let dynamic;
                let (lit, dist) = if kind == 1 {
                    (&fixed_lit, &fixed_dist)
                } else {
                    dynamic = dynamic_tables(&mut bits)?;
                    (&dynamic.0, &dynamic.1)
                };
                loop {
                    let sym = lit.decode(&mut bits)?;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let i = usize::from(sym - 257);
                        let len = usize::from(*LENGTH_BASE.get(i)?)
                            + bits.take(u32::from(LENGTH_EXTRA[i]))? as usize;
                        let d = usize::from(dist.decode(&mut bits)?);
                        let distance = usize::from(*DIST_BASE.get(d)?)
                            + bits.take(u32::from(DIST_EXTRA[d]))? as usize;
                        if distance > out.len() {
                            return None;
                        }
                        let from = out.len() - distance;
                        for k in 0..len {
                            let b = out[from + k];
                            out.push(b);
                        }
                    }
                    if out.len() >= CHUNK + WINDOW {
                        total += (out.len() - flushed) as u64;
                        if total > limit || !sink(&out[flushed..]) {
                            return Some(());
                        }
                        out.drain(..out.len() - WINDOW);
                        flushed = WINDOW;
                    }
                }
            }
            _ => return None,
        }
        if last == 1 {
            break;
        }
    }
    total += (out.len() - flushed) as u64;
    if total <= limit {
        sink(&out[flushed..]);
    }
    Some(())
}

/// The literal/length and distance codes of a dynamic block.
fn dynamic_tables(bits: &mut Bits<'_>) -> Option<(Huffman, Huffman)> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let hlit = bits.take(5)? as usize + 257;
    let hdist = bits.take(5)? as usize + 1;
    let hclen = bits.take(4)? as usize + 4;
    let mut cl = [0u8; 19];
    for &i in ORDER.iter().take(hclen) {
        cl[i] = bits.take(3)? as u8;
    }
    let clh = Huffman::new(&cl)?;
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < hlit + hdist {
        let sym = clh.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => (*lengths.get(i.checked_sub(1)?)?, 3 + bits.take(2)? as usize),
            17 => (0, 3 + bits.take(3)? as usize),
            18 => (0, 11 + bits.take(7)? as usize),
            _ => return None,
        };
        if i + repeat > lengths.len() {
            return None;
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    Some((
        Huffman::new(&lengths[..hlit])?,
        Huffman::new(&lengths[hlit..])?,
    ))
}

/// The whole output of a member (stored or deflated), at most `limit` bytes.
pub(crate) fn read_member(bytes: &[u8], m: &Member, limit: u64) -> Option<Vec<u8>> {
    if m.size > limit {
        return None;
    }
    if let Some(data) = stored(bytes, m) {
        return Some(data.to_vec());
    }
    let mut out = Vec::new();
    stream_member(bytes, m, limit, &mut |chunk| {
        out.extend_from_slice(chunk);
        true
    })?;
    Some(out)
}

/// Streams a member's output to `sink` (see [`inflate`]).
pub(crate) fn stream_member(
    bytes: &[u8],
    m: &Member,
    limit: u64,
    sink: &mut dyn FnMut(&[u8]) -> bool,
) -> Option<()> {
    if let Some(data) = stored(bytes, m) {
        sink(data);
        return Some(());
    }
    if m.method != 8 {
        return None;
    }
    let at = usize::try_from(m.offset).ok()?;
    if !bytes.get(at..)?.starts_with(&LOCAL) {
        return None;
    }
    let name_len = usize::from(u16_at(bytes, at + 26)?);
    let extra_len = usize::from(u16_at(bytes, at + 28)?);
    let start = at + 30 + name_len + extra_len;
    let end = start.checked_add(usize::try_from(m.compressed).ok()?)?;
    inflate(bytes.get(start..end)?, limit.min(m.size), sink)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn deflate(data: &[u8], level: i64) -> Vec<u8> {
        use std::io::Write as _;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut cursor);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(level));
            z.start_file("m", opts).unwrap();
            z.write_all(data).unwrap();
            z.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn round_trip(data: &[u8], level: i64) -> Vec<u8> {
        let pkg = deflate(data, level);
        let members = directory(&pkg).unwrap();
        read_member(&pkg, &members[0], u64::MAX).unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(256), ..ProptestConfig::default() })]

        /// P-INFLATE: what the zip crate deflates, the in-house inflater restores (every level, text and noise).
        #[test]
        fn p_inflate_round_trip(
            words in prop::collection::vec(prop::sample::select(vec!["<row r=\"", "\">", "<c><v>", "1", "22", "hidden=\"1\"", " ", "x"]), 0..400),
            noise in prop::collection::vec(any::<u8>(), 0..2000),
            level in 1i64..10,
        ) {
            let text: Vec<u8> = words.concat().into_bytes();
            prop_assert_eq!(round_trip(&text, level), text);
            prop_assert_eq!(round_trip(&noise, level), noise);
        }
    }

    #[test]
    fn large_outputs_stream_in_chunks() {
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
        assert_eq!(round_trip(&data, 6), data);
        let rep = b"<row r=\"1\" hidden=\"1\"><c r=\"A1\"><v>1</v></c></row>".repeat(100_000);
        assert_eq!(round_trip(&rep, 9), rep);
        // A limit stops the output.
        let pkg = deflate(&rep, 6);
        let m = &directory(&pkg).unwrap()[0];
        let mut seen = 0usize;
        stream_member(&pkg, m, 2 << 20, &mut |c| {
            seen += c.len();
            true
        });
        assert!(seen <= 2 << 20, "{seen}");
        // A damaged stream is refused, never a panic.
        let mut bad = pkg.clone();
        for b in bad.iter_mut().skip(40).take(200) {
            *b ^= 0x5A;
        }
        let _ = read_member(&bad, m, u64::MAX);
    }
}
