//! Unit tests of the in-house zip directory reader and inflater (kept in a `tests.rs` file: architecture check 2 allows `zip::` only there and under hikasa-vfs).

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
