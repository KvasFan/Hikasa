//! Random inputs of the W7 properties (P-IL, P-XML, P-FXML, the generated documents of the schema test): text pairs
//! whose lines hold the characters the escaping must handle, and folder trees on `MemoryVfs` whose names hold spaces,
//! controls, markup and non-ASCII letters.

use std::sync::Arc;

use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, NullProgress, Spec};
use hikasa_folder::{
    ContentEngines, FolderComparison, FolderEnv, FolderModel, FolderRules, NullObserver,
};
use hikasa_text::{LoadOptions, TextComparison, TextEncoding, TextRules};
use hikasa_vfs::{MemMeta, MemoryVfs, StreamOptions, Vfs};
use proptest::prelude::*;

use super::{FakeRules, compare_bytes_with, default_now, host, mspec};

/// The characters of generated lines: plain letters, whitespace, markup, characters XML 1.0 cannot carry, the
/// non-characters U+FFFE and U+FFFF, bidirectional controls, U+FFFD, combining marks and emoji parts.
pub const HOSTILE: &[char] = &[
    'a',
    'b',
    'c',
    'x',
    ' ',
    ' ',
    '\t',
    '&',
    '<',
    '>',
    '"',
    '\'',
    ']',
    '\u{0}',
    '\u{1}',
    '\u{1b}',
    '\u{b}',
    '\u{c}',
    '\u{7f}',
    '\u{85}',
    '\u{fffe}',
    '\u{ffff}',
    '\u{202e}',
    '\u{2066}',
    '\u{fffd}',
    'é',
    '\u{301}',
    '日',
    '\u{1f468}',
    '\u{200d}',
    '\u{1f469}',
];

/// The characters of a generated file name for a spec in an attribute: a backslash, the characters of an escape and a
/// control on Unix; on Windows (where a backslash separates and a control is refused) the spec's own `C:\…` has the
/// backslashes.
pub const NAME_CHARS: &[char] = if cfg!(windows) {
    &['a', 'u', '{', '1', '}', ' ']
} else {
    &['a', 'u', '{', '1', '}', ' ', '\\', '\u{1}']
};

/// Plain characters only (properties that read the text form back, where tabs and controls are shown escaped).
pub const PLAIN: &[char] = &['a', 'b', 'c', 'x', 'y', ' ', '-', '+', '[', ']', 'é', '日'];

/// A line of `alphabet`.
pub fn line(alphabet: &'static [char]) -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(alphabet), 0..12)
        .prop_map(|c| c.into_iter().collect())
}

/// A pair of documents: the left's lines, and the right made from them by edits (a line kept, changed, dropped, or
/// one added), so pairs have same, changed and orphan rows. Each ends with a newline or not.
pub fn text_pair(alphabet: &'static [char]) -> impl Strategy<Value = (String, String)> {
    (
        proptest::collection::vec(line(alphabet), 0..14),
        proptest::collection::vec((0u8..6, line(alphabet)), 0..14),
        any::<[bool; 2]>(),
    )
        .prop_map(|(left, edits, ends)| {
            let mut right = Vec::new();
            for (i, l) in left.iter().enumerate() {
                match edits.get(i).map(|(op, _)| *op) {
                    Some(0) => {}
                    Some(1) => right.push(edits[i].1.clone()),
                    Some(2) => {
                        right.push(l.clone());
                        right.push(edits[i].1.clone());
                    }
                    Some(3) => right.push(format!("{l}{}", edits[i].1)),
                    _ => right.push(l.clone()),
                }
            }
            for (_, extra) in edits.iter().skip(left.len()) {
                right.push(extra.clone());
            }
            let join = |lines: &[String], end: bool| {
                let mut s = lines.join("\n");
                if end && !lines.is_empty() {
                    s.push('\n');
                }
                s
            };
            (join(&left, ends[0]), join(&right, ends[1]))
        })
}

/// Compares two strings as UTF-8 (no detection, so every character reaches the report), under `rules`.
pub fn compare_strings(left: &str, right: &str, rules: &TextRules) -> TextComparison {
    compare_strings_cpl("left.txt", left, right, rules, None)
}

/// [`compare_strings`] with the left file's name and a characters-per-line limit (`None`: the format's).
pub fn compare_strings_cpl(
    left_name: &str,
    left: &str,
    right: &str,
    rules: &TextRules,
    chars_per_line: Option<u32>,
) -> TextComparison {
    let mut load = LoadOptions::default();
    load.decode.override_encoding = Some(TextEncoding::UTF_8);
    load.chars_per_line_limit = chars_per_line;
    compare_bytes_with(
        (left_name, left.as_bytes()),
        ("right.txt", right.as_bytes()),
        rules,
        &load,
    )
}

/// Names of generated folder entries.
pub const NAMES: &[&str] = &[
    "a",
    "b b",
    "c\u{1}d",
    "é",
    "x&<y>\"",
    "dir",
    "Z",
    "t\tab",
    "\u{202e}rtl",
    "n\\b",
];

/// One generated node: a file (with content per side) or a folder, on the left, the right, or both.
#[derive(Debug, Clone)]
pub struct GenNode {
    pub name: usize,
    pub folder: bool,
    pub on: [bool; 2],
    pub content: [u8; 2],
    pub mtime: [u8; 2],
    pub children: Vec<GenNode>,
}

/// Random trees.
pub fn folder_tree() -> impl Strategy<Value = Vec<GenNode>> {
    let leaf = (
        0..NAMES.len(),
        any::<bool>(),
        1u8..4,
        any::<[u8; 2]>(),
        any::<[u8; 2]>(),
    )
        .prop_map(|(name, folder, on, content, mtime)| GenNode {
            name,
            folder,
            on: [on & 1 != 0, on & 2 != 0],
            content,
            mtime,
            children: Vec::new(),
        });
    let tree = leaf.prop_recursive(3, 24, 4, |inner| {
        (inner.clone(), proptest::collection::vec(inner, 0..4)).prop_map(|(mut node, children)| {
            if node.folder {
                node.children = children;
            }
            node
        })
    });
    proptest::collection::vec(tree, 1..5)
}

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn insert(vfs: &MemoryVfs, dir: &str, nodes: &[GenNode], side: usize) {
    let mut seen = std::collections::BTreeSet::new();
    for node in nodes {
        if !node.on[side] || !seen.insert(node.name) {
            continue;
        }
        let path = format!("{dir}/{}", NAMES[node.name]);
        if node.folder {
            vfs.insert_dir(&host(&path), meta(1_000_000)).unwrap();
            insert(vfs, &path, &node.children, side);
        } else {
            // `0 ` differs from `0` in whitespace only: unimportant under the rules-based content test.
            let bytes = ["0", "1", "2", "0 "][usize::from(node.content[side] % 4)]
                .as_bytes()
                .to_vec();
            vfs.insert_file(
                &host(&path),
                bytes,
                meta(1_000_000 + i64::from(node.mtime[side] % 3) * 1000),
            )
            .unwrap();
        }
    }
}

/// The tree on a fresh `MemoryVfs` below `/l` and `/r`. A name on both sides is one kind on both (the first node of
/// a name decides on each side, and both see the same list).
pub fn materialize(nodes: &[GenNode]) -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    insert(&vfs, "/l", nodes, 0);
    insert(&vfs, "/r", nodes, 1);
    vfs
}

/// Compares `left` and `right` on `vfs` with `rules` on the current thread.
pub fn compare_on(vfs: &dyn Vfs, left: Spec, right: Spec, rules: &FolderRules) -> FolderComparison {
    let engines = ContentEngines::with_stream_options(
        StreamOptions::default(),
        Some(Arc::new(FakeRules { limit: 64 })),
    );
    let now = default_now();
    let cancel = CancelToken::new();
    let progress = NullProgress;
    let env = FolderEnv {
        vfs,
        content: &engines,
        pool: None,
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &NullObserver,
        now: &now,
        content_budget: 1 << 30,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let model = FolderModel::new(left, right, rules.clone()).unwrap();
    hikasa_folder::compare_into(&model, &env).unwrap()
}

/// [`compare_on`] of `/l` and `/r` on a `MemoryVfs`.
pub fn compare_memory(vfs: &MemoryVfs, rules: &FolderRules) -> FolderComparison {
    compare_on(vfs, mspec("/l"), mspec("/r"), rules)
}
