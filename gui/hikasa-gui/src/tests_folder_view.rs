//! M2 W9a acceptance A9.1 and A9.3 (D-234, D-236, D-237), the toolkit-free parts: the column layout (toggle, move with
//! Name first, resize clamped, fit, the header-click cycle, the layout keys written and read back, an unknown id dropped
//! with one warning), the cell texts of each column (DOS attributes, Unix modes, uids, a CRC, a folder total, a lossy
//! name, `(r)(s)(h)`, local times in a fixed zone) and each row of the glyph table; the Legend's table is the tree's.

use std::collections::BTreeMap;

use hikasa_core::{ContentKind, ContentMethod, FileTime, Side};
use hikasa_folder::select::SideSet;
use hikasa_folder::view::SortKey;
use hikasa_folder::{
    Attrs, Category, ColourClass, CompareState, ContentSummary, DiffFlags, DosAttrs, EntryKind,
    EntryView, FolderRules, NodeFlags, NodeId, NodeKind, NodeView, ScanStatus, UnixMeta,
};
use hikasa_session::api::human::TimeZone;
use hikasa_session::settings::state::LayoutValue;

use crate::folder::cells::{CellInput, attr_marks, cell_of};
use crate::folder::columns::{ColumnId, ColumnLayout, MAX_WIDTH, min_width};
use crate::folder::glyphs::{
    CentreIcon, GLYPH_TABLE, Glyph, GlyphColour, GlyphInput, GlyphView, centre_of, glyph_of,
};

#[test]
fn columns_toggle_move_resize_fit_and_sort() {
    let mut layout = ColumnLayout::default();
    assert_eq!(
        layout.shown(false),
        [ColumnId::Name, ColumnId::Size, ColumnId::Modified]
    );
    // Path only while flat.
    assert_eq!(layout.shown(true).last(), Some(&ColumnId::Path));
    // Name cannot be hidden or moved; others toggle and move behind it.
    layout.toggle(ColumnId::Name);
    assert!(layout.is_visible(ColumnId::Name));
    layout.toggle(ColumnId::Crc);
    layout.toggle(ColumnId::Size);
    assert_eq!(
        layout.shown(false),
        [ColumnId::Name, ColumnId::Modified, ColumnId::Crc]
    );
    layout.move_to(ColumnId::Crc, 0);
    assert_eq!(layout.shown(false)[..2], [ColumnId::Name, ColumnId::Crc]);
    layout.move_to(ColumnId::Name, 3);
    assert_eq!(layout.visible[0], ColumnId::Name);
    layout.move_to(ColumnId::Crc, 99);
    assert_eq!(layout.visible.last(), Some(&ColumnId::Crc));
    // Resize clamps at the minimum and 2,000; fit takes the widest cell.
    layout.resize(ColumnId::Crc, 1.0);
    assert_eq!(layout.width(ColumnId::Crc), min_width(ColumnId::Crc));
    layout.resize(ColumnId::Crc, 1e9);
    assert_eq!(layout.width(ColumnId::Crc), MAX_WIDTH);
    layout.fit(ColumnId::Crc, 70.0);
    assert_eq!(layout.width(ColumnId::Crc), 82.0);
    layout.resize(ColumnId::Name, 500.0);
    assert!(!layout.widths.contains_key(&ColumnId::Name));
    // The header cycle: ascending, descending, tree order; the other side starts again.
    layout.header_click(ColumnId::Modified, Side::Right);
    let asc = SortKey {
        column: ColumnId::Modified,
        side: Side::Right,
        descending: false,
    };
    assert_eq!(layout.sort, Some(asc));
    layout.header_click(ColumnId::Modified, Side::Right);
    assert_eq!(
        layout.sort,
        Some(SortKey {
            descending: true,
            ..asc
        })
    );
    layout.header_click(ColumnId::Modified, Side::Right);
    assert_eq!(layout.sort, None);
    layout.header_click(ColumnId::Crc, Side::Left);
    layout.header_click(ColumnId::Crc, Side::Right);
    assert_eq!(
        layout.sort.map(|s| (s.side, s.descending)),
        Some((Side::Right, false))
    );
    // Hiding the sorted column ends the sort.
    layout.toggle(ColumnId::Crc);
    assert_eq!(layout.sort, None);
}

#[test]
fn the_layout_keys_round_trip_and_unknown_ids_warn_once() {
    let mut layout = ColumnLayout::default();
    layout.toggle(ColumnId::Owner);
    layout.resize(ColumnId::Owner, 70.0);
    layout.header_click(ColumnId::Size, Side::Right);
    layout.header_click(ColumnId::Size, Side::Right);
    let keys: BTreeMap<&str, LayoutValue> = layout.to_layout().into_iter().collect();
    assert_eq!(
        keys["columns"],
        LayoutValue::Texts(
            [
                ColumnId::Name,
                ColumnId::Size,
                ColumnId::Modified,
                ColumnId::Path,
                ColumnId::Owner
            ]
            .map(|c| c.as_str().to_owned())
            .to_vec()
        )
    );
    assert_eq!(
        keys["widths"],
        LayoutValue::Numbers(BTreeMap::from([(
            ColumnId::Owner.as_str().to_owned(),
            70.0
        )]))
    );
    assert_eq!(
        keys["sort"],
        LayoutValue::Text(ColumnId::Size.as_str().into())
    );
    assert_eq!(keys["sort_side"], LayoutValue::Text("right".into()));
    assert_eq!(keys["sort_descending"], LayoutValue::Bool(true));
    let (back, warning) = ColumnLayout::from_values(&|k| keys.get(k).cloned());
    assert_eq!(back, layout);
    assert_eq!(warning, None);
    // Through `state.toml`'s layout table.
    let mut state = hikasa_session::settings::State::default();
    for (key, value) in layout.to_layout() {
        state.set_layout_value("folder-compare", key, value);
    }
    let (again, _) = ColumnLayout::from_values(&|k| state.layout_value("folder-compare", k));
    assert_eq!(again, layout);
    // An unknown id is dropped with one warning; a bad width is clamped.
    let bad: BTreeMap<&str, LayoutValue> = BTreeMap::from([
        (
            "columns",
            LayoutValue::Texts(vec![
                ColumnId::Size.as_str().to_owned(),
                "version".to_owned(),
                ColumnId::Crc.as_str().to_owned(),
            ]),
        ),
        (
            "widths",
            LayoutValue::Numbers(BTreeMap::from([
                (ColumnId::Crc.as_str().to_owned(), 5.0),
                ("bogus".to_owned(), 9.0),
            ])),
        ),
    ]);
    let (read, warning) = ColumnLayout::from_values(&|k| bad.get(k).cloned());
    assert_eq!(
        read.visible,
        [ColumnId::Name, ColumnId::Size, ColumnId::Crc]
    );
    assert_eq!(read.width(ColumnId::Crc), min_width(ColumnId::Crc));
    let warning = warning.unwrap();
    assert!(warning.starts_with("[invalid_config_value]"), "{warning}");
    assert!(
        warning.contains("`version`") && warning.contains("`bogus`"),
        "{warning}"
    );
}

fn entry<'a>(name: &'a str, attrs: Attrs) -> EntryView<'a> {
    EntryView {
        name,
        name_lossy: false,
        kind: EntryKind::File,
        size: 1_234_567,
        mtime: FileTime::from_unix(1_700_000_000, 0),
        ctime: None,
        attrs,
        link_target: None,
        followed: false,
        crc32: Some(0x00ab_cdef),
        filtered: false,
        stat_skipped: false,
        dir: None,
    }
}

fn spot() -> GlyphView {
    GLYPH_TABLE[0].0
}

#[test]
fn the_cells_of_every_column() {
    let unix = Attrs::Unix {
        meta: UnixMeta {
            mode: 0o444,
            uid: 1001,
            gid: 50,
        },
    };
    let kathmandu = TimeZone::posix("NPT-5:45").unwrap();
    let input = CellInput {
        folder: false,
        scanned: true,
        folder_bytes: 0,
        zone: &kathmandu,
        colour: ColourClass::Same,
        glyph: spot(),
    };
    let e = entry("report.tar.gz", unix);
    let cell = cell_of(&e, &input);
    assert_eq!(cell.ext, "gz");
    assert_eq!(cell.size, "1\u{202f}234\u{202f}567");
    assert_eq!(cell.modified, "2023-11-15 03:58:20");
    assert_eq!(cell.attributes, "-r--r--r--");
    assert_eq!(cell.crc, "00abcdef");
    assert_eq!((cell.owner.as_str(), cell.group.as_str()), ("1001", "50"));
    assert_eq!(cell.marks, "(r)");
    assert_eq!(cell.text(ColumnId::Name, false), "report.tar.gz (r)");
    assert_eq!(cell.text(ColumnId::Name, true), "report.tar.gz");
    // UTC: another day.
    let utc = cell_of(
        &e,
        &CellInput {
            zone: &TimeZone::UTC,
            ..input
        },
    );
    assert_eq!(utc.modified, "2023-11-14 22:13:20");
    // DOS attributes, the letters and the marks.
    let dos = Attrs::Dos {
        attrs: DosAttrs::READONLY | DosAttrs::HIDDEN | DosAttrs::SYSTEM | DosAttrs::ARCHIVE,
    };
    let d = entry("x", dos);
    assert_eq!(attr_marks(&d), "(r)(s)(h)");
    assert_eq!(cell_of(&d, &input).attributes, "AHRS");
    assert_eq!(cell_of(&d, &input).owner, "");
    // A writable Unix file: no mark; a dotfile gets none for being hidden.
    let w = entry(
        ".rc",
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o644,
                uid: 0,
                gid: 0,
            },
        },
    );
    assert_eq!(attr_marks(&w), "");
    assert_eq!(cell_of(&w, &input).ext, "");
    // Any write bit counts (`mode & 0o222`): the owner may not write, the group may: no mark (mutation AD).
    let group = entry(
        "g",
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o464,
                uid: 0,
                gid: 0,
            },
        },
    );
    assert_eq!(attr_marks(&group), "");
    // A folder: its total once scanned, nothing before.
    let folder = CellInput {
        folder: true,
        folder_bytes: 4096,
        ..input
    };
    assert_eq!(cell_of(&e, &folder).size, "4\u{202f}096");
    assert_eq!(cell_of(&e, &folder).ext, "");
    assert_eq!(
        cell_of(
            &e,
            &CellInput {
                scanned: false,
                ..folder
            }
        )
        .size,
        ""
    );
    // A lossy name keeps its replacement characters and says so; control characters are escaped; the flat Path.
    let lossy = EntryView {
        name: "a\u{fffd}b\n",
        name_lossy: true,
        dir: Some("x/y"),
        ..e
    };
    let c = cell_of(&lossy, &input);
    assert!(c.lossy);
    assert_eq!(c.name, "a\u{fffd}b\\n");
    assert_eq!(c.path, "x/y");
}

fn node<'a>(
    kind: NodeKind,
    scan: ScanStatus,
    left: Option<EntryView<'a>>,
    right: Option<EntryView<'a>>,
) -> NodeView<'a> {
    NodeView {
        id: NodeId(1),
        parent: Some(NodeId::ROOT),
        kind,
        state: CompareState::Same,
        diffs: DiffFlags::empty(),
        time: None,
        content: None,
        scan,
        flags: NodeFlags::empty(),
        left,
        right,
    }
}

#[test]
fn each_row_of_the_glyph_table() {
    let dir = EntryView {
        kind: EntryKind::Dir,
        ..entry("d", Attrs::None)
    };
    let rules = FolderRules::default();
    let mut manual = FolderRules::default();
    manual.handling.auto_scan_subfolders = false;
    let mut no_orphans = FolderRules::default();
    no_orphans.handling.scan_orphan_folders = false;
    let at = |rules: &FolderRules, running: bool, failed: bool, n: &NodeView<'_>| {
        glyph_of(
            n,
            Side::Left,
            &GlyphInput {
                rules,
                running,
                failed_here: failed,
            },
        )
    };
    let both = |scan| node(NodeKind::Folder, scan, Some(dir), Some(dir));
    let want = |base, colour, error, link| GlyphView {
        base,
        colour,
        error,
        link,
    };
    // A file: the spot.
    let file = node(
        NodeKind::File,
        ScanStatus::NotApplicable,
        Some(entry("f", Attrs::None)),
        None,
    );
    assert_eq!(at(&rules, false, false, &file), GLYPH_TABLE[0].0);
    // Scanned folder.
    let scanned = at(&rules, false, false, &both(ScanStatus::Scanned));
    assert_eq!(scanned, GLYPH_TABLE[1].0);
    // Not scanned: while running, after a stop (auto-scan on), an orphan left unscanned → hollow.
    let hollow = want(Glyph::FolderHollow, GlyphColour::Unknown, false, false);
    assert_eq!(
        at(&rules, true, false, &both(ScanStatus::NotScanned)),
        hollow
    );
    assert_eq!(
        at(&rules, false, false, &both(ScanStatus::NotScanned)),
        hollow
    );
    let orphan = node(NodeKind::Folder, ScanStatus::NotScanned, Some(dir), None);
    assert_eq!(at(&no_orphans, false, false, &orphan), hollow);
    assert_eq!(hollow, GLYPH_TABLE[2].0);
    // Auto-scan off: filled with `unopened_folder` (not while running).
    let filled = want(Glyph::FolderFilled, GlyphColour::Unopened, false, false);
    assert_eq!(
        at(&manual, false, false, &both(ScanStatus::NotScanned)),
        filled
    );
    assert_eq!(
        at(&manual, true, false, &both(ScanStatus::NotScanned)),
        hollow
    );
    assert_eq!(filled, GLYPH_TABLE[3].0);
    // Failed on this side: the red X; on the other side only: a plain folder.
    let failed = at(&rules, false, true, &both(ScanStatus::Failed));
    assert_eq!(failed, GLYPH_TABLE[4].0);
    assert_eq!(at(&rules, false, false, &both(ScanStatus::Failed)), scanned);
    // Links, followed or not, files or folders: the arrow.
    let unfollowed = node(
        NodeKind::Symlink,
        ScanStatus::NotApplicable,
        Some(entry("l", Attrs::None)),
        None,
    );
    assert_eq!(at(&rules, false, false, &unfollowed), GLYPH_TABLE[5].0);
    let followed_dir = EntryView {
        followed: true,
        ..dir
    };
    let followed = node(
        NodeKind::Folder,
        ScanStatus::Scanned,
        Some(followed_dir),
        None,
    );
    assert_eq!(
        at(&rules, false, false, &followed),
        want(Glyph::Folder, GlyphColour::Class, false, true)
    );
    // An archive: the zipper, as a file and opened as a folder.
    let mut zip = node(
        NodeKind::File,
        ScanStatus::NotApplicable,
        Some(entry("a.zip", Attrs::None)),
        None,
    );
    zip.flags = NodeFlags::ARCHIVE;
    assert_eq!(at(&rules, false, false, &zip), GLYPH_TABLE[6].0);
    let mut opened = both(ScanStatus::Scanned);
    opened.flags = NodeFlags::ARCHIVE;
    assert_eq!(at(&rules, false, false, &opened).base, Glyph::Archive);
    // Every table row is distinct (the Legend lists each once).
    for (i, a) in GLYPH_TABLE.iter().enumerate() {
        for b in &GLYPH_TABLE[i + 1..] {
            assert_ne!(a.0, b.0);
        }
    }
}

#[test]
fn the_centre_column() {
    let e = entry("f", Attrs::None);
    let mut n = node(NodeKind::File, ScanStatus::NotApplicable, Some(e), Some(e));
    // The newer side's arrow: left newer points right.
    let left = centre_of(&n, Category::LeftNewer, false);
    assert_eq!(left.icon, CentreIcon::Newer(Side::Left));
    assert_eq!((left.mark, left.tooltip.as_str()), ("▶", "Left is newer"));
    assert_eq!(centre_of(&n, Category::RightNewer, false).mark, "◀");
    assert_eq!(
        centre_of(&n, Category::Same, false).icon,
        CentreIcon::State(Category::Same)
    );
    // A content result wins.
    for (kind, mark) in [
        (ContentKind::BinarySame, "="),
        (ContentKind::BinaryDifferent, "≠"),
        (ContentKind::RulesSame, "≈"),
        (ContentKind::UnimportantOnly, "~"),
        (ContentKind::ImportantDifferences, "!"),
    ] {
        n.content = Some(ContentSummary {
            method: ContentMethod::Rules,
            kind,
        });
        let c = centre_of(&n, Category::LeftNewer, false);
        assert_eq!((c.icon, c.mark), (CentreIcon::Content(kind), mark));
    }
    assert_eq!(
        centre_of(&n, Category::Same, false).tooltip,
        "Rules-based: important differences"
    );
    // Ignored badge and the Space mark.
    n.flags = NodeFlags::IGNORED;
    let c = centre_of(&n, Category::Same, true);
    assert!(c.ignored && c.marked);
    assert!(c.tooltip.ends_with(" · Ignored · Marked"), "{}", c.tooltip);
    let _ = SideSet::all();
}
