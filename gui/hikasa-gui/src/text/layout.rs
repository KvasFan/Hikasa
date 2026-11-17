//! The layouts and display options of a text view (M2 W11a/W11b; contract §1.11.4, §1.11.5, §1.11.11, §1.11.14,
//! D-305), toolkit-free: *Word Wrap*, *Overwrite Mode* (per view, not persisted), *File Info*, side by side or over
//! under, syntax colours, visible whitespace, *Zoom Pane*, the splitter, the details panel, the ruler, the find panel
//! and the browse mode. The persistent ones are layout keys of the view type (`[layout.text-compare]`,
//! `[layout.text-edit]`, D-108); an unknown word falls back with one `invalid_config_value` warning. The options of
//! OPT-014 and OPT-017 the panes draw with are [`Tweaks`], read from the window's settings; [`pane_geometry`] places
//! the panes.

use hikasa_session::settings::state::LayoutValue;
use hikasa_session::settings::{Options, State};
use hikasa_text::nav::BrowseMode;

/// The splitter's range (D-305).
pub const SPLIT_RANGE: (f32, f32) = (0.1, 0.9);
/// One `ZoomLeft`/`ZoomRight` step.
pub const SPLIT_STEP: f32 = 0.05;
/// The details panel's height range (points).
pub const DETAILS_HEIGHT: (f32, f32) = (60.0, 600.0);
/// The details panel's height before the user drags it.
pub const DETAILS_DEFAULT: f32 = 120.0;
/// The width of the copy strip (points).
pub const STRIP_W: f32 = 18.0;
/// The height of the divider of over-under panes (points).
pub const DIVIDER_H: f32 = 6.0;

/// The bottom panel of the view (D-306).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Details {
    /// None.
    #[default]
    None,
    /// Text Details: the current row's lines, editable.
    Text,
    /// Hex Details: the bytes of the current lines.
    Hex,
    /// Alignment Details: the current row's characters aligned (Text Compare only).
    Alignment,
}

impl Details {
    /// The layout word.
    pub fn word(self) -> &'static str {
        match self {
            Details::None => "none",
            Details::Text => "text",
            Details::Hex => "hex",
            Details::Alignment => "alignment",
        }
    }

    fn of_word(word: &str) -> Option<Details> {
        Some(match word {
            "none" => Details::None,
            "text" => Details::Text,
            "hex" => Details::Hex,
            "alignment" => Details::Alignment,
            _ => return None,
        })
    }
}

/// The find panel shown above the panes (D-307).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FindShown {
    /// No panel.
    #[default]
    None,
    /// The Find panel.
    Find,
    /// The Replace panel.
    Replace,
}

impl FindShown {
    /// The layout word.
    pub fn word(self) -> &'static str {
        match self {
            FindShown::None => "none",
            FindShown::Find => "find",
            FindShown::Replace => "replace",
        }
    }

    fn of_word(word: &str) -> Option<FindShown> {
        Some(match word {
            "none" => FindShown::None,
            "find" => FindShown::Find,
            "replace" => FindShown::Replace,
            _ => return None,
        })
    }
}

/// The layout word of a browse mode (`[layout] browse_mode`).
pub fn browse_word(mode: BrowseMode) -> &'static str {
    match mode {
        BrowseMode::DiffSection => "diff_section",
        BrowseMode::Difference => "difference",
        BrowseMode::Edit => "edit",
        BrowseMode::Find => "find",
    }
}

fn browse_of_word(word: &str) -> Option<BrowseMode> {
    Some(match word {
        "diff_section" => BrowseMode::DiffSection,
        "difference" => BrowseMode::Difference,
        "edit" => BrowseMode::Edit,
        "find" => BrowseMode::Find,
        _ => return None,
    })
}

/// The display flags of a view.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// Long lines wrap at the pane's width (`WordWrap`).
    pub word_wrap: bool,
    /// Typing replaces characters (`Overwriting`, Ins; per view, not persisted).
    pub overwrite: bool,
    /// The File Info header above each pane (`ShowSpecInfo`).
    pub header: bool,
    /// The left pane above the right one (`OverUnder`; `SideBySide` clears it).
    pub over_under: bool,
    /// Syntax colours (`ShowSyntaxHighlighting`, default on).
    pub syntax: bool,
    /// Visible whitespace (`ShowWS`).
    pub whitespace: bool,
    /// The focus pane alone fills the view (`ZoomPane`; not persisted).
    pub zoom: bool,
    /// The left (or upper) pane's share of the view (0.1–0.9).
    pub split: f32,
    /// The bottom panel.
    pub details: Details,
    /// Its height (points).
    pub details_height: f32,
    /// *Lock Visible Whitespace* of the panel: it shows the marks whatever `whitespace` says.
    pub details_whitespace: bool,
    /// The ruler above the panes (`ShowRuler`).
    pub ruler: bool,
    /// The find panel.
    pub find_panel: FindShown,
    /// What the toolbar's arrows step by (`BrowseMode`, D-309).
    pub browse_mode: BrowseMode,
    /// The font family of the view type (`DisplayFont`; `None`: the theme's editor font).
    pub font_family: Option<String>,
}

impl Default for Display {
    fn default() -> Display {
        Display {
            word_wrap: false,
            overwrite: false,
            header: true,
            over_under: false,
            syntax: true,
            whitespace: false,
            zoom: false,
            split: 0.5,
            details: Details::None,
            details_height: DETAILS_DEFAULT,
            details_whitespace: false,
            ruler: false,
            find_panel: FindShown::None,
            browse_mode: BrowseMode::DiffSection,
            font_family: None,
        }
    }
}

impl Display {
    /// Reads the view type's layout (a value of the wrong type is ignored; an unknown word falls back and is returned
    /// as an `invalid_config_value` warning's text). `one_pane`: a Text Edit view (no layout, no alignment panel, no
    /// splitter; browse mode `edit` by default). `sticky`: `sticky_splitter_position` (else the split is 50 %).
    pub fn read_layout(
        &mut self,
        layout: &State,
        view: &str,
        one_pane: bool,
        sticky: bool,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        if one_pane {
            self.browse_mode = BrowseMode::Edit;
        }
        let flag = |key: &str| {
            layout
                .layout_value(view, key)
                .and_then(LayoutValue::as_bool)
        };
        let word = |key: &str| match layout.layout_value(view, key) {
            Some(LayoutValue::Text(w)) => Some(w),
            _ => None,
        };
        let number = |key: &str| {
            layout
                .layout_value(view, key)
                .and_then(LayoutValue::as_float)
        };
        let mut bad = |key: &str, w: &str| {
            warnings.push(format!(
                "invalid_config_value: [layout.{view}] {key} = \"{w}\" is not a value of this key; the default is used"
            ));
        };
        if let Some(on) = flag("word_wrap") {
            self.word_wrap = on;
        }
        if let Some(on) = flag("header") {
            self.header = on;
        }
        if let Some(on) = flag("syntax") {
            self.syntax = on;
        }
        if let Some(on) = flag("whitespace") {
            self.whitespace = on;
        }
        if let Some(on) = flag("ruler") {
            self.ruler = on;
        }
        if let Some(on) = flag("details_whitespace") {
            self.details_whitespace = on;
        }
        if !one_pane && let Some(w) = word("layout") {
            match w.as_str() {
                "side_by_side" => self.over_under = false,
                "over_under" => self.over_under = true,
                _ => bad("layout", &w),
            }
        }
        if let Some(w) = word("details") {
            match Details::of_word(&w) {
                Some(Details::Alignment) if one_pane => bad("details", &w),
                Some(d) => self.details = d,
                None => bad("details", &w),
            }
        }
        if let Some(h) = number("details_height") {
            self.details_height = (h as f32).clamp(DETAILS_HEIGHT.0, DETAILS_HEIGHT.1);
        }
        if let Some(w) = word("find_panel") {
            match FindShown::of_word(&w) {
                Some(f) => self.find_panel = f,
                None => bad("find_panel", &w),
            }
        }
        if let Some(w) = word("browse_mode") {
            match browse_of_word(&w) {
                Some(m) if one_pane && !matches!(m, BrowseMode::Edit | BrowseMode::Find) => {
                    bad("browse_mode", &w);
                }
                Some(m) => self.browse_mode = m,
                None => bad("browse_mode", &w),
            }
        }
        if !one_pane
            && sticky
            && let Some(f) = number("splitter")
        {
            self.split = (f as f32).clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
        }
        if let Some(w) = word("font_family") {
            self.font_family = (!w.is_empty()).then_some(w);
        }
        warnings
    }

    /// The layout keys these flags own (`splitter` only with `sticky_splitter_position`; a Text Edit view has no
    /// `layout` and no `splitter`).
    pub fn keys(&self, one_pane: bool, sticky: bool) -> Vec<(&'static str, LayoutValue)> {
        let mut keys = vec![
            ("word_wrap", LayoutValue::Bool(self.word_wrap)),
            ("header", LayoutValue::Bool(self.header)),
            ("syntax", LayoutValue::Bool(self.syntax)),
            ("whitespace", LayoutValue::Bool(self.whitespace)),
            ("details", LayoutValue::Text(self.details.word().to_owned())),
            (
                "details_height",
                LayoutValue::Float(f64::from(self.details_height.round())),
            ),
            (
                "details_whitespace",
                LayoutValue::Bool(self.details_whitespace),
            ),
            ("ruler", LayoutValue::Bool(self.ruler)),
            (
                "find_panel",
                LayoutValue::Text(self.find_panel.word().to_owned()),
            ),
            (
                "browse_mode",
                LayoutValue::Text(browse_word(self.browse_mode).to_owned()),
            ),
        ];
        if !one_pane {
            let word = if self.over_under {
                "over_under"
            } else {
                "side_by_side"
            };
            keys.push(("layout", LayoutValue::Text(word.to_owned())));
            if sticky {
                keys.push((
                    "splitter",
                    LayoutValue::Float((f64::from(self.split) * 100.0).round() / 100.0),
                ));
            }
        }
        if let Some(family) = &self.font_family {
            keys.push(("font_family", LayoutValue::Text(family.clone())));
        }
        keys
    }

    /// Moves the splitter by `delta` (clamped).
    pub fn move_split(&mut self, delta: f32) {
        self.set_split(self.split + delta);
    }

    /// Sets the splitter (a drag; clamped).
    pub fn set_split(&mut self, split: f32) {
        self.split = split.clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
    }
}

/// The options of OPT-014, OPT-017 and the theme's `[file] current_line_only_over_under` the panes draw with
/// (read by the window from its settings; the defaults are the options' defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tweaks {
    /// `syntax_on_difference_lines`.
    pub syntax_on_difference_lines: bool,
    /// `crosshatch_beyond_eof`.
    pub crosshatch_beyond_eof: bool,
    /// `right_gutter_for_left_editor`: the left pane's numbers on its right, next to the strip.
    pub right_gutter_for_left_editor: bool,
    /// `use_orphan_color`.
    pub use_orphan_color: bool,
    /// `extra_line_spacing_px`.
    pub extra_line_spacing_px: u32,
    /// `column_line_at` (0: off).
    pub column_line_at: u32,
    /// `dim_inactive_pane_percent` (0–90).
    pub dim_inactive_pane_percent: u32,
    /// `single_vertical_scrollbar`.
    pub single_vertical_scrollbar: bool,
    /// `sticky_splitter_position`.
    pub sticky_splitter_position: bool,
    /// The theme's `current_line_only_over_under`: over-under draws the current line across both panes.
    pub current_line_across: bool,
    /// `[next_difference] wrap_around` (OPT-007; the steps of D-309 and the find panel's preset).
    pub wrap_around: bool,
    /// `[next_difference] show_message_panel`: *No more differences.* in the message panel, else a beep.
    pub show_message_panel: bool,
    /// `[next_difference] go_to_first_difference_on_load`.
    pub first_difference_on_load: bool,
    /// `[text_editing] init_find_with_current_word` (D-307).
    pub init_find_with_current_word: bool,
}

impl Default for Tweaks {
    fn default() -> Tweaks {
        Tweaks::of(&Options::default(), false)
    }
}

impl Tweaks {
    /// The tweaks of `options` and the theme's flag.
    pub fn of(options: &Options, current_line_across: bool) -> Tweaks {
        let d = &options.tweaks.editor_display;
        let i = &options.tweaks.interface;
        Tweaks {
            syntax_on_difference_lines: d.syntax_on_difference_lines,
            crosshatch_beyond_eof: d.crosshatch_beyond_eof,
            right_gutter_for_left_editor: d.right_gutter_for_left_editor,
            use_orphan_color: d.use_orphan_color,
            extra_line_spacing_px: d.extra_line_spacing_px,
            column_line_at: d.column_line_at,
            dim_inactive_pane_percent: d.dim_inactive_pane_percent.min(90),
            single_vertical_scrollbar: i.single_vertical_scrollbar,
            sticky_splitter_position: i.sticky_splitter_position,
            current_line_across,
            wrap_around: options.next_difference.wrap_around,
            show_message_panel: options.next_difference.show_message_panel,
            first_difference_on_load: options.next_difference.go_to_first_difference_on_load,
            init_find_with_current_word: options.text_editing.init_find_with_current_word,
        }
    }
}

/// Where the panes go in the view's body (§1.11.14): side by side or over under, the splitter, *Zoom Pane*, one pane
/// for Text Edit. Rectangles are `[x, y, w, h]` from the body's top left; the shapes tests compare them exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneGeometry {
    /// The panes shown: side index (0 left, 1 right) and rectangle.
    pub panes: Vec<(usize, [f32; 4])>,
    /// The copy strip between side-by-side panes (`None`: over under, zoomed or one pane).
    pub strip: Option<[f32; 4]>,
    /// The gutter of each pane is on its right (the left pane with `right_gutter_for_left_editor`, side by side).
    pub gutter_right: [bool; 2],
    /// Copy buttons in each pane's gutter (over under, D-299).
    pub buttons_in_gutter: bool,
}

/// The panes' rectangles in a body of `w` × `h` points (`focus`: the focus pane's index, shown alone when zoomed).
pub fn pane_geometry(
    display: &Display,
    tweaks: &Tweaks,
    one_pane: bool,
    focus: usize,
    w: f32,
    h: f32,
) -> PaneGeometry {
    if one_pane || display.zoom {
        let k = if one_pane { 0 } else { focus };
        return PaneGeometry {
            panes: vec![(k, [0.0, 0.0, w, h])],
            strip: None,
            gutter_right: [false, false],
            buttons_in_gutter: false,
        };
    }
    let split = display.split.clamp(SPLIT_RANGE.0, SPLIT_RANGE.1);
    if display.over_under {
        let top = ((h - DIVIDER_H) * split).floor().max(1.0);
        let bottom = (h - DIVIDER_H - top).max(1.0);
        return PaneGeometry {
            panes: vec![
                (0, [0.0, 0.0, w, top]),
                (1, [0.0, top + DIVIDER_H, w, bottom]),
            ],
            strip: None,
            gutter_right: [false, false],
            buttons_in_gutter: true,
        };
    }
    let left = ((w - STRIP_W) * split).floor().max(10.0);
    let right = (w - STRIP_W - left).max(10.0);
    PaneGeometry {
        panes: vec![
            (0, [0.0, 0.0, left, h]),
            (1, [left + STRIP_W, 0.0, right, h]),
        ],
        strip: Some([left, 0.0, STRIP_W, h]),
        gutter_right: [tweaks.right_gutter_for_left_editor, false],
        buttons_in_gutter: false,
    }
}

/// The help topic of the Display Font dialog.
pub const FONT_TOPIC: &str = "dialog-display-font";
/// The sizes the dialog takes (D-305).
pub const FONT_SIZES: (u32, u32) = (6, 72);

/// The Display Font dialog (`DisplayFont`, D-305): a family the GUI knows and a size (6–72) for the view type
/// (`font_family`, `font_size`); *Use for the theme's Editor font* writes `theme.toml` too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayFontDialog {
    /// The view.
    pub view: hikasa_session::ipc::ViewId,
    /// The families offered (the fonts of the theme's slots, D-160).
    pub families: Vec<String>,
    /// The family chosen (`""`: the theme's editor font).
    pub family: String,
    /// The size as typed.
    pub size: String,
    /// *Use for the theme's Editor font*.
    pub theme: bool,
    /// Why OK did not close.
    pub error: Option<String>,
    chosen: Option<(Option<String>, u32, bool)>,
}

impl DisplayFontDialog {
    /// The dialog of `view` with its family and size.
    pub fn new(
        view: hikasa_session::ipc::ViewId,
        families: Vec<String>,
        family: Option<String>,
        size: u32,
    ) -> DisplayFontDialog {
        DisplayFontDialog {
            view,
            families,
            family: family.unwrap_or_default(),
            size: size.to_string(),
            theme: false,
            error: None,
            chosen: None,
        }
    }

    /// The family (`None`: the theme's editor font), the size and *Use for the theme* after OK (once).
    pub fn take_chosen(&mut self) -> Option<(Option<String>, u32, bool)> {
        self.chosen.take()
    }
}

impl crate::dialog::DialogModel for DisplayFontDialog {
    fn title(&self) -> String {
        "Display Font".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        FONT_TOPIC
    }

    fn buttons(&self) -> Vec<crate::dialog::DialogButton> {
        use crate::dialog::{ButtonRole, DialogButton};
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: true,
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(
        &mut self,
        role: crate::dialog::ButtonRole,
        _svc: &hikasa_session::Service,
    ) -> crate::dialog::DialogStep {
        use crate::dialog::{ButtonRole, DialogStep};
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        match self.size.trim().parse::<u32>() {
            Ok(n) if (FONT_SIZES.0..=FONT_SIZES.1).contains(&n) => {
                let family =
                    (!self.family.trim().is_empty()).then(|| self.family.trim().to_owned());
                self.chosen = Some((family, n, self.theme));
                DialogStep::Close
            }
            _ => {
                self.error = Some(format!(
                    "The size is a number from {} to {}.",
                    FONT_SIZES.0, FONT_SIZES.1
                ));
                DialogStep::Stay
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
