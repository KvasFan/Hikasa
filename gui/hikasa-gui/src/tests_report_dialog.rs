//! M2 W13c acceptance A13.17, A13.18, A13.19 (D-366): the report dialog of every view kind through the models. The
//! bytes the dialog writes (a file, the clipboard, the print and browser files) equal the CLI's report of the same
//! comparison with the same words (the Service's `report_request` + `report_plan` + `report_execute` over a headless
//! comparison, as `hikasa <kind> compare … --report …` and `hikasa report text|folder` run them), with
//! `SOURCE_DATE_EPOCH` fixed; the display default is the view's filter and is never remembered; *Just selection*;
//! refusals; Stop; remembered choices per kind; the table's phase-3 migration; relative paths; the folder report
//! with file links; View in Browser and Print Preview; the edited text view and the changed folder view.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hikasa_core::{JobKind, Side};
use hikasa_report::ReportKind;
use hikasa_report::vocabulary::vocabulary;
use hikasa_session::api::{
    FolderCompareRequest, HexCompareRequest, ReportFlags, ReportSource, ReportTarget,
    TableCompareRequest, TextCompareRequest,
};
use hikasa_session::clipboard::FixedClipboard;
use hikasa_session::launcher::{Recorded, RecordingLauncher};
use hikasa_text::{EditRequest, TextPos};

use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::report_dialog::{BROWSER_BUTTON, PRINT_BUTTON, ReportAction, ReportDialog, ReportTo};
use crate::tests::{TestApp, World, text_model};
use crate::tests_sync_view::find;

const EPOCH: &str = "1700000000";

/// A world with the report time fixed, a recording launcher (answers `launches`) and a Hex format.
fn world_with(launches: Vec<Option<i32>>) -> (World, Arc<RecordingLauncher>) {
    let launcher = Arc::new(RecordingLauncher::with_answers(launches));
    let l = Arc::clone(&launcher);
    let world = World::with_files_and_init(
        &[("formats/firmware.toml", crate::tests_hex_open::HEX_FORMAT)],
        move |init| {
            init.env.source_date_epoch = Some(EPOCH.to_owned());
            init.launcher = l;
        },
    );
    (world, launcher)
}

fn world() -> World {
    world_with(Vec::new()).0
}

/// An app with a fixed clipboard.
fn app_of(world: &World) -> (TestApp, Arc<FixedClipboard>) {
    let mut app = world.app(None);
    let clip = Arc::new(FixedClipboard::text("unused"));
    app.0.set_script_clipboard(Arc::clone(&clip) as _);
    (app, clip)
}

fn os(p: &Path) -> OsString {
    p.as_os_str().to_owned()
}

fn job(world: &World, kind: JobKind) -> hikasa_session::JobControl {
    world.service.job(kind, Arc::new(hikasa_core::NullProgress))
}

/// Presses `role` of the report dialog and waits for its answer.
fn press(world: &World, app: &mut TestApp, role: ButtonRole) -> (bool, String) {
    let w = app.win();
    if let Some(d) = find::<ReportDialog>(app) {
        d.message = None;
    }
    app.0.dialog_button(w, role);
    for _ in 0..1000 {
        world.settle(app);
        if let Some(d) = find::<ReportDialog>(app)
            && !d.running()
            && !d.waiting()
            && d.message.is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    find::<ReportDialog>(app)
        .and_then(|d| d.message.clone())
        .expect("an answer")
}

/// The CLI's report of `source` with `flags` into `file`: its bytes.
fn cli_bytes(world: &World, source: ReportSource<'_>, flags: &ReportFlags, file: &Path) -> Vec<u8> {
    let target = ReportTarget::File(os(file));
    let mut req = world
        .service
        .report_request(source.kind(), "side-by-side", flags, &target)
        .expect("request");
    let control = job(world, JobKind::Report);
    let plan = world
        .service
        .report_plan(source, &mut req, &target, flags.select.as_ref(), &control)
        .expect("plan");
    world
        .service
        .report_execute(source, &req, plan, None, &control)
        .expect("execute");
    std::fs::read(file).expect("written")
}

/// The headless comparisons of each kind, as the CLI runs them.
enum Cli {
    Text(Box<hikasa_session::api::TextCompareResponse>),
    Folder(
        Box<FolderCompareRequest>,
        Box<hikasa_session::api::PreparedFolderCompare>,
        Box<hikasa_session::api::FolderCompareResponse>,
    ),
    Table(Box<hikasa_session::api::TableCompareResponse>),
    Hex(Box<hikasa_session::api::HexCompareResponse>),
    Picture(Box<hikasa_session::api::PictureCompareResponse>),
}

impl Cli {
    fn of(world: &World, kind: ReportKind, (l, r): (&Path, &Path)) -> Cli {
        match kind {
            ReportKind::Text => {
                let req = TextCompareRequest {
                    left: os(l),
                    right: os(r),
                    ..TextCompareRequest::default()
                };
                Cli::Text(Box::new(
                    world
                        .service
                        .text_compare(&req, &job(world, JobKind::TextCompare), None)
                        .expect("text"),
                ))
            }
            ReportKind::Folder => {
                let req = FolderCompareRequest {
                    left: os(l),
                    right: os(r),
                    ..FolderCompareRequest::default()
                };
                let prepared = world.service.folder_prepare(&req).expect("prepare");
                let resp = world
                    .service
                    .folder_run(
                        &prepared,
                        &job(world, JobKind::FolderCompare),
                        &hikasa_folder::NullObserver,
                    )
                    .expect("run");
                Cli::Folder(Box::new(req), Box::new(prepared), Box::new(resp))
            }
            ReportKind::Table => {
                let req = TableCompareRequest {
                    left: os(l),
                    right: os(r),
                    ..TableCompareRequest::default()
                };
                Cli::Table(Box::new(
                    world
                        .service
                        .table_compare(&req, &job(world, JobKind::TableCompare))
                        .expect("table"),
                ))
            }
            ReportKind::Hex => {
                let req = HexCompareRequest {
                    left: os(l),
                    right: os(r),
                    ..HexCompareRequest::default()
                };
                Cli::Hex(Box::new(
                    world
                        .service
                        .hex_compare(&req, &job(world, JobKind::HexCompare))
                        .expect("hex"),
                ))
            }
            ReportKind::Picture => Cli::Picture(Box::new(crate::tests_picture_open::cli(
                world,
                (&l.to_path_buf(), &r.to_path_buf()),
                |_| {},
            ))),
        }
    }

    fn source(&self) -> ReportSource<'_> {
        match self {
            Cli::Text(r) => ReportSource::Text(r),
            Cli::Folder(request, prepared, response) => ReportSource::Folder {
                request,
                prepared,
                response,
            },
            Cli::Table(r) => ReportSource::Table(r),
            Cli::Hex(r) => ReportSource::Hex(r),
            Cli::Picture(r) => ReportSource::Picture(r),
        }
    }
}

/// Opens a view of `kind` over generated inputs; returns the pair.
fn open_kind(world: &World, app: &mut TestApp, kind: ReportKind) -> (PathBuf, PathBuf) {
    let pair = match kind {
        ReportKind::Text => crate::tests::text_pair(world),
        ReportKind::Folder => crate::tests::folder_pair(world),
        ReportKind::Table => (
            world.write(
                "work/t/a.csv",
                "id,name,price\n1,apple,1.5\n2,pear,2\n3,plum,0.75\n",
            ),
            world.write(
                "work/t/b.csv",
                "id,name,price\n1,apple,1.5\n2,Pear,2.5\n4,fig,3\n",
            ),
        ),
        ReportKind::Hex => {
            let a = crate::tests_hex_open::bytes(7, 3000);
            let b = crate::tests_hex_open::with_insert(&a, 700, 5, 2000);
            crate::tests_hex_open::write_pair(world, &a, &b)
        }
        ReportKind::Picture => crate::tests_picture_open::pair(world),
    };
    world.open(app, &pair.0, &pair.1);
    world.settle(app);
    pair
}

const KINDS: [ReportKind; 5] = [
    ReportKind::Text,
    ReportKind::Folder,
    ReportKind::Table,
    ReportKind::Hex,
    ReportKind::Picture,
];

/// A13.17: for each kind, every layout with each output its form offers (and a custom stylesheet): the bytes of
/// *Write Report* to a file and to the clipboard equal the CLI's report with the same words.
#[test]
fn a13_17_every_kind_and_layout_writes_the_cli_bytes() {
    for kind in KINDS {
        let world = world();
        let (mut app, clip) = app_of(&world);
        let pair = open_kind(&world, &mut app, kind);
        let cli = Cli::of(&world, kind, (&pair.0, &pair.1));
        let css = world.write("home/style.css", "body { color: teal; }\n");
        app.execute(CommandId::Report);
        assert!(find::<ReportDialog>(&mut app).is_some(), "{kind:?}");
        for (n, layout) in vocabulary(kind).layouts.iter().enumerate() {
            let outputs: Vec<&str> = {
                let d = find::<ReportDialog>(&mut app).expect("dialog");
                d.layout = layout.word;
                d.outputs().iter().map(|o| o.word).collect()
            };
            let mut cases: Vec<Option<&str>> = outputs.into_iter().map(Some).collect();
            if cases.is_empty() {
                cases.push(None);
            }
            for (k, output) in cases.into_iter().enumerate() {
                let file = world.path(&format!("home/r-{n}-{k}.out"));
                let flags = {
                    let d = find::<ReportDialog>(&mut app).expect("dialog");
                    d.to = ReportTo::File;
                    d.path = file.display().to_string();
                    if let Some(o) = output {
                        d.output = o;
                        d.css = css.display().to_string();
                    }
                    d.flags(ReportAction::Write)
                };
                let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
                assert!(ok, "{kind:?} {} {output:?}: {text}", layout.word);
                let expected = cli_bytes(
                    &world,
                    cli.source(),
                    &flags,
                    &world.path(&format!("home/cli-{n}-{k}.out")),
                );
                let written = std::fs::read(&file).expect("written");
                assert_eq!(
                    String::from_utf8_lossy(&written),
                    String::from_utf8_lossy(&expected),
                    "{kind:?} {} {output:?}",
                    layout.word
                );
                // The clipboard gets the same report (its text, or its HTML).
                find::<ReportDialog>(&mut app).expect("dialog").to = ReportTo::Clipboard;
                let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
                assert!(ok, "{kind:?} {} clipboard: {text}", layout.word);
                let payload = clip.payloads().last().cloned().expect("a payload");
                let placed = payload.html.unwrap_or(payload.text);
                assert_eq!(
                    placed,
                    String::from_utf8_lossy(&expected),
                    "{kind:?} {} clipboard",
                    layout.word
                );
            }
        }
    }
}

/// A13.17: the dialog's words of each kind's default layout, as the CLI would be given them.
#[test]
fn a13_17_the_words_of_each_default_layout() {
    // One string per kind (no table of vocabulary words in the GUI's sources, check 7).
    let expected = [
        (ReportKind::Text, "line-numbers display-all"),
        (
            ReportKind::Folder,
            "column-size column-timestamp display-all",
        ),
        (ReportKind::Table, "line-numbers display-all"),
        (
            ReportKind::Hex,
            "line-numbers context-0 display-differences",
        ),
        (ReportKind::Picture, "mode-tolerance checkerboard"),
    ];
    for (kind, words) in expected {
        let world = world();
        let (mut app, _) = app_of(&world);
        open_kind(&world, &mut app, kind);
        if kind == ReportKind::Folder {
            app.execute(CommandId::FavCompareAll);
        }
        app.execute(CommandId::Report);
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        let flags = d.flags(ReportAction::Write);
        assert_eq!(
            flags.words.layout.as_deref(),
            Some("side-by-side"),
            "{kind:?}"
        );
        assert_eq!(flags.words.options.join(" "), words, "{kind:?}");
        assert_eq!(
            flags.words.output_options.first().map(String::as_str),
            Some("html-color"),
            "{kind:?}"
        );
    }
}

/// A13.17: the display default is the view's filter for each kind and is never remembered (the mutation "the
/// display default taken from the remembered choice" fails here).
#[test]
fn a13_17_the_display_default_is_the_views_and_never_remembered() {
    // The display word after each filter, one string (check 7).
    let words = "display-mismatches display-matches display-matches display-same";
    let cases = [
        (ReportKind::Text, CommandId::FavDiff),
        (ReportKind::Folder, CommandId::FavSame),
        (ReportKind::Table, CommandId::FavSame),
        (ReportKind::Hex, CommandId::FavSame),
    ]
    .into_iter()
    .zip(words.split_whitespace())
    .map(|((kind, filter), word)| (kind, filter, word));
    for (kind, filter, word) in cases {
        let world = world();
        let (mut app, _) = app_of(&world);
        open_kind(&world, &mut app, kind);
        app.execute(CommandId::FavCompareAll);
        world.settle(&mut app);
        app.execute(CommandId::Report);
        let file = world.path("home/first.out");
        {
            let d = find::<ReportDialog>(&mut app).expect("dialog");
            let all = vocabulary(kind).displays[0].word;
            assert_eq!(d.display, Some(all), "{kind:?}: the view shows all");
            // A choice other than the view's, written and remembered.
            d.display = Some(vocabulary(kind).displays[1].word);
            d.path = file.display().to_string();
        }
        let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
        assert!(ok, "{kind:?}: {text}");
        let w = app.win();
        app.0.dialog_button(w, ButtonRole::Close);
        world.settle(&mut app);
        let remembered = world.service.report_choices(kind);
        assert!(
            remembered
                .options
                .iter()
                .all(|o| !o.starts_with("display-")),
            "{kind:?}: {remembered:?}"
        );
        app.execute(filter);
        world.settle(&mut app);
        app.execute(CommandId::Report);
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert_eq!(d.display, Some(word), "{kind:?}: the view's filter again");
        assert_eq!(
            d.path,
            file.display().to_string(),
            "{kind:?}: the file is remembered"
        );
    }
}

/// A13.17: remembered choices per kind round trip through `state.toml`: layout, options, output, the file; the
/// title is not remembered.
#[test]
fn a13_17_remembered_choices_round_trip_per_kind() {
    for kind in KINDS {
        let world = world();
        let (mut app, _) = app_of(&world);
        open_kind(&world, &mut app, kind);
        app.execute(CommandId::Report);
        let vocab = vocabulary(kind);
        let layout = vocab.layouts.last().expect("a layout");
        let (output, options) = {
            let d = find::<ReportDialog>(&mut app).expect("dialog");
            d.layout = layout.word;
            let output = d.outputs().last().map(|o| o.word);
            if let Some(o) = output {
                d.output = o;
            }
            d.title = "Not remembered".into();
            d.path = "kept.out".into();
            (output, d.options.clone())
        };
        let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
        assert!(ok, "{kind:?}: {text}");
        let w = app.win();
        app.0.dialog_button(w, ButtonRole::Close);
        world.settle(&mut app);
        let state = std::fs::read_to_string(world.path("config/state.toml")).expect("state");
        assert!(
            state.contains(&format!("[dialogs.report-{}]", kind.as_str())),
            "{state}"
        );
        app.execute(CommandId::Report);
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert_eq!(d.layout, layout.word, "{kind:?}");
        if let Some(o) = output {
            assert_eq!(d.output, o, "{kind:?}");
        }
        assert_eq!(d.path, "kept.out");
        assert!(d.title.is_empty(), "the title is not remembered");
        // The options remembered are the ones written (the view's words aside).
        for word in &options {
            if !word.starts_with("mode-") && !word.starts_with("context-") {
                assert!(d.options.contains(word), "{kind:?}: {word}");
            }
        }
    }
}

/// A13.17: the table's phase-3 state (`html = false`, `folder`) is read once: plain text and the folder of relative
/// paths; the next remember replaces the table without them (the mutation "the table dialog's `html = false` read as
/// HTML" fails here).
#[test]
fn a13_17_the_table_migration_of_html_and_folder() {
    let world = world();
    let folder = world.path("work/reports");
    std::fs::create_dir_all(&folder).expect("mkdir");
    std::fs::write(
        world.path("config/state.toml"),
        format!(
            "schema_version = 1\n\n[dialogs.report-table]\nlayout = \"interleaved\"\nhtml = \"false\"\nfolder = {:?}\n",
            folder.display().to_string()
        ),
    )
    .expect("state");
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Table);
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert_eq!(d.layout, "interleaved");
        assert_eq!(d.output, "text", "html = false is plain text");
        assert_eq!(d.folder.as_deref(), Some(folder.as_path()));
        d.path = "table.txt".into();
        assert_eq!(d.file(), Ok(folder.join("table.txt")));
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    assert!(folder.join("table.txt").is_file());
    world.settle(&mut app);
    let state = std::fs::read_to_string(world.path("config/state.toml")).expect("state");
    let table = state
        .split("[dialogs.report-table]")
        .nth(1)
        .expect("the table's choices");
    let table = table.split("\n[").next().unwrap_or(table);
    assert!(!table.contains("html ="), "{state}");
    assert!(!table.contains("folder ="), "{state}");
    assert!(table.contains("output_to = \"file\""), "{state}");
}

/// A13.17: a relative file is in the home folder, never in the GUI's current folder; `-` and `clipboard://` are
/// refused; an existing file is replaced after *Replace* only.
#[test]
fn a13_17_relative_paths_and_the_replace_prompt() {
    let world = world();
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    let home = world.path("home");
    find::<ReportDialog>(&mut app).expect("dialog").path = "rel.html".into();
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    assert!(home.join("rel.html").is_file());
    assert!(!world.path("work/rel.html").exists(), "never the cwd");
    std::fs::write(home.join("rel.html"), "precious").expect("write");
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(!ok && text.contains("exists"), "{text}");
    assert_eq!(
        std::fs::read_to_string(home.join("rel.html")).expect("kept"),
        "precious"
    );
    assert_eq!(
        find::<ReportDialog>(&mut app).expect("dialog").buttons()[0].label,
        "Replace"
    );
    let (ok, _) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok);
    assert_ne!(
        std::fs::read_to_string(home.join("rel.html")).expect("replaced"),
        "precious"
    );
    for refused in ["-", "clipboard://"] {
        find::<ReportDialog>(&mut app).expect("dialog").path = refused.into();
        let (ok, _) = press(&world, &mut app, ButtonRole::Ok);
        assert!(!ok, "{refused}");
    }
}

/// A13.17: a refused combination greys the actions with the core's message; F1 is the dialog's topic.
#[test]
fn a13_17_a_refused_combination_greys_the_actions() {
    let world = world();
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Folder);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    assert_eq!(d.help_topic(), "dialog-report");
    d.layout = "side-by-side";
    d.options.insert("column-none");
    d.check(&world.service);
    let why = d.refusal().expect("refused").to_owned();
    assert!(why.contains("column"), "{why}");
    assert!(d.buttons().iter().take(3).all(|b| !b.enabled));
    d.options.remove("column-none");
    d.check(&world.service);
    assert!(d.refusal().is_none());
    assert!(d.buttons()[0].enabled);
}

/// A13.17: *Just selection* of text lines equals `--lines`, of folder rows `--select`.
#[test]
fn a13_17_just_selection_of_lines_and_rows() {
    // Text: lines 10–12 of the left pane.
    let world = world();
    let (mut app, _) = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Text);
    let view = app.active_tab().expect("tab").id;
    text_model(&mut app, view)
        .pane_mut(Side::Left)
        .select(TextPos::new(9, 0), TextPos::new(11, 3));
    app.execute(CommandId::Report);
    let file = world.path("home/lines.txt");
    let flags = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert!(d.can_scope());
        d.just_selection = true;
        d.output = "text";
        d.path = file.display().to_string();
        d.flags(ReportAction::Write)
    };
    assert_eq!(flags.lines.as_deref(), Some("left:10-12"));
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    let cli = Cli::of(&world, ReportKind::Text, (&pair.0, &pair.1));
    let expected = cli_bytes(
        &world,
        cli.source(),
        &flags,
        &world.path("home/lines-cli.txt"),
    );
    assert_eq!(std::fs::read(&file).expect("written"), expected);
    // A layout without rows takes no scope.
    find::<ReportDialog>(&mut app).expect("dialog").layout = "summary";
    assert!(!find::<ReportDialog>(&mut app).expect("dialog").can_scope());
    // Folders: one selected row.
    let world = world_with(Vec::new()).0;
    let (mut app, _) = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Folder);
    let view = app.active_tab().expect("tab").id;
    app.execute(CommandId::FavCompareAll);
    app.execute(CommandId::ExpandAll);
    crate::tests_folder_ops::select(&mut app, view, "same.txt");
    app.execute(CommandId::Report);
    let file = world.path("home/rows.txt");
    let flags = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.layout = "list";
        assert!(d.can_scope());
        d.just_selection = true;
        d.path = file.display().to_string();
        d.flags(ReportAction::Write)
    };
    assert!(flags.select.is_some());
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    let written = std::fs::read_to_string(&file).expect("written");
    assert!(written.contains("same.txt"), "{written}");
    assert!(!written.contains("main.c"), "only the selection: {written}");
    let cli = Cli::of(&world, ReportKind::Folder, (&pair.0, &pair.1));
    let expected = cli_bytes(
        &world,
        cli.source(),
        &flags,
        &world.path("home/rows-cli.txt"),
    );
    assert_eq!(written.as_bytes(), expected.as_slice());
}

/// A13.17: the folder report with links to file reports compares each pair and writes their reports.
#[test]
fn a13_17_the_folder_report_with_file_links() {
    let world = world();
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Folder);
    app.execute(CommandId::Report);
    let file = world.path("home/linked.html");
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.layout = "side-by-side";
        d.options.insert("include-file-links");
        d.path = file.display().to_string();
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    let html = std::fs::read_to_string(&file).expect("written");
    assert!(html.contains("<a href="), "{html}");
    let files = world.path("home/linked_files");
    let pairs: Vec<_> = std::fs::read_dir(&files)
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(!pairs.is_empty(), "the pair reports are written");
}

/// A13.17: Stop of a report that waits for a text view's recompare.
#[test]
fn a13_17_stop_while_waiting() {
    let world = world();
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    let view = app.active_tab().expect("tab").id;
    edit_left(&mut app, view, "X");
    app.execute(CommandId::Report);
    let svc = world.service.clone();
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    d.path = world.path("home/w.html").display().to_string();
    if d.wants_text_source() {
        d.on_button(ButtonRole::Ok, &svc);
        assert!(d.waiting());
        assert_eq!(d.buttons()[3].label, "Stop");
        d.on_button(ButtonRole::Close, &svc);
        assert!(!d.waiting());
        assert_eq!(
            d.message.as_ref().map(|m| m.1.as_str()),
            Some("The report was stopped.")
        );
    }
}

/// Types `text` at the start of the left pane's line 2.
fn edit_left(app: &mut TestApp, view: hikasa_session::ipc::ViewId, text: &str) {
    let at = TextPos::new(1, 0);
    let m = text_model(app, view);
    assert!(crate::text::edit::edit(
        m,
        Side::Left,
        EditRequest::Replace {
            range: at..at,
            text: text.into(),
            keep_eols: false,
        },
        crate::text::edit::Origin::Typing { at_ms: 0 },
        None,
    ));
}

/// A13.18: View in Browser and Print Preview write below `<cache>/print/`, call `open_associated` once each, and
/// their files equal the CLI's `--output-to browser` / `printer` with the same words (the print words included); a
/// failing launcher leaves the file and shows its path.
#[test]
fn a13_18_view_in_browser_and_print_preview() {
    let (world, launcher) = world_with(vec![Some(0), Some(0), Some(0), Some(0), None]);
    let (mut app, _) = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Text);
    let cli = Cli::of(&world, ReportKind::Text, (&pair.0, &pair.1));
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.print = ("print-mono", "print-landscape");
    }
    for (button, action, target) in [
        (
            BROWSER_BUTTON,
            ReportAction::ViewInBrowser,
            ReportTarget::Browser,
        ),
        (
            PRINT_BUTTON,
            ReportAction::PrintPreview,
            ReportTarget::Printer,
        ),
    ] {
        let before = launcher.calls().len();
        let flags = find::<ReportDialog>(&mut app)
            .expect("dialog")
            .flags(action);
        if action == ReportAction::PrintPreview {
            assert!(
                flags
                    .words
                    .output_options
                    .contains(&"print-mono".to_owned())
            );
            assert!(
                flags
                    .words
                    .output_options
                    .contains(&"print-landscape".to_owned())
            );
        }
        let (ok, text) = press(&world, &mut app, button);
        assert!(ok, "{text}");
        let calls = launcher.calls();
        assert_eq!(calls.len(), before + 1, "one open");
        let Some(Recorded::Associated(opened)) = calls.last().cloned() else {
            panic!("{calls:?}");
        };
        assert!(
            opened.components().any(|c| c.as_os_str() == "print"),
            "{}",
            opened.display()
        );
        let dialog_bytes = std::fs::read(&opened).expect("dialog's file");
        // The CLI's run of the same words to the same target.
        let mut req = world
            .service
            .report_request(ReportKind::Text, "side-by-side", &flags, &target)
            .expect("request");
        let control = job(&world, JobKind::Report);
        let plan = world
            .service
            .report_plan(cli.source(), &mut req, &target, None, &control)
            .expect("plan");
        let out = world
            .service
            .report_execute(cli.source(), &req, plan, None, &control)
            .expect("execute");
        let cli_file = out.opened.expect("opened");
        assert_eq!(
            String::from_utf8_lossy(&dialog_bytes),
            String::from_utf8_lossy(&std::fs::read(&cli_file).expect("cli's file"))
        );
    }
    // The launcher fails (its 5th call): the file stays and the message names it.
    let (ok, text) = press(&world, &mut app, BROWSER_BUTTON);
    assert!(!ok);
    let Some(Recorded::Associated(file)) = launcher.calls().last().cloned() else {
        panic!("no call");
    };
    assert!(text.contains(&file.display().to_string()), "{text}");
    assert!(file.is_file(), "the file stays");
}

/// A13.19: a Text Compare view with unsaved edits and a pending recompare: the report waits for the install and
/// equals a report of the saved files except the header's *(unsaved edits)* mark on the edited side.
#[test]
fn a13_19_an_edited_text_view_reports_after_the_install_with_the_mark() {
    let world = world();
    let (mut app, _) = app_of(&world);
    let (l, r) = open_kind(&world, &mut app, ReportKind::Text);
    let view = app.active_tab().expect("tab").id;
    edit_left(&mut app, view, "X");
    app.execute(CommandId::Report);
    let file = world.path("home/edited.txt");
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.output = "text";
        d.path = file.display().to_string();
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    let edited = std::fs::read_to_string(&file).expect("written");
    // The same text saved: the CLI's report, with the mark after the left spec.
    let saved = std::fs::read_to_string(&l)
        .expect("left")
        .replacen("int v1 ", "Xint v1 ", 1);
    std::fs::write(&l, saved).expect("save");
    let flags = find::<ReportDialog>(&mut app)
        .expect("dialog")
        .flags(ReportAction::Write);
    let cli = Cli::of(&world, ReportKind::Text, (&l, &r));
    let fresh = String::from_utf8(cli_bytes(
        &world,
        cli.source(),
        &flags,
        &world.path("home/saved.txt"),
    ))
    .expect("utf-8");
    let spec = l.display().to_string();
    let marked = fresh.replacen(&spec, &format!("{spec} (unsaved edits)"), 1);
    assert_ne!(marked, fresh);
    assert_eq!(edited, marked);
    assert!(!edited.contains(&format!("{} (unsaved edits)", r.display())));
}

/// A13.19: without edits the text report has no mark (`unsaved` all false).
#[test]
fn a13_19_without_edits_no_mark() {
    let world = world();
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    let file = world.path("home/clean.txt");
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.output = "text";
        d.path = file.display().to_string();
    }
    let (ok, _) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok);
    assert!(
        !std::fs::read_to_string(&file)
            .expect("written")
            .contains("(unsaved edits)")
    );
}

/// A13.19: a Folder Compare view after a Copy reports the current tree, equal to a fresh `report folder` of the
/// changed folders.
#[test]
fn a13_19_a_changed_folder_view_reports_the_current_tree() {
    use crate::tests_folder_ops::{open, put, select};
    let world = world();
    put(&world, "l", "a.txt", "left\n", 2_000);
    put(&world, "r", "a.txt", "right\n", 1_000);
    put(&world, "l", "new.txt", "new\n", 1_000);
    let (mut app, _) = app_of(&world);
    let view = open(&world, &mut app);
    select(&mut app, view, "new.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    crate::tests_folder_ops::press(&world, &mut app, ButtonRole::Yes);
    world.settle(&mut app);
    assert!(world.path("work/r/new.txt").is_file(), "copied");
    app.execute(CommandId::Report);
    let file = world.path("home/folder.txt");
    let flags = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.layout = "list";
        d.path = file.display().to_string();
        d.flags(ReportAction::Write)
    };
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    let cli = Cli::of(
        &world,
        ReportKind::Folder,
        (&world.path("work/l"), &world.path("work/r")),
    );
    let expected = cli_bytes(
        &world,
        cli.source(),
        &flags,
        &world.path("home/folder-cli.txt"),
    );
    assert_eq!(
        std::fs::read_to_string(&file).expect("written"),
        String::from_utf8(expected).expect("utf-8")
    );
}

/// A13.17: a display word in the remembered options (a hand-edited or older state file) is not the default: the
/// view's filter is (the mutation "the display default taken from the remembered choice" fails here).
#[test]
fn a13_17_a_remembered_display_word_is_not_the_default() {
    let world = world();
    std::fs::write(
        world.path("config/state.toml"),
        "schema_version = 1\n\n[dialogs.report-text]\nlayout = \"side-by-side\"\noptions = [\"display-matches\"]\n",
    )
    .expect("state");
    let (mut app, _) = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    assert_eq!(
        d.display,
        Some(vocabulary(ReportKind::Text).displays[0].word)
    );
}
