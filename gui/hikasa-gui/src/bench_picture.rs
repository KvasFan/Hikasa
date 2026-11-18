//! G11 in the window (M2 W13b, §1.13.21, D-372): the scripted modes `picture_open`, `picture_zoom`, `picture_pan`,
//! `picture_drag` and `picture_tolerance` of `HIKASA_GUI_BENCH`. Each opens the command line's two pictures, waits
//! for the first screen (every visible pane drawn at its level), runs its phases one step a frame, and records per
//! phase the frame work, the frames with an undrawn part of the area, the most texture bytes held, the time from the
//! last step until every visible tile is drawn, and the memory after the phase; `picture_drag` and `picture_tolerance`
//! end with the statistics the CLI must equal (`perf_gate.rs` compares them).

use std::time::Instant;

use serde_json::{Value, json};

use crate::app::{AppCore, ViewModel};
use crate::bench::{memory, round, stats};
use crate::picture::PictureViewModel;
use crate::picture::viewport::PaneRect;
use hikasa_picture::Pane;
use hikasa_session::ipc::ViewId;

/// Which G11 script runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureMode {
    /// The first screen only.
    Open,
    /// 60 Ctrl+wheel steps up, 60 down; Actual Size and Fit 10 times.
    Zoom,
    /// At 100 % and 400 %: a drag of 7 points a frame for 600 frames, then a pane width a frame for 120 frames.
    Pan,
    /// 120 frames of offset drag (1 pixel a frame), then 20 Ctrl+arrow nudges.
    Drag,
    /// Tolerance 0 → 64 a step a frame, then Range, then Blend 50 %.
    Tolerance,
}

impl PictureMode {
    /// The mode's bench name.
    pub fn name(self) -> &'static str {
        match self {
            PictureMode::Open => "picture_open",
            PictureMode::Zoom => "picture_zoom",
            PictureMode::Pan => "picture_pan",
            PictureMode::Drag => "picture_drag",
            PictureMode::Tolerance => "picture_tolerance",
        }
    }

    /// The phases: name and frames.
    fn phases(self) -> &'static [(&'static str, u32)] {
        match self {
            PictureMode::Open => &[],
            PictureMode::Zoom => &[("wheel_in", 60), ("wheel_out", 60), ("actual_fit", 20)],
            PictureMode::Pan => &[
                ("drag_7_points_100", 600),
                ("drag_pane_width_100", 120),
                ("drag_7_points_400", 600),
                ("drag_pane_width_400", 120),
            ],
            PictureMode::Drag => &[("offset_drag", 120), ("nudges", 20)],
            PictureMode::Tolerance => &[("tolerance_0_64", 65), ("range", 1), ("blend_50", 1)],
        }
    }
}

/// Idle frames after the last phase before the memory is sampled.
const TAIL_FRAMES: u32 = 60;
/// Frames a settle may take before the script goes on regardless (reported).
const SETTLE_LIMIT_MS: f64 = 5_000.0;

#[derive(Debug)]
enum Stage {
    /// Until the first screen.
    Opening,
    /// A phase runs: its index and frame.
    Phase(usize, u32),
    /// After a phase: until every visible tile is drawn.
    Settle(usize, Instant),
    /// A frame between phases: the memory sample of the previous step is not counted in a phase's frame work.
    Gap(usize),
    /// Idle frames before the end.
    Tail(u32),
    Done,
}

#[derive(Debug, Default)]
struct Phase {
    works: Vec<f64>,
    /// The model's tile frame of each frame (ms) and the frame works above 8 ms with their frame index.
    tile_frames: Vec<f64>,
    slow: Vec<(usize, f64, f64)>,
    undrawn_frames: u32,
    undrawn_max: f64,
    textures_max: u64,
    max_jobs: usize,
    synchronized: bool,
    /// When the difference pane's tiles of the final state were all drawn after the last step (ms).
    difference_ms: Option<f64>,
    /// When the final options landed after the last step (ms).
    landed_ms: Option<f64>,
}

/// Whether a memory sample counts (§1.13.21, W11a review F9): the process has no swapped pages and the machine has
/// at least 10 GB available; with the load average.
fn sample_validity() -> Value {
    let swap = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("VmSwap:"))
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<u64>().ok())
        });
    let available = std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| {
        t.lines()
            .find_map(|l| l.strip_prefix("MemAvailable:"))
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
    });
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let valid = swap == Some(0) && available.is_some_and(|kib| kib >= 10_000_000);
    json!({
        "swap_kib": swap,
        "available_gb": available.map(|kib| round(kib as f64 / 1_000_000.0)),
        "load": load.split_whitespace().take(3).collect::<Vec<_>>().join(" "),
        "valid": valid,
    })
}

/// The state of a G11 script.
#[derive(Debug)]
pub struct PictureBench {
    mode: PictureMode,
    stage: Stage,
    phase: Phase,
    phases: Vec<Value>,
    process_start: Instant,
    open_at: Option<Instant>,
    first_screen: Option<Instant>,
    first_screen_memory: Value,
    textures_max: u64,
    /// Where a drag started and how far it went (points).
    drag: Option<((f64, f64), f64)>,
    stats: Value,
    words: Value,
}

impl PictureBench {
    /// A script of `mode`.
    pub fn new(mode: PictureMode, process_start: Instant) -> PictureBench {
        PictureBench {
            mode,
            stage: Stage::Opening,
            phase: Phase {
                synchronized: true,
                ..Phase::default()
            },
            phases: Vec::new(),
            process_start,
            open_at: None,
            first_screen: None,
            first_screen_memory: Value::Null,
            textures_max: 0,
            drag: None,
            stats: Value::Null,
            words: Value::Null,
        }
    }

    /// True when the script ended.
    pub fn finished(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    fn model<'a>(core: &'a mut AppCore, view: ViewId) -> Option<&'a mut PictureViewModel> {
        match core.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Picture(m)) => Some(m),
            _ => None,
        }
    }

    fn rect(m: &PictureViewModel, pane: Pane) -> Option<PaneRect> {
        m.slots().iter().find(|s| s.pane == pane).map(|s| s.picture)
    }

    /// This frame's step, before the view is drawn.
    pub fn drive(&mut self, core: &mut AppCore, view: Option<ViewId>) {
        let Some(view) = view else {
            return;
        };
        self.open_at.get_or_insert_with(Instant::now);
        let service = core.service_handle();
        let mode = self.mode;
        let Some(m) = PictureBench::model(core, view) else {
            return;
        };
        match self.stage {
            Stage::Opening => {
                if m.comparison().is_some() && m.tiles.complete() && !m.tiles.wanted_is_empty() {
                    self.first_screen = Some(Instant::now());
                    self.first_screen_memory = memory();
                    self.stage = if mode.phases().is_empty() {
                        Stage::Tail(0)
                    } else {
                        Stage::Gap(0)
                    };
                }
            }
            Stage::Phase(index, frame) => {
                let (_, frames) = mode.phases()[index];
                self.step(m, &service, index, frame);
                self.stage = if frame + 1 >= frames {
                    m.release();
                    self.drag = None;
                    Stage::Settle(index, Instant::now())
                } else {
                    Stage::Phase(index, frame + 1)
                };
            }
            Stage::Settle(index, since) => {
                let ms = since.elapsed().as_secs_f64() * 1000.0;
                let landed = m.comparison().is_some_and(|c| *c.options() == m.options);
                if landed && self.phase.landed_ms.is_none() {
                    self.phase.landed_ms = Some(ms);
                }
                if landed && self.phase.difference_ms.is_none() && m.tiles.pane_complete(2) {
                    self.phase.difference_ms = Some(ms);
                }
                if (landed && !m.busy() && m.tiles.complete() && !m.tiles.busy())
                    || ms > SETTLE_LIMIT_MS
                {
                    let name = mode.phases()[index].0;
                    let p = std::mem::replace(
                        &mut self.phase,
                        Phase {
                            synchronized: true,
                            ..Phase::default()
                        },
                    );
                    self.phases.push(json!({
                        "name": name,
                        "frame_work_ms": stats(&p.works),
                        "tile_frame_ms": stats(&p.tile_frames),
                        "slow_frames": p.slow,
                        "undrawn_frames": p.undrawn_frames,
                        "undrawn_max_fraction": p.undrawn_max,
                        "textures_max_mib": round(p.textures_max as f64 / 1_048_576.0),
                        "max_option_jobs": p.max_jobs,
                        "synchronized": p.synchronized,
                        "tiles_after_last_step_ms": round(ms),
                        "difference_after_last_step_ms": p.difference_ms.map(round),
                        "landed_after_last_step_ms": p.landed_ms.map(round),
                        "settled": ms <= SETTLE_LIMIT_MS,
                        "memory": memory(),
                        "sample": sample_validity(),
                    }));
                    self.stage = if index + 1 < mode.phases().len() {
                        Stage::Gap(index + 1)
                    } else {
                        Stage::Tail(0)
                    };
                }
            }
            Stage::Gap(index) => self.stage = Stage::Phase(index, 0),
            Stage::Tail(n) => {
                if n >= TAIL_FRAMES {
                    if let Some(r) = m.response(&service) {
                        self.stats = serde_json::to_value(r.json().stats).unwrap_or(Value::Null);
                        let o = m.options;
                        self.words = json!({
                            "offset": format!("{},{}", o.geometry.offset.0, o.geometry.offset.1),
                            "tolerance": o.tolerance,
                            "left_orientation": o.geometry.orientation.left.as_str(),
                            "right_orientation": o.geometry.orientation.right.as_str(),
                        });
                    }
                    self.stage = Stage::Done;
                } else {
                    self.stage = Stage::Tail(n + 1);
                }
            }
            Stage::Done => {}
        }
    }

    /// One step of phase `index` at frame `frame`.
    fn step(
        &mut self,
        m: &mut PictureViewModel,
        service: &hikasa_session::Service,
        index: usize,
        frame: u32,
    ) {
        let left = PictureBench::rect(m, Pane::Left);
        let diff = PictureBench::rect(m, Pane::Difference);
        // A seeded pointer: two thirds across, a third down the left pane.
        let pointer = left.map(|r| (r.x + r.w * 2.0 / 3.0, r.y + r.h / 3.0));
        match (self.mode, index) {
            (PictureMode::Zoom, 0) => {
                if let Some(p) = pointer
                    && m.scale() < 8.0
                {
                    m.wheel(p, (0.0, 1.0), true, false);
                }
            }
            (PictureMode::Zoom, 1) => {
                if let Some(p) = pointer {
                    m.wheel(p, (0.0, -1.0), true, false);
                }
            }
            (PictureMode::Zoom, _) => {
                m.zoom_to(if frame % 2 == 0 {
                    crate::picture::ZoomChoice::Actual
                } else {
                    crate::picture::ZoomChoice::Fit
                });
            }
            (PictureMode::Pan, k) => {
                let Some(r) = left else {
                    return;
                };
                if frame == 0 {
                    m.zoom_to(crate::picture::ZoomChoice::Percent(if k < 2 {
                        100
                    } else {
                        400
                    }));
                    m.release();
                    let start = r.center();
                    m.press(start, false);
                    self.drag = Some((start, 0.0));
                }
                let per = if k % 2 == 0 { 7.0 } else { r.w };
                if let Some((start, moved)) = self.drag.as_mut() {
                    // Back and forth every 100 frames (7 points) or 10 frames (a pane width): the area stays in view.
                    let period = if k % 2 == 0 { 100 } else { 10 };
                    let sign = if (frame / period) % 2 == 0 { -1.0 } else { 1.0 };
                    *moved += sign * per;
                    let to = (start.0 + *moved, start.1 + *moved * 0.25);
                    m.drag_to(service, to);
                }
            }
            (PictureMode::Drag, 0) => {
                let Some(r) = diff else {
                    return;
                };
                if frame == 0 {
                    m.release();
                    m.press(r.center(), false);
                    self.drag = Some((r.center(), 0.0));
                }
                // One area pixel a frame: `scale / ppp` points.
                let per = m.scale() / m.viewport.ppp;
                if let Some((start, moved)) = self.drag.as_mut() {
                    *moved += per;
                    m.drag_to(service, (start.0 + *moved, start.1));
                }
            }
            (PictureMode::Drag, _) => {
                if frame == 0 {
                    m.release();
                }
                m.nudge(service, 0, 1, true);
            }
            (PictureMode::Tolerance, 0) => m.set_tolerance(service, frame.min(64) as u8),
            (PictureMode::Tolerance, 1) => m.set_mode(hikasa_picture::DiffMode::Range),
            (PictureMode::Tolerance, _) => m.set_blend(50),
            (PictureMode::Open, _) => {}
        }
    }

    /// After the frame was drawn: its work goes to the running phase; the undrawn check, the texture count and the
    /// synchronization of the panes.
    pub fn end_frame(&mut self, core: &mut AppCore, view: Option<ViewId>, work: Option<f64>) {
        let Some(view) = view else {
            return;
        };
        let Some(m) = PictureBench::model(core, view) else {
            return;
        };
        let jobs = m.option_jobs_running();
        let bytes = m.tiles.bytes();
        self.textures_max = self.textures_max.max(bytes);
        if !matches!(self.stage, Stage::Phase(..) | Stage::Settle(..)) {
            return;
        }
        if let (Stage::Phase(..), Some(w)) = (&self.stage, work) {
            self.phase.works.push(w);
            self.phase.tile_frames.push(m.last_frame_ms());
            if w > 8.0 && self.phase.slow.len() < 8 {
                self.phase
                    .slow
                    .push((self.phase.works.len(), w, m.last_frame_ms()));
            }
        }
        self.phase.textures_max = self.phase.textures_max.max(bytes);
        self.phase.max_jobs = self.phase.max_jobs.max(jobs);
        let undrawn = [Pane::Left, Pane::Right, Pane::Difference]
            .into_iter()
            .map(|p| m.undrawn(p, 12))
            .fold(0.0, f64::max);
        if undrawn > 0.0 {
            self.phase.undrawn_frames += 1;
            self.phase.undrawn_max = self.phase.undrawn_max.max(undrawn);
        }
        // Every pane's centre shows one area point.
        let scale = m.scale();
        let area = m.area();
        let centres: Vec<(f64, f64)> = m
            .slots()
            .iter()
            .map(|s| {
                m.viewport
                    .to_area(s.picture, scale, area, s.picture.center())
            })
            .collect();
        if centres
            .windows(2)
            .any(|w| (w[0].0 - w[1].0).abs() > 1e-6 || (w[0].1 - w[1].1).abs() > 1e-6)
        {
            self.phase.synchronized = false;
        }
    }

    /// The script's report.
    pub fn report(&self) -> Value {
        let since = |at: Option<Instant>, from: Instant| {
            at.map_or(Value::Null, |t| {
                json!(round(t.duration_since(from).as_secs_f64() * 1000.0))
            })
        };
        json!({
            "mode": self.mode.name(),
            "first_screen_ms": {
                "from_process_start": since(self.first_screen, self.process_start),
                "from_view_open": self.open_at.map_or(Value::Null, |o| since(self.first_screen, o)),
            },
            "memory_at_first_screen": self.first_screen_memory,
            "textures_max_mib": round(self.textures_max as f64 / 1_048_576.0),
            "phases": self.phases,
            "stats": self.stats,
            "words": self.words,
        })
    }
}
