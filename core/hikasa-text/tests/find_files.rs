//! A10.23 (D-284), the engine's share: `find_in_files` on `MemoryVfs` trees (§1.10.19). Pre-order by name, masks,
//! recursion, hidden items, binary and too-large files, limits (`find_limit_reached`, a file's stop), previews, and
//! the event stream identical with no pool and with 1, 2 and 8 threads. The Service, disk trees, archives, External
//! formats, the cancel and the memory rows are `hikasa-session`'s `text_find_files.rs`.

mod common;

use common::save::spec;
use hikasa_core::{CaseSensitivity, Ctl, MaskOptions, NameFilter, WarningCode};
use hikasa_text::find::FindQuery;
use hikasa_text::find::files::{FilesEvent, FilesQuery, FilesSink, find_in_files};
use hikasa_vfs::{MemMeta, MemoryVfs};

/// Every event as one line.
#[derive(Default)]
struct Record(Vec<String>);

impl FilesSink for Record {
    fn event(&mut self, e: FilesEvent<'_>) {
        self.0.push(match e {
            FilesEvent::File {
                index,
                spec,
                encoding,
            } => format!("file {index} {} {}", spec.canonical(), encoding.name()),
            FilesEvent::Hit {
                file,
                line,
                column,
                len_chars,
                preview,
                preview_range,
                truncated,
            } => format!(
                "hit {file} {line}:{column}+{len_chars} {preview:?} {preview_range:?} {truncated}"
            ),
            FilesEvent::Skipped { spec, reason } => {
                format!("skip {} {}", spec.canonical(), reason.as_str())
            }
            FilesEvent::Done {
                file,
                hits,
                stopped,
            } => format!("done {file} {hits} {stopped}"),
        });
    }
}

fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let put = |path: &str, bytes: &[u8]| {
        vfs.insert_file(&common::host(path), bytes.to_vec(), MemMeta::default())
            .unwrap();
    };
    put("/r/a.txt", b"foo\nbar foo\n");
    put("/r/b.bin", b"foo\0\0\0\x01\x02binary");
    put("/r/.hidden/x.txt", b"foo hidden\n");
    put("/r/sub/c.c", b"int foo;\r\n");
    put("/r/sub/d.txt", b"nothing\n");
    put("/r/e.log", b"x\nfoo");
    put("/r/big.txt", &b"foo ".repeat(300));
    vfs
}

fn query(text: &str) -> FilesQuery {
    let mut q = FilesQuery::new(
        FindQuery {
            text: text.to_owned(),
            regex: false,
            match_case: true,
            whole_word: false,
        },
        vec![spec("/r")],
    );
    q.limits.max_file_bytes = 10_000;
    q
}

fn run(
    vfs: &MemoryVfs,
    q: &FilesQuery,
    threads: Option<usize>,
) -> (Vec<String>, hikasa_text::find::files::FilesSummary) {
    let registry = common::registry();
    let pool = threads.map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    let mut rec = Record::default();
    let summary =
        find_in_files(vfs, &registry, q, &mut rec, pool.as_ref(), &Ctl::detached()).unwrap();
    (rec.0, summary)
}

#[test]
fn a_tree_in_pre_order_by_name() {
    let vfs = tree();
    let mut q = query("foo");
    q.limits.max_file_bytes = 100;
    let (events, summary) = run(&vfs, &q, None);
    let host = |p: &str| spec(p).canonical();
    let expected = vec![
        format!("file 0 {} UTF-8", host("/r/a.txt")),
        "hit 0 1:1+3 \"foo\" 0..3 false".to_owned(),
        "hit 0 2:5+3 \"bar foo\" 4..7 false".to_owned(),
        "done 0 2 false".to_owned(),
        format!("skip {} binary", host("/r/b.bin")),
        format!("skip {} too_large", host("/r/big.txt")),
        format!("file 1 {} UTF-8", host("/r/e.log")),
        "hit 1 2:1+3 \"foo\" 0..3 false".to_owned(),
        "done 1 1 false".to_owned(),
        format!("file 2 {} UTF-8", host("/r/sub/c.c")),
        "hit 2 1:5+3 \"int foo;\" 4..7 false".to_owned(),
        "done 2 1 false".to_owned(),
        format!("file 3 {} UTF-8", host("/r/sub/d.txt")),
        "done 3 0 false".to_owned(),
    ];
    assert_eq!(events, expected);
    assert_eq!(
        (
            summary.files_searched,
            summary.files_matched,
            summary.hits,
            summary.skipped
        ),
        (4, 3, 4, 2)
    );
    assert!(!summary.limit_reached);
}

#[test]
fn masks_recursion_and_hidden_items() {
    let vfs = tree();
    let mut q = query("foo");
    q.masks = NameFilter::parse(
        "*.txt;*.c;-d.txt",
        MaskOptions {
            case: CaseSensitivity::Sensitive,
        },
    )
    .unwrap();
    let files = |q: &FilesQuery| -> Vec<String> {
        run(&vfs, q, None)
            .0
            .into_iter()
            .filter(|e| e.starts_with("file"))
            .map(|e| {
                e.rsplit_once(' ')
                    .unwrap()
                    .0
                    .split(' ')
                    .nth(2)
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    let names = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .map(|s| s.rsplit(['/', '\\']).next().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(files(&q)), ["a.txt", "big.txt", "c.c"]);
    q.include_hidden = true;
    assert_eq!(names(files(&q)), ["x.txt", "a.txt", "big.txt", "c.c"]);
    q.recursive = false;
    assert_eq!(names(files(&q)), ["a.txt", "big.txt"]);
}

#[test]
fn limits_stop_a_file_and_the_search() {
    let vfs = tree();
    let mut q = query("foo");
    q.limits.max_matches_per_file = 1;
    let (events, summary) = run(&vfs, &q, None);
    assert!(events.contains(&"done 0 1 true".to_owned()), "{events:#?}");
    assert!(!summary.limit_reached);
    q.limits.max_matches_per_file = 10_000;
    q.limits.max_matches = 3;
    let (events, summary) = run(&vfs, &q, None);
    assert_eq!(summary.hits, 3);
    assert!(summary.limit_reached);
    assert_eq!(summary.warnings[0].code, WarningCode::FindLimitReached);
    // The search stopped inside big.txt: nothing after it.
    assert!(
        events.last().unwrap().starts_with("done 1 1 true"),
        "{events:#?}"
    );
}

#[test]
fn previews_are_cut_around_the_hit() {
    let vfs = tree();
    let mut q = query("foo");
    q.limits.preview_chars = 6;
    q.limits.max_matches = 4;
    let (events, _) = run(&vfs, &q, None);
    // big.txt is `foo foo …`: its second hit is centred in six characters.
    assert!(
        events.contains(&"hit 1 1:5+3 \" foo f\" 1..4 true".to_owned()),
        "{events:#?}"
    );
}

#[test]
fn the_stream_does_not_depend_on_the_pool() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let mut rng = common::XorShift(0x000F_11E5);
    for i in 0..300 {
        let mut text = String::new();
        for _ in 0..rng.below(40) {
            text.push_str(["foo ", "bar ", "x\n", "\r\n", "foofoo\n", "é"][rng.below(6) as usize]);
        }
        let dir = ["a", "b/c", "b", "d/e/f"][rng.below(4) as usize];
        vfs.insert_file(
            &common::host(&format!("/r/{dir}/{i:03}.txt")),
            text.into_bytes(),
            MemMeta::default(),
        )
        .unwrap();
    }
    let mut q = query("foo");
    q.limits.max_matches = 2_000;
    q.limits.max_matches_per_file = 5;
    let (base, summary) = run(&vfs, &q, None);
    assert!(summary.files_searched == 300 || summary.limit_reached);
    for threads in [1, 2, 8] {
        let (events, s) = run(&vfs, &q, Some(threads));
        assert_eq!(events, base, "{threads} threads");
        assert_eq!(s, summary);
    }
}

#[test]
fn a_missing_root_refuses_the_search() {
    let vfs = tree();
    let mut q = query("foo");
    q.roots = vec![spec("/nope")];
    let registry = common::registry();
    let mut rec = Record::default();
    assert!(find_in_files(&vfs, &registry, &q, &mut rec, None, &Ctl::detached()).is_err());
    // A file root is searched as given, whatever the masks say.
    q.roots = vec![spec("/r/e.log")];
    q.masks = NameFilter::parse(
        "*.txt",
        MaskOptions {
            case: CaseSensitivity::Sensitive,
        },
    )
    .unwrap();
    let (events, _) = run(&vfs, &q, None);
    assert_eq!(events.len(), 3, "{events:#?}");
}
