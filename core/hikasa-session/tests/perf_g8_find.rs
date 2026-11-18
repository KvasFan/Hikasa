//! Gate G8 find files (§1.10.26), by hand and in a release build (`--ignored --nocapture`): `text find` of a word over
//! G7's 10,000 small files (100 folders of 100 files, here text of about 1 KiB each) on the local disk (the temporary
//! folder: run with TMPDIR on a disk), through `Service::text_find` as the CLI calls it, warm (the second run counts):
//! ≤ 1 s, and the stream and the document identical with 1 and 8 threads. The release binary's own figure
//! (`hikasa text find` under `taskset`) is measured by the gate script beside it.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use common::{facts, fixed_clock, job};
use hikasa_core::JobKind;
use hikasa_session::api::TextFindRequest;
use hikasa_session::{Service, ServiceInit};
use hikasa_text::find::files::{FilesEvent, FilesSink};

const FOLDERS: usize = 100;
const FILES: usize = 100;

/// G7's shape with text: 100 folders of 100 files of 24 lines, about 1 KiB each.
fn tree(root: &Path) {
    let mut seed = 0x5EED_F11Eu64;
    for folder in 0..FOLDERS {
        let dir = root.join(format!("d{folder:03}"));
        std::fs::create_dir_all(&dir).unwrap();
        for file in 0..FILES {
            let mut text = String::new();
            for line in 0..24 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let word = ["return", "value", "count", "index", "total"][(seed % 5) as usize];
                text.push_str(&format!(
                    "    int {word}_{line} = compute({}, {});\n",
                    seed % 1000,
                    (seed >> 10) % 100
                ));
            }
            std::fs::write(dir.join(format!("f{file:03}.c")), text).unwrap();
        }
    }
}

fn service(root: &Path, threads: usize) -> Service {
    let init = ServiceInit {
        config_dir: Some(root.join("config")),
        threads: std::num::NonZeroUsize::new(threads),
        ..ServiceInit::new(
            facts(root),
            fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    Service::new(init).unwrap()
}

/// Every event as the CLI's line would show it.
#[derive(Default)]
struct Lines(Vec<String>);

impl FilesSink for Lines {
    fn event(&mut self, e: FilesEvent<'_>) {
        self.0.push(format!("{e:?}"));
    }
}

#[test]
#[ignore = "gate G8 find files, by hand, in a release build"]
fn g8_find_files_through_text_find_on_disk() {
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let t = root.join("tree");
    tree(&t);
    let req = TextFindRequest {
        pattern: "return".into(),
        roots: vec![t.clone().into_os_string()],
        recursive: true,
        match_case: true,
        keep_hits: true,
        ..TextFindRequest::default()
    };
    let mut streams = Vec::new();
    let mut documents = Vec::new();
    let mut pass = true;
    for threads in [8, 1] {
        let service = service(&root, threads);
        let mut times = Vec::new();
        let mut lines = Lines::default();
        let mut dto = None;
        for _ in 0..3 {
            lines = Lines::default();
            let start = Instant::now();
            let outcome = service
                .text_find(&req, &mut lines, &job(&service, JobKind::TextFind))
                .unwrap();
            times.push(start.elapsed().as_secs_f64());
            assert_eq!(outcome.dto.counts.files_searched, (FOLDERS * FILES) as u64);
            dto = Some(outcome.dto);
        }
        let mut warm = times[1..].to_vec();
        warm.sort_by(f64::total_cmp);
        let ok = threads != 8 || warm[0] <= 1.0;
        pass &= ok;
        println!(
            "gate G8 find files through text find on disk ({threads} threads): {} events, {} hits, cold {:.3} s, \
             warm {:.3} s and {:.3} s (limit 1 s with 8 threads); load {} {}",
            lines.0.len(),
            dto.as_ref().map_or(0, |d| d.counts.hits),
            times[0],
            times[1],
            times[2],
            std::fs::read_to_string("/proc/loadavg")
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or(""),
            if ok { "PASS" } else { "FAIL" }
        );
        streams.push(lines.0);
        documents.push(dto);
    }
    let same = streams[0] == streams[1] && documents[0] == documents[1];
    println!(
        "gate G8 find files through text find: the same stream and document with 1 and 8 threads: {same}"
    );
    assert!(pass && same);
}
