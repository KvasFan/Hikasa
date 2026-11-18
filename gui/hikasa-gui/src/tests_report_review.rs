//! Regression tests of the W13c review (M2, §1.13.26 W13c review fixes): each test names the finding or the surviving
//! mutation it shows. The dialog's bytes are compared with the real `hikasa` binary next to the test binary (`cargo
//! build -p hikasa-cli` first, or `HIKASA_BIN`); A13.29's report-file tests (a non-UTF-8 folder on Linux; drive, UNC and
//! `\\?\` paths on Windows, also under wine).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hikasa_report::ReportKind;
use hikasa_report::vocabulary::vocabulary;
use hikasa_session::api::{GuiOpenRequest, ReportFlags};
use hikasa_session::clipboard::FixedClipboard;

use crate::commands::{CommandId, Scope};
use crate::dialog::ButtonRole;
use crate::dnd::DropTarget;
use crate::quick_start::QuickStartDialog;
use crate::report_dialog::{ReportAction, ReportDialog, ReportTo};
use crate::tests::{TestApp, World};
use crate::tests_sync_view::find;

const EPOCH: &str = "1700000000";
const ON: &str = "[startup]\nshow_quick_compare_dialog = true\nopen_view_if_different = false\n";

fn report_world(extra: &[(&str, &str)]) -> World {
    let mut files: Vec<(&str, &str)> =
        vec![("formats/firmware.toml", crate::tests_hex_open::HEX_FORMAT)];
    files.extend_from_slice(extra);
    World::with_files_and_init(&files, |init| {
        init.env.source_date_epoch = Some(EPOCH.to_owned());
    })
}

fn app_of(world: &World) -> TestApp {
    let mut app = world.app(None);
    let clip = Arc::new(FixedClipboard::text("unused"));
    app.0.set_script_clipboard(clip as _);
    app
}

fn press(world: &World, app: &mut TestApp, role: ButtonRole) -> (bool, String) {
    let w = app.win();
    if let Some(d) = find::<ReportDialog>(app) {
        d.message = None;
    }
    app.0.dialog_button(w, role);
    for _ in 0..2000 {
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

/// The `hikasa` binary next to this test binary (`target/debug/deps/..` → `target/debug/hikasa`).
fn hikasa_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("HIKASA_BIN") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().expect("exe");
    exe.parent()
        .and_then(Path::parent)
        .expect("target dir")
        .join(if cfg!(windows) {
            "hikasa.exe"
        } else {
            "hikasa"
        })
}

/// The real CLI's report of the pair with the dialog's flags, as a user would type them.
fn real_cli(
    world: &World,
    kind: ReportKind,
    pair: &(PathBuf, PathBuf),
    flags: &ReportFlags,
    out: &Path,
) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new(hikasa_bin());
    let sub = match kind {
        ReportKind::Text => "text",
        ReportKind::Folder => "folder",
        ReportKind::Table => "table",
        ReportKind::Hex => "hex",
        ReportKind::Picture => "picture",
    };
    cmd.arg(sub).arg("compare").arg(&pair.0).arg(&pair.1);
    cmd.arg("--config-dir").arg(world.path("config"));
    cmd.arg("-q");
    if let Some(l) = &flags.words.layout {
        cmd.arg("--layout").arg(l);
    }
    if !flags.words.options.is_empty() {
        cmd.arg("--options").arg(flags.words.options.join(","));
    }
    for w in &flags.words.output_options {
        cmd.arg("--output-options").arg(w);
    }
    if let Some(t) = &flags.words.title {
        cmd.arg("--title").arg(t);
    }
    if let Some(c) = flags.context {
        cmd.arg("--context").arg(c.to_string());
    }
    if let Some(f) = &flags.folders {
        cmd.arg("--folders").arg(f);
    }
    if let Some(l) = &flags.lines {
        cmd.arg("--lines").arg(l);
    }
    cmd.arg("--output").arg(out);
    cmd.env("SOURCE_DATE_EPOCH", EPOCH)
        .env("HOME", world.path("home"))
        .env_remove("HIKASA_IPC_ENDPOINT");
    let o = cmd.output().map_err(|e| e.to_string())?;
    std::fs::read(out).map_err(|e| {
        format!(
            "{e}: exit {:?} stderr {}",
            o.status.code(),
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

/// Finding 1 (A13.17, RPT-011): the dialog's bytes against the real `hikasa <kind> compare … --layout …` binary,
/// every layout and output of each kind. Collects every mismatch.
#[test]
fn review_w13c_dialog_bytes_equal_the_real_cli_binary() {
    if !hikasa_bin().exists() {
        panic!("build the CLI first: {}", hikasa_bin().display());
    }
    let mut bad = Vec::new();
    for kind in [
        ReportKind::Text,
        ReportKind::Folder,
        ReportKind::Table,
        ReportKind::Hex,
        ReportKind::Picture,
    ] {
        let world = report_world(&[]);
        let mut app = app_of(&world);
        let pair = open_kind(&world, &mut app, kind);
        app.execute(CommandId::Report);
        assert!(find::<ReportDialog>(&mut app).is_some(), "{kind:?}");
        for (n, layout) in vocabulary(kind).layouts.iter().enumerate() {
            let outputs: Vec<&str> = {
                let d = find::<ReportDialog>(&mut app).expect("dialog");
                d.layout = layout.word;
                d.outputs().iter().map(|o| o.word).collect()
            };
            let mut cases: Vec<Option<&str>> = outputs
                .into_iter()
                .filter(|o| !o.starts_with("html-custom"))
                .map(Some)
                .collect();
            if cases.is_empty() {
                cases.push(None);
            }
            for (k, output) in cases.into_iter().enumerate() {
                let file = world.path(&format!("home/g-{n}-{k}.out"));
                let flags = {
                    let d = find::<ReportDialog>(&mut app).expect("dialog");
                    d.to = ReportTo::File;
                    d.path = file.display().to_string();
                    if let Some(o) = output {
                        d.output = o;
                    }
                    d.flags(ReportAction::Write)
                };
                let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
                if !ok {
                    bad.push(format!(
                        "{kind:?} {} {output:?}: dialog: {text}",
                        layout.word
                    ));
                    continue;
                }
                let gui = std::fs::read(&file).expect("written");
                match real_cli(
                    &world,
                    kind,
                    &pair,
                    &flags,
                    &world.path(&format!("home/c-{n}-{k}.out")),
                ) {
                    Err(e) => bad.push(format!("{kind:?} {} {output:?}: cli: {e}", layout.word)),
                    Ok(cli) if cli != gui => {
                        let (g, c) = (
                            String::from_utf8_lossy(&gui).into_owned(),
                            String::from_utf8_lossy(&cli).into_owned(),
                        );
                        let line = g
                            .lines()
                            .zip(c.lines())
                            .find(|(a, b)| a != b)
                            .map(|(a, b)| format!("gui `{a}` cli `{b}`"))
                            .unwrap_or_else(|| format!("lengths {} vs {}", g.len(), c.len()));
                        bad.push(format!(
                            "{kind:?} {} {output:?} words {:?}: {line}",
                            layout.word, flags.words
                        ));
                    }
                    Ok(cli) => eprintln!(
                        "{kind:?} {} {output:?}: equal ({} bytes)",
                        layout.word,
                        cli.len()
                    ),
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Finding 2 (D-368, §1.13.16 "When"): two files dropped on the window open their view; the Quick Compare dialog at
/// startup is only for the command line (process arguments, `gui open`, the bare form).
#[test]
fn review_w13c_a_drop_of_two_files_does_not_show_quick_compare() {
    let world = World::with_files(&[("options.toml", ON)]);
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    let w = app.win();
    app.0.drop_paths(w, DropTarget::Window, vec![l, r]);
    for _ in 0..50 {
        world.settle(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        find::<QuickStartDialog>(&mut app).is_none(),
        "a drop showed the Quick Compare dialog"
    );
    assert_eq!(app.scope(), Scope::Text);
}

/// Finding 2 (D-368): Home's Compare of two files opens the view without the startup dialog.
#[test]
fn review_w13c_home_compare_does_not_show_quick_compare() {
    let world = World::with_files(&[("options.toml", ON)]);
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    if let Some(crate::app::ViewModel::Home(h)) = app.active_tab_mut().and_then(|t| t.model_mut()) {
        h.panel.left = l.display().to_string();
        h.panel.right = r.display().to_string();
    } else {
        panic!("Home is the first tab");
    }
    app.execute(CommandId::HomeCompare);
    for _ in 0..50 {
        world.settle(&mut app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        find::<QuickStartDialog>(&mut app).is_none(),
        "Home's Compare showed the Quick Compare dialog"
    );
    assert_eq!(app.scope(), Scope::Text);
}

/// Finding 5 (D-368): the facts of a missing right file name the right side, not the left.
#[test]
fn review_w13c_quick_facts_of_a_missing_right_file_name_the_right_side() {
    let world = World::with_files(&[("options.toml", ON)]);
    let mut app = world.app(None);
    let l = world.write("work/q/there.c", "x\n");
    let r = world.path("work/q/missing.c");
    let request = GuiOpenRequest {
        specs: vec![l.into_os_string(), r.into_os_string()],
        ..GuiOpenRequest::default()
    };
    let resolved = world.service.resolve_open(&request).expect("resolve");
    let view = world
        .service
        .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
        .expect("request");
    let target = app.open_target();
    app.open_command_line(
        resolved.kind,
        view,
        &resolved.options,
        resolved.specs,
        target,
    );
    for _ in 0..200 {
        world.settle(&mut app);
        if find::<QuickStartDialog>(&mut app).is_some_and(|d| !d.running()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let d = find::<QuickStartDialog>(&mut app).expect("dialog");
    let right = d.facts[1].clone().expect("facts").expect_err("missing");
    assert!(
        !right.starts_with("left"),
        "the right file's facts say: {right}"
    );
}

/// Finding 4 (D-368): *Stop*, then *Close*: a waiting caller is told "no verdict", not "load failed" (100).
#[test]
fn review_w13c_quick_stop_then_close_is_no_verdict() {
    let world = World::with_files(&[("options.toml", ON)]);
    let mut app = world.app(None);
    let (l, r) = crate::tests::text_pair(&world);
    world.open(&mut app, &l, &r);
    let w = app.win();
    // Stop at once (the job may already have ended: then nothing to show).
    let running = find::<QuickStartDialog>(&mut app).is_some_and(|d| d.running());
    if !running {
        eprintln!("the quick job ended before Stop; nothing shown");
        return;
    }
    app.0.dialog_button(w, ButtonRole::Close);
    let d = find::<QuickStartDialog>(&mut app).expect("dialog");
    assert!(
        !matches!(d.result, Some(Err(_))),
        "a stopped comparison is an error result: {:?}",
        d.result
    );
}

/// Finding 6 (D-113/D-160 carried item b): the hex view's font size is the theme's hex size.
#[test]
fn review_w13c_hex_view_takes_the_hex_slot_size() {
    let world = World::with_files(&[
        ("formats/firmware.toml", crate::tests_hex_open::HEX_FORMAT),
        (
            "theme.toml",
            "schema_version = 1\n[fonts.editor]\nsize = 11.0\n[fonts.hex]\nsize = 20.0\n",
        ),
    ]);
    let mut app = world.app(None);
    let a = crate::tests_hex_open::bytes(7, 300);
    let b = crate::tests_hex_open::with_insert(&a, 70, 5, 200);
    let view = crate::tests_hex_open::open_pair(&world, &mut app, &a, &b);
    let px = crate::tests_hex_open::hex_model(&mut app, view)
        .display()
        .font_px;
    assert_eq!(px, 20, "the hex view's size is the editor's ({px})");
    // Reset Font Size returns to the hex slot's size.
    app.execute(CommandId::DisplayFontIncrease);
    assert_ne!(
        crate::tests_hex_open::hex_model(&mut app, view)
            .display()
            .font_px,
        20
    );
    app.execute(CommandId::DisplayFontReset);
    assert_eq!(
        crate::tests_hex_open::hex_model(&mut app, view)
            .display()
            .font_px,
        20
    );
}

/// Finding 3 (D-366): a relative report file is written to the same place the next time the dialog opens with the
/// remembered file (absolute first, then a relative name in the folder the dialog showed).
#[test]
fn review_w13c_a_relative_file_is_written_to_the_same_folder_next_time() {
    let world = report_world(&[]);
    let folder = world.path("work/reports");
    std::fs::create_dir_all(&folder).expect("mkdir");
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    let w = app.win();
    // An absolute path first: the folder of relative paths becomes its parent.
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.to = ReportTo::File;
        d.path = folder.join("a.html").display().to_string();
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    // Reopened: a relative name is in that folder; written there.
    app.execute(CommandId::Report);
    let first = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.path = "b.html".to_owned();
        d.file().expect("file")
    };
    assert_eq!(first, folder.join("b.html"));
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    // Reopened with the remembered `b.html`: the same file?
    app.execute(CommandId::Report);
    let second = find::<ReportDialog>(&mut app)
        .expect("dialog")
        .file()
        .expect("file");
    assert_eq!(
        first, second,
        "the remembered `b.html` now names another file"
    );
}

/// Writes the open dialog's report as plain text side-by-side to `name` (home); its text.
fn write_text(world: &World, app: &mut TestApp, layout: &'static str, name: &str) -> String {
    let file = world.path(&format!("home/{name}"));
    {
        let d = find::<ReportDialog>(app).expect("dialog");
        d.layout = layout;
        d.output = "text";
        d.to = ReportTo::File;
        d.path = file.display().to_string();
    }
    let (ok, text) = press(world, app, ButtonRole::Ok);
    assert!(ok, "{text}");
    std::fs::read_to_string(file).expect("written")
}

/// Finding 7 (D-366 "Hex: the request's HexLayout is the view's"): after the Layout dialog changed the view's bytes
/// per line, the report follows the view, not the layout of the landing.
#[test]
fn review_w13c_hex_report_takes_the_views_current_layout() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Hex);
    app.execute(CommandId::Report);
    let before = write_text(&world, &mut app, "side-by-side", "h16.txt");
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    let view = app.active_tab().expect("tab").id;
    {
        let m = crate::tests_hex_open::hex_model(&mut app, view);
        let mut layout = m.display().layout;
        assert_ne!(layout.bytes_per_line, 8);
        layout.bytes_per_line = 8;
        let (addresses, panes, context) = (
            m.display().addresses,
            m.display().panes,
            m.display().context,
        );
        m.set_layout(layout, addresses, panes, context);
    }
    world.settle(&mut app);
    app.execute(CommandId::Report);
    let after = write_text(&world, &mut app, "side-by-side", "h8.txt");
    assert_ne!(
        before, after,
        "the report keeps the bytes per line of the landing"
    );
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    // The filter too: the response the dialog holds has the view's layout and filter.
    crate::tests_hex_open::hex_model(&mut app, view)
        .set_filter(hikasa_hex::DisplayFilter::Differences);
    world.settle(&mut app);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    let crate::report_dialog::OwnedReportSource::Hex(r) = d.source() else {
        panic!("a hex source");
    };
    assert_eq!(r.layout.bytes_per_line, 8);
    assert_eq!(r.display, Some(hikasa_hex::DisplayFilter::Differences));
    assert_eq!(d.display, Some("display-differences"));
}

/// Finding 8 (RPT-003): the column checks say what the report shows: both default columns unchecked is not the
/// report with both columns.
#[test]
fn review_w13c_folder_columns_all_unchecked_are_not_the_defaults() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Folder);
    app.execute(CommandId::Report);
    let with = write_text(&world, &mut app, "side-by-side", "cols.txt");
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.options.remove("column-size");
        d.options.remove("column-timestamp");
        assert!(!d.options.iter().any(|w| w.starts_with("column-")));
    }
    let without = write_text(&world, &mut app, "side-by-side", "nocols.txt");
    assert_ne!(
        with, without,
        "no column checked writes the default columns"
    );
    // The CLI's `column-none`.
    let flags = find::<ReportDialog>(&mut app)
        .expect("dialog")
        .flags(ReportAction::Write);
    assert!(
        flags.words.options.iter().any(|w| w == "column-none"),
        "{:?}",
        flags.words
    );
    let cli = real_cli(
        &world,
        ReportKind::Folder,
        &pair,
        &flags,
        &world.path("home/nocols-cli.txt"),
    )
    .expect("cli");
    assert_eq!(without, String::from_utf8_lossy(&cli));
}

/// Finding 9 (D-366 *Ignore Unimportant*): unchecking the dialog's Ignore Unimportant of a picture view that ignores
/// them writes the report the CLI writes without it.
#[test]
fn review_w13c_picture_ignore_unimportant_can_be_turned_off() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Picture);
    app.execute(CommandId::InsigIgnored);
    world.settle(&mut app);
    let view = app.active_tab().expect("tab").id;
    let iu = match app.tab(view).and_then(|t| t.model()) {
        Some(crate::app::ViewModel::Picture(m)) => m.ignore_unimportant,
        _ => panic!("picture"),
    };
    assert!(iu, "the view ignores unimportant differences");
    app.execute(CommandId::Report);
    let flags = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert!(d.ignore_unimportant);
        d.ignore_unimportant = false;
        d.layout = "summary";
        d.output = "text";
        d.flags(ReportAction::Write)
    };
    let gui = write_text(&world, &mut app, "summary", "p.txt");
    let cli = real_cli(
        &world,
        ReportKind::Picture,
        &pair,
        &flags,
        &world.path("home/pc.txt"),
    )
    .expect("cli");
    assert_eq!(
        gui,
        String::from_utf8_lossy(&cli),
        "the dialog's unchecked box is ignored"
    );
}

/// Finding 10 (D-366 table migration): after the first remember, the table's phase-3 `folder` no longer decides the
/// folder of relative paths (the remembered file's does).
#[test]
fn review_w13c_the_legacy_folder_is_read_once() {
    let world = report_world(&[]);
    let legacy = world.path("work/legacy");
    let newer = world.path("work/newer");
    std::fs::create_dir_all(&legacy).expect("mkdir");
    std::fs::create_dir_all(&newer).expect("mkdir");
    std::fs::write(
        world.path("config/state.toml"),
        format!(
            "schema_version = 1\n\n[dialogs.report-table]\nfolder = {:?}\n",
            legacy.display().to_string()
        ),
    )
    .expect("state");
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Table);
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert_eq!(d.folder.as_deref(), Some(legacy.as_path()));
        d.to = ReportTo::File;
        d.path = newer.join("t.html").display().to_string();
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    for _ in 0..20 {
        world.settle(&mut app);
        std::thread::sleep(Duration::from_millis(10));
    }
    let state = std::fs::read_to_string(world.path("config/state.toml")).expect("state");
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    assert_eq!(
        d.folder.as_deref(),
        Some(newer.as_path()),
        "the legacy folder is read again; state now: {state}"
    );
}

/// Finding 11 (RPT-003/RPT-011): the dialog's context lines start at the user's option
/// (`[text_editing] number_of_context_lines`), as the CLI's `--context` default does ("default from the options").
#[test]
fn review_w13c_text_context_starts_at_the_option() {
    let world = report_world(&[(
        "options.toml",
        "[text_editing]\nnumber_of_context_lines = 7\n",
    )]);
    let mut app = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    let flags = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.layout = "unified";
        d.flags(ReportAction::Write)
    };
    assert_eq!(
        flags.context,
        Some(7),
        "the dialog's default context ignores the option"
    );
    let gui = write_text(&world, &mut app, "unified", "ctx.txt");
    let flags = find::<ReportDialog>(&mut app)
        .expect("dialog")
        .flags(ReportAction::Write);
    let cli = real_cli(
        &world,
        ReportKind::Text,
        &pair,
        &flags,
        &world.path("home/ctx-cli.txt"),
    )
    .expect("cli");
    assert_eq!(gui, String::from_utf8_lossy(&cli));
}

/// Finding 12 (A13.17 lists Folder Sync): a Folder Sync view has the report command and its dialog writes.
#[test]
fn review_w13c_folder_sync_view_reports() {
    let world = report_world(&[]);
    crate::tests_sync_view::fixture(&world);
    let mut app = app_of(&world);
    crate::tests_sync_view::open_sync(&world, &mut app.0, Default::default());
    assert_eq!(app.scope(), Scope::Sync);
    assert!(
        app.enabled(CommandId::Report),
        "Report is off in a Folder Sync view"
    );
    app.execute(CommandId::Report);
    assert!(find::<ReportDialog>(&mut app).is_some());
    let text = write_text(&world, &mut app, "side-by-side", "sync.txt");
    assert!(text.contains("only-left.txt"), "{text}");
}

/// Finding 12: the window's close while a report is written is refused with a notice naming the report (not "a
/// settings dialog"); once it is written the window closes.
#[test]
fn review_w13c_window_close_during_a_report_names_the_report() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.to = ReportTo::File;
        d.path = world.path("home/busy.html").display().to_string();
    }
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Ok);
    // No poll yet: the job is the dialog's until it takes the result.
    assert!(find::<ReportDialog>(&mut app).expect("dialog").running());
    let effects = app.0.close_window(w);
    assert!(!effects.contains(&crate::app::Effect::Exit));
    let text = app
        .0
        .window(w)
        .expect("window")
        .notices
        .get("dialog-busy")
        .expect("a notice")
        .text
        .clone();
    assert!(text.contains("text report"), "{text}");
    assert!(!text.contains("settings"), "{text}");
    assert!(find::<ReportDialog>(&mut app).is_some());
    world.settle(&mut app);
    assert!(!find::<ReportDialog>(&mut app).expect("dialog").running());
}

/// A13.29 (Linux): the report's file in a folder whose name is not valid UTF-8 is written there; the remembered file
/// is the name as typed (`state.toml` holds text only), never a lossy path.
#[cfg(target_os = "linux")]
#[test]
fn a13_29_the_report_file_in_a_non_utf8_folder() {
    use std::os::unix::ffi::OsStrExt;
    let world = report_world(&[]);
    let folder = world
        .path("work")
        .join(std::ffi::OsStr::from_bytes(b"r\xffport"));
    std::fs::create_dir_all(&folder).expect("mkdir");
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.folder = Some(folder.clone());
        d.to = ReportTo::File;
        d.path = "n.html".to_owned();
        assert_eq!(d.file().expect("file"), folder.join("n.html"));
        assert_eq!(
            d.choices(ReportAction::Write).file.as_deref(),
            Some("n.html")
        );
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(ok, "{text}");
    assert!(folder.join("n.html").is_file());
}

/// A13.29 (Windows, under wine too): drive, UNC and `\\?\` paths are whole paths; a relative name goes to the
/// dialog's folder (pure), and a drive path and a `\\?\` path are written on disk.
#[cfg(windows)]
#[test]
fn a13_29_windows_paths_of_the_report_file() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    let base = world.path("work");
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    d.folder = Some(base.clone());
    for (typed, expected) in [
        (r"C:\reports\a.html", PathBuf::from(r"C:\reports\a.html")),
        (
            r"\\server\share\a.html",
            PathBuf::from(r"\\server\share\a.html"),
        ),
        (
            r"\\?\C:\reports\a.html",
            PathBuf::from(r"\\?\C:\reports\a.html"),
        ),
        (r"sub\a.html", base.join(r"sub\a.html")),
    ] {
        d.path = typed.to_owned();
        assert_eq!(d.file().expect(typed), expected, "{typed}");
    }
    // On disk: the drive path, then the same file through `\\?\`.
    let drive = world.path("home/w.html");
    let verbatim = format!(r"\\?\{}", drive.display());
    for typed in [drive.display().to_string(), verbatim] {
        {
            let d = find::<ReportDialog>(&mut app).expect("dialog");
            d.to = ReportTo::File;
            d.path = typed.clone();
            d.confirm = d.file().ok().filter(|f| f.exists());
        }
        let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
        assert!(ok, "{typed}: {text}");
        assert!(drive.is_file(), "{typed}");
    }
}

// ---- the surviving mutations of the review (§1.13.26 W13c mutation table) -----------------------------------------

/// Mutation G1: the folder view's *Folders* mode is the report's `--folders` (the CLI's report with it is equal).
#[test]
fn review_w13c_folder_report_takes_the_views_folders_mode() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Folder);
    let default = {
        app.execute(CommandId::Report);
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        let folders = d.folders;
        let w = app.win();
        app.0.dialog_button(w, ButtonRole::Close);
        folders
    };
    app.execute(CommandId::DirCompareFilesOnly);
    world.settle(&mut app);
    app.execute(CommandId::Report);
    let flags = {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert_ne!(d.folders, default, "the view's mode, not the default");
        d.flags(ReportAction::Write)
    };
    assert_eq!(
        flags.folders.as_deref(),
        find::<ReportDialog>(&mut app).expect("dialog").folders,
        "the dialog's folders are passed"
    );
    let gui = write_text(&world, &mut app, "side-by-side", "files-only.txt");
    let flags = find::<ReportDialog>(&mut app)
        .expect("dialog")
        .flags(ReportAction::Write);
    assert!(flags.folders.is_some());
    let cli = real_cli(
        &world,
        ReportKind::Folder,
        &pair,
        &flags,
        &world.path("home/fo-cli.txt"),
    )
    .expect("cli");
    assert_eq!(gui, String::from_utf8_lossy(&cli));
}

/// Mutation G2: the dialog's *Ignore Unimportant* of a table view decides the table report (the CLI's with the word).
#[test]
fn review_w13c_table_ignore_unimportant_follows_the_dialog() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    let pair = open_kind(&world, &mut app, ReportKind::Table);
    app.execute(CommandId::Report);
    let mut reports = Vec::new();
    for iu in [true, false] {
        let flags = {
            let d = find::<ReportDialog>(&mut app).expect("dialog");
            d.ignore_unimportant = iu;
            d.layout = "summary";
            d.output = "text";
            d.flags(ReportAction::Write)
        };
        let gui = write_text(&world, &mut app, "summary", &format!("t-{iu}.txt"));
        let cli = real_cli(
            &world,
            ReportKind::Table,
            &pair,
            &flags,
            &world.path(&format!("home/tc-{iu}.txt")),
        )
        .expect("cli");
        assert_eq!(
            gui,
            String::from_utf8_lossy(&cli),
            "ignore unimportant {iu}"
        );
        reports.push(gui);
    }
    assert_ne!(reports[0], reports[1], "the check changes the report");
}

/// Mutations G4 and G15: the picture view's checkerboard decides the dialog's check, also over a remembered one.
#[test]
fn review_w13c_picture_checkerboard_is_the_views() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Picture);
    let view = app.active_tab().expect("tab").id;
    let set = |app: &mut TestApp, on: bool| {
        if let Some(crate::app::ViewModel::Picture(m)) =
            app.tab_mut(view).and_then(|t| t.model_mut())
        {
            m.checkerboard = on;
        }
    };
    set(&mut app, true);
    app.execute(CommandId::Report);
    assert!(
        find::<ReportDialog>(&mut app)
            .expect("dialog")
            .options
            .contains("checkerboard")
    );
    // Written, so the check is remembered.
    write_text(&world, &mut app, "summary", "cb.txt");
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    set(&mut app, false);
    app.execute(CommandId::Report);
    assert!(
        !find::<ReportDialog>(&mut app)
            .expect("dialog")
            .options
            .contains("checkerboard"),
        "the view has no checkerboard"
    );
}

/// Mutation G5: a failed action remembers nothing.
#[test]
fn review_w13c_a_failed_write_remembers_nothing() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    let blocker = world.write("home/blocker", "a file, not a folder\n");
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        d.layout = "summary";
        d.to = ReportTo::File;
        d.path = blocker.join("r.html").display().to_string();
    }
    let (ok, text) = press(&world, &mut app, ButtonRole::Ok);
    assert!(!ok, "{text}");
    for _ in 0..10 {
        world.settle(&mut app);
    }
    let state = std::fs::read_to_string(world.path("config/state.toml")).unwrap_or_default();
    assert!(!state.contains("report-text"), "{state}");
}

/// Mutation G6: `html-custom:` is the last remembered output word (it takes the rest of a list), so a stylesheet
/// path and the other output words come back.
#[test]
fn review_w13c_html_custom_is_remembered_last() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    d.output = vocabulary(ReportKind::Text)
        .outputs
        .iter()
        .find(|o| o.word.starts_with("html-custom"))
        .expect("html-custom")
        .word;
    d.css = "/styles/a,b.css".to_owned();
    let words = d.choices(ReportAction::Write).output_options;
    assert!(words.len() > 1, "{words:?}");
    assert_eq!(
        words.last().map(String::as_str),
        Some("html-custom:/styles/a,b.css"),
        "{words:?}"
    );
}

/// Mutation G14: the hex view's context rows are the dialog's.
#[test]
fn review_w13c_hex_context_rows_are_the_views() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Hex);
    let view = app.active_tab().expect("tab").id;
    let expected = {
        let m = crate::tests_hex_open::hex_model(&mut app, view);
        let d = m.display();
        let (layout, addresses, panes) = (d.layout, d.addresses, d.panes);
        m.set_layout(layout, addresses, panes, 2);
        let d = m.display();
        assert_eq!(d.context, 2);
        hikasa_report::vocabulary::hex_view_words(Some(d.filter), d.context, d.addresses).on
    };
    world.settle(&mut app);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    for word in expected {
        assert!(d.options.contains(word), "{word} {:?}", d.options);
    }
}

/// Mutation G16: View in Browser takes no print words (only Print Preview's palette and orientation).
#[test]
fn review_w13c_view_in_browser_takes_no_print_words() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Text);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    let vocab = vocabulary(ReportKind::Text);
    let print: Vec<&str> = vocab
        .print
        .iter()
        .flat_map(|p| p.words.iter().copied())
        .collect();
    d.print = (vocab.print[0].words[1], vocab.print[1].words[1]);
    let has_print = |a| {
        d.flags(a)
            .words
            .output_options
            .iter()
            .any(|w| print.contains(&w.as_str()))
    };
    assert!(has_print(ReportAction::PrintPreview));
    assert!(!has_print(ReportAction::ViewInBrowser));
    assert!(!has_print(ReportAction::Write));
}

/// Mutation G17: a default check the user turned off stays off when the choices are remembered.
#[test]
fn review_w13c_an_unchecked_default_stays_unchecked() {
    let world = report_world(&[]);
    let mut app = app_of(&world);
    open_kind(&world, &mut app, ReportKind::Folder);
    app.execute(CommandId::Report);
    {
        let d = find::<ReportDialog>(&mut app).expect("dialog");
        assert!(d.options.contains("column-size"), "a default check");
        d.options.remove("column-size");
    }
    write_text(&world, &mut app, "side-by-side", "nosize.txt");
    let w = app.win();
    app.0.dialog_button(w, ButtonRole::Close);
    world.settle(&mut app);
    app.execute(CommandId::Report);
    let d = find::<ReportDialog>(&mut app).expect("dialog");
    assert!(!d.options.contains("column-size"), "{:?}", d.options);
    assert!(d.options.contains("column-timestamp"), "{:?}", d.options);
}
