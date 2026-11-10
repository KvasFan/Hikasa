//! AT.30 (m2-table-contract §7, phase 3): the Table Compare view through the command catalogue, without a window
//! (§1.4.16): the grid's data of a CSV pair and of two workbooks, every implemented entry of the `table-compare`
//! section with its effect, the navigation of `nav::*` under the display filter, the columns (hide, Hide Same Columns,
//! widths), Copy through a fixed clipboard, Go To, Info, the report dialog, the session tabs, and the session's `[view]`
//! and `[display]` round trip. The view never recomputes: every state asserted here is the core's.

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::Side;
use hikasa_session::clipboard::{ClipboardProvider, FixedClipboard};
use hikasa_session::commands::{CommandStatus, catalogue};
use hikasa_session::ipc::ViewId;
use hikasa_table::{CellState, RowFilter, RowKind, SheetState};

use crate::commands::{CommandId, Scope, lookup};
use crate::dialog::{ButtonRole, DialogModel};
use crate::table_view::{GridLayout, TablePhase, TableViewModel};
use crate::tests::{TestApp, World};

/// The model of a table view.
pub(crate) fn table_model(app: &mut TestApp, view: ViewId) -> &mut TableViewModel {
    match app.tab_mut(view).expect("tab").model_mut().expect("live") {
        crate::app::ViewModel::Table(m) => m,
        _ => panic!("not a table view"),
    }
}

fn find<T: DialogModel>(app: &mut TestApp) -> Option<&mut T> {
    let w = app.win();
    app.0.window_mut(w).and_then(|w| w.dialogs.find_mut::<T>())
}

/// A keyed CSV pair: id 2 changes its price (important) and its name's case (important), id 3 left only, id 4 right
/// only, id 1 same.
fn csv_pair(world: &World) -> (PathBuf, PathBuf) {
    (
        world.write(
            "work/t/a.csv",
            "id,name,price,note\n1,apple,1.5,x\n2,pear,2,y\n3,plum,0.75,z\n",
        ),
        world.write(
            "work/t/b.csv",
            "id,name,price,note\n1,apple,1.5,x\n2,Pear,2.5,y\n4,fig,3,w\n",
        ),
    )
}

/// The two workbooks of `tests/fixtures/table` (sheets Same, Prices, Old / New) copied into the world.
fn books(world: &World) -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/table");
    let copy = |name: &str| {
        let to = world.path(&format!("work/books/{name}"));
        std::fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
        std::fs::copy(dir.join(name), &to).expect("copy");
        to
    };
    (copy("books-left.xlsx"), copy("books-right.xlsx"))
}

fn open(world: &World, app: &mut TestApp, (l, r): (PathBuf, PathBuf)) -> ViewId {
    let view = world.open(app, &l, &r);
    world.settle(app);
    assert_eq!(
        app.scope(),
        Scope::Table,
        "two tables open in Table Compare (T-46)"
    );
    view
}

fn keyed(world: &World, app: &mut TestApp) -> ViewId {
    // `id` keys the rows: the table-compare defaults carry it (T-45: a view without words takes them).
    let mut defaults = world
        .service
        .session_defaults(hikasa_session::sessions::SessionType::TableCompare)
        .expect("defaults");
    if let hikasa_session::sessions::SessionRules::Table(t) = &mut defaults.rules {
        t.rules.rules.keys = vec!["id".into()];
    }
    world
        .service
        .session_save_defaults(&defaults)
        .expect("save defaults");
    open(world, app, csv_pair(world))
}

/// AT.30: the grid of a keyed CSV pair: rows in key order with each side's row label, cell texts and states from
/// `cell_state`, hatched sides of orphan rows, column heads with the key mark, the status bar and the sheet tab.
#[test]
fn at30_the_grid_of_a_keyed_csv_pair() {
    let world = World::new();
    let mut app = world.app(None);
    let view = keyed(&world, &mut app);
    let m = table_model(&mut app, view);
    assert_eq!(m.phase(), TablePhase::Ready);
    assert_eq!(m.filter(), RowFilter::ALL);
    assert_eq!(m.len(), 4);
    let rows: Vec<_> = (0..4).map(|p| m.row_at(p).expect("row")).collect();
    let kinds: Vec<RowKind> = rows.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [
            RowKind::Same,
            RowKind::Changed,
            RowKind::LeftOnly,
            RowKind::RightOnly
        ]
    );
    assert_eq!(rows[1].labels, [Some(3), Some(3)], "the text lines");
    assert_eq!(rows[2].labels, [Some(4), None]);
    let head = m.column_head(0).expect("id");
    assert_eq!(head.key, Some(1));
    assert_eq!(head.names[0].as_deref(), Some("id"));
    assert_eq!(head.letters, [Some("A".to_owned()), Some("A".to_owned())]);
    let price = m.cell(rows[1].row, 2, Side::Right);
    assert_eq!(price.text.as_deref(), Some("2.5"));
    assert_eq!(price.state, CellState::Important);
    assert_eq!(m.cell(rows[1].row, 3, Side::Left).state, CellState::Same);
    assert_eq!(
        m.cell(rows[2].row, 1, Side::Right).text,
        None,
        "a left-only row"
    );
    assert_eq!(
        m.cell(rows[2].row, 1, Side::Left).text.as_deref(),
        Some("plum")
    );
    let tabs = m.sheet_tabs();
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].state, SheetState::Important);
    m.click(1, 2, Side::Right);
    let status = m.status_items();
    assert!(
        status.left.contains("C3 (right): important difference"),
        "{}",
        status.left
    );
    assert!(
        status.right[0].contains("Important differences"),
        "{:?}",
        status.right
    );
    // The thumbnail marks the changed row and the orphans.
    assert_eq!(m.thumbnail(4), vec![0, 2, 3, 3]);
}

/// What a command must have done on a fresh view.
type Effect = fn(&World, &mut TestApp, ViewId);

/// AT.30 for `Scope::Table`: every implemented entry of the `table-compare` section has a presentation row, and each
/// command of the view does what it says on a fresh view of the keyed pair (or of the workbooks, for the sheets).
#[test]
fn at30_every_table_entry_has_its_effect() {
    let section = catalogue().section("table-compare").expect("table-compare");
    for entry in section
        .commands
        .iter()
        .filter(|e| e.status == CommandStatus::Implemented)
    {
        let id = CommandId::from_name(&entry.id).expect("known");
        assert!(
            lookup(Scope::Table, id).is_some_and(|r| r.scope == Scope::Table),
            "{id:?} is implemented but has no Table row"
        );
    }
    let effects: &[(CommandId, Effect)] = &[
        (CommandId::DiffItemNext, |_, app, view| {
            let m = table_model(app, view);
            assert_eq!(
                m.cursor().map(|c| (c.row, c.col)),
                Some((1, 1)),
                "row 2: its first differing cell"
            );
            assert!(app.enabled(CommandId::DiffItemNext), "orphans follow");
        }),
        (CommandId::DiffItemPrev, |_, app, view| {
            assert_eq!(table_model(app, view).cursor().map(|c| c.row), Some(3));
        }),
        (CommandId::DiffNext, |_, app, view| {
            assert_eq!(
                table_model(app, view).cursor().map(|c| (c.row, c.col)),
                Some((1, 1))
            );
        }),
        (CommandId::DiffPrev, |_, app, view| {
            assert_eq!(table_model(app, view).cursor().map(|c| c.row), Some(3));
        }),
        (CommandId::FavCompareAll, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 4);
        }),
        (CommandId::FavDiff, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 3);
            assert_eq!(app.checked(CommandId::FavDiff), Some(true));
        }),
        (CommandId::FavSame, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 1);
        }),
        (CommandId::FavCompareNone, |_, app, view| {
            let m = table_model(app, view);
            assert_eq!(m.len(), 0);
            assert!(m.message().is_some_and(|t| t.contains("display filter")));
        }),
        (CommandId::TogSame, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 3);
            assert_eq!(app.checked(CommandId::TogSame), Some(false));
        }),
        (CommandId::TogLeftOrphan, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 3);
        }),
        (CommandId::TogRightOrphan, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 3);
        }),
        (CommandId::TogDiff, |_, app, view| {
            assert_eq!(table_model(app, view).len(), 3);
        }),
        (CommandId::InsigIgnored, |_, app, view| {
            assert!(table_model(app, view).filter().ignore_unimportant);
        }),
        (CommandId::ColHideSame, |_, app, view| {
            // id (a key) stays; name and price differ; note is same.
            assert_eq!(table_model(app, view).shown_columns(), &[0, 1, 2]);
        }),
        (CommandId::ColDetectSize, |_, app, view| {
            let w = table_model(app, view).column_head(1).expect("name").width;
            assert!((w - 7.0).abs() < 0.01, "pear/apple + 2: {w}");
        }),
        (CommandId::ShowThumbnail, |_, app, view| {
            assert!(!table_model(app, view).show_thumbnail());
        }),
        (CommandId::ShowDetails, |_, app, view| {
            assert!(table_model(app, view).show_details());
        }),
        (CommandId::GoTo, |_, app, _| {
            assert!(find::<crate::table_dialogs::GoToDialog>(app).is_some());
        }),
        (CommandId::DiffSheetNext, |_, _, _| {}),
        (CommandId::DiffSheetPrev, |_, _, _| {}),
        (CommandId::SideBySide, |_, app, view| {
            assert_eq!(table_model(app, view).layout(), GridLayout::SideBySide);
        }),
        (CommandId::OverUnder, |_, app, view| {
            assert_eq!(table_model(app, view).layout(), GridLayout::OverUnder);
            assert_eq!(app.checked(CommandId::SideBySide), Some(false));
        }),
        (CommandId::Report, |_, app, _| {
            assert!(find::<crate::table_dialogs::TableReportDialog>(app).is_some());
        }),
    ];
    for entry in section
        .commands
        .iter()
        .filter(|e| e.status == CommandStatus::Implemented)
    {
        let id = CommandId::from_name(&entry.id).expect("known");
        assert!(
            effects.iter().any(|(e, _)| *e == id),
            "{id:?} has no effect in this test"
        );
    }
    for &(id, check) in effects {
        let world = World::new();
        let mut app = world.app(None);
        let view = keyed(&world, &mut app);
        assert!(app.enabled(id), "{id:?} is available");
        app.execute(id);
        world.settle(&mut app);
        check(&world, &mut app, view);
    }
}

/// TB-022 over workbooks: the next and previous difference sheets skip the same one; a sheet tab click shows its
/// rows; each sheet keeps its cursor; Go To finds a row label on the sheet chosen.
#[test]
fn sheets_navigation_and_go_to() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open(&world, &mut app, books(&world));
    let m = table_model(&mut app, view);
    let tabs = m.sheet_tabs();
    let states: Vec<SheetState> = tabs.iter().map(|t| t.state).collect();
    assert_eq!(
        states,
        [
            SheetState::Same,
            SheetState::Important,
            SheetState::Important
        ]
    );
    assert_eq!(
        tabs[2].caption, "Old → New",
        "a renamed pair names both sheets"
    );
    assert_eq!(m.active_sheet(), 0);
    app.execute(CommandId::DiffSheetNext);
    let m = table_model(&mut app, view);
    assert_eq!(m.active_sheet(), 1);
    assert!(m.cursor().is_some(), "the first difference of the sheet");
    app.execute(CommandId::DiffSheetNext);
    assert_eq!(table_model(&mut app, view).active_sheet(), 2);
    app.execute(CommandId::DiffSheetPrev);
    assert_eq!(table_model(&mut app, view).active_sheet(), 1);
    // Go To row 3, column `name`, on the Prices sheet.
    app.execute(CommandId::GoTo);
    {
        let d = find::<crate::table_dialogs::GoToDialog>(&mut app).expect("go to");
        assert_eq!(d.sheets.join(","), "Same,Prices,Old");
        d.row = "3".into();
        d.column = "name".into();
    }
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let m = table_model(&mut app, view);
    let cursor = m.cursor().expect("cursor");
    assert_eq!(
        m.row_at(m.position_of(cursor.row).unwrap()).unwrap().labels[0],
        Some(3)
    );
    assert_eq!(cursor.col, 1);
    assert_eq!(m.address().as_deref(), Some("B3"));
    // A label that is not shown is a message.
    app.execute(CommandId::GoTo);
    find::<crate::table_dialogs::GoToDialog>(&mut app)
        .expect("go to")
        .row = "99".into();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let message = app
        .active_tab()
        .and_then(|t| t.message.clone())
        .expect("a message");
    assert!(message.text.contains("99"), "{message:?}");
}

/// Copy places the current cell, or the row after a click on its number, on the window's own clipboard.
#[test]
fn copy_cell_and_row() {
    let world = World::new();
    let mut app = world.app(None);
    let fixed = Arc::new(FixedClipboard::text("unused"));
    app.0
        .set_script_clipboard(Arc::clone(&fixed) as Arc<dyn ClipboardProvider>);
    let view = keyed(&world, &mut app);
    table_model(&mut app, view).click(1, 2, Side::Right);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    table_model(&mut app, view).click_row(2, Side::Left);
    app.execute(CommandId::Copy);
    world.settle(&mut app);
    let texts: Vec<String> = fixed.payloads().into_iter().map(|p| p.text).collect();
    assert_eq!(texts, ["2.5", "3\tplum\t0.75\tz"]);
}

/// Hidden columns, widths and the sheet live in the session's `[view]`; the filter in `[display]` (T-45); a view
/// opened from the session shows them again.
#[test]
fn the_session_keeps_the_grid() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open(&world, &mut app, books(&world));
    {
        let m = table_model(&mut app, view);
        m.set_sheet(1);
        m.hide_column(2);
        m.set_width(1, 20.0);
    }
    app.execute(CommandId::FavDiff);
    let tab = app.tab(view).expect("tab");
    let capture = crate::app::AppCore::capture_for_test(tab).expect("capture");
    let (session, _) = capture.session(&world.service).expect("session");
    assert_eq!(
        session.view.get("sheet").and_then(|v| v.as_str()),
        Some("Prices")
    );
    let hidden = session
        .view
        .get("hidden_columns")
        .expect("hidden")
        .to_string();
    assert!(hidden.contains("Prices/price"), "{hidden}");
    assert_eq!(
        session.display.get("filter").and_then(|v| v.as_str()),
        RowFilter::DIFFERENCES.preset_word()
    );
    // Saved and opened again in a new tab.
    let reference = hikasa_session::sessions::SessionRef::parse("tables/books").expect("ref");
    world
        .service
        .session_save(&reference, &session, false)
        .expect("save");
    let target = crate::app::OpenTarget::Window(app.win());
    app.0
        .open_named(reference, crate::app::OpenHow::Target(target));
    world.settle(&mut app);
    let again = app.active_tab().expect("tab").id;
    assert_ne!(again, view);
    let m = table_model(&mut app, again);
    assert_eq!(m.active_sheet(), 1);
    assert_eq!(m.shown_columns(), &[0, 1]);
    assert_eq!(m.column_head(1).expect("name").width, 20.0);
    assert_eq!(m.filter(), RowFilter::DIFFERENCES);
}

/// Rules opens the Session Settings on the Sheets tab; a word of the Rows tab is checked by core (an engine that does
/// not exist keeps OK off) and OK recompares the view with the key typed there.
#[test]
fn rules_tabs_recompare_the_view() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open(&world, &mut app, csv_pair(&world));
    // Unkeyed, rows align by content.
    assert!(table_model(&mut app, view).response().is_some());
    app.execute(CommandId::Rules);
    world.settle(&mut app);
    let w = app.win();
    let d = find::<crate::session_dialog::SessionSettingsDialog>(&mut app).expect("settings");
    let titles: Vec<String> = d.tabs.iter().map(|t| t.title()).collect();
    assert_eq!(titles, ["Specs", "Sheets", "Columns", "Rows", "Parsing"]);
    assert_eq!(d.tabs[d.active].title(), "Sheets");
    d.tab_mut::<crate::table_tabs::RowsTab>()
        .expect("rows")
        .fields
        .algorithm = "quantum".into();
    let svc = world.service.clone();
    d.poll(&svc);
    assert!(
        d.tab_error().is_some_and(|(_, e)| e.contains("quantum")),
        "{:?}",
        d.tab_error()
    );
    let rows = d.tab_mut::<crate::table_tabs::RowsTab>().expect("rows");
    rows.fields.algorithm.clear();
    rows.fields.keys = "id".into();
    d.poll(&svc);
    assert!(d.tab_error().is_none(), "{:?}", d.tab_error());
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    world.settle(&mut app);
    let m = table_model(&mut app, view);
    let rules = &m.request().rules.rules;
    assert_eq!(rules.keys, ["id"]);
    let kinds: Vec<RowKind> = (0..m.len())
        .map(|p| m.row_at(p).expect("row").kind)
        .collect();
    assert_eq!(
        kinds,
        [
            RowKind::Same,
            RowKind::Changed,
            RowKind::LeftOnly,
            RowKind::RightOnly
        ]
    );
}

/// Info shows the statistics per sheet; the report dialog writes a report file through the Service.
#[test]
fn info_and_report() {
    let world = World::new();
    let mut app = world.app(None);
    let view = keyed(&world, &mut app);
    app.execute(CommandId::Info);
    let d = find::<crate::info_dialog::InfoDialog>(&mut app).expect("info");
    let text = d.text();
    assert!(text.starts_with("Table Compare Info"), "{text}");
    assert!(
        text.contains("Changed rows (important / unimportant)"),
        "{text}"
    );
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    app.execute(CommandId::Report);
    let out = world.path("work/report.html");
    {
        let d = find::<crate::table_dialogs::TableReportDialog>(&mut app).expect("report");
        d.path = out.display().to_string();
        d.layout = "side-by-side";
    }
    app.0.dialog_button(w, ButtonRole::Ok);
    for _ in 0..200 {
        world.settle(&mut app);
        let done = find::<crate::table_dialogs::TableReportDialog>(&mut app)
            .is_some_and(|d| d.message.is_some());
        if done {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let d = find::<crate::table_dialogs::TableReportDialog>(&mut app).expect("report");
    assert_eq!(
        d.message.as_ref().map(|m| m.0),
        Some(true),
        "{:?}",
        d.message
    );
    let html = std::fs::read_to_string(&out).expect("written");
    assert!(html.contains("<html") && html.contains("plum"), "{html}");
    let _ = view;
}

/// A table view's outcome is its verdict under its ignore-unimportant (D-23: 13 here).
#[test]
fn the_outcome_is_the_verdict() {
    let world = World::new();
    let mut app = world.app(None);
    let view = keyed(&world, &mut app);
    let service = world.service.clone();
    let outcome = table_model(&mut app, view).outcome(&service);
    let (_, code) = outcome.close(hikasa_session::ipc::CloseReason::User, false);
    assert_eq!(code, hikasa_core::ExitCode::RulesDifferent);
}

/// TB-022 under the display filter: with changed rows hidden, the next difference row is the first orphan the grid
/// shows (`nav::next_difference_row` skips rows the filter hides).
#[test]
fn navigation_skips_hidden_rows() {
    let world = World::new();
    let mut app = world.app(None);
    let view = keyed(&world, &mut app);
    app.execute(CommandId::TogDiff);
    let m = table_model(&mut app, view);
    assert_eq!(m.len(), 3);
    app.execute(CommandId::DiffItemNext);
    let m = table_model(&mut app, view);
    assert_eq!(
        m.cursor().map(|c| c.row),
        Some(2),
        "the left-only row, not the hidden changed one"
    );
    app.execute(CommandId::DiffNext);
    assert_eq!(table_model(&mut app, view).cursor().map(|c| c.row), Some(3));
    assert!(
        !app.enabled(CommandId::DiffItemNext),
        "no difference after the last orphan"
    );
}

/// FC-061 and T-46: a folder row's Open of a CSV pair opens Table Compare; Open With ▸ Table Compare too.
#[test]
fn a_folder_row_opens_tables_in_the_grid() {
    use crate::tests_folder_ops::{open as open_folders, put, select};
    let world = World::new();
    put(&world, "l", "a.csv", "id,v\n1,2\n", 1_000);
    put(&world, "r", "a.csv", "id,v\n1,3\n", 1_000);
    let mut app = world.app(None);
    let view = open_folders(&world, &mut app);
    world.settle(&mut app);
    select(&mut app, view, "a.csv");
    let tabs = app.tabs().len();
    app.execute(CommandId::Open);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 1);
    assert_eq!(app.scope(), Scope::Table);
    let table = app.active_tab().expect("tab").id;
    assert_eq!(table_model(&mut app, table).len(), 1);
    // Open With ▸ Table Compare from the folder view.
    let w = app.win();
    let index = app
        .0
        .window(w)
        .expect("window")
        .index_of(view)
        .expect("folder tab");
    app.activate(index);
    select(&mut app, view, "a.csv");
    app.0.execute(
        w,
        crate::window::Invocation::with(
            CommandId::OpenWithList,
            crate::window::CommandArg::OpenWith(crate::window::OpenWithItem::TableCompare),
        ),
    );
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), tabs + 2);
    assert_eq!(app.scope(), Scope::Table);
}

/// Ignore Unimportant Differences: a difference within a tolerance shows as same in the grid, its row too; the
/// display word of each preset is what `[display] filter` keeps.
#[test]
fn ignored_unimportant_cells_show_as_same() {
    let world = World::new();
    let mut defaults = world
        .service
        .session_defaults(hikasa_session::sessions::SessionType::TableCompare)
        .expect("defaults");
    if let hikasa_session::sessions::SessionRules::Table(t) = &mut defaults.rules {
        t.rules.rules.keys = vec!["id".into()];
        t.rules.rules.tolerance = vec!["price=1".into()];
    }
    world
        .service
        .session_save_defaults(&defaults)
        .expect("save defaults");
    let mut app = world.app(None);
    let (l, r) = (
        world.write("work/u/a.csv", "id,price\n1,2\n"),
        world.write("work/u/b.csv", "id,price\n1,2.5\n"),
    );
    let view = open(&world, &mut app, (l, r));
    let m = table_model(&mut app, view);
    assert_eq!(m.cell(0, 1, Side::Left).state, CellState::Unimportant);
    assert_eq!(
        m.row_at(0).expect("row").importance,
        hikasa_table::Importance::Unimportant
    );
    app.execute(CommandId::InsigIgnored);
    let m = table_model(&mut app, view);
    assert_eq!(m.cell(0, 1, Side::Left).state, CellState::Same);
    assert_eq!(
        m.row_at(0).expect("row").importance,
        hikasa_table::Importance::None
    );
    for (id, preset) in [
        (CommandId::FavSame, RowFilter::SAME),
        (CommandId::FavCompareNone, RowFilter::NONE),
        (CommandId::FavDiff, RowFilter::DIFFERENCES),
        (CommandId::FavCompareAll, RowFilter::ALL),
    ] {
        app.execute(id);
        let word = table_model(&mut app, view).display_word();
        assert_eq!(Some(word), preset.preset_word(), "{id:?}");
    }
}
