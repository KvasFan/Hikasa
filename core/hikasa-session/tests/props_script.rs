//! P-SCRIPT (A8.10, §1.8.15, D-227): a script's command is the CLI command of the same words. Random twin trees on disk
//! (the Service's router serves local files: W8a note 12), random settings, random `expand`/`select`, then one random
//! command with `option confirm:yes-to-all`; the same Service calls made directly with the request the CLI builds for
//! the matching flags give the same outcome document, the same trees and the same report bytes. A second command
//! (`folder-report`) checks that the refresh after the first leaves the model equal to a fresh comparison.

mod common;
#[path = "script_common/mod.rs"]
mod script_common;

use std::collections::BTreeMap;
use std::ffi::OsString;

use hikasa_core::JobKind;
use hikasa_folder::NullObserver;
use hikasa_folder::ops::{ConfirmMode, FolderOp, PathStructure, Recycle, RenameMask, TouchFrom};
use hikasa_folder::select::SideSet;
use hikasa_report::{ReportKind, ReportWords};
use hikasa_session::api::{
    ContentsKind, ContentsRequest, ExpandWords, FolderCompareRequest, FolderOpRequest,
    FolderSyncRequest, OpOverrides, OpRequest, ReportFlags, ReportSource, ReportTarget,
    SelectRequest, TextCompareRequest, resolve_folder_display, sync_options,
};
use hikasa_vfs::ops::OverwritePolicy;
use proptest::prelude::*;
use script_common::{Ui, World, yes};

/// One entry of a generated tree: a path among a few names, its sides and its bytes.
#[derive(Debug, Clone)]
struct Entry {
    path: &'static str,
    left: Option<u8>,
    right: Option<u8>,
}

const PATHS: [&str; 6] = [
    "a.txt",
    "b.dat",
    "sub/c.txt",
    "sub/d.txt",
    "sub/deep/e.txt",
    "z.txt",
];

fn entries() -> impl Strategy<Value = Vec<Entry>> {
    proptest::collection::vec(
        (
            0..PATHS.len(),
            proptest::option::of(0u8..3),
            proptest::option::of(0u8..3),
        ),
        1..8,
    )
    .prop_map(|list| {
        let mut seen = std::collections::BTreeSet::new();
        list.into_iter()
            .filter(|(i, _, _)| seen.insert(*i))
            .map(|(i, left, right)| Entry {
                path: PATHS[i],
                left,
                right,
            })
            .collect()
    })
}

fn build(w: &World, tree: &[Entry]) {
    w.dir("l");
    w.dir("r");
    for entry in tree {
        for (side, version) in [("l", entry.left), ("r", entry.right)] {
            if let Some(v) = version {
                let content = format!("{}{}\n", entry.path, "x".repeat(usize::from(v)));
                w.file(
                    &format!("{side}/{}", entry.path),
                    &content,
                    1_000_000 + i64::from(v) * 10,
                );
            }
        }
    }
}

/// The command of the case: its script words and the direct call.
#[derive(Debug, Clone)]
enum Cmd {
    Copy(bool),
    Move(bool),
    CopyTo(u8),
    Delete(u8),
    Rename,
    Touch(u8),
    Attrib,
    Sync(u8),
    Compare,
    FolderReport,
    TextReport,
}

fn cmd() -> impl Strategy<Value = Cmd> {
    prop_oneof![
        any::<bool>().prop_map(Cmd::Copy),
        any::<bool>().prop_map(Cmd::Move),
        (0u8..3).prop_map(Cmd::CopyTo),
        (0u8..3).prop_map(Cmd::Delete),
        Just(Cmd::Rename),
        (0u8..3).prop_map(Cmd::Touch),
        Just(Cmd::Attrib),
        (0u8..4).prop_map(Cmd::Sync),
        Just(Cmd::Compare),
        Just(Cmd::FolderReport),
        Just(Cmd::TextReport),
    ]
}

const SIDES: [&str; 3] = ["left", "right", "all"];
const MASKS: [&str; 6] = [
    "all",
    "left.diff.files",
    "right.orphan.all",
    "all.orphan.files",
    "left.newer",
    "all.exact.files",
];

fn sides(i: u8) -> SideSet {
    match i {
        0 => SideSet::LEFT,
        1 => SideSet::RIGHT,
        _ => SideSet::all(),
    }
}

/// The script line of a command.
fn line(cmd: &Cmd) -> String {
    match cmd {
        Cmd::Copy(right) => format!(
            "copy {}",
            if *right { "left->right" } else { "right->left" }
        ),
        Cmd::Move(right) => format!("move {}", if *right { "lt->rt" } else { "rt->lt" }),
        Cmd::CopyTo(s) => format!("copyto {} path:base out", SIDES[usize::from(*s)]),
        Cmd::Delete(s) => format!("delete {}", SIDES[usize::from(*s)]),
        Cmd::Rename => "rename regexpr \"(.*)\\.txt\" \"$1.md\"".to_owned(),
        Cmd::Touch(s) => format!("touch {}:2026-01-02T03:04:05Z", SIDES[usize::from(*s)]),
        Cmd::Attrib => "attrib +r".to_owned(),
        Cmd::Sync(m) => [
            "sync update:left->right",
            "sync update:right->left",
            "sync update:all",
            "sync mirror:left->right",
        ][usize::from(*m)]
        .to_owned(),
        Cmd::Compare => "compare crc".to_owned(),
        Cmd::FolderReport => "folder-report layout:summary output-to:f.txt".to_owned(),
        Cmd::TextReport => "text-report layout:summary output-to:t.txt l/a.txt r/a.txt".to_owned(),
    }
}

/// Replaces the sandbox's path by `<TMP>` in a JSON value (twin worlds have twin paths).
fn normal(w: &World, value: serde_json::Value) -> serde_json::Value {
    let root = w.dir.path().display().to_string();
    let text = value
        .to_string()
        .replace(&root.replace('\\', "\\\\"), "<TMP>")
        .replace(&root, "<TMP>");
    serde_json::from_str(&text).unwrap()
}

/// The direct Service calls of `folder op`, `folder sync`, `folder op compare`, `report` for the same words.
fn direct(
    w: &World,
    criteria: Option<&str>,
    expand_all: bool,
    masks: &[&str],
    cmd: &Cmd,
) -> Option<serde_json::Value> {
    let s = &w.service;
    let request = FolderCompareRequest {
        left: OsString::from("l"),
        right: OsString::from("r"),
        criteria: criteria.map(str::to_owned),
        ..FolderCompareRequest::default()
    };
    let job = common::job(s, JobKind::FolderCompare);
    let select = SelectRequest {
        masks: masks.iter().map(|m| (*m).to_owned()).collect(),
        paths: Vec::new(),
        selection: None,
        include_hidden: None,
        display: Some(resolve_folder_display(Some("all"), None, false, false).unwrap()),
        expand: if expand_all {
            ExpandWords::All
        } else {
            ExpandWords::None
        },
        iu: false,
    };
    let op = |op: OpRequest| -> Option<serde_json::Value> {
        let prepared = s.folder_prepare_in(&request, &job).ok()?;
        s.folder_run(&prepared, &job, &NullObserver).ok()?;
        let req = FolderOpRequest {
            op,
            select: select.clone(),
            read_only: Default::default(),
            overrides: OpOverrides::default(),
        };
        let plan = s.folder_op_plan(prepared.model(), &req, &job).ok()?;
        let plan = s
            .folder_confirmation_strict(&plan, Some(ConfirmMode::YesToAll))
            .finish(plan)
            .ok()?;
        let outcome = s.folder_op_execute(plan, &job).ok()?;
        Some(serde_json::to_value(outcome.dto()).unwrap())
    };
    let copy = |to, moving| {
        let overwrite = OverwritePolicy::Always;
        OpRequest::Op(if moving {
            FolderOp::Move { to, overwrite }
        } else {
            FolderOp::Copy { to, overwrite }
        })
    };
    use hikasa_core::Side::{Left, Right};
    match cmd {
        Cmd::Copy(right) => op(copy(if *right { Right } else { Left }, false)),
        Cmd::Move(right) => op(copy(if *right { Right } else { Left }, true)),
        Cmd::CopyTo(i) => op(OpRequest::CopyTo {
            sides: sides(*i),
            target: OsString::from("out"),
            structure: PathStructure::Base,
            overwrite: OverwritePolicy::Always,
        }),
        Cmd::Delete(i) => op(OpRequest::Op(FolderOp::Delete {
            sides: sides(*i),
            recycle: Recycle::default(),
        })),
        Cmd::Rename => op(OpRequest::Op(FolderOp::Rename {
            sides: SideSet::all(),
            mask: RenameMask::Regex {
                old: "(.*)\\.txt".into(),
                new: "$1.md".into(),
            },
        })),
        Cmd::Touch(i) => op(OpRequest::Op(FolderOp::Touch {
            sides: sides(*i),
            from: TouchFrom::At(hikasa_session::api::parse_time("2026-01-02T03:04:05Z").unwrap()),
        })),
        Cmd::Attrib => op(OpRequest::Op(FolderOp::Attrib {
            sides: SideSet::all(),
            change: hikasa_session::api::parse_attrib_change(&["+r".to_owned()]).unwrap(),
        })),
        Cmd::Sync(m) => {
            let preset =
                ["update-right", "update-left", "update-both", "mirror-right"][usize::from(*m)];
            let words = hikasa_session::ipc::SyncWords {
                preset: Some(preset.to_owned()),
                ..Default::default()
            };
            let options = sync_options(&words, &Default::default(), cfg!(windows)).unwrap();
            let prepared = s.folder_sync_prepare(&request, &options, &job).ok()?;
            s.folder_run(&prepared, &job, &NullObserver).ok()?;
            let req = FolderSyncRequest {
                options,
                ..FolderSyncRequest::default()
            };
            let mut plan = s.folder_sync_plan(prepared.model(), &req, &job).ok()?;
            let stand_in = plan.op.clone();
            let op = std::mem::replace(&mut plan.op, stand_in);
            plan.op = s
                .folder_confirmation_strict(&op, Some(ConfirmMode::YesToAll))
                .finish(op)
                .ok()?;
            let outcome = s.folder_sync_execute(plan, &job).ok()?;
            Some(serde_json::to_value(outcome.dto(&prepared.model().read(), false)).unwrap())
        }
        Cmd::Compare => {
            let prepared = s.folder_prepare_in(&request, &job).ok()?;
            s.folder_run(&prepared, &job, &NullObserver).ok()?;
            let req = ContentsRequest {
                select,
                kind: ContentsKind::Compare(
                    hikasa_session::api::parse_content_method("crc").unwrap(),
                ),
                text_rules: prepared.text_rules().cloned(),
            };
            Some(
                serde_json::to_value(s.folder_contents(prepared.model(), &req, &job).ok()?)
                    .unwrap(),
            )
        }
        Cmd::FolderReport | Cmd::TextReport => {
            let folder = matches!(cmd, Cmd::FolderReport);
            let flags = ReportFlags {
                words: ReportWords {
                    layout: Some("summary".into()),
                    ..ReportWords::default()
                },
                ..ReportFlags::default()
            };
            let (kind, file) = if folder {
                (ReportKind::Folder, "f.txt")
            } else {
                (ReportKind::Text, "t.txt")
            };
            let target = ReportTarget::File(w.path(file).into_os_string());
            let mut req = s
                .report_request(kind, "side-by-side", &flags, &target)
                .ok()?;
            let report_job = common::job(s, JobKind::Report);
            let outcome = if folder {
                let prepared = s.folder_prepare_in(&request, &job).ok()?;
                let response = s.folder_run(&prepared, &job, &NullObserver).ok()?;
                let source = ReportSource::Folder {
                    request: &request,
                    prepared: &prepared,
                    response: &response,
                };
                let plan = s
                    .report_plan(source, &mut req, &target, None, &report_job)
                    .ok()?;
                s.report_execute(source, &req, plan, None, &report_job)
                    .ok()?
            } else {
                let text = TextCompareRequest {
                    left: OsString::from("l/a.txt"),
                    right: OsString::from("r/a.txt"),
                    ..TextCompareRequest::default()
                };
                let resp = s
                    .text_compare(&text, &common::job(s, JobKind::TextCompare), None)
                    .ok()?;
                let source = ReportSource::Text(&resp);
                let plan = s
                    .report_plan(source, &mut req, &target, None, &report_job)
                    .ok()?;
                s.report_execute(source, &req, plan, None, &report_job)
                    .ok()?
            };
            Some(serde_json::to_value(outcome.dto()).unwrap())
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(24)
    ))]

    #[test]
    fn p_script(
        tree in entries(),
        criteria in proptest::option::of(prop_oneof![Just("size"), Just("crc"), Just("size timestamp")]),
        expand_all in any::<bool>(),
        masks in proptest::collection::vec(0..MASKS.len(), 1..3),
        cmd in cmd(),
    ) {
        let masks: Vec<&str> = masks.into_iter().map(|i| MASKS[i]).collect();
        let a = World::new();
        let b = World::new();
        build(&a, &tree);
        build(&b, &tree);
        let mut source = String::from("option confirm:yes-to-all\n");
        if let Some(c) = criteria {
            source.push_str(&format!("criteria {c}\n"));
        }
        source.push_str("load l r\n");
        if expand_all {
            source.push_str("expand all\n");
        }
        source.push_str(&format!("select {}\n", masks.join(" ")));
        source.push_str(&line(&cmd));
        source.push_str("\nfolder-report layout:summary output-to:after.txt\n");
        let mut ui = Ui::default();
        let outcome = a.run_with(&source, yes(), &mut ui);
        let index = outcome.commands.len() - 2;
        let from_script = outcome.commands[index].result.clone();
        let from_direct = direct(&b, criteria, expand_all, &masks, &cmd);
        match (from_script, from_direct) {
            (Some(s), Some(d)) => prop_assert_eq!(normal(&a, s), normal(&b, d), "{}\n{:?}", source, ui.lines),
            (None, None) => {}
            (s, d) => prop_assert!(false, "one side failed: script {:?} direct {:?}\n{}\n{:?}", s.is_some(), d.is_some(), source, ui.lines),
        }
        prop_assert_eq!(a.dump("l"), b.dump("l"));
        prop_assert_eq!(a.dump("r"), b.dump("r"));
        if a.path("out").exists() || b.path("out").exists() {
            prop_assert_eq!(a.dump("out"), b.dump("out"));
        }
        for file in ["f.txt", "t.txt"] {
            let read = |w: &World| std::fs::read_to_string(w.path(file)).ok().map(|t| t.replace(&w.dir.path().display().to_string(), "<TMP>"));
            prop_assert_eq!(read(&a), read(&b), "{}", file);
        }
        // The refresh after a mutation left the model a fresh comparison sees (`compare` adds content results a fresh
        // comparison does not have, and a report changes nothing: they are not refreshes).
        if matches!(cmd, Cmd::Compare | Cmd::FolderReport | Cmd::TextReport) {
            return Ok(());
        }
        let report = |w: &World| std::fs::read_to_string(w.path("after.txt")).ok().map(|t| t.replace(&w.dir.path().display().to_string(), "<TMP>"));
        let fresh_req = FolderCompareRequest {
            left: OsString::from("l"),
            right: OsString::from("r"),
            criteria: criteria.map(str::to_owned),
            ..FolderCompareRequest::default()
        };
        let s = &b.service;
        let job = common::job(s, JobKind::FolderCompare);
        let prepared = s.folder_prepare_in(&fresh_req, &job).unwrap();
        let response = s.folder_run(&prepared, &job, &NullObserver).unwrap();
        let flags = ReportFlags { words: ReportWords { layout: Some("summary".into()), ..ReportWords::default() }, ..ReportFlags::default() };
        let target = ReportTarget::File(b.path("after.txt").into_os_string());
        let mut req = s.report_request(ReportKind::Folder, "side-by-side", &flags, &target).unwrap();
        let source_ = ReportSource::Folder { request: &fresh_req, prepared: &prepared, response: &response };
        let report_job = common::job(s, JobKind::Report);
        let plan = s.report_plan(source_, &mut req, &target, None, &report_job).unwrap();
        s.report_execute(source_, &req, plan, None, &report_job).unwrap();
        prop_assert_eq!(report(&a), report(&b), "{}", source);
        let _ = BTreeMap::<u8, u8>::new();
    }
}
