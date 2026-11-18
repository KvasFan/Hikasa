//! The viewport of the Picture Compare view (PX-002, §1.13.10, D-362): one zoom and one centre for every pane
//! (synchronized panning), in `f64`. The window passes each pane's rectangle in points and `pixels_per_point`; tests
//! pass fixed rectangles, so every click, zoom and drag is tested headless with exact numbers.
//!
//! *Scale* is physical screen pixels per area pixel; a pane shows `scale / pixels_per_point` points per area pixel.
//! The centre is the area point (area pixels from its top-left corner) at every pane's centre. Toolkit-free.

/// The fixed zoom steps (Zoom In / Out, Ctrl+wheel notches): 1/256 … 64.
pub const ZOOM_STEPS: [f64; 25] = [
    1. / 256.,
    1. / 128.,
    1. / 64.,
    1. / 32.,
    1. / 16.,
    1. / 12.,
    1. / 8.,
    1. / 6.,
    1. / 4.,
    1. / 3.,
    1. / 2.,
    2. / 3.,
    1.,
    1.5,
    2.,
    3.,
    4.,
    6.,
    8.,
    12.,
    16.,
    24.,
    32.,
    48.,
    64.,
];

/// The smallest scale.
pub const MIN_SCALE: f64 = ZOOM_STEPS[0];
/// The largest scale (Fit enlarges small pictures up to it).
pub const MAX_SCALE: f64 = ZOOM_STEPS[24];
/// Fit's margin around the area in every pane (points).
pub const FIT_MARGIN: f64 = 8.0;
/// Points of the area that stay in every pane (**open default**).
pub const KEEP: f64 = 32.0;
/// The pixel grid is drawn from this scale on (1,600 %, **open default**).
pub const GRID_SCALE: f64 = 16.0;

/// A pane's rectangle in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneRect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

impl PaneRect {
    /// A rectangle.
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> PaneRect {
        PaneRect { x, y, w, h }
    }

    /// Its centre.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// True when the point lies inside.
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

/// The zoom of the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    /// The whole area fits every shown pane (the open default); the scale follows the panes' sizes.
    Fit,
    /// A fixed scale (physical pixels per area pixel).
    Scale(f64),
}

/// The viewport: one zoom and one centre for every pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// The zoom.
    pub zoom: Zoom,
    /// The area point at each pane's centre (area pixels).
    pub center: (f64, f64),
    /// Physical pixels per point (the window's).
    pub ppp: f64,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport {
            zoom: Zoom::Fit,
            center: (0.0, 0.0),
            ppp: 1.0,
        }
    }
}

/// The scale at which `area` (pixels) fits every pane with the margin, clamped to [`MIN_SCALE`]…[`MAX_SCALE`].
pub fn fit_scale(panes: &[PaneRect], area: (f64, f64), ppp: f64) -> f64 {
    let (aw, ah) = (area.0.max(1.0), area.1.max(1.0));
    let points = panes
        .iter()
        .map(|p| {
            let w = (p.w - 2.0 * FIT_MARGIN).max(1.0);
            let h = (p.h - 2.0 * FIT_MARGIN).max(1.0);
            (w / aw).min(h / ah)
        })
        .fold(f64::INFINITY, f64::min);
    if !points.is_finite() {
        return 1.0;
    }
    (points * ppp).clamp(MIN_SCALE, MAX_SCALE)
}

/// The next zoom step above `scale` (`up`) or below it; `None` at the end.
pub fn step_from(scale: f64, up: bool) -> Option<f64> {
    if up {
        ZOOM_STEPS
            .iter()
            .copied()
            .find(|&s| s > scale * (1.0 + 1e-9))
    } else {
        ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|&s| s < scale * (1.0 - 1e-9))
    }
}

impl Viewport {
    /// The scale with Fit resolved for the panes and the area.
    pub fn scale(&self, panes: &[PaneRect], area: (f64, f64)) -> f64 {
        match self.zoom {
            Zoom::Fit => fit_scale(panes, area, self.ppp),
            Zoom::Scale(s) => s,
        }
    }

    /// The centre with Fit resolved (Fit centres the area).
    pub fn center_of(&self, area: (f64, f64)) -> (f64, f64) {
        match self.zoom {
            Zoom::Fit => (area.0 / 2.0, area.1 / 2.0),
            Zoom::Scale(_) => self.center,
        }
    }

    /// Points per area pixel at `scale`.
    fn k(&self, scale: f64) -> f64 {
        scale / self.ppp
    }

    /// The screen point (points) of the area point `p` in `pane` at `scale`.
    pub fn to_screen(
        &self,
        pane: PaneRect,
        scale: f64,
        area: (f64, f64),
        p: (f64, f64),
    ) -> (f64, f64) {
        let (cx, cy) = pane.center();
        let c = self.center_of(area);
        let k = self.k(scale);
        (cx + (p.0 - c.0) * k, cy + (p.1 - c.1) * k)
    }

    /// The area point under the screen point `s` (points) of `pane` at `scale`.
    pub fn to_area(
        &self,
        pane: PaneRect,
        scale: f64,
        area: (f64, f64),
        s: (f64, f64),
    ) -> (f64, f64) {
        let (cx, cy) = pane.center();
        let c = self.center_of(area);
        let k = self.k(scale);
        (c.0 + (s.0 - cx) / k, c.1 + (s.1 - cy) / k)
    }

    /// Fixes the scale and the centre (Fit resolved), so that a zoom or a pan starts from what is shown.
    fn freeze(&mut self, panes: &[PaneRect], area: (f64, f64)) -> f64 {
        let scale = self.scale(panes, area);
        self.center = self.center_of(area);
        self.zoom = Zoom::Scale(scale);
        scale
    }

    /// Zooms to `to` (clamped to the ends) keeping the area point under `cursor` (points, in `pane`) under it, then
    /// clamps the centre (P-VIEWPORT: the point stays exactly whenever the clamp has nothing to do).
    pub fn zoom_at(
        &mut self,
        panes: &[PaneRect],
        area: (f64, f64),
        pane: PaneRect,
        cursor: (f64, f64),
        to: f64,
    ) {
        let scale = self.freeze(panes, area);
        let to = to.clamp(MIN_SCALE, MAX_SCALE);
        let a = self.to_area(pane, scale, area, cursor);
        let (cx, cy) = pane.center();
        let k = self.k(to);
        self.center = (a.0 - (cursor.0 - cx) / k, a.1 - (cursor.1 - cy) / k);
        self.zoom = Zoom::Scale(to);
        self.clamp(panes, area);
    }

    /// One zoom step up or down about `cursor` (a wheel notch, Zoom In/Out about a pane's centre); false at the end.
    pub fn step_at(
        &mut self,
        panes: &[PaneRect],
        area: (f64, f64),
        pane: PaneRect,
        cursor: (f64, f64),
        up: bool,
    ) -> bool {
        let scale = self.scale(panes, area);
        match step_from(scale, up) {
            Some(to) => {
                self.zoom_at(panes, area, pane, cursor, to);
                true
            }
            None => false,
        }
    }

    /// Pans by a drag of `delta` points (the area moves with the pointer), then clamps.
    pub fn pan_by(&mut self, panes: &[PaneRect], area: (f64, f64), delta: (f64, f64)) {
        let scale = self.freeze(panes, area);
        let k = self.k(scale);
        self.center = (self.center.0 - delta.0 / k, self.center.1 - delta.1 / k);
        self.clamp(panes, area);
    }

    /// Sets the centre (area pixels) at the current scale, then clamps.
    pub fn center_on(&mut self, panes: &[PaneRect], area: (f64, f64), at: (f64, f64)) {
        self.freeze(panes, area);
        self.center = at;
        self.clamp(panes, area);
    }

    /// The range the centre may take so that at least [`KEEP`] points of the area (all of it when it is smaller,
    /// or the pane's size) stay in every pane: `([x0, x1], [y0, y1])`.
    pub fn center_range(&self, panes: &[PaneRect], area: (f64, f64)) -> ((f64, f64), (f64, f64)) {
        let k = self.k(self.scale(panes, area));
        let axis = |len: f64, sizes: &mut dyn Iterator<Item = f64>| {
            let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
            for size in sizes {
                let keep = KEEP.min(len * k).min(size);
                lo = lo.max((keep - size / 2.0) / k);
                hi = hi.min(len - (keep - size / 2.0) / k);
            }
            if lo > hi {
                let mid = (lo + hi) / 2.0;
                (mid, mid)
            } else {
                (lo, hi)
            }
        };
        (
            axis(area.0, &mut panes.iter().map(|p| p.w)),
            axis(area.1, &mut panes.iter().map(|p| p.h)),
        )
    }

    /// Keeps [`KEEP`] points of the area in every pane (Fit needs nothing).
    pub fn clamp(&mut self, panes: &[PaneRect], area: (f64, f64)) {
        if matches!(self.zoom, Zoom::Fit) || panes.is_empty() {
            return;
        }
        let ((x0, x1), (y0, y1)) = self.center_range(panes, area);
        self.center = (self.center.0.clamp(x0, x1), self.center.1.clamp(y0, y1));
    }

    /// Fits the area rectangle `bounds` (area pixels: x, y, w, h) into the panes (Info's *Show*).
    pub fn show_rect(
        &mut self,
        panes: &[PaneRect],
        area: (f64, f64),
        bounds: (f64, f64, f64, f64),
    ) {
        let scale = fit_scale(panes, (bounds.2.max(1.0), bounds.3.max(1.0)), self.ppp);
        self.zoom = Zoom::Scale(scale);
        self.center = (bounds.0 + bounds.2 / 2.0, bounds.1 + bounds.3 / 2.0);
        self.clamp(panes, area);
    }

    /// The visible part of the area in a pane: `(x0, y0, x1, y1)` in area pixels (may extend past the area).
    pub fn visible(&self, pane: PaneRect, scale: f64, area: (f64, f64)) -> (f64, f64, f64, f64) {
        let a = self.to_area(pane, scale, area, (pane.x, pane.y));
        let b = self.to_area(pane, scale, area, (pane.x + pane.w, pane.y + pane.h));
        (a.0, a.1, b.0, b.1)
    }

    /// A pane's scroll bars: the shown fraction of the area on each axis as `(start, length)` within 0…1, `None`
    /// on an axis where the whole area is shown.
    pub fn scrollbars(
        &self,
        pane: PaneRect,
        scale: f64,
        area: (f64, f64),
    ) -> [Option<(f64, f64)>; 2] {
        let (x0, y0, x1, y1) = self.visible(pane, scale, area);
        let bar = |a: f64, b: f64, len: f64| {
            if len <= 0.0 || (a <= 0.0 && b >= len) {
                return None;
            }
            let s = (a / len).clamp(0.0, 1.0);
            let e = (b / len).clamp(0.0, 1.0);
            Some((s, (e - s).max(0.0)))
        };
        [bar(x0, x1, area.0), bar(y0, y1, area.1)]
    }
}
