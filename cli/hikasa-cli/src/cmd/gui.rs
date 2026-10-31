//! `hikasa gui open|ping|list|close|quit` and the `--gui` of the compare commands (§3.6; D-22, D-23, D-49, D-50,
//! D-51).
//!
//! `gui open` arguments were parsed by `parse_open_args` before the service existed (main, §1.9.1); `open_view`
//! sends a request with `Service::gui_open_with` (launching the GUI through [`ProcessGuiLauncher`] unless
//! `--no-launch`). Its exit code is 0 when the view opened, or the view's raw code with `--wait` (D-23). The
//! control commands never launch; no GUI is exit 100 (D-22).

use std::path::Path;

use hikasa_core::{ExitCode, Warning};
use hikasa_session::api::dto::{
    GUI_CLOSE_SCHEMA, GUI_LIST_SCHEMA, GUI_OPEN_SCHEMA, GUI_PING_SCHEMA, GUI_QUIT_SCHEMA,
    GuiCloseDto, GuiListDto, GuiQuitDto,
};
use hikasa_session::api::{ApiOp, GuiLauncher, GuiOpenRequest, OpenArgs, find_gui_exe, human};
use hikasa_session::ipc::{RetryPolicy, ViewId};

use hikasa_session::api::{GuiOpenReport, SingleArg};
use hikasa_session::package::ImportChoices;

use super::Ctx;
use crate::args::GuiCmd;
use crate::launcher::ProcessGuiLauncher;

/// Exit code of a successful control command.
const OK: u8 = ExitCode::Success.code();

/// Runs a `gui` subcommand. `open` is the parsed `gui open` command line.
pub fn run(ctx: &mut Ctx<'_>, cmd: &GuiCmd, open: Option<&OpenArgs>) -> u8 {
    let op = ApiOp::GuiControl;
    match cmd {
        GuiCmd::Open(_) => match open {
            Some(open) => open_args(ctx, open),
            None => {
                let err = hikasa_session::ApiError::Internal(
                    "the gui open arguments were not parsed".into(),
                );
                ctx.fail(GUI_OPEN_SCHEMA, &err, ApiOp::GuiOpen { wait: false }, &[])
            }
        },
        GuiCmd::Ping => match ctx.service.gui_ping_dto() {
            Ok(dto) => {
                if ctx.out.json() {
                    ctx.json_success(GUI_PING_SCHEMA, OK, dto, &[], op)
                } else {
                    let line = format!(
                        "{}\t{}",
                        hikasa_report::escape_controls(&dto.server),
                        dto.protocol_version
                    );
                    let written = ctx.out.lines([line]);
                    ctx.human_done(written, OK, GUI_PING_SCHEMA, &[], op)
                }
            }
            Err(e) => ctx.fail(GUI_PING_SCHEMA, &e, op, &[]),
        },
        GuiCmd::List => match ctx.service.gui_list() {
            Ok(views) => {
                if ctx.out.json() {
                    ctx.json_success(GUI_LIST_SCHEMA, OK, GuiListDto { views }, &[], op)
                } else {
                    let written = ctx.out.lines(views.iter().map(human::gui_list_line));
                    ctx.human_done(written, OK, GUI_LIST_SCHEMA, &[], op)
                }
            }
            Err(e) => ctx.fail(GUI_LIST_SCHEMA, &e, op, &[]),
        },
        GuiCmd::Close { id } => match ctx.service.gui_close(ViewId(*id)) {
            Ok(()) => {
                if ctx.out.json() {
                    ctx.json_success(GUI_CLOSE_SCHEMA, OK, GuiCloseDto { view: *id }, &[], op)
                } else {
                    ctx.human_done(Ok(()), OK, GUI_CLOSE_SCHEMA, &[], op)
                }
            }
            Err(e) => ctx.fail(GUI_CLOSE_SCHEMA, &e, op, &[]),
        },
        GuiCmd::Session {
            reference,
            wait,
            no_launch,
        } => load_session(ctx, reference, *wait, *no_launch, &[]),
        GuiCmd::Workspace { name, no_launch } => load_workspace(ctx, name, *no_launch, &[]),
        GuiCmd::Quit => match ctx.service.gui_quit() {
            Ok(()) => {
                if ctx.out.json() {
                    ctx.json_success(GUI_QUIT_SCHEMA, OK, GuiQuitDto {}, &[], op)
                } else {
                    ctx.human_done(Ok(()), OK, GUI_QUIT_SCHEMA, &[], op)
                }
            }
            Err(e) => ctx.fail(GUI_QUIT_SCHEMA, &e, op, &[]),
        },
    }
}

/// Sends `request` to the GUI (`gui open` and `--gui`): `hikasa.gui_open.v1` with `--json`, nothing otherwise;
/// the exit code is `GuiOpenReport::exit_code` (0, or the view's raw code with `--wait`) or the error's under
/// `ApiOp::GuiOpen { wait }` (100/102/103). `own` are the warnings of the command line (`switch_ignored`).
pub fn open_view(
    ctx: &mut Ctx<'_>,
    request: &GuiOpenRequest,
    no_launch: bool,
    own: &[Warning],
) -> u8 {
    let op = ApiOp::GuiOpen { wait: request.wait };
    let launcher = (!no_launch).then(|| ProcessGuiLauncher {
        exe: find_gui_exe(ctx.service.env()),
    });
    let launcher = launcher.as_ref().map(|l| l as &dyn GuiLauncher);
    match ctx
        .service
        .gui_open_with(request, launcher, &RetryPolicy::default())
    {
        Ok(report) => {
            let code = report.exit_code();
            if ctx.out.json() {
                ctx.json_success(GUI_OPEN_SCHEMA, code, report.dto(), own, op)
            } else {
                ctx.human_done(Ok(()), code, GUI_OPEN_SCHEMA, own, op)
            }
        }
        Err(e) => ctx.fail(GUI_OPEN_SCHEMA, &e, op, own),
    }
}

/// `gui open` with its parsed arguments: one argument that names a session, a workspace or a settings package
/// takes the D-88 forms; everything else opens a view.
fn open_args(ctx: &mut Ctx<'_>, open: &OpenArgs) -> u8 {
    let request = &open.request;
    if let [arg] = request.specs.as_slice()
        && request.kind.is_none()
    {
        match ctx.service.classify_single_arg(arg) {
            SingleArg::Spec => {}
            SingleArg::Session(reference) => {
                return load_session(
                    ctx,
                    &reference.to_string(),
                    request.wait,
                    open.no_launch,
                    &open.warnings,
                );
            }
            SingleArg::Workspace(name) => {
                return load_workspace(ctx, name.as_str(), open.no_launch, &open.warnings);
            }
            SingleArg::Package(path) => {
                if request.options.extra.contains_key("silent") {
                    let result =
                        ctx.service
                            .settings_import(&path, &ImportChoices::default(), false);
                    return super::settings::import_done(ctx, result);
                }
                return import_package(ctx, &path, open.no_launch, &open.warnings);
            }
        }
    }
    open_view(ctx, request, open.no_launch, &open.warnings)
}

/// `gui open <file.hikasa-pkg>` without `--silent` (D-127): the GUI shows its import wizard (exit 0).
fn import_package(ctx: &mut Ctx<'_>, path: &Path, no_launch: bool, own: &[Warning]) -> u8 {
    let op = ApiOp::GuiOpen { wait: false };
    let launcher = launcher(ctx, no_launch);
    let launcher = launcher.as_ref().map(|l| l as &dyn GuiLauncher);
    match ctx
        .service
        .gui_import_package(path, launcher, &RetryPolicy::default())
    {
        Ok(()) => {
            if ctx.out.json() {
                let dto = hikasa_session::api::dto::GuiOpenDto {
                    view: 0,
                    waited: false,
                    result: None,
                    view_exit_code: None,
                    spooled_stdin: None,
                    views: Vec::new(),
                };
                ctx.json_success(GUI_OPEN_SCHEMA, OK, dto, own, op)
            } else {
                ctx.human_done(Ok(()), OK, GUI_OPEN_SCHEMA, own, op)
            }
        }
        Err(e) => ctx.fail(GUI_OPEN_SCHEMA, &e, op, own),
    }
}

/// The launcher of a GUI command (`None` with `--no-launch`).
fn launcher(ctx: &Ctx<'_>, no_launch: bool) -> Option<ProcessGuiLauncher> {
    (!no_launch).then(|| ProcessGuiLauncher {
        exe: find_gui_exe(ctx.service.env()),
    })
}

/// `gui session` and `gui open <session>` (D-89): `hikasa.gui_open.v1`.
fn load_session(
    ctx: &mut Ctx<'_>,
    reference: &str,
    wait: bool,
    no_launch: bool,
    own: &[Warning],
) -> u8 {
    let op = ApiOp::GuiOpen { wait };
    let launcher = launcher(ctx, no_launch);
    let launcher = launcher.as_ref().map(|l| l as &dyn GuiLauncher);
    match ctx
        .service
        .gui_load_session(reference, wait, launcher, &RetryPolicy::default())
    {
        Ok(report) => report_done(ctx, &report, own, op),
        Err(e) => ctx.fail(GUI_OPEN_SCHEMA, &e, op, own),
    }
}

/// The end of a `gui_open` report.
fn report_done(ctx: &mut Ctx<'_>, report: &GuiOpenReport, own: &[Warning], op: ApiOp) -> u8 {
    let code = report.exit_code();
    if ctx.out.json() {
        ctx.json_success(GUI_OPEN_SCHEMA, code, report.dto(), own, op)
    } else {
        ctx.human_done(Ok(()), code, GUI_OPEN_SCHEMA, own, op)
    }
}

/// `gui workspace` and `gui open <workspace>` (D-89): `hikasa.gui_open.v1` with every opened view.
fn load_workspace(ctx: &mut Ctx<'_>, name: &str, no_launch: bool, own: &[Warning]) -> u8 {
    let op = ApiOp::GuiOpen { wait: false };
    let launcher = launcher(ctx, no_launch);
    let launcher = launcher.as_ref().map(|l| l as &dyn GuiLauncher);
    match ctx
        .service
        .gui_load_workspace(name, launcher, &RetryPolicy::default())
    {
        Ok(views) => {
            if ctx.out.json() {
                let ids: Vec<u64> = views.iter().map(|v| v.view.0).collect();
                let dto = hikasa_session::api::dto::GuiOpenDto {
                    view: ids.first().copied().unwrap_or(0),
                    waited: false,
                    result: None,
                    view_exit_code: None,
                    spooled_stdin: None,
                    views: ids,
                };
                ctx.json_success(GUI_OPEN_SCHEMA, OK, dto, own, op)
            } else {
                ctx.human_done(Ok(()), OK, GUI_OPEN_SCHEMA, own, op)
            }
        }
        Err(e) => ctx.fail(GUI_OPEN_SCHEMA, &e, op, own),
    }
}
