//! Table Compare phase 3 in the Service (m2-table-contract §6.6, T-52…T-54): the `OpenView` of `table compare --gui`
//! (`GuiOpenRequest::for_table`, `OpenOptions::table`) and its inverse `Service::open_view_request`, the local checks
//! of `check_open`, the view kind of two tables (CLI-002), and the session of a Table Compare view
//! (`session_for_view`, `session_view_of`).

mod common;

use common::{os, service, write};
use hikasa_core::{Pair, Spec};
use hikasa_session::api::{
    GuiOpenRequest, SheetRuleWords, TableCompareRequest, TableFormatWords, TableRuleWords,
    ViewRequest, infer_view_kind,
};
use hikasa_session::ipc::{OpenOptions, ViewKind};
use hikasa_session::sessions::{SessionRules, SessionType};
use hikasa_session::{ApiError, Service};
use hikasa_table::RowFilter;
use toml::Table;

fn words() -> (TableFormatWords, TableRuleWords) {
    (
        TableFormatWords {
            delimiters: vec![";".into()],
            decimal: Some(",".into()),
            ..TableFormatWords::default()
        },
        TableRuleWords {
            sheets: Some("position".into()),
            rules: SheetRuleWords {
                keys: vec!["id".into()],
                tolerance: vec!["v=0.5".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
    )
}

fn table_request(service: &Service, w: &std::path::Path) -> TableCompareRequest {
    let _ = service;
    let a = write(w, "a.csv", b"id;v\n1;1,5\n2;7\n");
    let b = write(w, "b.csv", b"id;v\n2;7\n1;1,75\n");
    let (parse, rules) = words();
    TableCompareRequest {
        left: os(&a),
        right: os(&b),
        formats: Pair::new(None, Some("Comma Separated Values".into())),
        parse,
        rules,
        ignore_unimportant: true,
        display: Some("differences".into()),
        ..TableCompareRequest::default()
    }
}

/// T-52: `table compare --gui` sends the words; the GUI's `open_view_request` gives back the same request, with the
/// grid's filter of `--display` and `--ignore-unimportant`; the view runs as the command line would.
#[test]
fn t52_the_open_view_carries_the_words() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let req = table_request(&service, &w);
    let open = GuiOpenRequest::for_table(&req, true);
    assert_eq!(open.kind, Some(ViewKind::TableCompare));
    assert!(open.wait);
    let resolved = service.resolve_open(&open).unwrap();
    assert_eq!(resolved.kind, ViewKind::TableCompare);
    service.check_open(&resolved).unwrap();
    let ViewRequest::Table { request, filter } = service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .unwrap()
    else {
        panic!("not a table view");
    };
    let (parse, rules) = words();
    assert_eq!(request.parse, parse);
    assert_eq!(request.rules, rules);
    assert_eq!(
        request.formats,
        Pair::new(None, Some("Comma Separated Values".to_owned()))
    );
    assert!(request.ignore_unimportant);
    assert_eq!(
        filter,
        RowFilter {
            ignore_unimportant: true,
            ..RowFilter::DIFFERENCES
        }
    );
    let job = common::job(&service, hikasa_core::JobKind::TableCompare);
    let view = service.table_compare(&request, &job).unwrap();
    let local = service.table_compare(&req, &job).unwrap();
    assert_eq!(view.kind, local.kind);
    assert_eq!(view.exit_code, local.exit_code);
    // Without words and display the grid shows every row.
    let bare = OpenOptions::default();
    let ViewRequest::Table { request, filter } = service
        .open_view_request(ViewKind::TableCompare, &resolved.specs, &bare)
        .unwrap()
    else {
        panic!("not a table view");
    };
    assert_eq!(filter, RowFilter::ALL);
    assert_eq!(request.rules, TableRuleWords::default());
}

/// T-41 through T-52: a bad word, an unknown format or a format of another kind is refused locally, before the GUI is
/// contacted; so is a display word the grid does not know.
#[test]
fn t52_check_open_refuses_what_table_compare_refuses() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let good = table_request(&service, &w);
    let cases: Vec<(TableCompareRequest, &str)> = vec![
        (
            TableCompareRequest {
                rules: TableRuleWords {
                    sheets: Some("sideways".into()),
                    ..TableRuleWords::default()
                },
                ..good.clone()
            },
            "sideways",
        ),
        (
            TableCompareRequest {
                parse: TableFormatWords {
                    kind: Some("xml".into()),
                    ..TableFormatWords::default()
                },
                ..good.clone()
            },
            "xml",
        ),
        (
            TableCompareRequest {
                display: Some("some".into()),
                ..good.clone()
            },
            "some",
        ),
        (
            TableCompareRequest {
                format: Some("c-cpp".into()),
                formats: Pair::default(),
                ..good.clone()
            },
            "table kind",
        ),
        (
            TableCompareRequest {
                format: Some("no such format".into()),
                formats: Pair::default(),
                ..good.clone()
            },
            "no such format",
        ),
    ];
    for (req, word) in cases {
        let resolved = service
            .resolve_open(&GuiOpenRequest::for_table(&req, false))
            .unwrap();
        let err = service.check_open(&resolved).unwrap_err();
        assert!(err.to_string().contains(word), "{word}: {err}");
        assert_eq!(
            err.exit_code(hikasa_session::api::ApiOp::GuiOpen { wait: false }),
            hikasa_core::ExitCode::UnknownError
        );
    }
}

/// T-54 (CLI-002): two files of a Table format open in Table Compare, other files in Text Compare, folders in
/// Folder Compare; a forced kind wins.
#[test]
fn t54_two_tables_open_in_table_compare() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let spec = |name: &str, body: &[u8]| {
        let path = write(&w, name, body);
        service.parse_spec(path.as_os_str()).unwrap()
    };
    let csv = [spec("a.csv", b"x\n1\n"), spec("b.csv", b"x\n2\n")];
    let mixed = [spec("c.csv", b"x\n1\n"), spec("d.xlsx", b"PK")];
    let text = [spec("a.c", b"int a;\n"), spec("b.c", b"int b;\n")];
    assert_eq!(
        infer_view_kind(&service, &csv, None).unwrap(),
        ViewKind::TableCompare
    );
    assert_eq!(
        infer_view_kind(&service, &mixed, None).unwrap(),
        ViewKind::TableCompare
    );
    assert_eq!(
        infer_view_kind(&service, &text, None).unwrap(),
        ViewKind::TextCompare
    );
    assert_eq!(
        infer_view_kind(&service, &csv, Some(ViewKind::TextCompare)).unwrap(),
        ViewKind::TextCompare
    );
    std::fs::create_dir_all(w.join("l")).unwrap();
    std::fs::create_dir_all(w.join("r")).unwrap();
    let dirs: [Spec; 2] = [
        service.parse_spec(w.join("l").as_os_str()).unwrap(),
        service.parse_spec(w.join("r").as_os_str()).unwrap(),
    ];
    assert_eq!(
        infer_view_kind(&service, &dirs, None).unwrap(),
        ViewKind::FolderCompare
    );
    assert!(ViewKind::TableCompare.is_served());
}

/// T-53: the session of a Table Compare view keeps its words and formats (`[rules]`, `[rules.parse]`, `[specs]
/// left_format`), and the view of that session runs the same request, its display from `[display] filter`.
#[test]
fn t53_the_session_of_a_table_view_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let req = table_request(&service, &w);
    let view = ViewRequest::Table {
        request: req.clone(),
        filter: RowFilter::ALL,
    };
    let mut display = Table::new();
    display.insert("filter".into(), toml::Value::String("same".into()));
    let mut layout = Table::new();
    layout.insert("sheet".into(), toml::Value::Integer(1));
    let (session, warnings) = service
        .session_for_view(&view, layout.clone(), display, None)
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(session.kind, SessionType::TableCompare);
    assert_eq!(session.specs.right_format, "Comma Separated Values");
    assert_eq!(session.specs.left_format, "");
    assert_eq!(session.view, layout);
    let SessionRules::Table(rules) = &session.rules else {
        panic!("{:?}", session.rules);
    };
    let (parse, words) = words();
    assert_eq!(rules.parse, parse);
    assert_eq!(rules.rules, words);
    let opened = service.session_view_of(&session, "t".into()).unwrap();
    assert_eq!(opened.kind, ViewKind::TableCompare);
    let ViewRequest::Table { request, filter } = opened.request else {
        panic!("not a table view");
    };
    assert_eq!(request.parse, parse);
    assert_eq!(request.rules, words);
    assert_eq!(request.formats, req.formats);
    assert_eq!(filter, RowFilter::SAME);
    // A session saved and read again opens the same view.
    let reference = hikasa_session::sessions::SessionRef::parse("tables/view").unwrap();
    service.session_save(&reference, &session, false).unwrap();
    let loaded = service.session_load(&reference).unwrap();
    let again = service.session_view_of(&loaded.value, "t".into());
    let again = match again {
        Ok(v) => v,
        Err(ApiError::Unsupported(m)) => panic!("{m}"),
        Err(e) => panic!("{e}"),
    };
    let ViewRequest::Table { request, .. } = again.request else {
        panic!("not a table view");
    };
    assert_eq!(request.rules, words);
}

/// T-53: a view without words takes the `table-compare` defaults' words; a view with words (here parser words only)
/// keeps them as typed, without the defaults' rules.
#[test]
fn t53_defaults_reach_only_views_without_words() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id;v\n1;2\n");
    let b = write(&w, "b.csv", b"id;v\n1;3\n");
    let mut defaults = service.session_defaults(SessionType::TableCompare).unwrap();
    if let SessionRules::Table(t) = &mut defaults.rules {
        t.rules.rules.keys = vec!["id".into()];
        t.parse.delimiters = vec![";".into()];
    }
    service.session_save_defaults(&defaults).unwrap();
    let specs = vec![a.display().to_string(), b.display().to_string()];
    let ViewRequest::Table { request, .. } = service
        .open_view_request(ViewKind::TableCompare, &specs, &OpenOptions::default())
        .unwrap()
    else {
        panic!("not a table view");
    };
    assert_eq!(request.rules.rules.keys, ["id"]);
    assert_eq!(request.parse.delimiters, [";"]);
    let typed = OpenOptions {
        table: Some(hikasa_session::ipc::TableWords {
            parse: TableFormatWords {
                delimiters: vec![",".into()],
                ..TableFormatWords::default()
            },
            ..hikasa_session::ipc::TableWords::default()
        }),
        ..OpenOptions::default()
    };
    let ViewRequest::Table { request, .. } = service
        .open_view_request(ViewKind::TableCompare, &specs, &typed)
        .unwrap()
    else {
        panic!("not a table view");
    };
    assert!(request.rules.rules.keys.is_empty(), "{:?}", request.rules);
    assert_eq!(request.parse.delimiters, [","]);
}

/// The `gui open` request of two files with `options`, resolved.
fn resolved(
    service: &Service,
    specs: [&std::path::Path; 2],
    options: OpenOptions,
) -> hikasa_session::api::ResolvedOpen {
    let open = GuiOpenRequest {
        specs: specs.iter().map(|p| p.as_os_str().to_owned()).collect(),
        options,
        ..GuiOpenRequest::default()
    };
    service.resolve_open(&open).unwrap()
}

/// T-60 (phase-3 review F4): `table compare a b --gui` without words takes none of the defaults' words, so the view
/// runs what the command line runs; a view opened without `OpenOptions::table` still takes them.
#[test]
fn t60_gui_without_words_runs_what_the_command_line_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v\n1,2\n2,3\n");
    let b = write(&w, "b.csv", b"id,v\n2,3\n1,2\n");
    let mut defaults = service.session_defaults(SessionType::TableCompare).unwrap();
    if let SessionRules::Table(t) = &mut defaults.rules {
        t.rules.rules.keys = vec!["id".into()];
    }
    service.session_save_defaults(&defaults).unwrap();
    let req = TableCompareRequest {
        left: os(&a),
        right: os(&b),
        ..TableCompareRequest::default()
    };
    let open = GuiOpenRequest::for_table(&req, true);
    let r = service.resolve_open(&open).unwrap();
    let ViewRequest::Table { request, .. } = service
        .open_view_request(r.kind, &r.specs, &r.options)
        .unwrap()
    else {
        panic!("not a table view");
    };
    assert!(request.rules.rules.keys.is_empty(), "{:?}", request.rules);
    // The verdict of the view's request is the command line's (unkeyed: the rows move, so they differ).
    let run = |req: &TableCompareRequest| {
        let req = req.clone();
        service
            .spawn(
                hikasa_core::JobKind::TableCompare,
                "t".into(),
                move |svc, job| svc.table_compare(&req, job),
            )
            .wait()
            .unwrap()
            .source()
            .verdict(false)
    };
    assert_eq!(run(&request), run(&req));
    let ViewRequest::Table { request, .. } = service
        .open_view_request(ViewKind::TableCompare, &r.specs, &OpenOptions::default())
        .unwrap()
    else {
        panic!("not a table view");
    };
    assert_eq!(request.rules.rules.keys, ["id"]);
}

/// T-59 (phase-3 review F8): one encoding for both sides of two tables is the parser's encoding; another option of
/// Text Compare opens the pair in Text Compare with a notice saying why.
#[test]
fn t59_text_options_on_two_tables() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v\n1,2\n");
    let b = write(&w, "b.csv", b"id,v\n1,3\n");
    let both = OpenOptions {
        encodings: [Some("windows-1252".into()), Some("windows-1252".into())],
        ..OpenOptions::default()
    };
    let r = resolved(&service, [&a, &b], both);
    assert_eq!(r.kind, ViewKind::TableCompare);
    assert!(r.options.notice.is_none());
    service.check_open(&r).unwrap();
    let ViewRequest::Table { request, .. } = service
        .open_view_request(r.kind, &r.specs, &r.options)
        .unwrap()
    else {
        panic!("not a table view");
    };
    assert_eq!(request.parse.encoding.as_deref(), Some("windows-1252"));
    let text_options: Vec<(&str, OpenOptions)> = vec![
        (
            "an encoding of one side",
            OpenOptions {
                encodings: [Some("utf-8".into()), None],
                ..OpenOptions::default()
            },
        ),
        (
            "an encoding of one side",
            OpenOptions {
                encodings: [Some("utf-8".into()), Some("windows-1252".into())],
                ..OpenOptions::default()
            },
        ),
        (
            "a text rule",
            OpenOptions {
                text_rules: Some(hikasa_text::TextRulesPatch {
                    skew_tolerance: Some(10),
                    ..Default::default()
                }),
                ..OpenOptions::default()
            },
        ),
        (
            "`--context`",
            OpenOptions {
                context_lines: Some(3),
                ..OpenOptions::default()
            },
        ),
        (
            "a Text Compare display word",
            OpenOptions {
                display: Some("context".into()),
                ..OpenOptions::default()
            },
        ),
    ];
    for (why, options) in text_options {
        let r = resolved(&service, [&a, &b], options);
        assert_eq!(r.kind, ViewKind::TextCompare, "{why}");
        let notice = r.options.notice.clone().unwrap_or_default();
        assert!(notice.contains(why), "{why}: {notice}");
        service.check_open(&r).unwrap();
    }
    let rules = write(&w, "rules.toml", b"");
    let open = GuiOpenRequest {
        specs: vec![os(&a), os(&b)],
        text_rules_file: Some(os(&rules)),
        ..GuiOpenRequest::default()
    };
    let r = service.resolve_open(&open).unwrap();
    assert_eq!(r.kind, ViewKind::TextCompare);
    assert!(
        r.options
            .notice
            .unwrap_or_default()
            .contains("a text rules file")
    );
    // A table display word keeps Table Compare; a forced kind is never changed.
    let r = resolved(
        &service,
        [&a, &b],
        OpenOptions {
            display: Some("same".into()),
            ..OpenOptions::default()
        },
    );
    assert_eq!(r.kind, ViewKind::TableCompare);
    let open = GuiOpenRequest {
        specs: vec![os(&a), os(&b)],
        kind: Some(ViewKind::TableCompare),
        options: OpenOptions {
            context_lines: Some(3),
            ..OpenOptions::default()
        },
        ..GuiOpenRequest::default()
    };
    assert_eq!(
        service.resolve_open(&open).unwrap().kind,
        ViewKind::TableCompare
    );
}

/// Phase-3 review F9, R23: `check_table_words` refuses `--format` with a side's own format, as `table compare` does,
/// and an unknown `--cells` word.
#[test]
fn f9_check_table_words_is_the_command_lines_check() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let mut req = table_request(&service, &w);
    service.check_table_words(&req).unwrap();
    req.format = Some("csv".into());
    let checked = service.check_table_words(&req).unwrap_err().to_string();
    assert!(checked.contains("--left-format"), "{checked}");
    let r2 = req.clone();
    let direct = service
        .spawn(
            hikasa_core::JobKind::TableCompare,
            "t".into(),
            move |svc, job| svc.table_compare(&r2, job).map(|_| ()),
        )
        .wait()
        .unwrap_err()
        .to_string();
    assert_eq!(checked, direct);
    let mut req = table_request(&service, &w);
    req.cells = Some("sideways".into());
    assert!(service.check_table_words(&req).is_err());
}

/// Phase-3 review R22: the session of a table view keeps the left side's own format.
#[test]
fn r22_the_session_keeps_the_left_format() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let mut req = table_request(&service, &w);
    req.formats = Pair::new(Some("csv".into()), None);
    let view = ViewRequest::Table {
        request: req,
        filter: RowFilter::ALL,
    };
    let (session, _) = service
        .session_for_view(&view, Table::new(), Table::new(), None)
        .unwrap();
    assert_eq!(session.specs.left_format, "csv");
    let ViewRequest::Table { request, .. } = service
        .session_view_of(&session, "t".into())
        .unwrap()
        .request
    else {
        panic!("not a table view");
    };
    assert_eq!(request.formats.left.as_deref(), Some("csv"));
}

/// Phase-3 review R24, F13: a pair is a table pair by the left name first, and under a folder's FF-002 overrides.
#[test]
fn r24_table_pairs_by_the_left_name_and_the_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let os = std::ffi::OsStr::new;
    assert!(service.is_table_pair(os("/w/a.csv"), os("/w/b.txt")));
    assert!(!service.is_table_pair(os("/w/b.txt"), os("/w/a.csv")));
    assert!(service.is_table_pair(os("/w/a.csv"), os("/w/b.csv")));
    let mut off = std::collections::BTreeMap::new();
    off.insert("csv".to_owned(), false);
    assert!(!service.is_table_pair_with(os("/w/a.csv"), os("/w/b.csv"), &off));
    assert!(service.is_table_pair_with(
        os("/w/a.csv"),
        os("/w/b.csv"),
        &std::collections::BTreeMap::new()
    ));
}
