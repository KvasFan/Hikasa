//! `hikasa picture compare` (M2 W12, §1.12.23): the words as typed go to the Service, which parses them (T11, check 7);
//! exit codes D-339 (1, 2, 12, 13, 100); `--gui` sends the words to the window's Picture Compare view after checking
//! them here (D-350).

use hikasa_core::Pair;
use hikasa_session::ApiError;
use hikasa_session::api::dto::GUI_OPEN_SCHEMA;
use hikasa_session::api::{
    ApiOp, GuiOpenRequest, PictureCompareRequest, PictureGeometryWords, PictureRuleWords,
};

use super::report::{Shape, asks_report, run_picture};
use super::{Ctx, gui};
use crate::args::{PictureCmd, PictureCompareArgs};

/// Runs `picture …`.
pub fn run(ctx: &mut Ctx<'_>, cmd: &PictureCmd) -> u8 {
    match cmd {
        PictureCmd::Compare(args) => compare(ctx, args),
    }
}

/// The request of the flags (the words as typed).
pub fn request_of(ctx: &Ctx<'_>, args: &PictureCompareArgs) -> PictureCompareRequest {
    PictureCompareRequest {
        left: args.left.clone(),
        right: args.right.clone(),
        format: args.format.clone(),
        formats: Pair::new(args.left_format.clone(), args.right_format.clone()),
        // The words typed on the Picture Compare defaults session (D-374).
        rules: ctx.service.picture_rules_on_defaults(&PictureRuleWords {
            mode: args.mode.clone(),
            tolerance: args.tolerance.clone(),
            blend: args.blend.clone(),
            auto_scale: args.auto_scale,
            tone_map: args.tone_map.clone(),
            max_megapixels: args.max_megapixels.clone(),
            ignore_unimportant: args.ignore_unimportant,
        }),
        geometry: PictureGeometryWords {
            left_orientation: args.left_orientation.clone(),
            right_orientation: args.right_orientation.clone(),
            offset: args.offset.clone(),
        },
        probes: args.probe.clone(),
        diff_output: args.diff_output.clone(),
        diff_pane: args.diff_pane.clone(),
        checkerboard: args.checkerboard,
        view_checkerboard: None,
        metadata: args.metadata,
        format_overrides: Default::default(),
        verbose: ctx
            .matches
            .try_get_one::<bool>("verbose")
            .ok()
            .flatten()
            .copied()
            .unwrap_or(false),
        dry_run: args.report.dry_run,
    }
}

fn compare(ctx: &mut Ctx<'_>, args: &PictureCompareArgs) -> u8 {
    let request = request_of(ctx, args);
    if args.gui {
        let op = ApiOp::GuiOpen { wait: args.wait };
        // The window is the output (T-52's rule).
        if asks_report(&args.report.report_args())
            || args.diff_output.is_some()
            || !args.probe.is_empty()
        {
            let refused = ApiError::InvalidArgument(
                "the report flags, --diff-output and --probe do not apply with --gui (the window shows the \
                 comparison)"
                    .to_owned(),
            );
            return ctx.fail(GUI_OPEN_SCHEMA, &refused, op, &[]);
        }
        if let Err(e) = ctx.service.check_picture_words(&request) {
            return ctx.fail(GUI_OPEN_SCHEMA, &e, op, &[]);
        }
        let open = GuiOpenRequest::for_picture(&request, args.wait);
        return gui::open_view(ctx, &open, args.no_launch, &[]);
    }
    run_picture(ctx, &args.report.report_args(), request, Shape::Compare)
}
