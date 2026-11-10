//! Gate G9 (contract §9.4), by hand: `HIKASA_G9_DIR=<dir> cargo test --release -p hikasa-table --test gates --
//! --ignored g9_generate` once, then each `g9_*` measurement in its own process (peak RSS: `VmHWM` of `/proc/self/status`).

mod common;

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use common::Rng;
use hikasa_core::{CaseSensitivity, Ctl, LocalPath, Spec};
use hikasa_table::{
    LoadRequest, SheetRules, TableCompareOptions, compare_specs, parse_column_list,
};
use hikasa_vfs::LocalVfs;

fn dir() -> PathBuf {
    PathBuf::from(std::env::var_os("HIKASA_G9_DIR").expect("HIKASA_G9_DIR"))
}

/// One CSV row: an id, 4 numbers, 2 dates, 3 words.
fn row(i: u64, rng: &mut Rng, out: &mut Vec<u8>) {
    let _ = writeln!(
        out,
        "{i},{}.{:02},{},{}.5,{},2024-{:02}-{:02},2025-01-{:02} 10:{:02},w{},item {},code{}",
        rng.below(100_000),
        rng.below(100),
        rng.below(1_000_000),
        rng.below(1000),
        rng.below(50),
        1 + rng.below(12),
        1 + rng.below(28),
        1 + rng.below(28),
        rng.below(60),
        rng.below(5000),
        rng.below(100_000),
        rng.below(10)
    );
}

#[test]
#[ignore = "G9 input generation, by hand"]
fn g9_generate() {
    let d = dir();
    std::fs::create_dir_all(&d).unwrap();
    // 1,000,000 × 10 CSV pair: the right side has 1 % changed cells, 0.1 % deleted and 0.1 % inserted rows.
    let header = b"id,a,b,c,d,day,stamp,w,item,code\n";
    let (mut l, mut r) = (header.to_vec(), header.to_vec());
    let mut rng = Rng(0x5EED_0009);
    for i in 0..1_000_000u64 {
        let mut line = Vec::with_capacity(96);
        row(i, &mut rng, &mut line);
        l.extend_from_slice(&line);
        match rng.below(1000) {
            0 => {}
            1 => {
                r.extend_from_slice(&line);
                row(2_000_000 + i, &mut rng, &mut r);
            }
            2..=11 => {
                let text = String::from_utf8(line).unwrap().replacen(",w", ",W", 1);
                r.extend_from_slice(text.as_bytes());
            }
            _ => r.extend_from_slice(&line),
        }
    }
    std::fs::write(d.join("left.csv"), &l).unwrap();
    std::fs::write(d.join("right.csv"), &r).unwrap();
    // 200,000 × 20 xlsx pair.
    for (name, seed, change) in [("left.xlsx", 7u64, false), ("right.xlsx", 7, true)] {
        let mut rng = Rng(seed);
        let mut book = rust_xlsxwriter::Workbook::new();
        let s = book.add_worksheet();
        for c in 0..20u16 {
            s.write_string(0, c, format!("col{c}")).unwrap();
        }
        for r in 1..=200_000u32 {
            s.write_number(r, 0, f64::from(r)).unwrap();
            for c in 1..20u16 {
                let v = rng.below(1_000_000);
                if c % 4 == 0 {
                    s.write_string(r, c, format!("s{}", v % 5000)).unwrap();
                } else {
                    let v = if change && r % 97 == 0 && c == 3 {
                        v + 1
                    } else {
                        v
                    };
                    s.write_number(r, c, v as f64 / 100.0).unwrap();
                }
            }
        }
        book.save(d.join(name)).unwrap();
    }
}

fn measure(left: &str, right: &str, keys: Option<&str>) {
    let d = dir();
    let spec = |n: &str| Spec::Local(LocalPath::new(d.join(n)).unwrap());
    let opts = TableCompareOptions {
        default_sheet: SheetRules {
            keys: keys
                .map(|k| parse_column_list(k).unwrap())
                .unwrap_or_default(),
            ..SheetRules::default()
        },
        ..TableCompareOptions::default()
    };
    let vfs = LocalVfs::default();
    let _ = CaseSensitivity::Sensitive;
    let start = Instant::now();
    let out = compare_specs(
        &vfs,
        &spec(left),
        &spec(right),
        &LoadRequest::default(),
        &opts,
        &Ctl::detached(),
    )
    .unwrap();
    let secs = start.elapsed().as_secs_f64();
    let bytes = std::fs::metadata(d.join(left)).unwrap().len()
        + std::fs::metadata(d.join(right)).unwrap().len();
    let c = &out.comparison;
    let s = c.stats().totals;
    let heap: usize = [c.left(), c.right()]
        .iter()
        .flat_map(|w| w.sheets().iter().map(hikasa_table::Sheet::heap_bytes))
        .sum();
    println!(
        "G9 {left} vs {right} keys {keys:?}: {secs:.3} s, input {bytes} bytes, model heap {heap} bytes, rows {} same {} changed {}/{} left-only {} right-only {} cells {} verdict {:?}",
        s.rows,
        s.same_rows,
        s.changed_important,
        s.changed_unimportant,
        s.left_only_important + s.left_only_unimportant,
        s.right_only_important + s.right_only_unimportant,
        s.cells_compared,
        c.kind(out.bytes_identical, false)
    );
    // Peak RSS of this process (the test generates nothing: the inputs are files).
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status
            .lines()
            .filter(|l| l.starts_with("VmHWM") || l.starts_with("VmRSS"))
        {
            println!("G9 {line}");
        }
    }
}

#[test]
#[ignore = "G9, by hand"]
fn g9_csv_keyed() {
    measure("left.csv", "right.csv", Some("id"));
}

#[test]
#[ignore = "G9, by hand"]
fn g9_csv_unkeyed() {
    measure("left.csv", "right.csv", None);
}

#[test]
#[ignore = "G9, by hand"]
fn g9_xlsx_keyed() {
    measure("left.xlsx", "right.xlsx", Some("col0"));
}

#[test]
#[ignore = "G9, by hand"]
fn g9_xlsx_unkeyed() {
    measure("left.xlsx", "right.xlsx", None);
}
