//! A5.11: every row of §1.5.8 through real records. No call reads the system zone: the zone comes from the injected
//! clock, and the tests pass two clocks.

mod common;

use std::sync::Arc;

use common::archives::{RawEntry, RawTar, compress, config_in, local, pax, raw_tar, raw_zip};
use hikasa_core::{FileTime, Spec};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveVfs, FollowLinks, LocalVfs, MemMeta, MemoryVfs, Vfs,
};
use jiff::tz::{TimeZone, offset};
use proptest::prelude::*;

const fn dos_date(year: u16, month: u16, day: u16) -> u16 {
    ((year - 1980) << 9) | (month << 5) | day
}

const fn dos_clock(hour: u16, minute: u16, second: u16) -> u16 {
    (hour << 11) | (minute << 5) | (second / 2)
}

/// A back-end whose clock lies in `zone`, over a memory tree with `bytes` as `name` (mtime 12:00).
fn back_end(zone: TimeZone, name: &str, bytes: Vec<u8>) -> (ArchiveVfs, Spec, tempfile::TempDir) {
    let mem = Arc::new(MemoryVfs::new(hikasa_core::CaseSensitivity::Sensitive));
    let meta = MemMeta {
        mtime: FileTime::parse("2026-10-19T12:00:00Z").ok(),
        ..MemMeta::default()
    };
    let path = common::host(&format!("/{name}"));
    mem.insert_file(&path, bytes, meta).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let vfs = ArchiveVfs::new(
        mem as Arc<dyn Vfs>,
        config_in(dir.path(), ArchiveLimits::default(), zone),
    );
    (vfs, common::mspec(&format!("/{name}")), dir)
}

fn zip_entry(date: u16, time: u16, extra: Vec<u8>) -> Vec<u8> {
    let mut entry = RawEntry::new(b"m", b"x");
    entry.dos_date = date;
    entry.dos_time = time;
    entry.extra = extra;
    raw_zip(&[entry])
}

fn time_of(zone: TimeZone, zip: Vec<u8>) -> Option<String> {
    let (vfs, file, _dir) = back_end(zone, "a.zip", zip);
    let spec = Spec::archive_member(file, "m");
    vfs.stat(&spec, FollowLinks::No)
        .unwrap()
        .mtime
        .map(FileTime::to_rfc3339)
}

fn unix_field(seconds: u32) -> Vec<u8> {
    let mut extra = vec![0x55, 0x54, 5, 0, 1];
    extra.extend_from_slice(&seconds.to_le_bytes());
    extra
}

fn ntfs_field(modified: u64, created: u64) -> Vec<u8> {
    let mut extra = vec![0x0a, 0, 32, 0, 0, 0, 0, 0, 1, 0, 24, 0];
    extra.extend_from_slice(&modified.to_le_bytes());
    extra.extend_from_slice(&0u64.to_le_bytes());
    extra.extend_from_slice(&created.to_le_bytes());
    extra
}

const INSTANT: u32 = 1_792_415_520;
const NOON: u16 = dos_clock(12, 0, 0);

#[test]
fn rows_1_to_4_dos_times() {
    let summer = dos_date(2026, 7, 15);
    let cet = || TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
    let plus2 = || offset(2).to_time_zone();
    let at = |zone, date, time| time_of(zone, zip_entry(date, time, Vec::new()));
    assert_eq!(
        at(plus2(), summer, NOON).as_deref(),
        Some("2026-07-15T10:00:00Z")
    );
    assert_eq!(
        at(offset(-5).to_time_zone(), summer, NOON).as_deref(),
        Some("2026-07-15T17:00:00Z")
    );
    let gap = at(cet(), dos_date(2026, 3, 29), dos_clock(2, 30, 0));
    assert_eq!(gap.as_deref(), Some("2026-03-29T01:30:00Z"));
    let fold = at(cet(), dos_date(2026, 10, 25), dos_clock(2, 30, 0));
    assert_eq!(fold.as_deref(), Some("2026-10-25T00:30:00Z"));
}

#[test]
fn rows_5_to_8_extra_fields() {
    let date = dos_date(2026, 7, 15);
    for zone in [TimeZone::UTC, offset(9).to_time_zone()] {
        let unix = time_of(zone.clone(), zip_entry(date, NOON, unix_field(INSTANT)));
        assert_eq!(unix.as_deref(), Some("2026-10-19T13:12:00Z"), "row 5");
        let filetime =
            |seconds: u64, hundred_ns: u64| (seconds + 11_644_473_600) * 10_000_000 + hundred_ns;
        let mut extra = unix_field(1);
        extra.extend(ntfs_field(
            filetime(u64::from(INSTANT), 5_000_000),
            filetime(1, 0),
        ));
        let zip = zip_entry(date, NOON, extra);
        assert_eq!(
            time_of(zone.clone(), zip.clone()).as_deref(),
            Some("2026-10-19T13:12:00.5Z"),
            "row 6"
        );
        let (vfs, file, _dir) = back_end(zone, "a.zip", zip);
        let entry = vfs
            .stat(&Spec::archive_member(file, "m"), FollowLinks::No)
            .unwrap();
        assert_eq!(entry.ctime, FileTime::from_unix(1, 0), "the creation time");
    }
    // Row 7.
    for (date, time) in [
        (dos_date(2026, 13, 1), NOON),
        (dos_date(2026, 1, 0), NOON),
        (date, NOON | 30),
        (0, NOON),
    ] {
        assert_eq!(
            time_of(TimeZone::UTC, zip_entry(date, time, Vec::new())),
            None
        );
    }
    // Row 8: a malformed field, and the DOS time wins.
    let malformed = time_of(
        offset(2).to_time_zone(),
        zip_entry(date, NOON, vec![0x55, 0x54, 3, 0, 1, 2, 3]),
    );
    assert_eq!(malformed.as_deref(), Some("2026-07-15T10:00:00Z"));
}

fn tar_time_of(tar: Vec<u8>) -> Option<String> {
    let (vfs, file, _dir) = back_end(TimeZone::UTC, "a.tar", tar);
    vfs.stat(&Spec::archive_member(file, "m"), FollowLinks::No)
        .unwrap()
        .mtime
        .map(FileTime::to_rfc3339)
}

#[test]
fn rows_9_to_11_tar() {
    let with = |value: &[u8]| {
        let mut tar = pax(&[("mtime", value)]);
        tar.extend(raw_tar(&[RawTar::file(b"m", b"x")]));
        tar_time_of(tar)
    };
    assert_eq!(
        with(b"1792415520.5").as_deref(),
        Some("2026-10-19T13:12:00.5Z")
    );
    assert_eq!(
        with(b"abc").as_deref(),
        Some("2026-10-19T13:12:00Z"),
        "row 11"
    );
    let epoch = RawTar {
        mtime: 0,
        ..RawTar::file(b"m", b"x")
    };
    assert_eq!(
        tar_time_of(raw_tar(&[epoch])).as_deref(),
        Some("1970-01-01T00:00:00Z")
    );
}

#[test]
fn row_12_gzip_mtime() {
    let at = |bytes: Vec<u8>| {
        let (vfs, file, _dir) = back_end(TimeZone::UTC, "x.txt.gz", bytes);
        let spec = Spec::archive_member(file, "x.txt");
        vfs.stat(&spec, FollowLinks::No)
            .unwrap()
            .mtime
            .map(FileTime::to_rfc3339)
    };
    let mut gz = compress(ArchiveFormat::Gz, b"x");
    gz[4..8].fill(0);
    assert_eq!(
        at(gz.clone()).as_deref(),
        Some("2026-10-19T12:00:00Z"),
        "the container"
    );
    gz[4..8].copy_from_slice(&INSTANT.to_le_bytes());
    assert_eq!(at(gz).as_deref(), Some("2026-10-19T13:12:00Z"));
    let bz2 = compress(ArchiveFormat::Bz2, b"x");
    let (vfs, file, _dir) = back_end(TimeZone::UTC, "x.txt.bz2", bz2);
    let entry = vfs
        .stat(&Spec::archive_member(file, "x.txt"), FollowLinks::No)
        .unwrap();
    assert_eq!(entry.mtime, FileTime::parse("2026-10-19T12:00:00Z").ok());
}

/// `LocalVfs` containers carry the same fields.
#[test]
fn times_on_disk() {
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let path = root.join("a.zip");
    std::fs::write(
        &path,
        zip_entry(dos_date(2026, 7, 15), NOON, unix_field(INSTANT)),
    )
    .unwrap();
    let vfs = ArchiveVfs::new(
        Arc::new(LocalVfs::default()) as Arc<dyn Vfs>,
        config_in(&root.join("cache"), ArchiveLimits::default(), TimeZone::UTC),
    );
    let entry = vfs
        .stat(&Spec::archive_member(local(&path), "m"), FollowLinks::No)
        .unwrap();
    assert_eq!(entry.mtime, FileTime::from_unix(i64::from(INSTANT), 0));
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/archive_time.txt"))]

    /// A member with a 0x5455 field reads the same instant in zones −12…+14; one with only DOS time reads the wall
    /// time back in any zone.
    #[test]
    fn zones(hours in -12i8..=14, seconds in 0u32..=u32::MAX, month in 1u16..=12, day in 1u16..=28, hour in 0u16..24) {
        let zone = offset(hours).to_time_zone();
        let date = dos_date(2026, month, day);
        let time = dos_clock(hour, 7, 30);
        let exact = time_of(zone.clone(), zip_entry(date, time, unix_field(seconds)));
        let utc = time_of(TimeZone::UTC, zip_entry(date, time, unix_field(seconds)));
        prop_assert_eq!(exact, utc);

        let read = time_of(zone.clone(), zip_entry(date, time, Vec::new())).unwrap();
        let stamp: jiff::Timestamp = read.parse().unwrap();
        let wall = stamp.to_zoned(zone).datetime();
        prop_assert_eq!(
            (wall.year(), wall.month() as u16, wall.day() as u16, wall.hour() as u16, wall.minute(), wall.second()),
            (2026, month, day, hour, 7, 30)
        );
    }
}
