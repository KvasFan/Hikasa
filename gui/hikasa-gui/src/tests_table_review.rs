//! The phase-3 review fixes of the Table Compare view (m2-table-contract §11.6): the session tabs write back only
//! what was edited (F1, F2), a replaced or closed view stops its comparison (F3), `--gui` without words (F4), Copy of
//! the full text (F5), duplicate column names (F10), navigation and hidden columns (F11, F12), the sheet tabs under
//! ignore-unimportant (F13), and one test for each mutation of the review that had survived (R2…R28).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hikasa_core::Side;
use hikasa_session::api::{GuiOpenRequest, TableCompareRequest};
use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::{Scope, SessionRules, SessionType};
use hikasa_table::{RowFilter, SheetState};

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::tests::{TestApp, World};
use crate::tests_table_view::table_model;

fn find<T: DialogModel>(app: &mut TestApp) -> Option<&mut T> {
    let w = app.win();
    app.0.window_mut(w).and_then(|w| w.dialogs.find_mut::<T>())
}

/// A pair under `header`: id 2 changes (name case, price), id 3 left only, id 4 right only, id 1 same.
pub(crate) fn pair(world: &World, header: &str) -> (PathBuf, PathBuf) {
    (
        world.write(
            "work/p/a.csv",
            &format!("{header}\n1,apple,1.5,x\n2,pear,2,y\n3,plum,0.75,z\n"),
        ),
        world.write(
            "work/p/b.csv",
            &format!("{header}\n1,apple,1.5,x\n2,Pear,2.5,y\n4,fig,3,w\n"),
        ),
    )
}

/// Opens `table compare a b --gui` with `edit` applied to the request (the words as typed).
pub(crate) fn open_words(
    world: &World,
    app: &mut TestApp,
    (l, r): (PathBuf, PathBuf),
    edit: impl FnOnce(&mut TableCompareRequest),
) -> ViewId {
    let mut req = TableCompareRequest {
        left: l.into_os_string(),
        right: r.into_os_string(),
        ..TableCompareRequest::default()
    };
    edit(&mut req);
    let open = GuiOpenRequest::for_table(&req, false);
    let resolved = world.service.resolve_open(&open).expect("resolve");
    let view = world
        .service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .expect("view request");
    let target = app.open_target();
    let id = app.open_local(
        resolved.kind,
        view,
        &resolved.options,
        resolved.specs,
        target,
    );
    world.settle(app);
    id
}

fn defaults_rules(world: &World) -> hikasa_session::api::TableSessionRules {
    match world
        .service
        .session_defaults(SessionType::TableCompare)
        .expect("defaults")
        .rules
    {
        SessionRules::Table(t) => *t,
        _ => panic!("not a table"),
    }
}

/// Opens the Session Settings of the active view, edits a tab, sets the scope and presses OK.
fn edit_settings(
    world: &World,
    app: &mut TestApp,
    scope: Scope,
    edit: impl FnOnce(&mut crate::session_dialog::SessionSettingsDialog),
) {
    app.execute(CommandId::Rules);
    world.settle(app);
    let w = app.win();
    let d = find::<crate::session_dialog::SessionSettingsDialog>(app).expect("settings");
    edit(d);
    d.scope = scope;
    let svc = world.service.clone();
    d.poll(&svc);
    assert!(d.tab_error().is_none(), "{:?}", d.tab_error());
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(app);
    world.settle(app);
}

// ---- F1, F2: the session tabs ------------------------------------------------------------------------------------

/// F1 (review probe 1): the view's own keys (`--key id --key name`) do not reach the defaults when only the skew is
/// changed with the scope *Update session defaults*; the view keeps them, one key an entry.
#[test]
fn f1_scope_defaults_take_only_the_edited_field() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.rules.rules.keys = vec!["id".into(), "name".into()];
    });
    assert!(table_model(&mut app, view).response().is_some());
    edit_settings(&world, &mut app, Scope::Defaults, |d| {
        d.tab_mut::<crate::table_tabs::RowsTab>()
            .expect("rows")
            .fields
            .skew = "40".into();
    });
    let defaults = defaults_rules(&world);
    assert!(
        defaults.rules.rules.keys.is_empty(),
        "the view's keys leaked into the defaults: {:?}",
        defaults.rules.rules.keys
    );
    assert_eq!(defaults.rules.rules.skew, Some(40));
    let m = table_model(&mut app, view);
    assert_eq!(m.request().rules.rules.keys, ["id", "name"]);
    assert_eq!(m.request().rules.rules.skew, Some(40));
}

/// F1 (review probe 1b): `--ignore-case name --ignore-case` keeps its order and stays out of the defaults when only
/// the default tolerance is edited.
#[test]
fn f1_column_rules_keep_their_order_and_their_scope() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.rules.rules.ignore_case = vec!["name".into(), String::new()];
    });
    assert!(table_model(&mut app, view).response().is_some());
    edit_settings(&world, &mut app, Scope::Defaults, |d| {
        d.tab_mut::<crate::table_tabs::ColumnsTab>()
            .expect("columns")
            .fields
            .default
            .tolerance = "0.5".into();
    });
    let defaults = defaults_rules(&world);
    assert!(
        defaults.rules.rules.ignore_case.is_empty(),
        "the view's case rules leaked into the defaults: {:?}",
        defaults.rules.rules.ignore_case
    );
    assert_eq!(defaults.rules.rules.tolerance, ["0.5"]);
    let m = table_model(&mut app, view);
    assert_eq!(m.request().rules.rules.ignore_case, ["name", ""]);
}

/// F2 (review probe 2): the Parsing tab keeps a one-space thousands separator when another field is edited, and
/// shows it as `space`.
#[test]
fn f2_the_parsing_tab_keeps_a_space_separator() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.parse.thousands = Some(" ".into());
        r.parse.delimiters = vec![",".into()];
    });
    assert!(table_model(&mut app, view).response().is_some());
    edit_settings(&world, &mut app, Scope::View, |d| {
        let tab = d
            .tab_mut::<crate::table_tabs::ParsingTab>()
            .expect("parsing");
        assert_eq!(tab.fields.thousands, "space");
        tab.fields.encoding = "utf-8".into();
    });
    let m = table_model(&mut app, view);
    assert_eq!(m.request().parse.encoding.as_deref(), Some("utf-8"));
    assert_eq!(m.request().parse.thousands.as_deref(), Some(" "));
}

// ---- F3: the comparison stops with its view ----------------------------------------------------------------------

/// A model whose comparison waits for its cancellation and reports it.
fn waiting_model(
    world: &World,
) -> (
    crate::table_view::TableViewModel,
    std::sync::mpsc::Receiver<()>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = parking_lot::Mutex::new(tx);
    let run = Arc::new(
        move |_: &hikasa_session::Service,
              job: &hikasa_session::JobControl,
              _: &TableCompareRequest| {
            let until = std::time::Instant::now() + Duration::from_secs(20);
            while !job.cancel.is_cancelled() && std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(2));
            }
            if job.cancel.is_cancelled() {
                let _ = tx.lock().send(());
            }
            Err(hikasa_session::ApiError::Cancelled)
        },
    );
    let model = crate::table_view::TableViewModel::with_runner_for_test(
        &world.service,
        TableCompareRequest {
            left: "a.csv".into(),
            right: "b.csv".into(),
            ..TableCompareRequest::default()
        },
        RowFilter::ALL,
        [None, None],
        Arc::new(|| {}),
        run,
    );
    (model, rx)
}

/// F3: a closed or replaced table view (dropped) cancels its comparison.
#[test]
fn f3_a_dropped_view_cancels_its_comparison() {
    let world = World::new();
    let (model, rx) = waiting_model(&world);
    assert!(model.busy());
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    drop(model);
    rx.recv_timeout(Duration::from_secs(10))
        .expect("the job saw its cancellation");
}

/// R28: a cancelled comparison says so, in the grid and the status bar.
#[test]
fn r28_a_cancelled_comparison_says_so() {
    let world = World::new();
    let (mut model, rx) = waiting_model(&world);
    model.cancel();
    rx.recv_timeout(Duration::from_secs(10)).expect("cancelled");
    for _ in 0..500 {
        if model.poll(&world.service) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(model.phase(), crate::table_view::TablePhase::Failed);
    assert_eq!(
        model.message().as_deref(),
        Some("The comparison was cancelled.")
    );
    assert_eq!(model.status_items().left, "Cancelled.");
}

// ---- F4 -----------------------------------------------------------------------------------------------------------

/// F4 (review probe 3): `table compare a b --gui` without words uses the words as typed (none), not the defaults'.
#[test]
fn f4_gui_without_words_is_not_the_defaults() {
    let world = World::new();
    let mut app = world.app(None);
    let mut defaults = world
        .service
        .session_defaults(SessionType::TableCompare)
        .expect("defaults");
    if let SessionRules::Table(t) = &mut defaults.rules {
        t.rules.rules.keys = vec!["id".into()];
    }
    world
        .service
        .session_save_defaults(&defaults)
        .expect("save");
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |_| {});
    let keys = &table_model(&mut app, view).request().rules.rules.keys;
    assert!(
        keys.is_empty(),
        "--gui without flags took the defaults' keys: {keys:?}"
    );
}

// ---- F5: Copy ------------------------------------------------------------------------------------------------------

/// F5 (review probe 10): Copy places the cell's full text, line breaks and all, not the grid's one-line, cut
/// drawing; a copied row quotes a field with a tab or a line break.
#[test]
fn f5_copy_keeps_the_cell_text() {
    let world = World::new();
    let mut app = world.app(None);
    let long = "x".repeat(600);
    let a = world.write(
        "work/c/a.csv",
        &format!("id,t,u\n1,\"line1\nline2\",q\n2,{long},q\n"),
    );
    let b = world.write(
        "work/c/b.csv",
        &format!("id,t,u\n1,\"line1\nline2!\",q\n2,{long}y,q\n"),
    );
    let view = open_words(&world, &mut app, (a, b), |_| {});
    let m = table_model(&mut app, view);
    m.click(0, 1, Side::Left);
    assert_eq!(m.copy_text().as_deref(), Some("line1\nline2"));
    m.click(1, 1, Side::Left);
    assert_eq!(m.copy_text().unwrap_or_default(), long);
    m.click_row(0, Side::Left);
    assert_eq!(m.copy_text().as_deref(), Some("1\t\"line1\nline2\"\tq"));
}

/// R12: a copied row leaves out the columns the view hides.
#[test]
fn r12_a_copied_row_leaves_hidden_columns_out() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.rules.rules.keys = vec!["id".into()];
    });
    let m = table_model(&mut app, view);
    m.hide_column(2);
    m.click_row(1, Side::Right);
    assert_eq!(m.copy_text().as_deref(), Some("2\tPear\ty"));
}

// ---- F10, F11: hidden columns ------------------------------------------------------------------------------------

/// F10 (review probe 4): of two columns with the same name, hiding one hides only it, the session keeps `x#2`, and a
/// width set on one is that column's.
#[test]
fn f10_columns_of_the_same_name_are_two_columns() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,x,x,note"), |_| {});
    let m = table_model(&mut app, view);
    assert_eq!(m.shown_columns(), &[0, 1, 2, 3]);
    m.hide_column(2);
    assert_eq!(m.shown_columns(), &[0, 1, 3]);
    m.set_width(1, 30.0);
    assert_eq!(m.column_head(1).expect("x").width, 30.0);
    assert_ne!(m.column_head(2).expect("x#2").width, 30.0);
    let keys = m.session_keys();
    let text = format!("{keys:?}");
    assert!(text.contains("/x#2"), "{text}");
    // The old key (the bare name) still reads, as the first column of that name.
    let mut state = hikasa_session::settings::State::default();
    let sheet = m.sheet_names()[0].clone();
    state.set_layout_value(
        "table-compare",
        "hidden_columns",
        hikasa_session::settings::state::LayoutValue::Texts(vec![format!("{sheet}/x")]),
    );
    m.apply_layout(&state, 13);
    assert_eq!(m.shown_columns(), &[0, 2, 3]);
}

/// F11 (review probe 5): next difference never lands on a column the rules hide.
#[test]
fn f11_navigation_skips_rule_hidden_columns() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.rules.rules.keys = vec!["id".into()];
        r.rules.rules.hide = vec!["price".into()];
        r.rules.rules.ignore_case = vec!["name".into()];
    });
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    assert_eq!(m.shown_columns(), &[0, 1, 3]);
    for _ in 0..4 {
        m.execute(&service, CommandId::DiffNext);
        let c = m.cursor().expect("cursor");
        assert!(m.shown_columns().contains(&c.col), "{c:?}");
    }
    m.execute(&service, CommandId::DiffItemNext);
    let c = m.cursor().expect("cursor");
    assert!(m.shown_columns().contains(&c.col), "{c:?}");
}

/// F11 (review probe 6): next difference does not show a column the user hid again.
#[test]
fn f11_navigation_never_edits_the_hidden_columns() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.rules.rules.keys = vec!["id".into()];
        r.rules.rules.ignore_case = vec!["name".into()];
    });
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    m.hide_column(2);
    for id in [
        CommandId::DiffNext,
        CommandId::DiffNext,
        CommandId::DiffItemNext,
    ] {
        m.execute(&service, id);
        assert_eq!(m.hidden_count(), 1, "{id:?}");
        assert!(!m.shown_columns().contains(&2));
        assert_ne!(m.cursor().expect("cursor").col, 2);
    }
    // Go To a hidden column lands on a shown one.
    assert!(m.go_to(None, 3, Some(2)));
    assert_ne!(m.cursor().expect("cursor").col, 2);
    assert_eq!(m.hidden_count(), 1);
}

// ---- the surviving mutations -------------------------------------------------------------------------------------

/// A keyed pair whose id 2 differs in `name` (unimportant) and `price`, id 5 only in `name`.
fn unimportant_pair(world: &World, app: &mut TestApp) -> ViewId {
    let a = world.write(
        "work/u/a.csv",
        "id,name,price\n1,apple,1\n2,pear,2\n5,kiwi,5\n",
    );
    let b = world.write(
        "work/u/b.csv",
        "id,name,price\n1,apple,1\n2,PEAR,3\n5,KIWI,5\n",
    );
    open_words(world, app, (a, b), |r| {
        r.rules.rules.keys = vec!["id".into()];
        r.rules.rules.unimportant = vec!["name".into()];
    })
}

/// R2: the thumbnail marks a row of unimportant differences as same under ignore-unimportant.
#[test]
fn r2_the_thumbnail_honours_ignore_unimportant() {
    let world = World::new();
    let mut app = world.app(None);
    let view = unimportant_pair(&world, &mut app);
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    assert_eq!(m.thumbnail(3), [0, 2, 1]);
    m.execute(&service, CommandId::InsigIgnored);
    assert_eq!(m.thumbnail(3), [0, 2, 0]);
}

/// R4: under ignore-unimportant, next difference row lands on the row's first important cell.
#[test]
fn r4_the_next_row_lands_on_an_important_cell_under_ignore_unimportant() {
    let world = World::new();
    let mut app = world.app(None);
    let view = unimportant_pair(&world, &mut app);
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    m.execute(&service, CommandId::DiffItemNext);
    assert_eq!(m.cursor().map(|c| (c.row, c.col)), Some((1, 1)));
    m.execute(&service, CommandId::InsigIgnored);
    m.set_cursor_for_test(None);
    m.execute(&service, CommandId::DiffItemNext);
    assert_eq!(m.cursor().map(|c| (c.row, c.col)), Some((1, 2)));
    m.execute(&service, CommandId::DiffItemNext);
    assert_eq!(
        m.cursor().map(|c| c.row),
        Some(1),
        "id 5 is no difference now"
    );
}

/// R14: the outcome of a view is its verdict under the view's ignore-unimportant.
#[test]
fn r14_the_outcome_honours_ignore_unimportant() {
    let world = World::new();
    let mut app = world.app(None);
    let a = world.write("work/o/a.csv", "id,name\n1,pear\n");
    let b = world.write("work/o/b.csv", "id,name\n1,PEAR\n");
    let view = open_words(&world, &mut app, (a, b), |r| {
        r.rules.rules.keys = vec!["id".into()];
        r.rules.rules.unimportant = vec!["name".into()];
    });
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    let before = m.outcome(&service);
    m.execute(&service, CommandId::InsigIgnored);
    let after = m.outcome(&service);
    assert_ne!(format!("{before:?}"), format!("{after:?}"));
    assert!(format!("{after:?}").contains("RulesSame"), "{after:?}");
}

/// R3: next difference skips a differing row the filter hides (changed rows hidden: the orphans are next).
#[test]
fn r3_next_difference_skips_rows_the_filter_hides() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |r| {
        r.rules.rules.keys = vec!["id".into()];
    });
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    m.execute(&service, CommandId::TogDiff);
    m.execute(&service, CommandId::DiffNext);
    let c = m.cursor().expect("cursor");
    assert!(m.position_of(c.row).is_some(), "{c:?}");
    assert_eq!(c.row, 2, "the left-only row");
    m.execute(&service, CommandId::DiffNext);
    assert_eq!(m.cursor().expect("cursor").row, 3);
    m.execute(&service, CommandId::DiffPrev);
    assert_eq!(m.cursor().expect("cursor").row, 2);
    m.execute(&service, CommandId::DiffPrev);
    assert_eq!(
        m.cursor().expect("cursor").row,
        2,
        "nothing before it is shown"
    );
}

/// R6, R7: Go To finds a label that only the right side has, and a column by its name in any case.
#[test]
fn r6_r7_go_to_finds_right_labels_and_any_case() {
    let world = World::new();
    let mut app = world.app(None);
    let a = world.write("work/g/a.csv", "id,name\n1,a\n2,b\n");
    let b = world.write("work/g/b.csv", "id,name\n1,a\n2,b\n3,c\n4,d\n");
    let view = open_words(&world, &mut app, (a, b), |r| {
        r.rules.rules.keys = vec!["id".into()];
    });
    let m = table_model(&mut app, view);
    assert_eq!(m.column_named("NAME"), Some(1));
    assert_eq!(m.column_named("b"), Some(1), "the letters, lower case");
    assert!(m.go_to(None, 5, None), "line 5 exists on the right only");
    assert_eq!(m.cursor().expect("cursor").row, 3);
}

/// R8, R9, R27 over the two workbooks: a filter change reaches every sheet, a preset keeps ignore-unimportant, and
/// Show Hidden Columns shows the active sheet's only.
#[test]
fn r8_r9_r27_filters_and_hidden_columns_per_sheet() {
    let world = World::new();
    let mut app = world.app(None);
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/table");
    let copy = |name: &str| {
        let to = world.path(&format!("work/books/{name}"));
        std::fs::create_dir_all(to.parent().expect("parent")).expect("mkdir");
        std::fs::copy(dir.join(name), &to).expect("copy");
        to
    };
    let view = open_words(
        &world,
        &mut app,
        (copy("books-left.xlsx"), copy("books-right.xlsx")),
        |_| {},
    );
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    // R8: sheet 1 shown first, then the filter changes on sheet 0.
    m.set_sheet(1);
    let all = m.len();
    m.hide_column(1);
    m.set_sheet(0);
    m.hide_column(0);
    m.execute(&service, CommandId::FavSame);
    m.set_sheet(1);
    let expected = m.response().expect("ready").comparison.sheets()[1]
        .visible(&RowFilter::SAME)
        .count() as u32;
    assert!(expected < all, "the sheet has differing rows");
    assert_eq!(m.len(), expected);
    // R27: Show Hidden Columns on sheet 1 keeps sheet 0's.
    m.show_all_columns();
    assert_eq!(m.hidden_count(), 0);
    m.set_sheet(0);
    assert_eq!(m.hidden_count(), 1);
    // R9: a preset keeps ignore-unimportant.
    m.execute(&service, CommandId::InsigIgnored);
    m.execute(&service, CommandId::FavCompareAll);
    assert!(m.filter().ignore_unimportant);
    assert_eq!(m.checked(CommandId::FavCompareAll), Some(true));
}

/// R11: the layout key of `[layout.table-compare]` is read.
#[test]
fn r11_the_layout_key_is_read() {
    let world = World::new();
    let mut app = world.app(None);
    let view = open_words(&world, &mut app, pair(&world, "id,name,price,note"), |_| {});
    let m = table_model(&mut app, view);
    let mut state = hikasa_session::settings::State::default();
    state.set_layout_value(
        "table-compare",
        "layout",
        hikasa_session::settings::state::LayoutValue::Text("over-under".into()),
    );
    m.apply_layout(&state, 13);
    assert_eq!(m.layout(), crate::table_view::GridLayout::OverUnder);
}

/// R13: the address of a right cell uses the right side's own column letters.
#[test]
fn r13_the_right_address_uses_the_right_letters() {
    let world = World::new();
    let mut app = world.app(None);
    let a = world.write("work/r/a.csv", "id,name,price\n1,a,1\n");
    let b = world.write("work/r/b.csv", "id,price,name\n1,2,a\n");
    let view = open_words(&world, &mut app, (a, b), |r| {
        r.rules.rules.keys = vec!["id".into()];
    });
    let m = table_model(&mut app, view);
    let price = m.column_named("price").expect("price");
    m.click(0, price, Side::Left);
    assert_eq!(m.address().as_deref(), Some("C2"));
    m.click(0, price, Side::Right);
    assert_eq!(m.address().as_deref(), Some("B2"));
}

/// F13: a sheet of unimportant differences shows the same spot as a same sheet under ignore-unimportant.
#[test]
fn f13_tab_spots_honour_ignore_unimportant() {
    let world = World::new();
    let mut app = world.app(None);
    let view = unimportant_pair(&world, &mut app);
    let service = world.service.clone();
    let m = table_model(&mut app, view);
    assert_eq!(m.sheet_tabs()[0].state, SheetState::Important);
    let a = world.write("work/v/a.csv", "id,name\n1,pear\n");
    let b = world.write("work/v/b.csv", "id,name\n1,PEAR\n");
    let view = open_words(&world, &mut app, (a, b), |r| {
        r.rules.rules.keys = vec!["id".into()];
        r.rules.rules.unimportant = vec!["name".into()];
    });
    let m = table_model(&mut app, view);
    assert_eq!(m.sheet_tabs()[0].state, SheetState::Unimportant);
    m.execute(&service, CommandId::InsigIgnored);
    assert_eq!(m.sheet_tabs()[0].state, SheetState::Same);
}

// ---- F6: the report dialog ---------------------------------------------------------------------------------------

/// Presses the report dialog's `role` and waits for its answer.
fn report_answer(world: &World, app: &mut TestApp, role: ButtonRole) -> Option<(bool, String)> {
    let w = app.win();
    if let Some(d) = find::<crate::table_dialogs::TableReportDialog>(app) {
        d.message = None;
    }
    app.0.dialog_button(w, role);
    for _ in 0..400 {
        world.settle(app);
        if find::<crate::table_dialogs::TableReportDialog>(app).is_some_and(|d| d.message.is_some())
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    find::<crate::table_dialogs::TableReportDialog>(app).and_then(|d| d.message.clone())
}

/// F6 (review probe 7): a relative file is in the home folder (then in the last report's folder) and the dialog
/// names it; an existing file is replaced only after *Replace*; `-` and `clipboard://` are refused; the report shows
/// the view's display and ignore-unimportant; R19, R20: row numbers reach the report, the HTML choice is
/// remembered as made.
#[test]
fn f6_the_report_dialog_names_its_file_and_asks_before_replacing() {
    let world = World::new();
    let mut app = world.app(None);
    let view = unimportant_pair(&world, &mut app);
    let service = world.service.clone();
    {
        let m = table_model(&mut app, view);
        m.execute(&service, CommandId::InsigIgnored);
        m.execute(&service, CommandId::FavSame);
    }
    let w = app.win();
    app.execute(CommandId::Report);
    let home = world.path("home");
    {
        let d = find::<crate::table_dialogs::TableReportDialog>(&mut app).expect("report");
        assert_eq!(d.display, RowFilter::SAME, "the view's display");
        assert_eq!(d.folder.as_deref(), Some(home.as_path()));
        d.path = "rel-report.txt".into();
        d.html = false;
        d.row_numbers = false;
        d.layout = "side-by-side";
        // R19: the row numbers reach the report's flags (`--no-line-numbers`, `--line-numbers`).
        assert_eq!(d.flags().line_numbers, Some(false));
        d.row_numbers = true;
        assert_eq!(d.flags().line_numbers, Some(true));
        d.row_numbers = false;
        assert_eq!(d.file(), Ok(home.join("rel-report.txt")));
    }
    let (ok, text) = report_answer(&world, &mut app, ButtonRole::Ok).expect("answer");
    assert!(ok, "{text}");
    assert!(
        text.contains(&home.join("rel-report.txt").display().to_string()),
        "{text}"
    );
    assert!(
        !world.path("work/rel-report.txt").exists(),
        "never the GUI's folder"
    );
    let report = std::fs::read_to_string(home.join("rel-report.txt")).expect("written");
    // Under ignore-unimportant, id 5 (unimportant only) is a same row: the Same rows show it.
    assert!(
        report.contains("kiwi") || report.contains("KIWI"),
        "{report}"
    );
    assert!(!report.contains("PEAR"), "id 2 differs: {report}");
    // The same file again: the dialog asks, the file is kept until Replace.
    std::fs::write(home.join("rel-report.txt"), "precious").expect("write");
    let (ok, text) = report_answer(&world, &mut app, ButtonRole::Ok).expect("answer");
    assert!(!ok && text.contains("exists"), "{text}");
    assert_eq!(
        std::fs::read_to_string(home.join("rel-report.txt")).expect("kept"),
        "precious"
    );
    {
        let d = find::<crate::table_dialogs::TableReportDialog>(&mut app).expect("report");
        assert!(d.replacing());
        assert_eq!(d.buttons()[0].label, "Replace");
    }
    let (ok, text) = report_answer(&world, &mut app, ButtonRole::Ok).expect("answer");
    assert!(ok, "{text}");
    assert_ne!(
        std::fs::read_to_string(home.join("rel-report.txt")).expect("replaced"),
        "precious"
    );
    for refused in ["-", "clipboard://"] {
        find::<crate::table_dialogs::TableReportDialog>(&mut app)
            .expect("report")
            .path = refused.into();
        let (ok, _) = report_answer(&world, &mut app, ButtonRole::Ok).expect("answer");
        assert!(!ok, "{refused}");
    }
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    // R20: the HTML choice made (plain text) is remembered; the folder of the last report too.
    app.execute(CommandId::Report);
    let d = find::<crate::table_dialogs::TableReportDialog>(&mut app).expect("report");
    assert!(!d.html, "the plain-text choice was remembered");
    assert_eq!(d.folder.as_deref(), Some(home.as_path()));
    d.path = "numbers.txt".into();
    d.row_numbers = true;
    let _ = report_answer(&world, &mut app, ButtonRole::Ok);
    // R19: the row numbers are what the check box says.
    let without = std::fs::read_to_string(home.join("rel-report.txt")).expect("first");
    let with = std::fs::read_to_string(home.join("numbers.txt")).expect("second");
    assert_ne!(
        without, with,
        "row numbers on and off give different reports"
    );
}
