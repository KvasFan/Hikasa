//! `hikasa gui open|ping|list|close|quit` and the `--gui` of the compare commands (§3.6; D-22, D-23, D-49, D-50,
//! D-51).
//!
//! `gui open` arguments were parsed by `parse_open_args` before the service existed (main, §1.9.1); `open_view`
//! sends a request with `Service::gui_open_with` (launching the GUI through [`ProcessGuiLauncher`] unless
//! `--no-launch`). Its exit code is 0 when the view opened, or the view's raw code with `--wait` (D-23). The
//! control commands never launch; no GUI is exit 100 (D-22).

use hikasa_core::{ExitCode, Warning};
use hikasa_session::api::dto::{
    GUI_CLOSE_SCHEMA, GUI_LIST_SCHEMA, GUI_OPEN_SCHEMA, GUI_PING_SCHEMA, GUI_QUIT_SCHEMA,
    GuiCloseDto, GuiListDto, GuiQuitDto,
};
use hikasa_session::api::{ApiOp, GuiLauncher, GuiOpenRequest, OpenArgs, find_gui_exe, human};
use hikasa_session::ipc::{RetryPolicy, ViewId};

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
            Some(open) => open_view(ctx, &open.request, open.no_launch, &open.warnings),
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
