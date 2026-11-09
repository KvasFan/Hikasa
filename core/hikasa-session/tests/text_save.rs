//! M2 W10b acceptance A10.15 (D-274, contract §1.10.14) through the Service: each save question produced and answered
//! (Yes, No, YesToAll, Cancel) by W6's machine as the adapters drive it (`Service::save_confirmation`, `SaveDriver`);
//! `ChangedOnDisk` (same size, other bytes) against `source_touched` (same bytes, a new time); unmappable characters
//! under `Ask`, `Refuse` and `Replace` in windows-1252 and Shift_JIS, written as `?` and never as numeric references;
//! `LossyRewritten`; the flags' answers (`--force`, `--yes`, `--unmappable replace`) and the refusal of open questions
//! with their counts (the CLI without a terminal, `cli_text_convert.rs` runs the binary); the exit codes of a save.

mod common;

use std::path::Path;
use std::time::{Duration, SystemTime};

use common::{job, os, service, write};
use hikasa_core::confirm::Reply;
use hikasa_core::{ExitCode, JobKind, Pair, Side, WarningCode};
use hikasa_formats::{TextEncoding, encode};
use hikasa_session::Service;
use hikasa_session::api::{
    LiveOptions, LiveTextView, SaveAnswers, TextCompareRequest, apply_save_report, save_exit_code,
};
use hikasa_text::save::{SavePlan, SaveQuestion, SaveReport, SaveRequest, UnmappablePolicy};
use hikasa_text::{EditRequest, EditableDocument, GroupHint, LineIdx, LineMark, TextPos};
use hikasa_vfs::ops::{ItemStatus, SkipReason};

// ---- helpers ---------------------------------------------------------------------------------------------------

/// Opens `left` and `right` live, with an encoding for both when given.
fn open(service: &Service, left: &Path, right: &Path, encoding: Option<&str>) -> LiveTextView {
    let req = TextCompareRequest {
        left: os(left),
        right: os(right),
        encodings: Pair::new(encoding.map(str::to_owned), encoding.map(str::to_owned)),
        ..TextCompareRequest::default()
    };
    service
        .text_compare_live(
            &req,
            LiveOptions::default(),
            &job(service, JobKind::TextCompare),
            None,
        )
        .unwrap()
}

/// Types `text` at the start of `line`.
fn type_at(doc: &mut EditableDocument, line: u32, text: &str) {
    let at = TextPos::new(line, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: text.into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

/// Appends `text` at the end of `line`.
fn append_at(doc: &mut EditableDocument, line: u32, text: &str) {
    let len = doc.line_len(LineIdx(line));
    let at = TextPos::new(line, len);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: text.into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

/// An in-place save without a backup.
fn in_place(unmappable: UnmappablePolicy) -> SaveRequest {
    SaveRequest::in_place(None, unmappable)
}

/// Plans one document's save.
fn plan_one(service: &Service, side: Side, doc: &EditableDocument, req: &SaveRequest) -> SavePlan {
    service
        .text_save_plan(&[(Some(side), doc, req)], &job(service, JobKind::TextSave))
        .unwrap()
}

/// Executes `plan` for `docs`.
fn execute(service: &Service, plan: SavePlan, docs: &[&EditableDocument]) -> SaveReport {
    service
        .text_save_execute(plan, docs, &job(service, JobKind::TextSave))
        .unwrap()
}

/// The question classes of item `i`.
fn questions(plan: &SavePlan, i: usize) -> Vec<SaveQuestion> {
    plan.items[i].questions()
}

/// Sets the modification time of `path` `secs` seconds after the epoch.
fn set_mtime(path: &Path, secs: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// Writes `options.toml` of the service's configuration folder.
fn options(root: &Path, text: &str) {
    write(root, "config/options.toml", text.as_bytes());
}

// ---- ChangedOnDisk and source_touched ----------------------------------------------------------------------------

#[test]
fn a10_15_changed_on_disk_is_asked_and_a_touched_source_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"one\ntwo\n");
    let right = write(&w, "r.txt", b"one\ntwo\n");
    set_mtime(&right, 1_700_000_000);

    // Same bytes, a new time: saved without a question, warning `source_touched`.
    let mut view = open(&service, &left, &right, None);
    type_at(&mut view.docs.right, 1, "X");
    set_mtime(&right, 1_700_000_100);
    let plan = plan_one(
        &service,
        Side::Right,
        &view.docs.right,
        &in_place(UnmappablePolicy::Ask),
    );
    assert!(questions(&plan, 0).is_empty(), "{:?}", plan.items[0].issues);
    assert!(plan.confirmed);
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.code == WarningCode::SourceTouched),
        "{:?}",
        plan.warnings
    );
    let report = execute(&service, plan, &[&view.docs.right]);
    assert_eq!(save_exit_code(&report), ExitCode::Success);
    assert_eq!(read(&right), b"one\nXtwo\n");
    assert_eq!(apply_save_report(&report, &mut [&mut view.docs.right]), 1);
    assert!(!view.docs.right.is_modified());
    assert_eq!(view.docs.right.mark(LineIdx(1)), LineMark::SavedEdit);

    // Same size, other bytes (the time kept): `ChangedOnDisk`, decided by the content.
    let mut view = open(&service, &left, &right, None);
    let stamp_time = std::fs::metadata(&right).unwrap().modified().unwrap();
    type_at(&mut view.docs.right, 0, "Y");
    std::fs::write(&right, b"ONE\nXtwo\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&right)
        .unwrap()
        .set_modified(stamp_time)
        .unwrap();
    let req = in_place(UnmappablePolicy::Ask);
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    assert_eq!(questions(&plan, 0), vec![SaveQuestion::ChangedOnDisk]);
    // No: declined, the file as it stands.
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    assert_eq!(driver.next().unwrap().kind, SaveQuestion::ChangedOnDisk);
    driver.reply(Reply::No);
    assert!(driver.next().is_none());
    let plan = driver.finish(plan).unwrap();
    let report = execute(&service, plan, &[&view.docs.right]);
    assert_eq!(report.status(0), Some(ItemStatus::Skipped));
    assert_eq!(report.op.results[0].skip, Some(SkipReason::Declined));
    assert_eq!(save_exit_code(&report), ExitCode::Success);
    assert_eq!(read(&right), b"ONE\nXtwo\n");
    assert_eq!(apply_save_report(&report, &mut [&mut view.docs.right]), 0);
    assert!(view.docs.right.is_modified());
    // Yes: the text as edited, encoded whole (`save_reencoded`, source_changed).
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    driver.reply(Reply::Yes);
    let plan = driver.finish(plan).unwrap();
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.code == WarningCode::SaveReencoded && w.message.contains("source_changed")),
        "{:?}",
        plan.warnings
    );
    let report = execute(&service, plan, &[&view.docs.right]);
    assert_eq!(save_exit_code(&report), ExitCode::Success);
    assert_eq!(read(&right), b"Yone\nXtwo\n");
}

// ---- ReadOnlyTarget ----------------------------------------------------------------------------------------------

/// Makes `path` read-only (DOS `R` on Windows, mode 0o444 on Unix).
fn set_read_only(path: &Path, on: bool) {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_readonly(on);
    std::fs::set_permissions(path, perms).unwrap();
}

/// True when this process writes read-only files anyway (root).
fn writes_anyway(path: &Path) -> bool {
    std::fs::OpenOptions::new().append(true).open(path).is_ok()
}

#[test]
fn a10_15_a_read_only_target_is_asked_unless_the_policy_drops_it() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"a\n");
    let right = write(&w, "r.txt", b"b\n");
    let mut view = open(&service, &left, &right, None);
    type_at(&mut view.docs.right, 0, "X");
    set_read_only(&right, true);
    if writes_anyway(&right) && !cfg!(windows) {
        eprintln!("skipped: this process writes read-only files (root)");
        set_read_only(&right, false);
        return;
    }
    let req = in_place(UnmappablePolicy::Ask);
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    assert_eq!(questions(&plan, 0), vec![SaveQuestion::ReadOnlyTarget]);
    // No.
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    assert_eq!(driver.next().unwrap().kind, SaveQuestion::ReadOnlyTarget);
    driver.reply(Reply::No);
    let report = execute(&service, driver.finish(plan).unwrap(), &[&view.docs.right]);
    assert_eq!(report.op.results[0].skip, Some(SkipReason::Declined));
    assert_eq!(read(&right), b"b\n");
    // `--yes` answers it: the file is written and stays read-only.
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    let driver = service.save_confirmation(
        &plan,
        SaveAnswers {
            yes: true,
            ..SaveAnswers::default()
        },
    );
    assert!(driver.next().is_none());
    let report = execute(&service, driver.finish(plan).unwrap(), &[&view.docs.right]);
    assert_eq!(
        save_exit_code(&report),
        ExitCode::Success,
        "{:?}",
        report.op.results
    );
    assert_eq!(read(&right), b"Xb\n");
    if cfg!(unix) {
        assert!(std::fs::metadata(&right).unwrap().permissions().readonly());
    }
    // `[file_operations] confirm_read_only = false` drops the question.
    options(tmp.path(), "[file_operations]\nconfirm_read_only = false\n");
    type_at(&mut view.docs.right, 0, "Y");
    apply_save_report(&report, &mut [&mut view.docs.right]);
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    let driver = service.save_confirmation(&plan, SaveAnswers::default());
    assert!(driver.next().is_none());
    assert!(driver.open_questions().is_empty());
    set_read_only(&right, false);
}

// ---- Unmappable characters ---------------------------------------------------------------------------------------

/// A legacy file: its text in `encoding`, loaded with that encoding; a character it cannot write typed into line 0.
fn unmappable_case(
    tmp: &Path,
    service: &Service,
    encoding: TextEncoding,
    text: &str,
) -> (std::path::PathBuf, LiveTextView) {
    let w = tmp.join("work");
    let (bytes, unmappable) = encode(text, encoding);
    assert!(!unmappable);
    let left = write(&w, "l.txt", b"x\n");
    let right = write(&w, &format!("{}.txt", encoding.name()), &bytes);
    let mut view = open(service, &left, &right, Some(encoding.name()));
    assert_eq!(view.docs.right.encoding().0.name(), encoding.name());
    type_at(&mut view.docs.right, 0, "\u{1F600}");
    (right, view)
}

#[test]
fn a10_15_unmappable_characters_ask_refuse_or_become_question_marks() {
    for (encoding, text) in [
        ("windows-1252", "caf\u{e9}\nna\u{ef}ve\n"),
        ("Shift_JIS", "\u{65e5}\u{672c}\n\u{8a9e}\n"),
    ] {
        let encoding = TextEncoding::for_label(encoding).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let service = service(tmp.path());
        let (path, view) = unmappable_case(tmp.path(), &service, encoding, text);
        let before = read(&path);
        let (expected, _) = encode(&format!("?{text}"), encoding);
        let doc = &view.docs.right;

        // Ask: the question; No declines, Yes writes '?' with `unmappable_replaced`.
        let plan = plan_one(&service, Side::Right, doc, &in_place(UnmappablePolicy::Ask));
        assert_eq!(
            questions(&plan, 0),
            vec![SaveQuestion::Unmappable],
            "{}",
            encoding.name()
        );
        let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
        let ask = driver.next().unwrap();
        assert!(ask.text.contains("U+1F600"), "{}", ask.text);
        driver.reply(Reply::No);
        let report = execute(&service, driver.finish(plan).unwrap(), &[doc]);
        assert_eq!(report.op.results[0].skip, Some(SkipReason::Declined));
        assert_eq!(read(&path), before);

        // Refuse: the item fails, naming the character; the file stays.
        let plan = plan_one(
            &service,
            Side::Right,
            doc,
            &in_place(UnmappablePolicy::Refuse),
        );
        assert!(questions(&plan, 0).is_empty());
        let error = plan.plan.items[0].error.clone().expect("refused");
        assert!(error.message.contains("U+1F600"), "{}", error.message);
        let report = execute(&service, plan, &[doc]);
        assert_eq!(save_exit_code(&report), ExitCode::UnknownError);
        assert_eq!(report.status(0), Some(ItemStatus::Failed));
        assert_eq!(read(&path), before);

        // Replace: no question, '?' and the warning; never a numeric reference.
        let plan = plan_one(
            &service,
            Side::Right,
            doc,
            &in_place(UnmappablePolicy::Replace),
        );
        assert!(questions(&plan, 0).is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.code == WarningCode::UnmappableReplaced)
        );
        let report = execute(&service, plan, &[doc]);
        assert_eq!(save_exit_code(&report), ExitCode::Success);
        let written = read(&path);
        assert_eq!(written, expected, "{}", encoding.name());
        assert!(!written.windows(2).any(|w| w == b"&#"));

        // Ask answered Yes, from the file as it stood.
        std::fs::write(&path, &before).unwrap();
        let (_, view) = unmappable_case(tmp.path(), &service, encoding, text);
        let doc = &view.docs.right;
        let plan = plan_one(&service, Side::Right, doc, &in_place(UnmappablePolicy::Ask));
        let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
        driver.reply(Reply::Yes);
        let plan = driver.finish(plan).unwrap();
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.code == WarningCode::UnmappableReplaced)
        );
        execute(&service, plan, &[doc]);
        assert_eq!(read(&path), expected);
        // `--unmappable replace` answers the question without a person.
        std::fs::write(&path, &before).unwrap();
        let (_, view) = unmappable_case(tmp.path(), &service, encoding, text);
        let plan = plan_one(
            &service,
            Side::Right,
            &view.docs.right,
            &in_place(UnmappablePolicy::Ask),
        );
        let driver = service.save_confirmation(
            &plan,
            SaveAnswers {
                unmappable_replace: true,
                ..SaveAnswers::default()
            },
        );
        assert!(driver.next().is_none());
    }
}

// ---- LossyRewritten ----------------------------------------------------------------------------------------------

#[test]
fn a10_15_an_edited_lossy_line_is_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"x\n");
    let right = write(&w, "r.txt", b"ok\nb\xffad\nend\n");
    let mut view = open(&service, &left, &right, Some("UTF-8"));
    // An edit elsewhere: the malformed line is spliced as it was, no question.
    append_at(&mut view.docs.right, 0, "!");
    let req = in_place(UnmappablePolicy::Ask);
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    assert!(questions(&plan, 0).is_empty(), "{:?}", plan.items[0].issues);
    // An edit of the malformed line asks.
    append_at(&mut view.docs.right, 1, "!");
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    assert_eq!(questions(&plan, 0), vec![SaveQuestion::LossyRewritten]);
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    driver.reply(Reply::YesToAll);
    let plan = driver.finish(plan).unwrap();
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.code == WarningCode::LossyLineRewritten)
    );
    execute(&service, plan, &[&view.docs.right]);
    assert_eq!(read(&right), b"ok!\nb\xef\xbf\xbdad!\nend\n");
}

// ---- ReplaceExisting and OverwriteBackup -------------------------------------------------------------------------

#[test]
fn a10_15_save_as_onto_a_file_asks_and_so_does_a_backup_over_a_stranger() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"l\n");
    let right = write(&w, "r.txt", b"r\n");
    let other = write(&w, "other.txt", b"other\n");
    write(&w, "other.txt.bak", b"a stranger\n");
    let mut view = open(&service, &left, &right, None);
    type_at(&mut view.docs.right, 0, "X");
    let req = SaveRequest {
        target: Some(service.parse_spec(other.as_os_str()).unwrap()),
        backup: service.backup_policy_for_save(false).unwrap(),
        unmappable: UnmappablePolicy::Ask,
        conversion: None,
        transform: None,
    };
    let plan = plan_one(&service, Side::Right, &view.docs.right, &req);
    assert_eq!(
        questions(&plan, 0),
        vec![SaveQuestion::ReplaceExisting, SaveQuestion::OverwriteBackup]
    );
    // Cancel: nothing is written.
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    assert_eq!(driver.next().unwrap().kind, SaveQuestion::ReplaceExisting);
    assert_eq!(driver.next().unwrap().remaining, 2);
    driver.reply(Reply::Cancel);
    assert!(driver.finish(plan.clone()).is_err());
    assert_eq!(read(&other), b"other\n");
    // Yes, Yes: replaced, its backup holds the replaced file.
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    driver.reply(Reply::Yes);
    assert_eq!(driver.next().unwrap().kind, SaveQuestion::OverwriteBackup);
    driver.reply(Reply::Yes);
    let report = execute(&service, driver.finish(plan).unwrap(), &[&view.docs.right]);
    assert_eq!(
        save_exit_code(&report),
        ExitCode::Success,
        "{:?}",
        report.op.results
    );
    assert_eq!(read(&other), b"Xr\n");
    assert_eq!(read(&w.join("other.txt.bak")), b"other\n");
    assert_eq!(read(&right), b"r\n");
    // Save As moved the document: its next save goes there without a question about replacing.
    apply_save_report(&report, &mut [&mut view.docs.right]);
    assert_eq!(
        view.docs.right.location().map(hikasa_core::Spec::canonical),
        Some(service.parse_spec(other.as_os_str()).unwrap().canonical())
    );
    type_at(&mut view.docs.right, 0, "Z");
    let plan = plan_one(
        &service,
        Side::Right,
        &view.docs.right,
        &in_place(UnmappablePolicy::Ask),
    );
    assert!(questions(&plan, 0).is_empty(), "{:?}", plan.items[0].issues);
}

// ---- YesToAll, NoToAll and the flags -----------------------------------------------------------------------------

#[test]
fn a10_15_yes_to_all_answers_a_class_and_the_flags_answer_theirs() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"one\n");
    let right = write(&w, "r.txt", b"two\n");
    let mut view = open(&service, &left, &right, None);
    type_at(&mut view.docs.left, 0, "L");
    type_at(&mut view.docs.right, 0, "R");
    std::fs::write(&left, b"ONE\n").unwrap();
    std::fs::write(&right, b"TWO\n").unwrap();
    let req = in_place(UnmappablePolicy::Ask);
    let docs = [
        (Some(Side::Left), &view.docs.left, &req),
        (Some(Side::Right), &view.docs.right, &req),
    ];
    let plan = service
        .text_save_plan(&docs, &job(&service, JobKind::TextSave))
        .unwrap();
    assert_eq!(plan.questions(), 2);

    // Open questions without a person: refused with their counts and the flag that answers them.
    let driver = service.save_confirmation(
        &plan,
        SaveAnswers {
            yes: true,
            ..SaveAnswers::default()
        },
    );
    let refusal = driver.refusal().expect("open questions").to_string();
    assert!(refusal.contains("2 questions need an answer"), "{refusal}");
    assert!(refusal.contains("--force"), "{refusal}");
    assert!(refusal.contains("changed_on_disk 2"), "{refusal}");
    // `--force` answers them.
    let driver = service.save_confirmation(
        &plan,
        SaveAnswers {
            force: true,
            ..SaveAnswers::default()
        },
    );
    assert!(driver.next().is_none() && driver.refusal().is_none());
    // No to all: every asking item declined.
    let driver = service.save_confirmation(
        &plan,
        SaveAnswers {
            no_to_all: true,
            ..SaveAnswers::default()
        },
    );
    let declined = driver.finish(plan.clone()).unwrap();
    assert!(
        declined
            .plan
            .items
            .iter()
            .all(|i| i.skip == Some(SkipReason::Declined))
    );
    // YesToAll at the first question answers the second.
    let mut driver = service.save_confirmation(&plan, SaveAnswers::default());
    assert_eq!(driver.next().unwrap().remaining, 2);
    driver.reply(Reply::YesToAll);
    assert!(driver.next().is_none());
    let plan = driver.finish(plan).unwrap();
    let report = execute(&service, plan, &[&view.docs.left, &view.docs.right]);
    assert_eq!(save_exit_code(&report), ExitCode::Success);
    assert_eq!(read(&left), b"Lone\n");
    assert_eq!(read(&right), b"Rtwo\n");
    assert_eq!(
        apply_save_report(&report, &mut [&mut view.docs.left, &mut view.docs.right]),
        2
    );
}

// ---- the live view's stamps and the exit codes -------------------------------------------------------------------

#[test]
fn a10_15_the_live_view_carries_the_load_stamps() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"left\n");
    let right = write(&w, "r.txt", b"right side\n");
    let view = open(&service, &left, &right, None);
    let stamps = view.stamps;
    assert_eq!(stamps.left.unwrap().size, 5);
    assert_eq!(stamps.right.unwrap().size, 11);
    assert_eq!(
        stamps.right.unwrap().crc32,
        view.docs.right.splice_source().unwrap().stamp.crc32
    );
    assert_ne!(stamps.left.unwrap().crc32, stamps.right.unwrap().crc32);
}

#[test]
fn a10_15_a_failed_item_is_exit_100_and_leaves_its_document_modified() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let left = write(&w, "l.txt", b"l\n");
    let right = write(&w, "r.txt", b"r\n");
    let mut view = open(&service, &left, &right, None);
    type_at(&mut view.docs.left, 0, "A");
    type_at(&mut view.docs.right, 0, "B");
    let req = in_place(UnmappablePolicy::Ask);
    let docs = [
        (Some(Side::Left), &view.docs.left, &req),
        (Some(Side::Right), &view.docs.right, &req),
    ];
    let plan = service
        .text_save_plan(&docs, &job(&service, JobKind::TextSave))
        .unwrap();
    // The right file changes between plan and execution: its item fails, the left one is written.
    std::fs::write(&right, b"changed\n").unwrap();
    let report = execute(&service, plan, &[&view.docs.left, &view.docs.right]);
    assert_eq!(report.status(0), Some(ItemStatus::Done));
    assert_eq!(report.status(1), Some(ItemStatus::Failed));
    assert_eq!(save_exit_code(&report), ExitCode::UnknownError);
    assert_eq!(
        apply_save_report(&report, &mut [&mut view.docs.left, &mut view.docs.right]),
        1
    );
    assert!(!view.docs.left.is_modified());
    assert!(view.docs.right.is_modified());
    assert_eq!(read(&right), b"changed\n");
}
