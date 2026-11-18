//! P-SCRIPT (A8.10, §1.8.15, D-227): a script's command is the CLI command of the same words. Random twin trees on disk
//! (the Service's router serves local files: W8a note 12) with W6's generators (a name in two cases, one in NFC or NFD,
//! read-only files, newer files, orphans, an empty folder), random settings (`criteria`, `filter`), random
//! `expand`/`select`, then one random command in a random confirm mode (`yes-to-all`, `no-to-all`, `prompt` with
//! scripted replies); the same Service calls made directly with the request the CLI builds for the matching flags and
//! the same answers give the same outcome document (or the same refusal), the same trees and the same report bytes.
//! `delete recyclebin=yes` is compared as a dry run (never the real recycle bin). A second command (`folder-report`)
//! checks that the refresh after the first leaves the model equal to a fresh comparison.

mod common;
#[path = "script_common/mod.rs"]
mod script_common;

use std::ffi::OsString;

use hikasa_core::JobKind;
use hikasa_folder::NullObserver;
use hikasa_folder::ops::{
    ConfirmMode, FolderOp, OpPlan, PathStructure, Recycle, RenameMask, Reply, TouchFrom,
};
use hikasa_folder::select::SideSet;
use hikasa_report::{ReportKind, ReportWords};
use hikasa_session::api::script::ScriptRunRequest;
use hikasa_session::api::{
    ContentsKind, ContentsRequest, ExpandWords, FolderCompareRequest, FolderOpRequest,
    FolderSyncRequest, OpOverrides, OpRequest, ReportFlags, ReportSource, ReportTarget,
    SelectRequest, TextCompareRequest, resolve_folder_display, sync_options,
};
use hikasa_vfs::ops::OverwritePolicy;
use proptest::prelude::*;
use script_common::{Ui, World, yes};

/// One entry of a generated tree: a path among a few names (one of them in two cases, one in NFC or NFD), its bytes
/// on each side, which spelling each side uses, and whether each side's file is read-only.
#[derive(Debug, Clone)]
struct Entry {
    path: usize,
    left: Option<u8>,
    right: Option<u8>,
    /// The other spelling on (left, right): upper case, or NFD for the accented name.
    spell: (bool, bool),
    read_only: (bool, bool),
}

/// The names, and their second spelling (W6's generators: case, NFC/NFD).
const PATHS: [(&str, &str); 8] = [
    ("a.txt", "A.TXT"),
    ("b.dat", "b.dat"),
    ("sub/c.txt", "sub/C.txt"),
    ("sub/d.txt", "sub/d.txt"),
    ("sub/deep/e.txt", "sub/deep/e.txt"),
    ("z.txt", "z.txt"),
    ("caf\u{e9}.txt", "cafe\u{301}.txt"),
    ("sub/n\u{e9}e.dat", "sub/ne\u{301}e.dat"),
];

fn entries() -> impl Strategy<Value = Vec<Entry>> {
    proptest::collection::vec(
        (
            0..PATHS.len(),
            proptest::option::of(0u8..3),
            proptest::option::of(0u8..3),
            any::<(bool, bool)>(),
            prop_oneof![
                6 => Just((false, false)),
                1 => Just((true, false)),
                1 => Just((false, true)),
                1 => Just((true, true)),
            ],
        ),
        1..9,
    )
    .prop_map(|list| {
        let mut seen = std::collections::BTreeSet::new();
        list.into_iter()
            .filter(|(i, ..)| seen.insert(*i))
            .map(|(path, left, right, spell, read_only)| Entry {
                path,
                left,
                right,
                spell,
                read_only,
            })
            .collect()
    })
}

/// The tree on disk: files first with their times, then read-only bits (a read-only file's time is set before), and an
/// empty folder on the left (`sync create-empty`); the folders' times last, the same in both worlds, so a document that
/// shows a folder row (the empty folder, `sub`) shows the same time on NTFS too, where a folder's time moves when a file
/// inside gets its time.
fn build(w: &World, tree: &[Entry], empty_folder: bool) {
    w.dir("l");
    w.dir("r");
    if empty_folder {
        w.dir("l/empty");
    }
    for entry in tree {
        let sides = [
            ("l", entry.left, entry.spell.0, entry.read_only.0),
            ("r", entry.right, entry.spell.1, entry.read_only.1),
        ];
        for (side, version, other, read_only) in sides {
            let Some(v) = version else { continue };
            let (first, second) = PATHS[entry.path];
            let path = if other { second } else { first };
            let content = format!("{first}{}\n", "x".repeat(usize::from(v)));
            let rel = format!("{side}/{path}");
            w.file(&rel, &content, 1_000_000 + i64::from(v) * 10);
            if read_only {
                script_common::set_read_only(&w.path(&rel), true);
            }
        }
    }
    script_common::set_folder_times(w, "l", 900_000);
    script_common::set_folder_times(w, "r", 900_000);
}

/// The command of the case: its script words and the direct call.
#[derive(Debug, Clone)]
enum Cmd {
    Copy(bool),
    Move(bool),
    CopyTo(u8),
    MoveTo(u8),
    Delete(u8),
    /// `delete recyclebin=yes`: compared as a dry run only (never the real recycle bin).
    Recycle(u8),
    Rename,
    RenameMask,
    Touch(u8),
    /// `touch left->right` (`true`) or `right->left`.
    TouchFrom(bool),
    Attrib(bool),
    /// A sync preset, `create-empty`, `visible`.
    Sync(u8, bool, bool),
    Compare,
    FolderReport,
    TextReport,
}

fn cmd() -> impl Strategy<Value = Cmd> {
    prop_oneof![
        any::<bool>().prop_map(Cmd::Copy),
        any::<bool>().prop_map(Cmd::Move),
        (0u8..3).prop_map(Cmd::CopyTo),
        (0u8..3).prop_map(Cmd::MoveTo),
        (0u8..3).prop_map(Cmd::Delete),
        (0u8..3).prop_map(Cmd::Recycle),
        Just(Cmd::Rename),
        Just(Cmd::RenameMask),
        (0u8..3).prop_map(Cmd::Touch),
        any::<bool>().prop_map(Cmd::TouchFrom),
        any::<bool>().prop_map(Cmd::Attrib),
        (0u8..4, any::<bool>(), any::<bool>()).prop_map(|(m, c, v)| Cmd::Sync(m, c, v)),
        Just(Cmd::Compare),
        Just(Cmd::FolderReport),
        Just(Cmd::TextReport),
    ]
}

/// How the questions are answered: `option confirm:yes-to-all`, `no-to-all`, or `prompt` with these replies (then
/// Cancel), the same replies to the direct calls' questions.
#[derive(Debug, Clone)]
enum Answers {
    YesToAll,
    NoToAll,
    Prompt(Vec<Reply>),
}

fn answers() -> impl Strategy<Value = Answers> {
    let reply = prop_oneof![
        Just(Reply::Yes),
        Just(Reply::No),
        Just(Reply::YesToAll),
        Just(Reply::NoToAll),
        Just(Reply::Cancel),
    ];
    prop_oneof![
        2 => Just(Answers::YesToAll),
        1 => Just(Answers::NoToAll),
        1 => proptest::collection::vec(reply, 0..4).prop_map(Answers::Prompt),
    ]
}

impl Answers {
    fn word(&self) -> &'static str {
        match self {
            Answers::YesToAll => "yes-to-all",
            Answers::NoToAll => "no-to-all",
            Answers::Prompt(_) => "prompt",
        }
    }

    /// The direct calls' confirmation of `plan`: W6's machine in this mode, answered with the same replies.
    fn finish(&self, s: &hikasa_session::Service, plan: OpPlan) -> Result<OpPlan, String> {
        let mode = match self {
            Answers::YesToAll => ConfirmMode::YesToAll,
            Answers::NoToAll => ConfirmMode::NoToAll,
            Answers::Prompt(_) => ConfirmMode::Prompt,
        };
        let mut machine = s.folder_confirmation_strict(&plan, Some(mode));
        let mut replies: std::collections::VecDeque<Reply> = match self {
            Answers::Prompt(replies) => replies.iter().copied().collect(),
            _ => Default::default(),
        };
        while machine.next().is_some() {
            machine.reply(replies.pop_front().unwrap_or(Reply::Cancel));
        }
        machine
            .finish(plan)
            .map_err(|_| "declined: nothing was written".to_owned())
    }
}

const SIDES: [&str; 3] = ["left", "right", "all"];
const MASKS: [&str; 7] = [
    "all",
    "left.diff.files",
    "right.orphan.all",
    "all.orphan.files",
    "left.newer",
    "all.exact.files",
    "all.folders",
];
const FILTERS: [&str; 3] = ["*.txt", "*.dat;sub\\", "-*.dat"];

fn sides(i: u8) -> SideSet {
    match i {
        0 => SideSet::LEFT,
        1 => SideSet::RIGHT,
        _ => SideSet::all(),
    }
}

const SYNCS: [(&str, &str); 4] = [
    ("update:left->right", "update-right"),
    ("update:right->left", "update-left"),
    ("update:all", "update-both"),
    ("mirror:left->right", "mirror-right"),
];

/// The script line of a command.
fn line(cmd: &Cmd) -> String {
    match cmd {
        Cmd::Copy(right) => format!(
            "copy {}",
            if *right { "left->right" } else { "right->left" }
        ),
        Cmd::Move(right) => format!("move {}", if *right { "lt->rt" } else { "rt->lt" }),
        Cmd::CopyTo(s) => format!("copyto {} path:base out", SIDES[usize::from(*s)]),
        Cmd::MoveTo(s) => format!("moveto {} path:relative out", SIDES[usize::from(*s)]),
        Cmd::Delete(s) => format!("delete {}", SIDES[usize::from(*s)]),
        Cmd::Recycle(s) => format!("delete recyclebin=yes {}", SIDES[usize::from(*s)]),
        Cmd::Rename => "rename regexpr \"(.*)\\.txt\" \"$1.md\"".to_owned(),
        Cmd::RenameMask => "rename *.bak".to_owned(),
        Cmd::Touch(s) => format!("touch {}:2026-01-02T03:04:05Z", SIDES[usize::from(*s)]),
        Cmd::TouchFrom(right) => format!("touch {}", if *right { "left->right" } else { "rt->lt" }),
        Cmd::Attrib(set) => if *set { "attrib +r" } else { "attrib -r" }.to_owned(),
        Cmd::Sync(m, create_empty, visible) => {
            let mut words = vec!["sync"];
            if *visible {
                words.push("visible");
            }
            if *create_empty {
                words.push("create-empty");
            }
            words.push(SYNCS[usize::from(*m)].0);
            words.join(" ")
        }
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

/// The settings words of a case.
#[derive(Debug, Clone, Copy)]
struct Settings {
    criteria: Option<&'static str>,
    filter: Option<&'static str>,
    expand_all: bool,
}

/// The direct Service calls of `folder op`, `folder sync`, `folder op compare`, `report` for the same words: the
/// outcome document, or the message of the call that refused.
fn direct(
    w: &World,
    settings: Settings,
    masks: &[&str],
    cmd: &Cmd,
    answers: &Answers,
) -> Result<serde_json::Value, String> {
    let s = &w.service;
    let text = |e: hikasa_session::ApiError| e.to_string();
    let mut request = FolderCompareRequest {
        left: OsString::from("l"),
        right: OsString::from("r"),
        criteria: settings.criteria.map(str::to_owned),
        ..FolderCompareRequest::default()
    };
    request.patch.name_filter = settings.filter.map(str::to_owned);
    let job = common::job(s, JobKind::FolderCompare);
    let display = resolve_folder_display(Some("all"), None, false, false).unwrap();
    let select = SelectRequest {
        masks: masks.iter().map(|m| (*m).to_owned()).collect(),
        paths: Vec::new(),
        selection: None,
        include_hidden: None,
        display: Some(display),
        expand: if settings.expand_all {
            ExpandWords::All
        } else {
            ExpandWords::None
        },
        iu: false,
    };
    let op_with = |op: OpRequest, dry_run: bool| -> Result<serde_json::Value, String> {
        let prepared = s.folder_prepare_in(&request, &job).map_err(text)?;
        s.folder_run(&prepared, &job, &NullObserver).map_err(text)?;
        let req = FolderOpRequest {
            op,
            select: select.clone(),
            read_only: Default::default(),
            overrides: OpOverrides::default(),
        };
        let plan = s
            .folder_op_plan(prepared.model(), &req, &job)
            .map_err(text)?;
        let outcome = if dry_run {
            s.folder_op_dry_run(plan)
        } else {
            let plan = answers.finish(s, plan)?;
            s.folder_op_execute(plan, &job).map_err(text)?
        };
        Ok(serde_json::to_value(outcome.dto()).unwrap())
    };
    let op = |op: OpRequest| op_with(op, false);
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
        Cmd::MoveTo(i) => op(OpRequest::MoveTo {
            sides: sides(*i),
            target: OsString::from("out"),
            structure: PathStructure::Relative,
            overwrite: OverwritePolicy::Always,
        }),
        Cmd::Delete(i) => op(OpRequest::Op(FolderOp::Delete {
            sides: sides(*i),
            recycle: Recycle::default(),
        })),
        Cmd::Recycle(i) => op_with(
            OpRequest::Op(FolderOp::Delete {
                sides: sides(*i),
                recycle: Recycle {
                    use_bin: true,
                    fallback: false,
                },
            }),
            true,
        ),
        Cmd::Rename => op(OpRequest::Op(FolderOp::Rename {
            sides: SideSet::all(),
            mask: RenameMask::Regex {
                old: "(.*)\\.txt".into(),
                new: "$1.md".into(),
            },
        })),
        Cmd::RenameMask => op(OpRequest::Op(FolderOp::Rename {
            sides: SideSet::all(),
            mask: RenameMask::Dos("*.bak".into()),
        })),
        Cmd::Touch(i) => op(OpRequest::Op(FolderOp::Touch {
            sides: sides(*i),
            from: TouchFrom::At(hikasa_session::api::parse_time("2026-01-02T03:04:05Z").unwrap()),
        })),
        Cmd::TouchFrom(right) => {
            let to = if *right { Right } else { Left };
            op(OpRequest::Op(FolderOp::Touch {
                sides: SideSet::of(to),
                from: TouchFrom::OtherSide { to },
            }))
        }
        Cmd::Attrib(set) => op(OpRequest::Op(FolderOp::Attrib {
            sides: SideSet::all(),
            change: hikasa_session::api::parse_attrib_change(&[
                if *set { "+r" } else { "-r" }.to_owned()
            ])
            .unwrap(),
        })),
        Cmd::Sync(m, create_empty, visible) => {
            let words = hikasa_session::ipc::SyncWords {
                preset: Some(SYNCS[usize::from(*m)].1.to_owned()),
                create_empty: *create_empty,
                ..Default::default()
            };
            let options = sync_options(&words, &Default::default(), cfg!(windows)).unwrap();
            let prepared = s
                .folder_sync_prepare(&request, &options, &job)
                .map_err(text)?;
            s.folder_run(&prepared, &job, &NullObserver).map_err(text)?;
            let req = FolderSyncRequest {
                options,
                visible: visible.then_some(display),
                ..FolderSyncRequest::default()
            };
            let mut plan = s
                .folder_sync_plan(prepared.model(), &req, &job)
                .map_err(text)?;
            let stand_in = plan.op.clone();
            let op = std::mem::replace(&mut plan.op, stand_in);
            plan.op = answers.finish(s, op)?;
            let outcome = s.folder_sync_execute(plan, &job).map_err(text)?;
            Ok(serde_json::to_value(outcome.dto(&prepared.model().read(), false)).unwrap())
        }
        Cmd::Compare => {
            let prepared = s.folder_prepare_in(&request, &job).map_err(text)?;
            s.folder_run(&prepared, &job, &NullObserver).map_err(text)?;
            let req = ContentsRequest {
                select,
                kind: ContentsKind::Compare(
                    hikasa_session::api::parse_content_method("crc").unwrap(),
                ),
                text_rules: prepared.text_rules().cloned(),
            };
            Ok(serde_json::to_value(
                s.folder_contents(prepared.model(), &req, &job)
                    .map_err(text)?,
            )
            .unwrap())
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
                .map_err(text)?;
            let report_job = common::job(s, JobKind::Report);
            let outcome = if folder {
                let prepared = s.folder_prepare_in(&request, &job).map_err(text)?;
                let response = s.folder_run(&prepared, &job, &NullObserver).map_err(text)?;
                let source = ReportSource::Folder {
                    request: &request,
                    prepared: &prepared,
                    response: &response,
                };
                let plan = s
                    .report_plan(source, &mut req, &target, None, &report_job)
                    .map_err(text)?;
                s.report_execute(source, &req, plan, None, &report_job)
                    .map_err(text)?
            } else {
                let text_req = TextCompareRequest {
                    left: OsString::from("l/a.txt"),
                    right: OsString::from("r/a.txt"),
                    ..TextCompareRequest::default()
                };
                let resp = s
                    .text_compare(&text_req, &common::job(s, JobKind::TextCompare), None)
                    .map_err(text)?;
                let source = ReportSource::Text(&resp);
                let plan = s
                    .report_plan(source, &mut req, &target, None, &report_job)
                    .map_err(text)?;
                s.report_execute(source, &req, plan, None, &report_job)
                    .map_err(text)?
            };
            Ok(serde_json::to_value(outcome.dto()).unwrap())
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
        filter in proptest::option::of(0..FILTERS.len()),
        expand_all in any::<bool>(),
        masks in proptest::collection::vec(0..MASKS.len(), 1..3),
        cmd in cmd(),
        answers in answers(),
        empty_folder in any::<bool>(),
    ) {
        let masks: Vec<&str> = masks.into_iter().map(|i| MASKS[i]).collect();
        let settings = Settings { criteria, filter: filter.map(|i| FILTERS[i]), expand_all };
        let dry_run = matches!(cmd, Cmd::Recycle(_));
        let a = World::new();
        let b = World::new();
        build(&a, &tree, empty_folder);
        build(&b, &tree, empty_folder);
        let mut source = format!("option confirm:{}\n", answers.word());
        if let Some(c) = settings.criteria {
            source.push_str(&format!("criteria {c}\n"));
        }
        if let Some(f) = settings.filter {
            source.push_str(&format!("filter \"{f}\"\n"));
        }
        source.push_str("load l r\n");
        if expand_all {
            source.push_str("expand all\n");
        }
        source.push_str(&format!("select {}\n", masks.join(" ")));
        source.push_str(&line(&cmd));
        source.push_str("\nfolder-report layout:summary output-to:after.txt\n");
        let mut ui = Ui { can_ask: true, ..Ui::default() };
        if let Answers::Prompt(replies) = &answers {
            ui.replies = replies.iter().copied().collect();
        }
        let req = ScriptRunRequest { confirm: None, dry_run, ..yes() };
        let outcome = a.run_with(&source, req, &mut ui);
        let index = outcome.commands.len() - 2;
        let command = &outcome.commands[index];
        let from_direct = direct(&b, settings, &masks, &cmd, &answers);
        match (&command.result, from_direct) {
            (Some(s), Ok(d)) => prop_assert_eq!(normal(&a, s.clone()), normal(&b, d), "{}\n{:?}", source, ui.lines),
            // Both refused: the script's command failed with the message of the call that refused.
            (None, Err(message)) => {
                prop_assert_eq!(command.status, hikasa_session::api::script::CommandStatus::Failed, "{}", source);
                let root = |w: &World| w.dir.path().display().to_string();
                let error = command.error.as_ref().map(|e| e.message.replace(&root(&a), "<TMP>"));
                prop_assert_eq!(error, Some(message.replace(&root(&b), "<TMP>")), "{}\n{:?}", source, ui.lines);
            }
            (s, d) => prop_assert!(false, "one side failed: script {:?} direct {:?}\n{}\n{:?}", s.is_some(), d, source, ui.lines),
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
        // comparison does not have, a report changes nothing, and a dry run writes no report: they are not refreshes).
        if dry_run || matches!(cmd, Cmd::Compare | Cmd::FolderReport | Cmd::TextReport) {
            return Ok(());
        }
        let report = |w: &World| std::fs::read_to_string(w.path("after.txt")).ok().map(|t| t.replace(&w.dir.path().display().to_string(), "<TMP>"));
        let mut fresh_req = FolderCompareRequest {
            left: OsString::from("l"),
            right: OsString::from("r"),
            criteria: settings.criteria.map(str::to_owned),
            ..FolderCompareRequest::default()
        };
        fresh_req.patch.name_filter = settings.filter.map(str::to_owned);
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
        prop_assert!(report(&a).is_some(), "{}", source);
        prop_assert_eq!(report(&a), report(&b), "{}", source);
    }
}
