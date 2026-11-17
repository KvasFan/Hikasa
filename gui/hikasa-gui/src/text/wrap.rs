//! Visual lines over [`Metrics`] (M2 W11a, contract §1.11.4, D-295): the geometry of one pane line — its display
//! text (tabs expanded, controls made visible: M1's [`layout_line`], display only), the x of every character
//! boundary, the visual lines word wrap breaks it into, the long-line window — computed without a toolkit, so caret
//! movement, wrap and clicks are tested headless with exact numbers ([`FixedMetrics`]). The window measures with
//! egui's font metrics (`ui/text.rs`).
//!
//! Text rules stay core's (check 7): grapheme cluster boundaries come from `hikasa_text::edit::cluster_starts`, a
//! character's display width (the test metrics) from `char_width`.

use hikasa_text::edit::{char_width, cluster_starts};

use super::layout_line;

/// What a geometry needs of a font.
pub trait Metrics {
    /// The advance of `c` (a character of the display text; a tab is expanded before, so it never comes).
    fn advance(&self, c: char) -> f32;
    /// The height of a visual line.
    fn line_height(&self) -> f32;
}

/// Test metrics (§1.11.2 rule 2): every character one unit, a wide one two, a zero-width one none (the columns
/// `nav::column_of_tabs` counts); lines one unit high; tabs of `tab` units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedMetrics {
    /// Tab stop in units.
    pub tab: u8,
}

impl Default for FixedMetrics {
    fn default() -> FixedMetrics {
        FixedMetrics { tab: 4 }
    }
}

impl Metrics for FixedMetrics {
    fn advance(&self, c: char) -> f32 {
        char_width(c) as f32
    }

    fn line_height(&self) -> f32 {
        1.0
    }
}

/// A line longer than this many display columns is laid out only in a window of this many columns around the
/// horizontal scroll position when word wrap is off (§1.11.4 *Long lines*).
pub const LONG_LINE_COLUMNS: usize = 4096;
/// With word wrap, at most this many visual lines of a line are laid out; a *line continues* mark follows.
pub const MAX_VISUAL_LINES: usize = 256;

/// One visual line of a [`LineGeom`]: boundaries `[start, end)` of the geometry's boundary list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visual {
    /// First boundary.
    pub start: usize,
    /// One past the last boundary of its text (the next visual line starts there).
    pub end: usize,
}

/// The geometry of one pane line (or of its window).
#[derive(Debug, Clone, PartialEq)]
pub struct LineGeom {
    /// The display text (of the window).
    pub text: String,
    /// Per character boundary of the source in the window: the source byte, the display byte, the x (from the line's
    /// start, so a window's first boundary has the x of everything before it).
    pub bounds: Vec<(u32, u32, f32)>,
    /// The visual lines (one without word wrap).
    pub visuals: Vec<Visual>,
    /// The line goes on after the last visual line laid out (the 256-line cap of word wrap).
    pub continues: bool,
    /// The line goes on before the window (no wrap, long lines).
    pub cut_before: bool,
    /// The line goes on after the window.
    pub cut_after: bool,
    /// Word wrap broke the line: x is counted from each visual line's start (without wrap: from the line's start).
    pub wrapped: bool,
    /// Per boundary of `bounds`: a grapheme cluster starts there (or the line ends): a hit lands only there.
    pub clusters: Vec<bool>,
}

/// True for a character after which word wrap may break (§1.11.4: the last space, tab or punctuation that fits).
fn break_after(c: char) -> bool {
    c.is_whitespace() || c.is_ascii_punctuation()
}

impl LineGeom {
    /// The geometry of `line` (its text) with tab stops every `tab` columns: without `wrap` one visual line, laid
    /// out in a [`LONG_LINE_COLUMNS`] window holding `focus_byte` (the caret, or the column of the horizontal scroll
    /// position) when the line is longer; with `wrap` the visual lines of `width` (at most [`MAX_VISUAL_LINES`],
    /// the window following `focus_byte` beyond them).
    pub fn new(
        line: &str,
        tab: u8,
        metrics: &dyn Metrics,
        wrap: Option<f32>,
        focus_byte: u32,
    ) -> LineGeom {
        let tab = usize::from(tab.max(1));
        let chars = line.chars().count();
        // The window of source characters laid out.
        let (c0, c1) = if wrap.is_none() && chars > LONG_LINE_COLUMNS {
            let focus = line
                .get(..(focus_byte as usize).min(line.len()))
                .map_or(0, |s| s.chars().count());
            let half = LONG_LINE_COLUMNS / 2;
            let start = focus.saturating_sub(half).min(chars - LONG_LINE_COLUMNS);
            (start, start + LONG_LINE_COLUMNS)
        } else {
            (0, chars)
        };
        // x before the window: the advances of what the window leaves out, measured once (O(line)).
        let mut x = 0.0f32;
        let mut col = 0usize;
        let mut b0 = 0usize;
        for (k, (byte, c)) in line.char_indices().enumerate() {
            if k == c0 {
                b0 = byte;
                break;
            }
            let (w, cols) = measure(c, col, tab, metrics);
            x += w;
            col += cols;
        }
        if c0 >= chars {
            b0 = line.len();
        }
        let body = &line[b0..];
        let layout = layout_line(body, tab);
        // Rebuild the display text from the window with the same rules, keeping per boundary the display byte.
        let mut text = String::with_capacity(layout.text.len());
        let mut bounds = Vec::with_capacity(c1 - c0 + 1);
        for (k, (byte, c)) in body.char_indices().enumerate() {
            if c0 + k >= c1 {
                break;
            }
            bounds.push(((b0 + byte) as u32, text.len() as u32, x));
            let shown = display_of(c, col, tab);
            for d in shown.chars() {
                x += metrics.advance(d);
            }
            col += if c == '\t' {
                tab - col % tab
            } else {
                shown.encode_utf16().count()
            };
            text.push_str(&shown);
        }
        let end_byte = if c1 >= chars {
            line.len()
        } else {
            line.char_indices().nth(c1).map_or(line.len(), |(b, _)| b)
        };
        bounds.push((end_byte as u32, text.len() as u32, x));
        let clusters = if line.is_ascii() {
            vec![true; bounds.len()]
        } else {
            // The window's clusters (its first boundary counts as one: a window may start inside a cluster).
            let starts = cluster_starts(&line[b0..end_byte]);
            bounds
                .iter()
                .map(|b| starts.binary_search(&(b.0 - b0 as u32)).is_ok())
                .collect()
        };
        let mut geom = LineGeom {
            text,
            bounds,
            visuals: Vec::new(),
            continues: false,
            cut_before: c0 > 0,
            cut_after: c1 < chars,
            wrapped: wrap.is_some(),
            clusters,
        };
        match wrap {
            None => {
                geom.visuals.push(Visual {
                    start: 0,
                    end: geom.bounds.len() - 1,
                });
            }
            Some(width) => geom.wrap(line, width.max(1.0), focus_byte),
        }
        geom
    }

    /// Breaks the boundaries into visual lines of `width`: at the last space, tab or punctuation that fits, else at
    /// the last grapheme cluster boundary that fits (never inside a cluster), always at least one cluster a line.
    fn wrap(&mut self, line: &str, width: f32, focus_byte: u32) {
        let n = self.bounds.len() - 1;
        let mut start = 0usize;
        while start < n {
            let x0 = self.bounds[start].2;
            let mut last_fit = None;
            let mut last_break = None;
            let mut end = n;
            for k in start + 1..=n {
                let (byte, _, x) = self.bounds[k];
                if !self.clusters[k] && k < n {
                    continue;
                }
                // A visual line keeps its content narrower than the width (the caret fits at its end).
                if x - x0 >= width {
                    end = last_break.or(last_fit).unwrap_or(k);
                    break;
                }
                last_fit = Some(k);
                let prev = line[..byte as usize].chars().next_back();
                if prev.is_some_and(break_after) {
                    last_break = Some(k);
                }
            }
            self.visuals.push(Visual { start, end });
            start = end;
        }
        if self.visuals.is_empty() {
            self.visuals.push(Visual { start: 0, end: 0 });
        }
        if self.visuals.len() > MAX_VISUAL_LINES {
            // A window of visual lines holding the focus (moving the caret into the rest shifts it).
            let k = self.boundary(focus_byte);
            let at = self
                .visuals
                .iter()
                .position(|v| k < v.end)
                .unwrap_or(self.visuals.len() - 1);
            let first = at.saturating_sub(MAX_VISUAL_LINES - 1);
            self.cut_before = first > 0;
            self.continues = first + MAX_VISUAL_LINES < self.visuals.len();
            self.visuals.drain(..first);
            self.visuals.truncate(MAX_VISUAL_LINES);
        }
    }

    /// The visual line holding the caret at source byte `byte` (the boundary at a visual line's end belongs to the
    /// next one, unless it ends the line) and its x.
    pub fn caret(&self, byte: u32) -> (usize, f32) {
        let k = self.boundary(byte);
        let v = self
            .visuals
            .iter()
            .position(|v| k < v.end || (k == v.end && v.end + 1 == self.bounds.len()))
            .unwrap_or(self.visuals.len().saturating_sub(1));
        (v, self.bounds[k].2 - self.x0(v))
    }

    /// The x that x values of visual line `v` count from: its start with word wrap, the line's start without.
    pub fn x0(&self, v: usize) -> f32 {
        match self.visuals.get(v) {
            Some(vis) if self.wrapped => self.bounds[vis.start].2,
            _ => 0.0,
        }
    }

    /// Where the display text of visual line `v` starts, in that line's x (a window's first boundary without wrap).
    pub fn draw_x(&self, v: usize) -> f32 {
        self.visuals
            .get(v)
            .map_or(0.0, |vis| self.bounds[vis.start].2 - self.x0(v))
    }

    /// The boundary index of source byte `byte` (the last boundary at or before it, clamped to the window).
    pub fn boundary(&self, byte: u32) -> usize {
        let k = self.bounds.partition_point(|b| b.0 <= byte);
        k.saturating_sub(1)
    }

    /// The source byte nearest to `x` on visual line `v` (a hit test): a grapheme cluster's start or the line's end,
    /// never a boundary inside a cluster (D-295).
    pub fn hit(&self, v: usize, x: f32) -> u32 {
        let Some(vis) = self
            .visuals
            .get(v.min(self.visuals.len().saturating_sub(1)))
        else {
            return self.bounds.first().map_or(0, |b| b.0);
        };
        let x0 = self.x0(v.min(self.visuals.len().saturating_sub(1)));
        let last = if vis.end + 1 == self.bounds.len() || v + 1 == self.visuals.len() {
            vis.end
        } else {
            // Not the boundary that starts the next visual line: the caret would show there.
            vis.end.saturating_sub(1).max(vis.start)
        };
        let mut best = (vis.start..=last)
            .find(|k| self.clusters[*k])
            .unwrap_or(vis.start);
        for k in vis.start..=last {
            if !self.clusters[k] {
                continue;
            }
            let d = (self.bounds[k].2 - x0 - x).abs();
            // On a tie the later boundary: after zero-width marks, never inside their cluster.
            if d <= (self.bounds[best].2 - x0 - x).abs() {
                best = k;
            }
        }
        self.bounds[best].0
    }

    /// The display text of visual line `v` and its x offset from the line's start.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn visual_text(&self, v: usize) -> (&str, f32) {
        let Some(vis) = self.visuals.get(v) else {
            return ("", 0.0);
        };
        let (a, b) = (
            self.bounds[vis.start].1 as usize,
            self.bounds[vis.end.min(self.bounds.len() - 1)].1 as usize,
        );
        (&self.text[a..b], self.bounds[vis.start].2)
    }

    /// The display byte range of source bytes `[a, b)` on visual line `v`, clipped to it, with their x from the visual
    /// line's start (selections, find hits, inline differences).
    pub fn span_x(&self, v: usize, a: u32, b: u32) -> Option<(f32, f32)> {
        let vis = self.visuals.get(v)?;
        let x0 = self.x0(v);
        let (lo, hi) = (self.bounds[vis.start].0, self.bounds[vis.end].0);
        let (a, b) = (a.max(lo), b.min(hi));
        if a > b || (a == b && a != lo) {
            return None;
        }
        let xa = self.bounds[self.boundary(a)].2 - x0;
        let xb = self.bounds[self.boundary(b)].2 - x0;
        Some((xa, xb))
    }
}

/// The display text of `c` at display column `col` (tabs to the next stop, controls as pictures, invisibles as
/// escapes: M1's rules, [`layout_line`]).
fn display_of(c: char, col: usize, tab: usize) -> String {
    if c == '\t' {
        return " ".repeat(tab - col % tab);
    }
    let mut buf = [0u8; 4];
    layout_line(c.encode_utf8(&mut buf), tab).text
}

/// The advance and the display columns of `c` at column `col`.
fn measure(c: char, col: usize, tab: usize, metrics: &dyn Metrics) -> (f32, usize) {
    let shown = display_of(c, col, tab);
    let w = shown.chars().map(|d| metrics.advance(d)).sum();
    let cols = if c == '\t' {
        tab - col % tab
    } else {
        shown.encode_utf16().count()
    };
    (w, cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom(text: &str, wrap: Option<f32>) -> LineGeom {
        LineGeom::new(text, 4, &FixedMetrics::default(), wrap, 0)
    }

    #[test]
    fn boundaries_measure_tabs_wide_and_combining_characters() {
        let g = geom("a\tb中e\u{301}", None);
        let xs: Vec<f32> = g.bounds.iter().map(|b| b.2).collect();
        assert_eq!(xs, vec![0.0, 1.0, 4.0, 5.0, 7.0, 8.0, 8.0]);
        assert_eq!(g.caret(2), (0, 4.0));
        assert_eq!(g.text, "a   b中e\u{301}");
        assert_eq!(
            g.hit(0, 3.0),
            2,
            "a click past the tab's middle goes after it"
        );
        assert_eq!(g.hit(0, 99.0), 9);
    }

    #[test]
    fn word_wrap_breaks_after_blanks_else_at_clusters() {
        let g = geom("alpha beta gamma", Some(8.0));
        let lines: Vec<&str> = (0..g.visuals.len()).map(|v| g.visual_text(v).0).collect();
        assert_eq!(lines, vec!["alpha ", "beta ", "gamma"]);
        // No break opportunity: at the width, never inside a cluster.
        let g = geom("abcde\u{301}fg", Some(5.0));
        let lines: Vec<&str> = (0..g.visuals.len()).map(|v| g.visual_text(v).0).collect();
        assert_eq!(lines, vec!["abcd", "e\u{301}fg"]);
        // The caret at a break belongs to the next visual line.
        let g = geom("alpha beta", Some(8.0));
        assert_eq!(g.caret(6), (1, 0.0));
        assert_eq!(g.caret(10), (1, 4.0));
    }

    #[test]
    fn long_lines_are_laid_out_in_a_window() {
        let text = "x".repeat(50_000);
        let g = LineGeom::new(&text, 4, &FixedMetrics::default(), None, 49_999);
        assert_eq!(g.bounds.len(), LONG_LINE_COLUMNS + 1);
        assert!(g.cut_before && !g.cut_after);
        assert_eq!(g.caret(49_999).1, 49_999.0);
        let g = LineGeom::new(&text, 4, &FixedMetrics::default(), Some(10.0), 0);
        assert_eq!(g.visuals.len(), MAX_VISUAL_LINES);
        assert!(g.continues);
    }
}
