//! A12.10 (D-335): value interpretation — known bytes to every field in both byte orders, a short tail, UTF-8
//! sequences, and `format`'s golden text.

mod common;

use common::{doc, golden};
use hikasa_hex::Radix;
use hikasa_hex::values::{ValueFormat, at, format, format_with, from_bytes};

#[test]
fn known_bytes() {
    let v = from_bytes(&[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(v.little.i8, Some(-128));
    assert_eq!(v.little.u8, Some(0x80));
    assert_eq!(v.little.i16, Some(0x0080));
    assert_eq!(v.big.i16, Some(i16::MIN));
    assert_eq!(v.big.u16, Some(0x8000));
    assert_eq!(v.big.i64, Some(i64::MIN));
    let max = from_bytes(&[0xFF; 8]);
    assert_eq!(max.little.u64, Some(u64::MAX));
    assert_eq!(max.big.i32, Some(-1));
    assert!(max.little.f64.unwrap().is_nan());
    let one = from_bytes(&1.0f32.to_le_bytes());
    assert_eq!(one.little.f32, Some(1.0));
    assert_eq!(one.little.f64, None);
    assert_eq!(
        from_bytes(&f32::INFINITY.to_be_bytes()).big.f32,
        Some(f32::INFINITY)
    );
    assert_eq!(
        from_bytes(&f64::NEG_INFINITY.to_le_bytes()).little.f64,
        Some(f64::NEG_INFINITY)
    );
    let sub = from_bytes(&1u32.to_le_bytes()).little.f32.unwrap();
    assert!(sub > 0.0 && !sub.is_normal());
    // A tail of 3 bytes: nothing from 32 bits up.
    let tail = from_bytes(&[1, 2, 3]);
    assert_eq!(tail.little.u16, Some(0x0201));
    assert_eq!(tail.big.u16, Some(0x0102));
    assert_eq!(
        (tail.little.u32, tail.little.f32, tail.big.u64),
        (None, None, None)
    );
    // UTF-8 of 1 to 4 bytes, and invalid sequences.
    assert_eq!(from_bytes(b"A").little.utf8, Some('A'));
    assert_eq!(from_bytes("é!".as_bytes()).big.utf8, Some('é'));
    assert_eq!(from_bytes("€".as_bytes()).little.utf8, Some('€'));
    assert_eq!(from_bytes("𝄞".as_bytes()).little.utf8, Some('𝄞'));
    assert_eq!(from_bytes(&[0xC3]).little.utf8, None);
    assert_eq!(from_bytes(&[0xE2, 0x28, 0xA1]).little.utf8, None);
    assert_eq!(from_bytes(&[0xFF]).little.utf8, None);
    assert_eq!(from_bytes(&[]).little.u8, None);
}

#[test]
fn values_of_a_document_and_their_text() {
    let bytes: Vec<u8> = [&1.5f64.to_le_bytes()[..], "é".as_bytes(), &[0xFF]].concat();
    let d = doc("d", &bytes);
    let mut out = String::new();
    for offset in [0u64, 8, 9, 10] {
        out.push_str(&format!(
            "== offset {offset}\n{}",
            format(&at(&d, offset).unwrap())
        ));
    }
    golden("values", &out);
}

/// A13.8 (W13a, D-360): `format_with` — every column choice and radix; with both columns in decimal it equals
/// `format` (`hex dump --values`'s text).
#[test]
fn a13_8_format_with_columns_and_radix() {
    let samples: [&[u8]; 4] = [
        &[0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        &[0xFF; 8],
        &[1, 2, 3],
        "é!".as_bytes(),
    ];
    let mut out = String::new();
    for bytes in samples {
        let v = from_bytes(bytes);
        assert_eq!(format_with(&v, ValueFormat::default()), format(&v));
        for little in [false, true] {
            for big in [false, true] {
                for radix in [Radix::Decimal, Radix::Hex] {
                    let text = format_with(&v, ValueFormat { little, big, radix });
                    assert_eq!(text.contains("Little Endian Values"), little);
                    assert_eq!(text.contains("Big Endian Values"), big);
                    if !little && !big {
                        assert!(text.is_empty());
                    }
                    if little && big {
                        out.push_str(&format!("== {bytes:02X?} {radix:?}\n{text}"));
                    }
                }
            }
        }
    }
    golden("values-format-with", &out);
}
