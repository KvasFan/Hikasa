//! G15 preview's core part, by hand and in a release build (`--ignored --nocapture`): `folder_op_plan` of Copy to Right
//! with every left item selected on a tree whose every file pair replaces (the G15 shape: a tree given by
//! `HIKASA_PLAN_TREE` with `left` and `right` folders, else 10,000 files a side made here), then `folder_op_preview` and
//! `folder_confirmation`, each timed (W9b review: the plan job's own time, apart from the window's first frame).

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{facts, fixed_clock, job, os};
use hikasa_core::{JobKind, Side};
use hikasa_folder::ops::FolderOp;
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::{NodeId, NullObserver};
use hikasa_session::api::{
    ExpandWords, FolderCompareRequest, FolderOpRequest, OpOverrides, OpRequest, SelectRequest,
    resolve_folder_display,
};
use hikasa_session::{Service, ServiceInit};
use hikasa_vfs::ops::OverwritePolicy;

fn made_tree(root: &Path) -> PathBuf {
    for side in ["left", "right"] {
        for folder in 0..100 {
            let dir = root.join(side).join(format!("d{folder:02}"));
            std::fs::create_dir_all(&dir).unwrap();
            for file in 0..100 {
                std::fs::write(
                    dir.join(format!("f{file:02}.txt")),
                    format!("{folder} {file}\n"),
                )
                .unwrap();
            }
        }
    }
    root.to_path_buf()
}

/// Drops the `node` and `refresh` keys (row ids) everywhere.
fn strip_nodes(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("node");
            map.remove("refresh");
            map.values_mut().for_each(strip_nodes);
        }
        serde_json::Value::Array(list) => list.iter_mut().for_each(strip_nodes),
        _ => {}
    }
}

#[test]
#[ignore = "by hand, release: G15 preview's plan time"]
fn g15_preview_plan_time() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = std::env::var_os("HIKASA_PLAN_TREE")
        .map(PathBuf::from)
        .unwrap_or_else(|| made_tree(tmp.path()));
    let init = ServiceInit {
        config_dir: Some(tmp.path().join("config")),
        ..ServiceInit::new(
            facts(tmp.path()),
            fixed_clock(),
            std::sync::Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    let request = FolderCompareRequest {
        left: os(&tree.join("left")),
        right: os(&tree.join("right")),
        ..FolderCompareRequest::default()
    };
    let control = job(&service, JobKind::FolderCompare);
    let began = Instant::now();
    let prepared = service.folder_prepare_in(&request, &control).unwrap();
    service
        .folder_run(&prepared, &control, &NullObserver)
        .unwrap();
    eprintln!("scan {:?}", began.elapsed());
    let model = std::sync::Arc::clone(prepared.model());
    let mut selection = Selection::default();
    let mut items = 0;
    {
        let read = model.read();
        let mut stack = vec![NodeId::ROOT];
        while let Some(id) = stack.pop() {
            for child in read.children(id) {
                if read.node(child).is_some_and(|v| v.left.is_some()) {
                    selection.insert(child, SideSet::LEFT);
                    items += 1;
                }
                stack.push(child);
            }
        }
    }
    let display = resolve_folder_display(Some("all"), None, false, false).unwrap();
    let mut plans = Vec::new();
    for round in 0..3 {
        let request = FolderOpRequest {
            op: OpRequest::Op(FolderOp::Copy {
                to: Side::Right,
                overwrite: OverwritePolicy::Always,
            }),
            select: SelectRequest {
                selection: Some(selection.clone()),
                display: Some(display),
                expand: ExpandWords::None,
                include_hidden: Some(false),
                ..SelectRequest::default()
            },
            read_only: Default::default(),
            overrides: OpOverrides::default(),
        };
        let control = job(&service, JobKind::FolderOp);
        let began = Instant::now();
        let plan = service.folder_op_plan(&model, &request, &control).unwrap();
        let planned = began.elapsed();
        let began = Instant::now();
        let preview = service.folder_op_preview(&plan);
        let previewed = began.elapsed();
        let began = Instant::now();
        let confirmation = service.folder_confirmation(&plan, None);
        let confirmed = began.elapsed();
        eprintln!(
            "round {round}: {items} selected, plan {planned:?} ({} items), preview {previewed:?} ({} lines), \
             confirmation {confirmed:?}",
            plan.plan.items.len(),
            preview.len()
        );
        drop(confirmation);
        // The temporary folder (the configuration) and the node ids (a parallel scan numbers its rows as they come)
        // differ from run to run: the plan is compared without them.
        let mut value = serde_json::to_value(&plan).unwrap();
        strip_nodes(&mut value);
        let json = value
            .to_string()
            .replace(&tmp.path().display().to_string(), "<tmp>");
        if let Some(dump) = std::env::var_os("HIKASA_PLAN_DUMP") {
            std::fs::write(dump, &json).unwrap();
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&json, &mut hasher);
        plans.push(std::hash::Hasher::finish(&hasher));
    }
    eprintln!("plan json hash {:016x}", plans[0]);
    assert!(
        plans.windows(2).all(|w| w[0] == w[1]),
        "the same plan each round"
    );
}
