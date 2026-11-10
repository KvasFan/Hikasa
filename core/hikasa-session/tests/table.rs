//! Table Compare phase 2 in the Service (m2-table-contract §6.4, §7 AT.20–AT.22): `Service::table_compare` on CSV and
//! generated xlsx pairs with every exit code and the errors of exit 100, a workbook inside a zip read through the VFS,
//! the folder content test of a tree of CSV and xlsx files under the Rules method (T-21), `table-compare` sessions
//! and `session run`, the Spreadsheet text conversion (T-22) and the regional settings of the locale (A16).

mod common;

use std::io::Write as _;
use std::path::Path;

use common::{facts, fixed_clock, job, os, service, write};
use hikasa_core::{ContentKind, ContentResult, ErrorKind, ExitCode, JobKind, Pair};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, NullObserver};
use hikasa_session::api::{
    FolderCompareRequest, SessionRunRequest, SheetRuleWords, SheetRulesWords, TableCompareRequest,
    TableFormatWords, TableRuleWords, TextCompareRequest,
};
use hikasa_session::sessions::{Session, SessionRef, SessionRules, SessionType};
use hikasa_session::{ApiError, Service, ServiceInit};
use rust_xlsxwriter::{DocProperties, ExcelDateTime, Format, Workbook};

/// A cell of a generated workbook.
enum C<'a> {
    N(f64),
    S(&'a str),
    D(u16, u8, u8),
    B(bool),
}

/// An xlsx of `sheets` (name, rows), with a fixed creation time so the bytes do not depend on the clock.
fn xlsx(sheets: &[(&str, Vec<Vec<C<'_>>>)]) -> Vec<u8> {
    let mut book = Workbook::new();
    let created = ExcelDateTime::from_ymd(2026, 1, 1).unwrap();
    book.set_properties(&DocProperties::new().set_creation_datetime(&created));
    let date = Format::new().set_num_format("yyyy-mm-dd");
    for (name, rows) in sheets {
        let sheet = book.add_worksheet();
        sheet.set_name(*name).unwrap();
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let (r, c) = (r as u32, c as u16);
                match cell {
                    C::N(n) => sheet.write_number(r, c, *n).map(drop),
                    C::S(s) => sheet.write_string(r, c, *s).map(drop),
                    C::D(y, m, d) => sheet
                        .write_datetime_with_format(
                            r,
                            c,
                            ExcelDateTime::from_ymd(*y, *m, *d).unwrap(),
                            &date,
                        )
                        .map(drop),
                    C::B(b) => sheet.write_boolean(r, c, *b).map(drop),
                }
                .unwrap();
            }
        }
    }
    book.save_to_buffer().unwrap()
}

/// The price list as a workbook: `id, name, price, date, ok` and three rows.
fn prices(pear: f64) -> Vec<u8> {
    xlsx(&[(
        "Prices",
        vec![
            vec![
                C::S("id"),
                C::S("name"),
                C::S("price"),
                C::S("date"),
                C::S("ok"),
            ],
            vec![
                C::N(1.0),
                C::S("apple"),
                C::N(1.5),
                C::D(2024, 1, 31),
                C::B(true),
            ],
            vec![
                C::N(2.0),
                C::S("pear"),
                C::N(pear),
                C::D(2024, 2, 1),
                C::B(false),
            ],
            vec![
                C::N(3.0),
                C::S("plum"),
                C::N(0.1 + 0.2),
                C::D(2024, 2, 2),
                C::B(true),
            ],
        ],
    )])
}

/// The price list as its CSV export (other spellings of the same values).
const PRICES_CSV: &str = "id,name,price,date,ok\n1,apple,1.50,2024-01-31,TRUE\n2,pear,2,2024-02-01,FALSE\n\
                          3,plum,0.3,2024-02-02,TRUE\n";

fn request(left: &Path, right: &Path) -> TableCompareRequest {
    TableCompareRequest {
        left: os(left),
        right: os(right),
        ..TableCompareRequest::default()
    }
}

fn compare(
    service: &Service,
    req: &TableCompareRequest,
) -> Result<(ContentKind, ExitCode), ApiError> {
    let resp = service.table_compare(req, &job(service, JobKind::TableCompare))?;
    Ok((resp.kind, resp.exit_code))
}

/// AT.20: the verdicts and exit codes of T-16 (1, 2, 12 and 2 with `--ignore-unimportant`, 13).
#[test]
fn at20_exit_codes_of_every_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v\n1,1.50\n2,7\n");
    let same = write(&w, "same.csv", b"id,v\n1,1.50\n2,7\n");
    let spelled = write(&w, "spelled.csv", b"id,v\n1,1.5\n2,7\n");
    let near = write(&w, "near.csv", b"id,v\n1,1.51\n2,7\n");
    let changed = write(&w, "changed.csv", b"id,v\n1,9\n2,7\n");
    let tolerant = |right: &Path, iu: bool| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                tolerance: vec!["v=0.02".into()],
                ignore_case: vec![String::new()],
                keys: vec!["id".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ignore_unimportant: iu,
        ..request(&a, right)
    };
    assert_eq!(
        compare(&service, &request(&a, &same)).unwrap(),
        (ContentKind::BinarySame, ExitCode::BinarySame)
    );
    assert_eq!(
        compare(&service, &request(&a, &spelled)).unwrap(),
        (ContentKind::RulesSame, ExitCode::RulesSame)
    );
    assert_eq!(
        compare(&service, &tolerant(&near, false)).unwrap(),
        (ContentKind::UnimportantOnly, ExitCode::Similar)
    );
    assert_eq!(
        compare(&service, &tolerant(&near, true)).unwrap(),
        (ContentKind::RulesSame, ExitCode::RulesSame)
    );
    assert_eq!(
        compare(&service, &request(&a, &changed)).unwrap(),
        (ContentKind::ImportantDifferences, ExitCode::RulesDifferent)
    );
    assert_eq!(ExitCode::BinarySame.code(), 1);
    assert_eq!(ExitCode::Similar.code(), 12);
    assert_eq!(ExitCode::RulesDifferent.code(), 13);
}

/// AT.20: the failures of exit 100, each with its kind; a bad word is refused before anything is read.
#[test]
fn at20_errors_are_exit_100_with_their_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v\n1,2\n");
    let missing = w.join("missing.csv");
    let op = hikasa_session::api::ApiOp::TableCompare;
    let fails = |req: TableCompareRequest| -> ApiError {
        let e = compare(&service, &req).unwrap_err();
        assert_eq!(e.exit_code(op), ExitCode::UnknownError, "{e}");
        e
    };
    // A bad word is refused before the (missing) files are read.
    for words in [
        TableRuleWords {
            sheets: Some("sideways".into()),
            ..TableRuleWords::default()
        },
        TableRuleWords {
            rules: SheetRuleWords {
                tolerance: vec!["lots".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
    ] {
        let e = fails(TableCompareRequest {
            rules: words,
            ..request(&missing, &missing)
        });
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
    }
    let e = fails(TableCompareRequest {
        parse: TableFormatWords {
            widths: Some("3;0".into()),
            ..TableFormatWords::default()
        },
        ..request(&missing, &missing)
    });
    assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
    // A column reference that names nothing lists the columns.
    let e = fails(TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                keys: vec!["nope".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..request(&a, &a)
    });
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    assert!(e.to_string().contains("id, v"), "{e}");
    // A missing file, a folder, a damaged workbook, a text format forced on a table.
    assert!(matches!(
        fails(request(&a, &missing)),
        ApiError::LoadFailed { .. }
    ));
    assert_eq!(fails(request(&a, &w)).kind(), ErrorKind::InvalidInput);
    let broken = write(&w, "broken.xlsx", b"PK\x03\x04 not a zip at all");
    assert_eq!(fails(request(&a, &broken)).kind(), ErrorKind::InvalidData);
    let e = fails(TableCompareRequest {
        format: Some("c-cpp".into()),
        ..request(&a, &a)
    });
    assert!(e.to_string().contains("table kind"), "{e}");
    let e = fails(TableCompareRequest {
        format: Some("csv".into()),
        formats: Pair::new(Some("csv".into()), None),
        ..request(&a, &a)
    });
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
}

/// AT.20, T-3: an xlsx against its CSV export is the same (other spellings, 15 digits, dates, bools), a changed price
/// is an important difference with the cells and rows the JSON names.
#[test]
fn at20_a_workbook_against_its_csv_export() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let book = write(&w, "prices.xlsx", &prices(2.0));
    let csv = write(&w, "prices.csv", PRICES_CSV.as_bytes());
    let keyed = |right: &Path| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                keys: vec!["id".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..request(&book, right)
    };
    let resp = service
        .table_compare(&keyed(&csv), &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(
        resp.kind,
        ContentKind::RulesSame,
        "{:?}",
        resp.human_lines(None)
    );
    let ids = resp
        .formats
        .as_ref()
        .map(|f| f.format.as_ref().and_then(|f| f.id.clone()));
    assert_eq!(ids.left.as_deref(), Some("spreadsheets"));
    assert_eq!(ids.right.as_deref(), Some("csv"));
    let other = write(&w, "other.xlsx", &prices(2.25));
    let resp = service
        .table_compare(&keyed(&other), &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.exit_code, ExitCode::RulesDifferent);
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(json["verdict"]["result"], "important_differences");
    assert_eq!(json["left"]["source"], "xlsx");
    let rows = json["sheets"][0]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["cells"][0]["left"]["value"], "2");
    assert_eq!(rows[0]["cells"][0]["right"]["value"], "2.25");
    let lines = resp.human_lines(Some(20));
    assert!(
        lines.iter().any(|l| l.contains("price: 2 \u{2192} 2.25")),
        "{lines:?}"
    );
}

/// AT.20, T-10: several sheets pair by name (case-insensitively), a sheet on one side only is important; per-sheet rules
/// of a session key one sheet and leave the other unkeyed.
#[test]
fn at20_sheets_and_per_sheet_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let rows = |b: f64| {
        vec![
            vec![C::S("code"), C::S("n")],
            vec![C::S("a"), C::N(1.0)],
            vec![C::S("b"), C::N(b)],
        ]
    };
    let left = write(
        &w,
        "l.xlsx",
        &xlsx(&[
            ("Data", rows(2.0)),
            ("Notes", rows(5.0)),
            ("Old", rows(1.0)),
        ]),
    );
    let right = write(
        &w,
        "r.xlsx",
        &xlsx(&[("notes", rows(5.0)), ("Data", rows(3.0))]),
    );
    let resp = service
        .table_compare(
            &request(&left, &right),
            &job(&service, JobKind::TableCompare),
        )
        .unwrap();
    let names: Vec<_> = resp
        .comparison
        .sheets()
        .iter()
        .map(|s| (s.name().to_owned(), s.state().as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("Data".to_owned(), "important"),
            ("Notes".to_owned(), "same"),
            ("Old".to_owned(), "left_only"),
        ]
    );
    let lines = resp.human_lines(Some(20));
    assert!(
        lines.contains(&"= Notes \u{2192} notes".to_owned()),
        "{lines:?}"
    );
    assert!(lines.contains(&"< Old (left only)".to_owned()), "{lines:?}");
    // Per-sheet rules (sessions only): Data keyed by `code`, the defaults elsewhere.
    let per = TableRuleWords {
        sheet: vec![SheetRulesWords {
            sheet: "data".into(),
            rules: SheetRuleWords {
                keys: vec!["code".into()],
                ..SheetRuleWords::default()
            },
        }],
        ..TableRuleWords::default()
    };
    let resp = service
        .table_compare(
            &TableCompareRequest {
                rules: per,
                ..request(&left, &right)
            },
            &job(&service, JobKind::TableCompare),
        )
        .unwrap();
    let json = serde_json::to_value(resp.json(&resp.view)).unwrap();
    assert_eq!(json["sheets"][0]["alignment"]["mode"], "keyed");
    assert_eq!(json["sheets"][1]["alignment"]["mode"], "engine");
}

/// A workbook inside a zip archive loads through the VFS (T-4): the member against the file it was made from.
#[test]
fn a_workbook_member_of_a_zip_through_the_vfs() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let book = prices(2.0);
    let file = write(&w, "prices.xlsx", &book);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    zip.start_file("books/prices.xlsx", opts).unwrap();
    zip.write_all(&book).unwrap();
    zip.start_file("books/prices.csv", opts).unwrap();
    zip.write_all(PRICES_CSV.as_bytes()).unwrap();
    let bytes = zip.finish().unwrap().into_inner();
    let archive = write(&w, "books.zip", &bytes);
    let member = |name: &str| {
        let mut spec = archive.as_os_str().to_owned();
        spec.push(format!("/books/{name}"));
        spec
    };
    let req = TableCompareRequest {
        left: member("prices.xlsx"),
        right: os(&file),
        ..TableCompareRequest::default()
    };
    let resp = service
        .table_compare(&req, &job(&service, JobKind::TableCompare))
        .unwrap();
    assert_eq!(resp.kind, ContentKind::BinarySame);
    assert!(
        resp.sides.left.spec.ends_with("prices.xlsx"),
        "{}",
        resp.sides.left.spec
    );
    let req = TableCompareRequest {
        left: member("prices.xlsx"),
        right: member("prices.csv"),
        ..TableCompareRequest::default()
    };
    assert_eq!(compare(&service, &req).unwrap().0, ContentKind::RulesSame);
}

/// AT.22, T-21: a folder comparison under the Rules method compares table files through the table engine: a
/// respelled CSV is same by the rules, a changed workbook cell is a difference, a damaged workbook is that pair's
/// error (never a byte comparison), a text file still goes through the text rules.
#[test]
fn at22_folder_compare_of_csv_and_xlsx_under_the_rules_method() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "l/data.csv", b"id,v\n1,1.50\n2,7\n");
    write(&w, "r/data.csv", b"id,v\n1,1.5\n2,7\n");
    write(&w, "l/book.xlsx", &prices(2.0));
    write(&w, "r/book.xlsx", &prices(2.5));
    write(&w, "l/same.xlsx", &prices(2.0));
    write(
        &w,
        "r/same.xlsx",
        &xlsx(&[(
            "Prices",
            vec![
                vec![
                    C::S("id"),
                    C::S("name"),
                    C::S("price"),
                    C::S("date"),
                    C::S("ok"),
                ],
                vec![
                    C::N(1.0),
                    C::S("apple"),
                    C::N(1.5),
                    C::D(2024, 1, 31),
                    C::B(true),
                ],
                vec![
                    C::N(2.0),
                    C::S("pear"),
                    C::N(2.0),
                    C::D(2024, 2, 1),
                    C::B(false),
                ],
                vec![
                    C::N(3.0),
                    C::S("plum"),
                    C::N(0.3),
                    C::D(2024, 2, 2),
                    C::B(true),
                ],
            ],
        )]),
    );
    write(&w, "l/bad.xlsx", b"not a workbook");
    write(&w, "r/bad.xlsx", b"not a workbook either");
    write(&w, "l/note.txt", b"a b\r\n");
    write(&w, "r/note.txt", b"a b\n");
    let req = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("rules-based".into()),
        ..FolderCompareRequest::default()
    };
    let resp = service
        .folder_compare(&req, &job(&service, JobKind::FolderCompare), &NullObserver)
        .unwrap();
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let json = serde_json::to_value(resp.json(&display)).unwrap();
    let result = |path: &str| -> String {
        let item = json["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["path"] == path)
            .unwrap_or_else(|| panic!("{path}: {json}"));
        item["content"]["result"]["result"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(result("data.csv"), "rules_same");
    assert_eq!(result("book.xlsx"), "important_differences");
    assert_eq!(result("same.xlsx"), "rules_same");
    assert_eq!(result("bad.xlsx"), "error");
    // Line endings are not compared by the text rules' defaults.
    assert_eq!(result("note.txt"), "rules_same");
}

/// AT.21: a table-compare session keeps the words (sparse against the defaults) and `session run` runs it.
#[test]
fn at21_sessions_round_trip_and_run() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id;v;w\n1;1,5;a\n2;7;b\n");
    let b = write(&w, "b.csv", b"id;v;w\n2;7;b\n1;1,50;a\n");
    let mut session = Session::builtin(SessionType::TableCompare);
    session.specs.left = a.display().to_string();
    session.specs.right = b.display().to_string();
    let SessionRules::Table(rules) = &mut session.rules else {
        panic!("{:?}", session.rules);
    };
    rules.parse.delimiters = vec![";".into()];
    rules.parse.decimal = Some(",".into());
    rules.parse.thousands = Some("none".into());
    rules.rules.rules.keys = vec!["id".into()];
    let reference = SessionRef::parse("tables/prices").unwrap();
    service.session_save(&reference, &session, false).unwrap();
    let text =
        std::fs::read_to_string(tmp.path().join("config/sessions/tables/prices.toml")).unwrap();
    assert!(text.contains("keys = [\"id\"]"), "{text}");
    assert!(text.contains("[rules.parse]"), "{text}");
    assert!(
        !text.contains("sort_rows"),
        "defaults are not written: {text}"
    );
    let SessionRunRequest::Table(req) = service.session_run_request(&reference).unwrap() else {
        panic!("not a table request");
    };
    assert_eq!(compare(&service, &req).unwrap().0, ContentKind::RulesSame);
    // Without the session's keys the rows differ by order.
    let unkeyed = TableCompareRequest {
        rules: TableRuleWords::default(),
        ..req
    };
    assert_ne!(
        compare(&service, &unkeyed).unwrap().0,
        ContentKind::RulesSame
    );
}

/// T-22: a Text Compare of two workbooks reads them as text through the Spreadsheet conversion (read-only).
#[test]
fn t22_text_compare_reads_workbooks_as_text() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let two = |pear: f64| {
        xlsx(&[
            ("One", vec![vec![C::S("a"), C::N(1.0)]]),
            ("Two", vec![vec![C::S("pear"), C::N(pear)]]),
        ])
    };
    let left = write(&w, "l.xlsx", &two(2.0));
    let right = write(&w, "r.xlsx", &two(3.0));
    let resp = service
        .text_compare(
            &TextCompareRequest {
                left: os(&left),
                right: os(&right),
                ..TextCompareRequest::default()
            },
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    let doc = resp.comparison.document(hikasa_core::Side::Left);
    let lines: Vec<String> = (0..doc.line_count())
        .map(|k| doc.line(hikasa_text::LineIdx(k)).into_owned())
        .collect();
    assert_eq!(lines, ["--- One ---", "a\t1", "--- Two ---", "pear\t2"]);
    assert!(doc.converted(), "read-only: converted");
    assert_eq!(
        resp.comparison.verdict(false),
        ContentResult::ImportantDifferences
    );
}

/// A16: `--regional system` reads numbers the locale's way (`de_DE`: `,` decimal, `.` thousands).
#[test]
fn a16_regional_settings_of_the_locale() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = facts(tmp.path());
    env.locale_numeric = Some("de_DE.UTF-8".into());
    env.locale_time = Some("de_DE.UTF-8".into());
    let init = ServiceInit {
        config_dir: Some(tmp.path().join("config")),
        threads: std::num::NonZeroUsize::new(2),
        ..ServiceInit::new(
            env,
            fixed_clock(),
            std::sync::Arc::new(common::UpperRunner::default()),
        )
    };
    let service = Service::new(init).unwrap();
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"n;d\n1.234,5;31.01.2024\n");
    let b = write(&w, "b.csv", b"n;d\n1234,50;2024-01-31\n");
    let parse = |regional: Option<&str>| TableCompareRequest {
        parse: TableFormatWords {
            delimiters: vec![";".into()],
            regional: regional.map(str::to_owned),
            ..TableFormatWords::default()
        },
        ..request(&a, &b)
    };
    assert_eq!(
        compare(&service, &parse(Some("system"))).unwrap().0,
        ContentKind::RulesSame
    );
    assert_eq!(
        compare(&service, &parse(None)).unwrap().0,
        ContentKind::ImportantDifferences
    );
    let r = service.system_regional();
    assert_eq!((r.decimal, r.thousands), (',', Some('.')));
}

/// T-24 and T-30 through the words: `--number` equates `007` and `7` (a leading zero makes the column text by
/// default); a per-sheet rule must name a column of its sheet (strict), while the rules of every sheet skip one.
#[test]
fn number_columns_and_strict_per_sheet_rules() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"code,v\n007,1\n010,2\n");
    let b = write(&w, "b.csv", b"code,v\n7,1\n10,2\n");
    assert_eq!(
        compare(&service, &request(&a, &b)).unwrap().0,
        ContentKind::ImportantDifferences
    );
    let number = TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                number: vec!["code".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..request(&a, &b)
    };
    assert_eq!(
        compare(&service, &number).unwrap().0,
        ContentKind::RulesSame
    );
    let text = TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                text: vec!["v".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ..number.clone()
    };
    assert_eq!(
        compare(&service, &text).unwrap().0,
        ContentKind::ImportantDifferences
    );
    // Rules of one sheet are strict: a column the sheet lacks is an error.
    let per_sheet = |column: &str| TableCompareRequest {
        rules: TableRuleWords {
            sheet: vec![SheetRulesWords {
                sheet: "1".into(),
                rules: SheetRuleWords {
                    keys: vec![column.into()],
                    ..SheetRuleWords::default()
                },
            }],
            ..TableRuleWords::default()
        },
        ..request(&a, &a)
    };
    assert_eq!(
        compare(&service, &per_sheet("code")).unwrap().0,
        ContentKind::BinarySame
    );
    let e = compare(&service, &per_sheet("missing")).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidInput, "{e}");
}

/// §6.5: with `--ignore-unimportant` the human output lists no unimportant cell; without, each is marked.
#[test]
fn the_human_output_under_ignore_unimportant() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.csv", b"id,v,w\n1,1.50,3\n2,7,4\n");
    let b = write(&w, "b.csv", b"id,v,w\n1,1.51,3\n2,7.01,5\n");
    let req = |iu: bool| TableCompareRequest {
        rules: TableRuleWords {
            rules: SheetRuleWords {
                tolerance: vec!["v=0.02".into()],
                ..SheetRuleWords::default()
            },
            ..TableRuleWords::default()
        },
        ignore_unimportant: iu,
        ..request(&a, &b)
    };
    let lines = |iu: bool| {
        service
            .table_compare(&req(iu), &job(&service, JobKind::TableCompare))
            .unwrap()
            .human_lines(None)
    };
    let all = lines(false);
    assert!(
        all.iter()
            .any(|l| l.ends_with("v: 1.50 \u{2192} 1.51 (unimportant)")),
        "{all:?}"
    );
    assert!(
        all.iter().any(|l| l.ends_with("w: 4 \u{2192} 5")),
        "{all:?}"
    );
    assert!(
        all.iter()
            .any(|l| l.ends_with("v: 7 \u{2192} 7.01 (unimportant)")),
        "{all:?}"
    );
    // Row 2 still differs (w), so it is shown; its unimportant cell is not.
    let iu = lines(true);
    assert!(!iu.iter().any(|l| l.contains("(unimportant)")), "{iu:?}");
    assert!(iu.iter().any(|l| l.ends_with("w: 4 \u{2192} 5")), "{iu:?}");
}
