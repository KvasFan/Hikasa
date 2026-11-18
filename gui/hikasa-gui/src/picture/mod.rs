//! The Picture Compare view model (PX-002…PX-004, PX-007…PX-009; M2 W13b, §1.13.9…§1.13.13, D-361…D-365):
//! `picture_load` and `picture_diff` jobs, then the option and geometry jobs of [`geometry`] by §1.13.9's table, the
//! pane generations that key the tiles of [`tiles`], the [`viewport`] (zoom steps, Fit, Actual Size, synchronized
//! panning), the offset drag and nudges, transforms, Auto Scale, the modes, Blend Toggle, the checkerboard, layouts,
//! full screen, the status bar, Info, Copy and Open Clipboard.
//!
//! Every pixel class, rendered pixel, statistic, probe and verdict is core's (`PictureComparison`,
//! `Service::picture_response`); the model holds geometry, input, focus and the order of jobs (check 7). Toolkit-free.

pub mod geometry;
pub mod info;
pub mod status;
pub mod tiles;
pub mod viewport;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hikasa_core::{JobKind, Pair, Side};
use hikasa_picture::{
    DiffMode, Geometry, Pane, PictureColors, PictureComparison, PictureOptions, PixelProbe,
};
use hikasa_session::api::{
    PictureCompareRequest, PictureCompareResponse, PictureLoaded, PictureViewState, ReportTarget,
    ViewOutcome, ViewRequest,
};
use hikasa_session::clipboard::OrderedClipboard;
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;
use hikasa_session::{ApiError, Service};
use parking_lot::{Condvar, Mutex};

use self::geometry::{OffsetDrag, Refine, RefineKind, Refined, Transform};
use self::tiles::{
    Basis, DrawItem, FrameInput, PaneFrame, RenderSource, TextureSink, TileState, pane_index,
};
use self::viewport::{PaneRect, Viewport, Zoom};
use crate::commands::{CommandId, Ctx};
use crate::job::{Job, Wake};
use crate::status::{JobStatus, LogNote, StatusItems};

/// The layout key of the view type (`[layout.picture-compare]`).
pub const VIEW: &str = "picture-compare";

/// The layouts (PX-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PictureLayout {
    /// Left | Right in one row, the difference pane under them across the width.
    #[default]
    SideBySide,
    /// Left above Right in one column, the difference pane beside them (*Over-Under Layout*).
    Stacked,
}

impl PictureLayout {
    /// The session word (D-349).
    pub fn word(self) -> &'static str {
        match self {
            PictureLayout::SideBySide => "side-by-side",
            PictureLayout::Stacked => "stacked",
        }
    }
}

/// An item of View ▸ Zoom ▸ (`ZoomList`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomChoice {
    /// Fit to Window.
    Fit,
    /// Actual Size.
    Actual,
    /// A percentage.
    Percent(u16),
}

impl ZoomChoice {
    /// The list's items in order.
    pub const LIST: [ZoomChoice; 8] = [
        ZoomChoice::Fit,
        ZoomChoice::Actual,
        ZoomChoice::Percent(25),
        ZoomChoice::Percent(50),
        ZoomChoice::Percent(100),
        ZoomChoice::Percent(200),
        ZoomChoice::Percent(400),
        ZoomChoice::Percent(800),
    ];

    /// The item's caption.
    pub fn caption(self) -> String {
        match self {
            ZoomChoice::Fit => "Fit to Window".to_owned(),
            ZoomChoice::Actual => "Actual Size".to_owned(),
            ZoomChoice::Percent(p) => format!("{p} %"),
        }
    }
}

/// The phase of the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PicturePhase {
    /// The pictures are decoded.
    Loading,
    /// They are compared.
    Comparing,
    /// The comparison landed.
    Ready,
    /// A side could not be loaded or the comparison failed.
    Failed,
}

/// What the view asks of its window (taken after each call).
#[derive(Debug, Clone, PartialEq)]
pub enum PictureRequest {
    /// A line of the message panel.
    Message(String),
    /// A warning notice with its log entry.
    Warning(String),
    /// Full screen of the window on or off (`ViewportCommand::Fullscreen`).
    FullScreen(bool),
    /// The browse dialog for a side (Open, *Browse…*).
    Browse(Side),
}

/// The kinds of job a test can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateKind {
    /// `picture_load`.
    Load,
    /// `picture_diff`.
    Diff,
    /// `with_options`.
    Options,
    /// `with_geometry`.
    Geometry,
}

/// A gate tests close to hold job bodies of a kind in flight.
#[derive(Debug, Default)]
pub struct Gate {
    closed: Mutex<HashSet<GateKind>>,
    cv: Condvar,
}

impl Gate {
    /// Holds jobs of `kind` until [`Gate::open`].
    pub fn close(&self, kind: GateKind) {
        self.closed.lock().insert(kind);
    }

    /// Lets jobs of `kind` run.
    pub fn open(&self, kind: GateKind) {
        self.closed.lock().remove(&kind);
        self.cv.notify_all();
    }

    /// Waits while `kind` is closed.
    pub fn pass(&self, kind: GateKind) {
        let mut closed = self.closed.lock();
        while closed.contains(&kind) {
            self.cv.wait(&mut closed);
        }
    }
}

/// Counters of the view's jobs (A13.10: nothing else runs).
#[derive(Debug, Default)]
pub struct JobCounters {
    /// `picture_load`.
    pub load: AtomicUsize,
    /// `picture_diff`.
    pub diff: AtomicUsize,
    /// `with_options`.
    pub options: AtomicUsize,
    /// `with_geometry`.
    pub geometry: AtomicUsize,
    /// `picture_bytes_identical`.
    pub identity: AtomicUsize,
    /// Option and geometry jobs running now.
    pub running: AtomicUsize,
    /// The most that ever ran at once (P-PICJOBS: one).
    pub max_running: AtomicUsize,
}

impl JobCounters {
    /// `[load, diff, options, geometry]`.
    pub fn snapshot(&self) -> [usize; 4] {
        [
            self.load.load(Ordering::SeqCst),
            self.diff.load(Ordering::SeqCst),
            self.options.load(Ordering::SeqCst),
            self.geometry.load(Ordering::SeqCst),
        ]
    }
}

/// How the view calls the Service (tests hold jobs with the gate and count them).
#[derive(Clone, Default)]
pub struct Calls {
    /// A gate (tests).
    pub gate: Option<Arc<Gate>>,
    /// The counters.
    pub counters: Arc<JobCounters>,
}

/// The pointer's state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// A pan (primary in the left or right pane, middle anywhere).
    Pan { last: (f64, f64) },
    /// The offset drag of the difference pane.
    Offset { last: (f64, f64), drag: OffsetDrag },
}

/// A pane's slot on screen: its header and its picture rectangle (points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneSlot {
    /// The pane.
    pub pane: Pane,
    /// The File Info header (zero height when hidden).
    pub header: PaneRect,
    /// The picture.
    pub picture: PaneRect,
}

/// The Picture Compare view model.
pub struct PictureViewModel {
    request: PictureCompareRequest,
    titles: [Option<String>; 2],
    names: [String; 2],
    wake: Wake,
    calls: Calls,
    clipboard: Arc<OrderedClipboard>,
    load: Option<Job<Arc<PictureLoaded>>>,
    diff: Option<Job<Refined>>,
    loaded: Option<Arc<PictureLoaded>>,
    comparison: Option<Arc<PictureComparison>>,
    identical: Option<bool>,
    errors: [Option<String>; 2],
    error: Option<String>,
    refine: Refine,
    copy: Option<(Job<(u32, u32)>, Pane)>,
    open_clipboard: Option<(Job<bool>, Side)>,
    /// The options wanted (the tolerance and the geometry; the landed ones are the comparison's).
    pub options: PictureOptions,
    /// The display mode (the session's; `blend_toggle` overrides its percentage).
    pub mode: DiffMode,
    /// F8's view-only override of the blend percentage.
    pub blend_toggle: Option<u8>,
    /// Ignore Unimportant.
    pub ignore_unimportant: bool,
    /// The viewport.
    pub viewport: Viewport,
    /// Left, right, difference shown.
    pub shown: [bool; 3],
    /// The focused pane.
    pub focus: Pane,
    /// The pane alone in the window (F11).
    pub full_screen: Option<Pane>,
    /// The transparency checkerboard.
    pub checkerboard: bool,
    /// The layout.
    pub layout: PictureLayout,
    /// The File Info headers.
    pub headers: bool,
    /// The fraction of the view the difference pane takes (`[layout.picture-compare] difference_split`).
    pub split: f64,
    /// Per pane: + 1 whenever its pixels change (§1.13.9's table).
    pub generations: [u64; 3],
    /// The tiles.
    pub tiles: TileState,
    colors: PictureColors,
    rects: Vec<PaneSlot>,
    drag: Option<Drag>,
    pointer: Option<(Pane, (f64, f64))>,
    probe: Option<PixelProbe>,
    frame_no: u64,
    session_center: Option<(f64, f64)>,
    requests: Vec<PictureRequest>,
    notes: Vec<LogNote>,
    write: crate::folder_view::WritePolicy,
    /// The geometry's message (`aspect_ratio_differs`) shown once per landing.
    warned_aspect: bool,
}

fn display_name(spec: &std::ffi::OsStr) -> String {
    crate::shared::spec_name(&spec.to_string_lossy()).to_owned()
}

impl PictureViewModel {
    /// Creates the view and starts loading. `geometry` and `mode` are the request's words parsed by core.
    pub fn new(
        service: &Service,
        request: PictureCompareRequest,
        geometry: Geometry,
        mode: DiffMode,
        titles: [Option<String>; 2],
        wake: Wake,
        clipboard: Arc<OrderedClipboard>,
    ) -> PictureViewModel {
        PictureViewModel::with_calls(
            service,
            request,
            geometry,
            mode,
            titles,
            wake,
            clipboard,
            Calls::default(),
        )
    }

    /// [`PictureViewModel::new`] with the jobs' gate and counters (tests).
    #[allow(clippy::too_many_arguments)]
    pub fn with_calls(
        service: &Service,
        request: PictureCompareRequest,
        geometry: Geometry,
        mode: DiffMode,
        titles: [Option<String>; 2],
        wake: Wake,
        clipboard: Arc<OrderedClipboard>,
        calls: Calls,
    ) -> PictureViewModel {
        let names = [display_name(&request.left), display_name(&request.right)];
        let settings = service.check_picture_words(&request).ok();
        let tolerance = settings.as_ref().map_or(0, |s| s.options.tolerance);
        let ignore_unimportant = settings.as_ref().is_some_and(|s| s.ignore_unimportant);
        let mut model = PictureViewModel {
            request,
            titles,
            names,
            wake,
            calls,
            clipboard,
            load: None,
            diff: None,
            loaded: None,
            comparison: None,
            identical: None,
            errors: [None, None],
            error: None,
            refine: Refine::default(),
            copy: None,
            open_clipboard: None,
            options: PictureOptions {
                tolerance,
                geometry,
            },
            mode,
            blend_toggle: None,
            ignore_unimportant,
            viewport: Viewport::default(),
            shown: [true; 3],
            focus: Pane::Left,
            full_screen: None,
            checkerboard: true,
            layout: PictureLayout::SideBySide,
            headers: true,
            split: 0.5,
            generations: [1; 3],
            tiles: TileState::default(),
            colors: service.picture_colors(),
            rects: Vec::new(),
            drag: None,
            pointer: None,
            probe: None,
            frame_no: 0,
            session_center: None,
            requests: Vec::new(),
            notes: Vec::new(),
            write: crate::folder_view::WritePolicy::default(),
            warned_aspect: false,
        };
        model.start(service);
        model
    }

    // ---- jobs ----------------------------------------------------------------------------------------------------

    /// (Re)loads both pictures and compares them (open, Reload, a tone map, a format, a spec).
    fn start(&mut self, service: &Service) {
        if let Some(job) = self.load.take() {
            job.cancel();
        }
        if let Some(job) = self.diff.take() {
            job.cancel();
        }
        self.refine.cancel();
        self.loaded = None;
        self.comparison = None;
        self.identical = None;
        self.errors = [None, None];
        self.error = None;
        self.probe = None;
        self.warned_aspect = false;
        for g in &mut self.generations {
            *g += 1;
        }
        let request = self.request.clone();
        let calls = self.calls.clone();
        let title = format!("{} ⇄ {}", self.names[0], self.names[1]);
        self.load = Some(Job::spawn(
            service,
            JobKind::PictureCompare,
            title,
            &self.wake,
            move |svc, job| {
                calls.counters.load.fetch_add(1, Ordering::SeqCst);
                geometry::pass(&calls.gate, GateKind::Load);
                Ok(Arc::new(svc.picture_load(&request, job)?))
            },
        ));
    }

    /// Compares the loaded pictures under the wanted options (`picture_diff`, and the bytes' identity when all same).
    fn start_diff(&mut self, service: &Service) {
        let Some(loaded) = self.loaded.clone() else {
            return;
        };
        if let Some(job) = self.diff.take() {
            job.cancel();
        }
        let options = self.options;
        let calls = self.calls.clone();
        let known = self.identical;
        let title = format!("{} ⇄ {}", self.names[0], self.names[1]);
        self.diff = Some(Job::spawn(
            service,
            JobKind::PictureCompare,
            title,
            &self.wake,
            move |svc, job| {
                calls.counters.diff.fetch_add(1, Ordering::SeqCst);
                geometry::pass(&calls.gate, GateKind::Diff);
                let comparison = svc.picture_diff(&loaded, &options, job)?;
                let identical = match known {
                    Some(k) => Some(k),
                    None => {
                        geometry::identity(svc, job, &loaded, &comparison, false, &calls.counters)?
                    }
                };
                Ok(Refined {
                    comparison,
                    identical,
                })
            },
        ));
    }

    /// Reload (and every change that decodes again).
    pub fn reload(&mut self, service: &Service) {
        self.start(service);
    }

    /// Polls the jobs (once per frame); true when something changed.
    pub fn poll(&mut self, service: &Service) -> bool {
        let mut changed = false;
        if let Some(result) = self.load.as_ref().and_then(Job::try_result) {
            self.load = None;
            changed = true;
            match result {
                Ok(loaded) => {
                    self.loaded = Some(loaded);
                    self.start_diff(service);
                }
                Err(e) => self.failed(e),
            }
        }
        if let Some(result) = self.diff.as_ref().and_then(Job::try_result) {
            self.diff = None;
            changed = true;
            match result {
                Ok(refined) => {
                    self.install(service, refined);
                    if self.landed_options() != Some(self.options) {
                        self.refine_now(service);
                    }
                }
                Err(e) => self.failed(e),
            }
        }
        if let Some((result, queued)) = self.refine.take() {
            changed = true;
            match result {
                Ok(refined) => {
                    self.install(service, refined);
                    if queued || self.landed_options() != Some(self.options) {
                        self.refine_now(service);
                    }
                }
                Err(ApiError::Cancelled) => {}
                Err(e) => {
                    self.requests.push(PictureRequest::Warning(e.to_string()));
                    self.notes.push(LogNote {
                        kind: LogKind::Warning,
                        text: e.to_string(),
                    });
                }
            }
        }
        if let Some(result) = self.copy.as_ref().and_then(|(j, _)| j.try_result()) {
            let pane = self.copy.take().map_or(Pane::Difference, |(_, p)| p);
            changed = true;
            match result {
                Ok((w, h)) => self.requests.push(PictureRequest::Message(format!(
                    "Copied the {} picture ({w} × {h}).",
                    pane_word(pane)
                ))),
                Err(e) => self.requests.push(PictureRequest::Message(e.to_string())),
            }
        }
        if let Some(result) = self
            .open_clipboard
            .as_ref()
            .and_then(|(j, _)| j.try_result())
        {
            let side = self.open_clipboard.take().map_or(Side::Right, |(_, s)| s);
            changed = true;
            match result {
                Ok(true) => {
                    let spec = std::ffi::OsString::from("clipboard://");
                    match side {
                        Side::Left => self.request.left = spec,
                        _ => self.request.right = spec,
                    }
                    self.names = [
                        display_name(&self.request.left),
                        display_name(&self.request.right),
                    ];
                    self.start(service);
                }
                Ok(false) => self
                    .requests
                    .push(PictureRequest::Message(NO_PICTURE.to_owned())),
                Err(e) => self.requests.push(PictureRequest::Message(e.to_string())),
            }
        }
        changed
    }

    fn failed(&mut self, e: ApiError) {
        let text = e.to_string();
        let left = self.request.left.to_string_lossy().into_owned();
        let side = match &e {
            ApiError::LoadFailed { side, .. } => Some(*side),
            ApiError::Picture(_, Some(spec)) if *spec == left => Some(Side::Left),
            ApiError::Picture(_, Some(_)) => Some(Side::Right),
            _ => None,
        };
        match side {
            Some(Side::Left) => self.errors[0] = Some(text.clone()),
            Some(Side::Right) => self.errors[1] = Some(text.clone()),
            _ => {}
        }
        if matches!(e, ApiError::Cancelled) {
            self.notes.push(LogNote {
                kind: LogKind::Summary,
                text: "Comparison cancelled.".to_owned(),
            });
        } else {
            self.notes.push(LogNote {
                kind: LogKind::Error,
                text: text.clone(),
            });
        }
        self.error = Some(text);
    }

    /// Installs a landed comparison: the pane generations of §1.13.9's table, the centre kept on the left picture when
    /// the area's origin moved.
    fn install(&mut self, service: &Service, refined: Refined) {
        let new = refined.comparison;
        if refined.identical.is_some() {
            self.identical = refined.identical;
        }
        if let Some(old) = &self.comparison {
            let (o, n) = (old.options(), new.options());
            let changed = |side: Side| {
                old.placement(side) != new.placement(side)
                    || o.geometry.orientation.get(side) != n.geometry.orientation.get(side)
                    || old.scaled() != new.scaled()
                        && (old.scaled().map(|s| s.0) == Some(side)
                            || new.scaled().map(|s| s.0) == Some(side))
            };
            if changed(Side::Left) {
                self.generations[0] += 1;
            }
            if changed(Side::Right) {
                self.generations[1] += 1;
            }
            let geometry = o.geometry != n.geometry || old.area() != new.area();
            let tolerance = o.tolerance != n.tolerance && self.shown_mode() == DiffMode::Tolerance;
            if geometry || tolerance {
                self.generations[2] += 1;
            }
            // The area's origin moved: the same left-picture point stays at the panes' centres.
            let (oa, na) = (old.area(), new.area());
            if (oa.x, oa.y) != (na.x, na.y) && matches!(self.viewport.zoom, Zoom::Scale(_)) {
                self.viewport.center.0 += (oa.x - na.x) as f64;
                self.viewport.center.1 += (oa.y - na.y) as f64;
            }
            // A new area size at Fit refits by itself; else the centre is clamped to the new area.
            if (oa.width, oa.height) != (na.width, na.height) {
                let panes = self.picture_rects();
                self.viewport.clamp(&panes, area_size(&new));
            }
        } else if let Some(center) = self.session_center.take()
            && matches!(self.viewport.zoom, Zoom::Scale(_))
        {
            self.viewport.center = center;
        }
        if !new.warnings().is_empty() && !self.warned_aspect {
            self.warned_aspect = true;
            for w in new.warnings() {
                self.requests
                    .push(PictureRequest::Message(w.to_warning().message));
            }
        }
        self.comparison = Some(new);
        // A probe of the old comparison is stale.
        if let Some((pane, p)) = self.pointer {
            self.update_probe(pane, p);
        }
        let _ = service;
    }

    /// The landed comparison's options.
    fn landed_options(&self) -> Option<PictureOptions> {
        self.comparison.as_ref().map(|c| *c.options())
    }

    /// Starts the option or geometry job for the wanted options (latest wins).
    fn refine_now(&mut self, service: &Service) -> Option<RefineKind> {
        let (Some(loaded), Some(base)) = (self.loaded.clone(), self.comparison.clone()) else {
            return None;
        };
        let know = self.identical.is_some();
        self.refine.request(
            service,
            &self.wake,
            &self.calls,
            &loaded,
            &base,
            self.options,
            know,
        )
    }

    /// Wants `options`: a job now, or when the running one lands.
    fn want(&mut self, service: &Service, options: PictureOptions) {
        self.options = options;
        self.refine_now(service);
    }

    /// True while a job runs (the window keeps polling).
    pub fn busy(&self) -> bool {
        self.load.is_some()
            || self.diff.is_some()
            || self.refine.busy()
            || self.copy.is_some()
            || self.open_clipboard.is_some()
    }

    /// True while a job's result is being published.
    pub fn settling(&self) -> bool {
        self.load.as_ref().is_some_and(Job::settling)
            || self.diff.as_ref().is_some_and(Job::settling)
            || self.refine.settling()
            || self.copy.as_ref().is_some_and(|(j, _)| j.settling())
    }

    /// Stops every job (the view is closing).
    pub fn cancel(&mut self) {
        if let Some(job) = &self.load {
            job.cancel();
        }
        if let Some(job) = &self.diff {
            job.cancel();
        }
        self.refine.cancel();
        if let Some((job, _)) = &self.copy {
            job.cancel();
        }
        self.tiles.cancel();
    }

    /// The running jobs (the job indicator).
    pub fn jobs(&self, view: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
        let mut out = Vec::new();
        let mut add = |title: &str, progress: (Option<hikasa_core::Phase>, Option<f32>)| {
            out.push(JobStatus {
                view,
                title: title.to_owned(),
                phase: progress.0,
                fraction: progress.1,
            });
        };
        if let Some(j) = &self.load {
            add(j.title(), j.progress());
        }
        if let Some(j) = &self.diff {
            add(j.title(), j.progress());
        }
        if let Some(j) = self.refine.job() {
            add(j.title(), j.progress());
        }
        if let Some((j, _)) = &self.copy {
            add(j.title(), j.progress());
        }
        out
    }

    /// The phase.
    pub fn phase(&self) -> PicturePhase {
        if self.comparison.is_some() {
            PicturePhase::Ready
        } else if self.error.is_some() {
            PicturePhase::Failed
        } else if self.loaded.is_some() {
            PicturePhase::Comparing
        } else {
            PicturePhase::Loading
        }
    }

    /// A side's load error.
    pub fn side_error(&self, side: Side) -> Option<&str> {
        match side {
            Side::Left => self.errors[0].as_deref(),
            _ => self.errors[1].as_deref(),
        }
    }

    /// The comparison's error.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The decoded pictures.
    pub fn loaded(&self) -> Option<&Arc<PictureLoaded>> {
        self.loaded.as_ref()
    }

    /// The landed comparison.
    pub fn comparison(&self) -> Option<&Arc<PictureComparison>> {
        self.comparison.as_ref()
    }

    /// The comparison request.
    pub fn request(&self) -> &PictureCompareRequest {
        &self.request
    }

    /// Tab caption.
    pub fn caption(&self) -> String {
        crate::shared::view_title(
            [self.titles[0].as_deref(), self.titles[1].as_deref()],
            [&self.names[0], &self.names[1]],
        )
    }

    /// The window's colours (UI-023): a change bumps the difference pane in Tolerance mode.
    pub fn set_colors(&mut self, colors: PictureColors) {
        if colors != self.colors {
            self.colors = colors;
            if self.shown_mode() == DiffMode::Tolerance {
                self.generations[2] += 1;
            }
        }
    }

    /// The colours the view renders with.
    pub fn colors(&self) -> PictureColors {
        self.colors
    }

    /// The mode the difference pane shows (Blend Toggle's percentage applied).
    pub fn shown_mode(&self) -> DiffMode {
        match (self.mode, self.blend_toggle) {
            (DiffMode::Blend { .. }, Some(p)) => DiffMode::Blend { percent: p },
            (mode, _) => mode,
        }
    }

    /// The response of the view's comparison (verdict, status, Info, reports, `--wait`): `picture_response`.
    pub fn response(&self, service: &Service) -> Option<PictureCompareResponse> {
        let (loaded, comparison) = (self.loaded.as_ref()?, self.comparison.as_ref()?);
        Some(service.picture_response(
            loaded,
            comparison,
            &PictureViewState {
                mode: self.mode,
                ignore_unimportant: self.ignore_unimportant,
                bytes_identical: self.identical.unwrap_or(false),
                colors: self.colors,
                verbose: false,
            },
        ))
    }

    /// The D-23 outcome when the view closes: the verdict under the view's tolerance, geometry and Ignore
    /// Unimportant (1, 2, 12, 13), `LoadFailed` (100) for a side that failed, `NoVerdict` while nothing landed.
    pub fn outcome(&mut self, service: &Service) -> ViewOutcome {
        self.poll(service);
        // A geometry or option job still running decides the verdict: it lands within a second or not at all.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while self.refine.busy() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll(service);
        }
        if self.comparison.is_none() && self.error.is_some() {
            return ViewOutcome::LoadFailed;
        }
        if self.refine.busy() || self.landed_options() != Some(self.options) {
            return ViewOutcome::NoVerdict;
        }
        match self.response(service) {
            Some(r) => ViewOutcome::Text {
                result: r.source().verdict(),
                ignore_unimportant: self.ignore_unimportant,
            },
            None if self.error.is_some() => ViewOutcome::LoadFailed,
            None => ViewOutcome::NoVerdict,
        }
    }

    // ---- panes and geometry on screen -----------------------------------------------------------------------------

    /// Arranges the shown panes in `content` (points) with headers of `header` points: the layout, the difference
    /// pane's split, full screen (the focused pane alone, no header). Kept for the pointer and the commands.
    pub fn arrange(&mut self, content: PaneRect, header: f64) -> Vec<PaneSlot> {
        let slot = |pane: Pane, r: PaneRect, header: f64| {
            let h = header.min(r.h);
            PaneSlot {
                pane,
                header: PaneRect::new(r.x, r.y, r.w, h),
                picture: PaneRect::new(r.x, r.y + h, r.w, (r.h - h).max(1.0)),
            }
        };
        let header = if self.headers { header } else { 0.0 };
        let rects = if let Some(pane) = self.full_screen {
            vec![slot(pane, content, 0.0)]
        } else {
            let diff = self.shown[2];
            let (main, side) = match (self.layout, diff) {
                (_, false) => (content, None),
                (PictureLayout::SideBySide, true) => {
                    let h = (content.h * (1.0 - self.split)).floor();
                    (
                        PaneRect::new(content.x, content.y, content.w, h),
                        Some(PaneRect::new(
                            content.x,
                            content.y + h,
                            content.w,
                            content.h - h,
                        )),
                    )
                }
                (PictureLayout::Stacked, true) => {
                    let w = (content.w * (1.0 - self.split)).floor();
                    (
                        PaneRect::new(content.x, content.y, w, content.h),
                        Some(PaneRect::new(
                            content.x + w,
                            content.y,
                            content.w - w,
                            content.h,
                        )),
                    )
                }
            };
            let (l, r) = match self.layout {
                PictureLayout::SideBySide => {
                    let w = (main.w / 2.0).floor();
                    (
                        PaneRect::new(main.x, main.y, w, main.h),
                        PaneRect::new(main.x + w, main.y, main.w - w, main.h),
                    )
                }
                PictureLayout::Stacked => {
                    let h = (main.h / 2.0).floor();
                    (
                        PaneRect::new(main.x, main.y, main.w, h),
                        PaneRect::new(main.x, main.y + h, main.w, main.h - h),
                    )
                }
            };
            let mut v = vec![slot(Pane::Left, l, header), slot(Pane::Right, r, header)];
            if let Some(d) = side {
                v.push(slot(Pane::Difference, d, header));
            }
            v
        };
        self.rects = rects.clone();
        rects
    }

    /// The panes' slots of the last [`PictureViewModel::arrange`].
    pub fn slots(&self) -> &[PaneSlot] {
        &self.rects
    }

    fn picture_rects(&self) -> Vec<PaneRect> {
        self.rects.iter().map(|s| s.picture).collect()
    }

    fn rect_of(&self, pane: Pane) -> Option<PaneRect> {
        self.rects
            .iter()
            .find(|s| s.pane == pane)
            .map(|s| s.picture)
    }

    /// The area's size in pixels (1 × 1 before a comparison).
    pub fn area(&self) -> (f64, f64) {
        self.comparison
            .as_ref()
            .map_or((1.0, 1.0), |c| area_size(c))
    }

    /// The scale (Fit resolved).
    pub fn scale(&self) -> f64 {
        self.viewport.scale(&self.picture_rects(), self.area())
    }

    /// Sets the window's pixels per point.
    pub fn set_ppp(&mut self, ppp: f64) {
        if ppp > 0.0 && (ppp - self.viewport.ppp).abs() > f64::EPSILON {
            self.viewport.ppp = ppp;
        }
    }

    /// The pane under a point.
    pub fn pane_at(&self, p: (f64, f64)) -> Option<Pane> {
        self.rects
            .iter()
            .find(|s| s.picture.contains(p))
            .map(|s| s.pane)
    }

    /// The screen rectangle of the area in `pane` (points): `(x0, y0, x1, y1)`.
    pub fn area_on_screen(&self, pane: Pane) -> Option<(f64, f64, f64, f64)> {
        let rect = self.rect_of(pane)?;
        let area = self.area();
        let scale = self.scale();
        let a = self.viewport.to_screen(rect, scale, area, (0.0, 0.0));
        let b = self.viewport.to_screen(rect, scale, area, area);
        Some((a.0, a.1, b.0, b.1))
    }

    /// The right picture's outline in the difference pane at the wanted offset while it differs from the landed one
    /// (an offset drag): `(x0, y0, x1, y1)` in points.
    pub fn offset_outline(&self) -> Option<(f64, f64, f64, f64)> {
        let c = self.comparison.as_ref()?;
        let landed = c.options().geometry.offset;
        let wanted = self.options.geometry.offset;
        if landed == wanted && !matches!(self.drag, Some(Drag::Offset { .. })) {
            return None;
        }
        let rect = self.rect_of(Pane::Difference)?;
        let placed = c.placement(Side::Right)?;
        let area = c.area();
        let x = (placed.x - area.x) as f64 + f64::from(wanted.0 - landed.0);
        let y = (placed.y - area.y) as f64 + f64::from(wanted.1 - landed.1);
        let scale = self.scale();
        let areaf = self.area();
        let a = self.viewport.to_screen(rect, scale, areaf, (x, y));
        let b = self.viewport.to_screen(
            rect,
            scale,
            areaf,
            (x + f64::from(placed.width), y + f64::from(placed.height)),
        );
        Some((a.0, a.1, b.0, b.1))
    }

    /// The File Info header of a pane (§1.13.13, `ShowSpecInfo`): a side's name, then its picture's facts from
    /// `PictureInfo` (dimensions, bit depth, the decoder's colour type, alpha, frames, the orientation applied, the
    /// tone map, the file size, *Converted*) or its load error; the difference pane's area, mode, tolerance and verdict.
    pub fn header_lines(&self, pane: Pane, service: &Service) -> Vec<String> {
        let side = match pane {
            Pane::Left => Some(Side::Left),
            Pane::Right => Some(Side::Right),
            Pane::Difference => None,
        };
        let Some(side) = side else {
            let mut out = Vec::new();
            if let Some(c) = &self.comparison {
                let a = c.area();
                out.push(format!(
                    "Difference {}  {}",
                    hikasa_report::picture::size_text(a.width, a.height),
                    status::mode_text(self)
                ));
                if let Some(r) = self.response(service) {
                    out.push(crate::table_view::verdict_text(r.kind).to_owned());
                }
            } else {
                out.push("Difference".to_owned());
            }
            return out;
        };
        let spec = match side {
            Side::Left => &self.request.left,
            _ => &self.request.right,
        };
        let mut out = vec![crate::shared::shown_path(&spec.to_string_lossy())];
        if let Some(e) = self.side_error(side) {
            out.push(e.to_owned());
            return out;
        }
        let Some(loaded) = &self.loaded else {
            out.push("Decoding…".to_owned());
            return out;
        };
        let picture = match side {
            Side::Left => &loaded.pictures.left,
            _ => &loaded.pictures.right,
        };
        let i = &picture.info;
        let mut facts = vec![
            hikasa_report::picture::size_text(i.width, i.height),
            i.format.as_str().to_owned(),
            format!("{} bits", i.bit_depth),
            i.color.as_str().to_owned(),
        ];
        if i.has_alpha {
            facts.push("alpha".to_owned());
        }
        if let hikasa_picture::FrameCount::Many(n) = i.frames {
            facts.push(format!("first of {n} frames"));
        }
        if let Some(o) = i.orientation {
            facts.push(format!("oriented {}", o.as_str()));
        }
        if let Some(t) = i.hdr {
            facts.push(format!("tone map {}", t.as_str()));
        }
        facts.push(format!("{} bytes", i.file_size));
        if i.converted {
            facts.push("Converted".to_owned());
        }
        out.push(facts.join("  "));
        out
    }

    // ---- frames: tiles and drawing --------------------------------------------------------------------------------

    /// One frame of the tiles: wanted keys, the batch job, ≤ 8 uploads into `sink`, evictions. `budget` is the bytes
    /// this view may hold. True when the window should paint again soon.
    pub fn frame(&mut self, service: &Service, budget: u64, sink: &mut dyn TextureSink) -> bool {
        self.frame_no += 1;
        let Some(input) = self.frame_input(budget) else {
            return false;
        };
        let panes: Vec<PaneFrame> = self
            .rects
            .iter()
            .map(|s| PaneFrame {
                pane: s.pane,
                rect: s.picture,
            })
            .collect();
        let source = self.comparison.as_ref().map(|c| RenderSource {
            comparison: Arc::clone(c),
            mode: self.shown_mode(),
            colors: self.colors,
            basis: basis_of(c),
        });
        let changed = self
            .tiles
            .frame(service, &self.wake, &input, &panes, source.as_ref(), sink);
        changed || self.tiles.busy() || self.drag.is_some()
    }

    fn frame_input(&self, budget: u64) -> Option<FrameInput> {
        let c = self.comparison.as_ref()?;
        Some(FrameInput {
            viewport: self.viewport,
            scale: self.scale(),
            basis: basis_of(c),
            target_offset: self.options.geometry.offset,
            generations: self.generations,
            frame: self.frame_no,
            budget,
        })
    }

    /// What the tile jobs render with now (tests render by hand).
    #[cfg(test)]
    pub(crate) fn render_source(&self) -> Option<RenderSource> {
        self.comparison.as_ref().map(|c| RenderSource {
            comparison: Arc::clone(c),
            mode: self.shown_mode(),
            colors: self.colors,
            basis: basis_of(c),
        })
    }

    /// What the painter draws in `pane`, under the pixel grid and the outline.
    pub fn draw_list(&self, pane: Pane) -> Vec<DrawItem> {
        let (Some(input), Some(rect)) =
            (self.frame_input(tiles::TEXTURE_BUDGET), self.rect_of(pane))
        else {
            return Vec::new();
        };
        self.tiles.draw_list(&input, &PaneFrame { pane, rect })
    }

    /// The pixel grid's lines of `pane` at scales ≥ 1,600 %: `(vertical x positions, horizontal y positions)` in
    /// points, clipped to the area and the pane.
    pub fn grid_lines(&self, pane: Pane) -> Option<(Vec<f64>, Vec<f64>)> {
        let scale = self.scale();
        if scale < viewport::GRID_SCALE {
            return None;
        }
        let rect = self.rect_of(pane)?;
        let area = self.area();
        let (x0, y0, x1, y1) = self.viewport.visible(rect, scale, area);
        let xs = (x0.max(0.0).ceil() as i64..=x1.min(area.0).floor() as i64)
            .map(|x| {
                self.viewport
                    .to_screen(rect, scale, area, (x as f64, 0.0))
                    .0
            })
            .collect();
        let ys = (y0.max(0.0).ceil() as i64..=y1.min(area.1).floor() as i64)
            .map(|y| {
                self.viewport
                    .to_screen(rect, scale, area, (0.0, y as f64))
                    .1
            })
            .collect();
        Some((xs, ys))
    }

    /// The checkerboard's anchor in `pane`: the area's top-left corner on screen (points) and the cell size in points
    /// (8 physical pixels, PX-007).
    pub fn checker_anchor(&self, pane: Pane) -> Option<((f64, f64), f64)> {
        let (x0, y0, _, _) = self.area_on_screen(pane)?;
        Some((
            (x0, y0),
            f64::from(hikasa_picture::render::CHECKER_CELL) / self.viewport.ppp,
        ))
    }

    /// Hides the view's tiles (its tab is not shown).
    pub fn hidden(&mut self, sink: &mut dyn TextureSink) {
        self.tiles.drop_tiles(sink);
    }

    // ---- pointer and keys ------------------------------------------------------------------------------------------

    /// The pointer moved to `p` (points): the probe of the area pixel under it (O(1), PX-008).
    pub fn pointer_moved(&mut self, p: (f64, f64)) {
        match self.pane_at(p) {
            Some(pane) => {
                self.pointer = Some((pane, p));
                self.update_probe(pane, p);
            }
            None => {
                self.pointer = None;
                self.probe = None;
            }
        }
    }

    /// The pointer left the panes.
    pub fn pointer_left(&mut self) {
        self.pointer = None;
        self.probe = None;
    }

    fn update_probe(&mut self, pane: Pane, p: (f64, f64)) {
        let (Some(c), Some(rect)) = (self.comparison.as_ref(), self.rect_of(pane)) else {
            return;
        };
        let a = self.viewport.to_area(rect, self.scale(), area_size(c), p);
        let area = c.area();
        self.probe = (a.0 >= 0.0
            && a.1 >= 0.0
            && a.0 < f64::from(area.width)
            && a.1 < f64::from(area.height))
        .then(|| c.probe(a.0 as u32, a.1 as u32));
    }

    /// The probe under the pointer.
    pub fn probe(&self) -> Option<&PixelProbe> {
        self.probe.as_ref()
    }

    /// A button pressed at `p`: focuses the pane; a primary press in the difference pane starts the offset drag, in
    /// the left or right pane a pan; a middle press pans in any pane.
    pub fn press(&mut self, p: (f64, f64), middle: bool) {
        let Some(pane) = self.pane_at(p) else {
            return;
        };
        self.focus = pane;
        self.drag = if pane == Pane::Difference && !middle && self.comparison.is_some() {
            Some(Drag::Offset {
                last: p,
                drag: OffsetDrag::new(self.options.geometry.offset),
            })
        } else {
            Some(Drag::Pan { last: p })
        };
    }

    /// The pointer moved to `p` with a button down.
    pub fn drag_to(&mut self, service: &Service, p: (f64, f64)) {
        match self.drag {
            Some(Drag::Pan { last }) => {
                let panes = self.picture_rects();
                let area = self.area();
                self.viewport
                    .pan_by(&panes, area, (p.0 - last.0, p.1 - last.1));
                self.drag = Some(Drag::Pan { last: p });
            }
            Some(Drag::Offset { last, mut drag }) => {
                let scale = self.scale();
                let d = drag.moved((p.0 - last.0, p.1 - last.1), scale, self.viewport.ppp);
                self.drag = Some(Drag::Offset { last: p, drag });
                if d != (0, 0) {
                    self.move_offset(service, d);
                }
            }
            None => {}
        }
        self.pointer_moved(p);
    }

    /// The button was released: a drag ends where it is.
    pub fn release(&mut self) {
        self.drag = None;
    }

    /// True while a drag runs (the window repaints every frame).
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Esc: a drag returns to the offset at its start; else full screen ends. True when it did something.
    pub fn escape(&mut self, service: &Service) -> bool {
        if let Some(Drag::Offset { drag, .. }) = self.drag.take() {
            let mut options = self.options;
            options.geometry.offset = drag.start;
            self.want(service, options);
            return true;
        }
        if self.drag.take().is_some() {
            return true;
        }
        if self.full_screen.is_some() {
            self.set_full_screen(false);
            return true;
        }
        false
    }

    /// The wheel at `p`: with Ctrl one zoom step a notch about the pointer; else a pan (Shift: horizontal).
    pub fn wheel(&mut self, p: (f64, f64), delta: (f64, f64), ctrl: bool, shift: bool) {
        let Some(pane) = self.pane_at(p).and_then(|pane| self.rect_of(pane)) else {
            return;
        };
        let panes = self.picture_rects();
        let area = self.area();
        if ctrl {
            let up = delta.1 > 0.0 || (delta.1 == 0.0 && delta.0 > 0.0);
            if delta != (0.0, 0.0) {
                self.viewport.step_at(&panes, area, pane, p, up);
            }
        } else if shift {
            let d = if delta.0 != 0.0 { delta.0 } else { delta.1 };
            self.viewport.pan_by(&panes, area, (d, 0.0));
        } else {
            self.viewport.pan_by(&panes, area, delta);
        }
    }

    /// A pinch about `p`: the scale times `factor`, clamped to the ends.
    pub fn pinch(&mut self, p: (f64, f64), factor: f64) {
        let Some(pane) = self.pane_at(p).and_then(|pane| self.rect_of(pane)) else {
            return;
        };
        let panes = self.picture_rects();
        let area = self.area();
        let to = self.viewport.scale(&panes, area) * factor;
        self.viewport.zoom_at(&panes, area, pane, p, to);
    }

    /// An arrow: nudges the offset 1 pixel (10 with Ctrl, PX-004).
    pub fn nudge(&mut self, service: &Service, dx: i64, dy: i64, ctrl: bool) {
        let k = if ctrl { 10 } else { 1 };
        self.move_offset(service, (dx * k, dy * k));
    }

    /// PgUp/PgDn (a pane height up or down) and Home/End (the area's top or bottom).
    pub fn page(&mut self, down: bool, end: Option<bool>) {
        let Some(rect) = self
            .rect_of(self.focus)
            .or_else(|| self.picture_rects().first().copied())
        else {
            return;
        };
        let panes = self.picture_rects();
        let area = self.area();
        match end {
            Some(bottom) => {
                let c = self.viewport.center_of(area);
                let y = if bottom {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                };
                self.viewport.center_on(&panes, area, (c.0, y));
                let ((_, _), (y0, y1)) = self.viewport.center_range(&panes, area);
                let c = self.viewport.center;
                self.viewport.center = (c.0, if bottom { y1 } else { y0 });
            }
            None => {
                let d = if down { -rect.h } else { rect.h };
                self.viewport.pan_by(&panes, area, (0.0, d));
            }
        }
    }

    /// Moves the wanted offset by `d` area pixels, within ±2²⁰ (a message at the limit).
    fn move_offset(&mut self, service: &Service, d: (i64, i64)) {
        let (offset, stopped) = geometry::add_offset(self.options.geometry.offset, d);
        if stopped {
            self.requests.push(PictureRequest::Message(
                "The offset stops at ±1,048,576 pixels.".to_owned(),
            ));
        }
        if offset != self.options.geometry.offset {
            let mut options = self.options;
            options.geometry.offset = offset;
            self.want(service, options);
        }
    }

    // ---- commands --------------------------------------------------------------------------------------------------

    /// Sets the tolerance (the toolbar spinner, the Picture tab): `with_options`.
    pub fn set_tolerance(&mut self, service: &Service, tolerance: u8) {
        if tolerance != self.options.tolerance {
            let mut options = self.options;
            options.tolerance = tolerance;
            self.request.rules.tolerance = Some(tolerance.to_string());
            self.want(service, options);
        }
    }

    /// Sets the mode (no job): the difference pane only.
    pub fn set_mode(&mut self, mode: DiffMode) {
        let mode = match (mode, self.mode) {
            // Blend keeps the session's percentage.
            (DiffMode::Blend { .. }, DiffMode::Blend { percent }) => DiffMode::Blend { percent },
            (DiffMode::Blend { .. }, _) => DiffMode::Blend {
                percent: self.blend_percent_word(),
            },
            (m, _) => m,
        };
        if mode != self.mode {
            self.mode = mode;
            self.blend_toggle = None;
            self.write_mode_words();
            self.generations[2] += 1;
        }
    }

    fn blend_percent_word(&self) -> u8 {
        self.request
            .rules
            .blend
            .as_deref()
            .and_then(|w| w.parse::<u8>().ok())
            .filter(|p| *p <= 100)
            .unwrap_or(DiffMode::DEFAULT_BLEND)
    }

    fn write_mode_words(&mut self) {
        self.request.rules.mode = Some(self.mode.as_str().to_owned());
        self.request.rules.blend = match self.mode {
            DiffMode::Blend { percent } => Some(percent.to_string()),
            _ => None,
        };
    }

    /// The blend slider: the percentage (Blend mode), clearing F8's override.
    pub fn set_blend(&mut self, percent: u8) {
        let percent = percent.min(100);
        let before = self.shown_mode();
        self.mode = DiffMode::Blend { percent };
        self.blend_toggle = None;
        self.write_mode_words();
        if self.shown_mode() != before {
            self.generations[2] += 1;
        }
    }

    /// Blend Toggle (F8): in Blend mode the shown percentage flips between 0 and 100; outside Blend mode it switches
    /// to Blend at 0 first. View-only (never dirties the session).
    pub fn blend_toggle(&mut self) {
        let before = self.shown_mode();
        match self.shown_mode() {
            DiffMode::Blend { percent } => {
                self.blend_toggle = Some(if percent == 0 { 100 } else { 0 });
            }
            _ => {
                let keep = self.blend_percent_word();
                self.mode = DiffMode::Blend { percent: keep };
                self.write_mode_words();
                self.blend_toggle = Some(0);
            }
        }
        if self.shown_mode() != before {
            self.generations[2] += 1;
        }
    }

    /// A transform of the current picture (the focused pane's; the difference pane's is the right one).
    pub fn transform(&mut self, service: &Service, t: Transform) {
        let geometry = geometry::transformed(self.options.geometry, self.focus, t);
        let mut options = self.options;
        options.geometry = geometry;
        self.request.geometry.left_orientation =
            Some(geometry.orientation.left.as_str().to_owned());
        self.request.geometry.right_orientation =
            Some(geometry.orientation.right.as_str().to_owned());
        self.want(service, options);
    }

    /// Auto Scale on or off.
    pub fn set_auto_scale(&mut self, service: &Service, on: bool) {
        let mut options = self.options;
        options.geometry.auto_scale = on;
        self.request.rules.auto_scale = on;
        self.warned_aspect = false;
        self.want(service, options);
    }

    /// Reset Difference Offset.
    pub fn reset_offset(&mut self, service: &Service) {
        let mut options = self.options;
        options.geometry.offset = (0, 0);
        self.want(service, options);
    }

    /// Full screen of the focused pane on or off.
    pub fn set_full_screen(&mut self, on: bool) {
        let target = on.then_some(self.focus);
        if target != self.full_screen {
            self.full_screen = target;
            self.requests.push(PictureRequest::FullScreen(on));
        }
    }

    /// Focuses the next (or previous) shown pane (F6, Shift+F6).
    pub fn cycle_focus(&mut self, back: bool) {
        let shown: Vec<Pane> = tiles::PANES
            .into_iter()
            .filter(|p| self.shown[pane_index(*p)])
            .collect();
        let at = shown.iter().position(|p| *p == self.focus).unwrap_or(0);
        let n = shown.len();
        if n > 0 {
            self.focus = shown[if back { (at + n - 1) % n } else { (at + 1) % n }];
        }
    }

    /// Zoom to a list item.
    pub fn zoom_to(&mut self, choice: ZoomChoice) {
        let panes = self.picture_rects();
        let area = self.area();
        match choice {
            ZoomChoice::Fit => self.viewport.zoom = Zoom::Fit,
            ZoomChoice::Actual => self.zoom_about_focus(1.0, &panes, area),
            ZoomChoice::Percent(p) => self.zoom_about_focus(f64::from(p) / 100.0, &panes, area),
        }
    }

    fn zoom_about_focus(&mut self, to: f64, panes: &[PaneRect], area: (f64, f64)) {
        let rect = self
            .rect_of(self.focus)
            .or_else(|| panes.first().copied())
            .unwrap_or(PaneRect::new(0.0, 0.0, 1.0, 1.0));
        self.viewport.zoom_at(panes, area, rect, rect.center(), to);
    }

    /// Zoom In or Out one step about the focused pane's centre.
    pub fn zoom_step(&mut self, up: bool) {
        let panes = self.picture_rects();
        let area = self.area();
        let rect = self
            .rect_of(self.focus)
            .or_else(|| panes.first().copied())
            .unwrap_or(PaneRect::new(0.0, 0.0, 1.0, 1.0));
        self.viewport.step_at(&panes, area, rect, rect.center(), up);
    }

    /// Info's *Show*: fits the viewport to the bounds of the non-same pixels.
    pub fn show_bounds(&mut self) {
        let Some(b) = self.comparison.as_ref().and_then(|c| c.stats().bounds) else {
            return;
        };
        let panes = self.picture_rects();
        let area = self.area();
        let o = self
            .comparison
            .as_ref()
            .map_or((0, 0), |c| (c.area().x, c.area().y));
        let _ = o;
        self.viewport.show_rect(
            &panes,
            area,
            (
                b.x as f64,
                b.y as f64,
                f64::from(b.width),
                f64::from(b.height),
            ),
        );
    }

    /// Swap Sides (§1.13.12): the pictures exchanged, `Geometry::swapped`, a `picture_diff` (no decode).
    pub fn swap(&mut self, service: &Service) {
        std::mem::swap(&mut self.request.left, &mut self.request.right);
        let f = &mut self.request.formats;
        std::mem::swap(&mut f.left, &mut f.right);
        self.titles.swap(0, 1);
        self.names.swap(0, 1);
        self.errors.swap(0, 1);
        let g = self.options.geometry.swapped();
        self.options.geometry = g;
        self.request.geometry.left_orientation = Some(g.orientation.left.as_str().to_owned());
        self.request.geometry.right_orientation = Some(g.orientation.right.as_str().to_owned());
        self.request.geometry.offset = Some(format!("{},{}", g.offset.0, g.offset.1));
        let Some(loaded) = self.loaded.clone() else {
            self.start(service);
            return;
        };
        self.refine.cancel();
        let swapped = Arc::new(PictureLoaded {
            specs: Pair::new(loaded.specs.right.clone(), loaded.specs.left.clone()),
            pictures: Pair::new(
                Arc::clone(&loaded.pictures.right),
                Arc::clone(&loaded.pictures.left),
            ),
            formats: Pair::new(loaded.formats.right.clone(), loaded.formats.left.clone()),
            sides: Pair::new(loaded.sides.right.clone(), loaded.sides.left.clone()),
            tone_map: loaded.tone_map,
        });
        self.loaded = Some(Arc::clone(&swapped));
        self.comparison = None;
        for g in &mut self.generations {
            *g += 1;
        }
        self.start_diff(service);
    }

    /// Copy (PX-009): the focused pane at full resolution in the shown mode with the view's checkerboard flag and the
    /// window's colours (`picture_write_with`), placed on the window's clipboard in a job.
    pub fn copy(&mut self, service: &Service) {
        let Some(c) = self.comparison.clone() else {
            return;
        };
        let pane = self.focus;
        let mode = self.shown_mode();
        let checkerboard = self.checkerboard;
        let colors = self.colors;
        let clipboard = Arc::clone(&self.clipboard);
        let ticket = clipboard.ticket();
        let job = Job::spawn(
            service,
            JobKind::PictureCompare,
            format!("copy the {} picture", pane_word(pane)),
            &self.wake,
            move |svc, job| {
                let written = svc.picture_write_with(
                    &c,
                    pane,
                    mode,
                    checkerboard,
                    &colors,
                    &ReportTarget::Clipboard,
                    job,
                )?;
                let image = written.clipboard.ok_or_else(|| {
                    ApiError::Internal("the image was not rendered for the clipboard".into())
                })?;
                clipboard
                    .in_turn(ticket, |c| {
                        c.write_image(image.width, image.height, &image.rgba)
                    })
                    .map_err(|e| ApiError::Vfs(e.to_vfs_write()))?;
                Ok((image.width, image.height))
            },
        );
        self.copy = Some((job, pane));
    }

    /// Open Clipboard (PX-009): the current picture's side (the difference pane: the right) becomes `clipboard://`
    /// and is reloaded; a clipboard without an image is a message.
    pub fn open_clipboard(&mut self, service: &Service) {
        let side = match self.focus {
            Pane::Left => Side::Left,
            _ => Side::Right,
        };
        let job = Job::spawn(
            service,
            JobKind::VfsRead,
            "read the clipboard".into(),
            &self.wake,
            move |svc, _| match svc.read_clipboard() {
                Ok(dto) => Ok(dto.kind == "image"),
                Err(_) => Ok(false),
            },
        );
        self.open_clipboard = Some((job, side));
    }

    /// A side's spec replaced (a header's path field, a drop, Open): reloads.
    pub fn set_spec(&mut self, service: &Service, side: Side, spec: std::ffi::OsString) {
        match side {
            Side::Left => self.request.left = spec,
            _ => self.request.right = spec,
        }
        self.names = [
            display_name(&self.request.left),
            display_name(&self.request.right),
        ];
        self.start(service);
    }

    /// A side's format (`None`: Auto): reloads.
    pub fn set_format(&mut self, service: &Service, side: Side, format: Option<String>) {
        match side {
            Side::Left => self.request.formats.left = format,
            _ => self.request.formats.right = format,
        }
        self.request.format = None;
        self.start(service);
    }

    /// The rule words a Picture tab wrote: what changed reruns by §1.13.9's table (tone map and megapixels reload).
    pub fn apply_rules(&mut self, service: &Service, request: PictureCompareRequest) {
        let Ok(settings) = service.check_picture_words(&request) else {
            return;
        };
        // What decodes again: the decode options as core parses them, a side's effective format.
        let sides = |r: &PictureCompareRequest| {
            [
                r.formats.left.clone().or_else(|| r.format.clone()),
                r.formats.right.clone().or_else(|| r.format.clone()),
            ]
        };
        let decode_changed = service
            .check_picture_words(&self.request)
            .map_or(true, |old| old.decode != settings.decode)
            || sides(&request) != sides(&self.request);
        self.request.rules = request.rules.clone();
        self.request.formats = request.formats.clone();
        self.request.format = request.format.clone();
        self.ignore_unimportant = settings.ignore_unimportant;
        let mode = settings.mode;
        if mode != self.mode {
            self.mode = mode;
            self.blend_toggle = None;
            self.generations[2] += 1;
        }
        let mut options = self.options;
        options.tolerance = settings.options.tolerance;
        options.geometry.auto_scale = settings.options.geometry.auto_scale;
        if decode_changed {
            self.options = options;
            self.start(service);
        } else {
            self.want(service, options);
        }
    }

    /// Executes a view command; false when it is not this view's.
    pub fn execute(&mut self, service: &Service, id: CommandId) -> bool {
        match id {
            CommandId::ZoomIn => self.zoom_step(true),
            CommandId::ZoomOut => self.zoom_step(false),
            CommandId::ZoomFit => self.zoom_to(ZoomChoice::Fit),
            CommandId::ZoomActual => self.zoom_to(ZoomChoice::Actual),
            CommandId::DiffTolerance => self.set_mode(DiffMode::Tolerance),
            CommandId::DiffRange => self.set_mode(DiffMode::Range),
            CommandId::DiffBlend => self.set_mode(DiffMode::Blend {
                percent: DiffMode::DEFAULT_BLEND,
            }),
            CommandId::BlendToggle => self.blend_toggle(),
            CommandId::AutoScale => {
                let on = !self.options.geometry.auto_scale;
                self.set_auto_scale(service, on);
            }
            CommandId::RotateRight => self.transform(service, Transform::RotateRight),
            CommandId::RotateLeft => self.transform(service, Transform::RotateLeft),
            CommandId::FlipHorz => self.transform(service, Transform::FlipHorz),
            CommandId::FlipVert => self.transform(service, Transform::FlipVert),
            CommandId::OffsetReset => self.reset_offset(service),
            CommandId::Checkerboard => self.checkerboard = !self.checkerboard,
            CommandId::ShowDifferencePane => {
                self.shown[2] = !self.shown[2];
                if !self.shown[2] && self.focus == Pane::Difference {
                    self.focus = Pane::Left;
                }
            }
            CommandId::SideBySide => self.layout = PictureLayout::SideBySide,
            CommandId::OverUnder => self.layout = PictureLayout::Stacked,
            CommandId::ShowSpecInfo => self.headers = !self.headers,
            CommandId::FullScreen => {
                let on = self.full_screen.is_none();
                self.set_full_screen(on);
            }
            CommandId::InsigIgnored => {
                self.ignore_unimportant = !self.ignore_unimportant;
                self.request.rules.ignore_unimportant = self.ignore_unimportant;
            }
            CommandId::GoNext => self.cycle_focus(false),
            CommandId::GoPrev => self.cycle_focus(true),
            CommandId::Reload => self.reload(service),
            CommandId::Copy => self.copy(service),
            CommandId::OpenClipboard => self.open_clipboard(service),
            CommandId::Open | CommandId::OpenLocal => {
                let side = match self.focus {
                    Pane::Left => Side::Left,
                    _ => Side::Right,
                };
                self.requests.push(PictureRequest::Browse(side));
            }
            _ => return false,
        }
        true
    }

    /// The predicates' part of the view (`PictureCompared`, `BlendMode`, `OffsetSet`).
    pub fn ctx_into(&self, ctx: &mut Ctx) {
        ctx.picture_compared = self.comparison.is_some();
        ctx.blend_mode = matches!(self.mode, DiffMode::Blend { .. });
        ctx.offset_set = self.options.geometry.offset != (0, 0);
    }

    /// The check and radio state of a command.
    pub fn checked(&self, id: CommandId) -> Option<bool> {
        Some(match id {
            CommandId::DiffTolerance => self.mode == DiffMode::Tolerance,
            CommandId::DiffRange => self.mode == DiffMode::Range,
            CommandId::DiffBlend => matches!(self.mode, DiffMode::Blend { .. }),
            CommandId::AutoScale => self.options.geometry.auto_scale,
            CommandId::Checkerboard => self.checkerboard,
            CommandId::ShowDifferencePane => self.shown[2],
            CommandId::SideBySide => self.layout == PictureLayout::SideBySide,
            CommandId::OverUnder => self.layout == PictureLayout::Stacked,
            CommandId::ShowSpecInfo => self.headers,
            CommandId::FullScreen => self.full_screen.is_some(),
            CommandId::InsigIgnored => self.ignore_unimportant,
            CommandId::ZoomFit => matches!(self.viewport.zoom, Zoom::Fit),
            _ => return None,
        })
    }

    /// What the view asked of its window since the last call.
    pub fn take_requests(&mut self) -> Vec<PictureRequest> {
        std::mem::take(&mut self.requests)
    }

    /// The log notes since the last call.
    pub fn take_notes(&mut self) -> Vec<LogNote> {
        std::mem::take(&mut self.notes)
    }

    /// Sets what the view may write (a picture view writes nothing; its session keeps the sides).
    pub fn set_write_policy(&mut self, write: crate::folder_view::WritePolicy) {
        self.write = write;
    }

    /// What the view may write.
    pub fn write_policy(&self) -> crate::folder_view::WritePolicy {
        self.write
    }

    /// The status bar (PX-008).
    pub fn status_items(&self, service: &Service) -> StatusItems {
        status::items(self, service)
    }

    // ---- layout and sessions ---------------------------------------------------------------------------------------

    /// Applies the layout of the view type (D-108): `layout`, `checkerboard`, `headers`, `difference_split` of
    /// `[layout.picture-compare]` (a session's `[view]` overlaid on it, D-349).
    pub fn apply_layout(&mut self, layout: &State) {
        let value = |key: &str| layout.layout_value(VIEW, key);
        if let Some(LayoutValue::Text(word)) = value("layout") {
            self.layout = if word == PictureLayout::Stacked.word() {
                PictureLayout::Stacked
            } else {
                PictureLayout::SideBySide
            };
        }
        if let Some(on) = value("checkerboard").and_then(LayoutValue::as_bool) {
            self.checkerboard = on;
        }
        if let Some(on) = value("headers").and_then(LayoutValue::as_bool) {
            self.headers = on;
        }
        if let Some(f) = value("difference_split").and_then(LayoutValue::as_float) {
            self.split = f.clamp(0.1, 0.9);
        }
    }

    /// Applies a session's view-only keys (D-370): `zoom`, `center`, `difference_pane`, read and checked by core
    /// (`picture_view_keys_of`; a refused table keeps the defaults).
    pub fn apply_session_view(&mut self, view: &hikasa_session::sessions::OpaqueTable) {
        let Ok(keys) = hikasa_session::api::picture_view_keys_of(view) else {
            return;
        };
        self.shown[2] = keys.difference_pane;
        self.viewport.zoom = match keys.zoom {
            hikasa_picture::Zoom::Fit => Zoom::Fit,
            hikasa_picture::Zoom::Actual => Zoom::Scale(1.0),
            hikasa_picture::Zoom::Scale(s) => Zoom::Scale(s),
        };
        if let Some(c) = keys.center {
            self.viewport.center = c;
            self.session_center = Some(c);
        }
    }

    /// The layout keys of the view type (`[layout.picture-compare]`).
    pub fn layout_keys(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            ("layout", LayoutValue::Text(self.layout.word().to_owned())),
            ("checkerboard", LayoutValue::Bool(self.checkerboard)),
            ("headers", LayoutValue::Bool(self.headers)),
            ("difference_split", LayoutValue::Float(self.split)),
        ]
    }

    /// The `[view]` table of the view's session (D-349, D-370), written by core (`picture_view_table`): the
    /// orientations, the offset, the checkerboard, the layout, the zoom, the centre (not at Fit), the difference pane.
    pub fn view_table(&self) -> hikasa_session::sessions::OpaqueTable {
        let zoom = match self.viewport.zoom {
            Zoom::Fit => hikasa_picture::Zoom::Fit,
            Zoom::Scale(s) => hikasa_picture::Zoom::Scale(s),
        };
        let keys = hikasa_session::api::PictureViewKeys {
            zoom,
            center: matches!(self.viewport.zoom, Zoom::Scale(_)).then_some(self.viewport.center),
            difference_pane: self.shown[2],
        };
        hikasa_session::api::picture_view_table(
            &self.options.geometry,
            &keys,
            self.checkerboard,
            self.layout == PictureLayout::Stacked,
        )
    }

    /// The inputs watched for changes on disk (D-118).
    pub fn inputs(&self) -> [String; 2] {
        [
            self.request.left.to_string_lossy().into_owned(),
            self.request.right.to_string_lossy().into_owned(),
        ]
    }

    /// The request that opens the same comparison again (`TabDuplicate`, sessions): the words of the view's state.
    pub fn duplicate(&self) -> (ViewRequest, [Option<String>; 2]) {
        let mut request = self.request.clone();
        let g = self.options.geometry;
        request.rules.tolerance = Some(self.options.tolerance.to_string());
        request.rules.auto_scale = g.auto_scale;
        request.rules.ignore_unimportant = self.ignore_unimportant;
        request.rules.mode = Some(self.mode.as_str().to_owned());
        request.rules.blend = match self.mode {
            DiffMode::Blend { percent } => Some(percent.to_string()),
            _ => None,
        };
        request.geometry.left_orientation = Some(g.orientation.left.as_str().to_owned());
        request.geometry.right_orientation = Some(g.orientation.right.as_str().to_owned());
        request.geometry.offset = Some(format!("{},{}", g.offset.0, g.offset.1));
        request.view_checkerboard = Some(self.checkerboard);
        (
            ViewRequest::Picture {
                request,
                geometry: g,
                mode: self.mode,
            },
            self.titles.clone(),
        )
    }
}

impl Drop for PictureViewModel {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The message of Open Clipboard without an image.
pub const NO_PICTURE: &str = "The clipboard holds no picture.";

/// `left`, `right`, `difference`.
pub fn pane_word(pane: Pane) -> &'static str {
    match pane {
        Pane::Left => "left",
        Pane::Right => "right",
        Pane::Difference => "difference",
    }
}

/// The area's size of a comparison in pixels.
fn area_size(c: &PictureComparison) -> (f64, f64) {
    let a = c.area();
    (f64::from(a.width.max(1)), f64::from(a.height.max(1)))
}

/// The basis of a landed comparison.
fn basis_of(c: &PictureComparison) -> Basis {
    let a = c.area();
    Basis {
        origin: (a.x, a.y),
        offset: c.options().geometry.offset,
        area: (a.width, a.height),
    }
}
