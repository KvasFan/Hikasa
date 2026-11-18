//! The report vocabulary for dialogs (M2 W13c, D-367): the layouts of each [`ReportKind`] with their forms, the
//! option words each layout takes (a check or a choice), the displays and the outputs, with English captions beside
//! the words (Q13). Pure data: the report dialog lists the words from here (words are core's, T11) and builds the
//! CLI's flags from the fields; [`ReportRequest::parse`](crate::ReportRequest::parse) reads them. The CLI does not
//! use the table.
//!
//! **Completeness, both ways** (`tests/vocabulary.rs`, A13.16): every word listed here parses for its kind and layout,
//! and every canonical word the parser accepts for a kind ([`crate::request::layout_words`],
//! [`crate::request::option_words`]) is listed here, or no layout of the kind uses it.

use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode};
use hikasa_hex::DisplayFilter as HexFilter;
use hikasa_picture::DiffMode;
use hikasa_table::RowFilter;
use hikasa_text::TextDisplayFilter;

use crate::request::{CONTEXT_WORDS, ReportKind};

/// The forms a layout writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Forms {
    /// HTML (`html-color`, `html-mono`, `html-custom:<css>`).
    pub html: bool,
    /// Plain text (`text`).
    pub text: bool,
    /// XML (the layout implies it).
    pub xml: bool,
    /// JSON (the layout implies it).
    pub json: bool,
    /// Plain text the layout implies (a patch, a CSV, a folder list): no palette word.
    pub patch: bool,
}

impl Forms {
    /// True when the layout implies its form: the output field is greyed.
    pub const fn implied(self) -> bool {
        !self.html && !self.text
    }
}

const HTML_TEXT: Forms = Forms {
    html: true,
    text: true,
    xml: false,
    json: false,
    patch: false,
};
const HTML: Forms = Forms {
    html: true,
    text: false,
    xml: false,
    json: false,
    patch: false,
};
const XML: Forms = Forms {
    html: false,
    text: false,
    xml: true,
    json: false,
    patch: false,
};
const JSON: Forms = Forms {
    html: false,
    text: false,
    xml: false,
    json: true,
    patch: false,
};
const PATCH: Forms = Forms {
    html: false,
    text: false,
    xml: false,
    json: false,
    patch: true,
};

/// An option of a layout: one word is a check, several are a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionEntry {
    /// The words (one: a check; several: a choice, mutually exclusive).
    pub words: &'static [&'static str],
    /// The caption.
    pub caption: &'static str,
    /// The word chosen when the dialog has no remembered one (a listed word; `None`: a check off, or no word).
    pub default: Option<&'static str>,
}

impl OptionEntry {
    /// True for a check (one word).
    pub const fn is_check(&self) -> bool {
        self.words.len() == 1
    }
}

/// A layout of a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutEntry {
    /// The layout word.
    pub word: &'static str,
    /// The caption.
    pub caption: &'static str,
    /// The forms it writes.
    pub forms: Forms,
    /// It takes a scope (*Just selection*: text lines, folder rows).
    pub scope: bool,
    /// It takes the display (the rows shown).
    pub display: bool,
    /// It takes a number of context lines (text: `unified`, `patch`, and the display *Context*).
    pub context_lines: bool,
    /// It takes the text wrap (`wrap-*`).
    pub wrap: bool,
    /// Its option words.
    pub options: &'static [OptionEntry],
}

/// A display (the rows a report shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayEntry {
    /// The `display-*` word.
    pub word: &'static str,
    /// The caption.
    pub caption: &'static str,
    /// It takes a number (text: the context lines).
    pub number: bool,
}

/// An output (the palette of a layout with an HTML or a text form).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputEntry {
    /// The word (`html-custom:` takes the stylesheet's path after it).
    pub word: &'static str,
    /// The caption.
    pub caption: &'static str,
    /// An HTML output (else plain text).
    pub html: bool,
}

/// The words of one kind's report dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportVocabulary {
    /// The kind.
    pub kind: ReportKind,
    /// Its layouts, the dialog's default first.
    pub layouts: &'static [LayoutEntry],
    /// Its displays (for the layouts that take one).
    pub displays: &'static [DisplayEntry],
    /// Some layout of the kind takes `ignore-unimportant`.
    pub ignore_unimportant: bool,
    /// The outputs of layouts with an HTML or a text form.
    pub outputs: &'static [OutputEntry],
    /// The print words (*Print Preview*): the palette and the orientation.
    pub print: &'static [OptionEntry],
    /// The text wrap (`wrap-*`) of the layouts that take it.
    pub wrap: Option<OptionEntry>,
    /// The folders of a folder report's display: the values of the CLI's `--folders` (not words of the request).
    pub folders: Option<OptionEntry>,
}

impl ReportVocabulary {
    /// The layout `word` (`None` when the kind has no such layout).
    pub fn layout(&self, word: &str) -> Option<&'static LayoutEntry> {
        self.layouts.iter().find(|l| l.word == word)
    }
}

/// The custom stylesheet output's word (the path follows it).
pub const HTML_CUSTOM_WORD: &str = "html-custom:";

/// The word of *Ignore Unimportant* (kinds whose [`ReportVocabulary::ignore_unimportant`] is set).
pub const IGNORE_UNIMPORTANT_WORD: &str = "ignore-unimportant";

const FOLDERS: OptionEntry = OptionEntry {
    words: &["structure", "always", "files-only"],
    caption: "Folders",
    default: Some("structure"),
};

const OUTPUTS: [OutputEntry; 4] = [
    OutputEntry {
        word: "html-color",
        html: true,
        caption: "HTML (colour)",
    },
    OutputEntry {
        word: "html-mono",
        html: true,
        caption: "HTML (monochrome)",
    },
    OutputEntry {
        word: HTML_CUSTOM_WORD,
        html: true,
        caption: "HTML with a stylesheet",
    },
    OutputEntry {
        word: "text",
        html: false,
        caption: "Plain text",
    },
];

const PRINT: [OptionEntry; 2] = [
    OptionEntry {
        words: &["print-color", "print-mono"],
        caption: "Print palette",
        default: Some("print-color"),
    },
    OptionEntry {
        words: &["print-portrait", "print-landscape"],
        caption: "Orientation",
        default: Some("print-portrait"),
    },
];

const LINE_NUMBERS: OptionEntry = OptionEntry {
    words: &["line-numbers", "no-line-numbers"],
    caption: "Line numbers",
    default: Some("line-numbers"),
};
const ROW_NUMBERS: OptionEntry = OptionEntry {
    words: &["line-numbers", "no-line-numbers"],
    caption: "Row numbers",
    default: Some("line-numbers"),
};
const ADDRESSES: OptionEntry = OptionEntry {
    words: &["line-numbers", "no-line-numbers"],
    caption: "Addresses",
    default: Some("line-numbers"),
};
const WRAP: OptionEntry = OptionEntry {
    words: &["wrap-none", "wrap-character", "wrap-word"],
    caption: "Wrap",
    default: Some("wrap-character"),
};

// ---- text ------------------------------------------------------------------------------------------------------

const TEXT_LAYOUTS: [LayoutEntry; 7] = [
    LayoutEntry {
        word: "side-by-side",
        caption: "Side by side",
        forms: HTML_TEXT,
        scope: true,
        display: true,
        context_lines: true,
        wrap: true,
        options: &[LINE_NUMBERS],
    },
    LayoutEntry {
        word: "interleaved",
        caption: "Interleaved",
        forms: HTML_TEXT,
        scope: true,
        display: true,
        context_lines: true,
        wrap: true,
        options: &[
            LINE_NUMBERS,
            OptionEntry {
                words: &["strikeout-left-diffs"],
                caption: "Strike out the left differences",
                default: None,
            },
            OptionEntry {
                words: &["strikeout-right-diffs"],
                caption: "Strike out the right differences",
                default: None,
            },
        ],
    },
    LayoutEntry {
        word: "summary",
        caption: "Summary",
        forms: HTML_TEXT,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "unified",
        caption: "Unified diff (with importance)",
        forms: PATCH,
        scope: false,
        display: false,
        context_lines: true,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "patch",
        caption: "Patch",
        forms: PATCH,
        scope: false,
        display: false,
        context_lines: true,
        wrap: false,
        options: &[OptionEntry {
            words: &["patch-unified", "patch-context", "patch-normal"],
            caption: "Patch style",
            default: Some("patch-unified"),
        }],
    },
    LayoutEntry {
        word: "statistics",
        caption: "Statistics (CSV)",
        forms: PATCH,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "xml",
        caption: "XML",
        forms: XML,
        scope: true,
        display: true,
        context_lines: true,
        wrap: false,
        options: &[],
    },
];

const TEXT_DISPLAYS: [DisplayEntry; 5] = [
    DisplayEntry {
        word: "display-all",
        caption: "All",
        number: false,
    },
    DisplayEntry {
        word: "display-mismatches",
        caption: "Differences",
        number: false,
    },
    DisplayEntry {
        word: "display-context",
        caption: "Differences with context",
        number: true,
    },
    DisplayEntry {
        word: "display-matches",
        caption: "Same",
        number: false,
    },
    DisplayEntry {
        word: "display-none",
        caption: "None",
        number: false,
    },
];

// ---- folders ---------------------------------------------------------------------------------------------------

const FOLDER_LAYOUTS: [LayoutEntry; 5] = [
    LayoutEntry {
        word: "side-by-side",
        caption: "Side by side",
        forms: HTML_TEXT,
        scope: true,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[
            OptionEntry {
                words: &["column-size"],
                caption: "Size",
                default: Some("column-size"),
            },
            OptionEntry {
                words: &["column-timestamp"],
                caption: "Modified",
                default: Some("column-timestamp"),
            },
            OptionEntry {
                words: &["column-attributes"],
                caption: "Attributes",
                default: None,
            },
            OptionEntry {
                words: &["column-crc"],
                caption: "CRC",
                default: None,
            },
            OptionEntry {
                words: &["column-owner"],
                caption: "Owner",
                default: None,
            },
            OptionEntry {
                words: &["column-group"],
                caption: "Group",
                default: None,
            },
            OptionEntry {
                words: &["column-status"],
                caption: "Status",
                default: None,
            },
            OptionEntry {
                words: &["column-none"],
                caption: "No columns",
                default: None,
            },
            OptionEntry {
                words: &["include-file-links"],
                caption: "Include links to file reports",
                default: None,
            },
        ],
    },
    LayoutEntry {
        word: "summary",
        caption: "Summary",
        forms: HTML_TEXT,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "list",
        caption: "List",
        forms: PATCH,
        scope: true,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "xml",
        caption: "XML",
        forms: XML,
        scope: true,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "json",
        caption: "JSON",
        forms: JSON,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[],
    },
];

const FOLDER_DISPLAYS: [DisplayEntry; 14] = [
    DisplayEntry {
        word: "display-all",
        caption: "All",
        number: false,
    },
    DisplayEntry {
        word: "display-mismatches",
        caption: "Differences",
        number: false,
    },
    DisplayEntry {
        word: "display-context",
        caption: "Differences with their folders",
        number: false,
    },
    DisplayEntry {
        word: "display-matches",
        caption: "Same",
        number: false,
    },
    DisplayEntry {
        word: "display-no-orphans",
        caption: "No orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-mismatches-no-orphans",
        caption: "Differences but no orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-orphans",
        caption: "Orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-left-newer",
        caption: "Left newer",
        number: false,
    },
    DisplayEntry {
        word: "display-right-newer",
        caption: "Right newer",
        number: false,
    },
    DisplayEntry {
        word: "display-left-newer-orphans",
        caption: "Left newer and left orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-right-newer-orphans",
        caption: "Right newer and right orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-left-orphans",
        caption: "Left orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-right-orphans",
        caption: "Right orphans",
        number: false,
    },
    DisplayEntry {
        word: "display-none",
        caption: "None",
        number: false,
    },
];

// ---- tables ----------------------------------------------------------------------------------------------------

const TABLE_LAYOUTS: [LayoutEntry; 5] = [
    LayoutEntry {
        word: "side-by-side",
        caption: "Side by side",
        forms: HTML_TEXT,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[ROW_NUMBERS],
    },
    LayoutEntry {
        word: "interleaved",
        caption: "Interleaved",
        forms: HTML_TEXT,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[ROW_NUMBERS],
    },
    LayoutEntry {
        word: "summary",
        caption: "Summary",
        forms: HTML_TEXT,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "xml",
        caption: "XML",
        forms: XML,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[],
    },
    LayoutEntry {
        word: "json",
        caption: "JSON",
        forms: JSON,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[OptionEntry {
            words: &["cells-differing", "cells-all"],
            caption: "Cells",
            default: Some("cells-differing"),
        }],
    },
];

const TABLE_DISPLAYS: [DisplayEntry; 4] = [
    DisplayEntry {
        word: "display-all",
        caption: "All rows",
        number: false,
    },
    DisplayEntry {
        word: "display-mismatches",
        caption: "Rows with differences",
        number: false,
    },
    DisplayEntry {
        word: "display-matches",
        caption: "Same rows",
        number: false,
    },
    DisplayEntry {
        word: "display-none",
        caption: "No rows",
        number: false,
    },
];

// ---- hex -------------------------------------------------------------------------------------------------------

const HEX_CONTEXT: OptionEntry = OptionEntry {
    words: &CONTEXT_WORDS,
    caption: "Context rows",
    default: Some("context-0"),
};

const HEX_LAYOUTS: [LayoutEntry; 3] = [
    LayoutEntry {
        word: "side-by-side",
        caption: "Side by side",
        forms: HTML_TEXT,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[ADDRESSES, HEX_CONTEXT],
    },
    LayoutEntry {
        word: "interleaved",
        caption: "Interleaved",
        forms: HTML_TEXT,
        scope: false,
        display: true,
        context_lines: false,
        wrap: false,
        options: &[ADDRESSES, HEX_CONTEXT],
    },
    LayoutEntry {
        word: "summary",
        caption: "Summary",
        forms: HTML_TEXT,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[],
    },
];

const HEX_DISPLAYS: [DisplayEntry; 4] = [
    DisplayEntry {
        word: "display-all",
        caption: "All",
        number: false,
    },
    DisplayEntry {
        word: "display-differences",
        caption: "Differences",
        number: false,
    },
    DisplayEntry {
        word: "display-same",
        caption: "Same",
        number: false,
    },
    DisplayEntry {
        word: "display-none",
        caption: "None",
        number: false,
    },
];

// ---- pictures --------------------------------------------------------------------------------------------------

const PICTURE_LAYOUTS: [LayoutEntry; 2] = [
    LayoutEntry {
        word: "side-by-side",
        caption: "Side by side",
        forms: HTML,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[
            OptionEntry {
                words: &["mode-tolerance", "mode-range", "mode-blend"],
                caption: "Difference mode",
                default: None,
            },
            OptionEntry {
                words: &["checkerboard"],
                caption: "Transparency checkerboard",
                default: None,
            },
            OptionEntry {
                words: &["full-size"],
                caption: "Full-size images",
                default: None,
            },
        ],
    },
    LayoutEntry {
        word: "summary",
        caption: "Summary",
        forms: HTML_TEXT,
        scope: false,
        display: false,
        context_lines: false,
        wrap: false,
        options: &[],
    },
];

static TEXT_VOCABULARY: ReportVocabulary = ReportVocabulary {
    kind: ReportKind::Text,
    layouts: &TEXT_LAYOUTS,
    displays: &TEXT_DISPLAYS,
    ignore_unimportant: true,
    outputs: &OUTPUTS,
    print: &PRINT,
    wrap: Some(WRAP),
    folders: None,
};
static FOLDER_VOCABULARY: ReportVocabulary = ReportVocabulary {
    kind: ReportKind::Folder,
    layouts: &FOLDER_LAYOUTS,
    displays: &FOLDER_DISPLAYS,
    ignore_unimportant: true,
    outputs: &OUTPUTS,
    print: &PRINT,
    wrap: None,
    folders: Some(FOLDERS),
};
static TABLE_VOCABULARY: ReportVocabulary = ReportVocabulary {
    kind: ReportKind::Table,
    layouts: &TABLE_LAYOUTS,
    displays: &TABLE_DISPLAYS,
    ignore_unimportant: true,
    outputs: &OUTPUTS,
    print: &PRINT,
    wrap: None,
    folders: None,
};
static HEX_VOCABULARY: ReportVocabulary = ReportVocabulary {
    kind: ReportKind::Hex,
    layouts: &HEX_LAYOUTS,
    displays: &HEX_DISPLAYS,
    ignore_unimportant: false,
    outputs: &OUTPUTS,
    print: &PRINT,
    wrap: None,
    folders: None,
};
static PICTURE_VOCABULARY: ReportVocabulary = ReportVocabulary {
    kind: ReportKind::Picture,
    layouts: &PICTURE_LAYOUTS,
    displays: &[],
    ignore_unimportant: true,
    outputs: &OUTPUTS,
    print: &PRINT,
    wrap: None,
    folders: None,
};

/// The vocabulary of `kind`'s report dialog.
pub fn vocabulary(kind: ReportKind) -> &'static ReportVocabulary {
    match kind {
        ReportKind::Text => &TEXT_VOCABULARY,
        ReportKind::Folder => &FOLDER_VOCABULARY,
        ReportKind::Table => &TABLE_VOCABULARY,
        ReportKind::Hex => &HEX_VOCABULARY,
        ReportKind::Picture => &PICTURE_VOCABULARY,
    }
}

/// A view's state as its report dialog's starting words (D-366: the display defaults to the view's filter; hex
/// addresses, picture mode and checkerboard to the view's).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ViewWords {
    /// The display word (`None`: the view's filter has no word, the kind's first display is taken).
    pub display: Option<&'static str>,
    /// The number of the display (text: the context lines of *Context*).
    pub context: Option<u32>,
    /// Option words the view turns on (a check on, a choice's word chosen).
    pub on: Vec<&'static str>,
    /// Check words the view turns off.
    pub off: Vec<&'static str>,
    /// The folders value of a folder view.
    pub folders: Option<&'static str>,
}

/// A Text Compare view's filter as report words.
pub fn text_view_words(filter: TextDisplayFilter) -> ViewWords {
    let (display, context) = match filter {
        TextDisplayFilter::All => (Some("display-all"), None),
        TextDisplayFilter::Differences => (Some("display-mismatches"), None),
        TextDisplayFilter::Same => (Some("display-matches"), None),
        TextDisplayFilter::None => (Some("display-none"), None),
        TextDisplayFilter::Context { lines } => (Some("display-context"), Some(lines)),
        _ => (None, None),
    };
    ViewWords {
        display,
        context,
        ..ViewWords::default()
    }
}

/// A Folder Compare or Folder Sync view's display as report words (a toggle set has no word).
pub fn folder_view_words(display: &DisplayOptions) -> ViewWords {
    use FolderDisplayFilter as F;
    let word = match display.filter {
        F::All => Some("display-all"),
        F::Differences => Some("display-mismatches"),
        F::Same => Some("display-matches"),
        F::NoOrphans => Some("display-no-orphans"),
        F::DifferencesNoOrphans => Some("display-mismatches-no-orphans"),
        F::Orphans => Some("display-orphans"),
        F::LeftNewer => Some("display-left-newer"),
        F::RightNewer => Some("display-right-newer"),
        F::LeftNewerAndLeftOrphans => Some("display-left-newer-orphans"),
        F::RightNewerAndRightOrphans => Some("display-right-newer-orphans"),
        F::LeftOrphans => Some("display-left-orphans"),
        F::RightOrphans => Some("display-right-orphans"),
        F::None => Some("display-none"),
        F::Toggles { .. } => None,
    };
    ViewWords {
        display: word,
        folders: Some(match display.folders {
            FolderMode::CompareStructure => "structure",
            FolderMode::AlwaysShowFolders => "always",
            FolderMode::OnlyFiles => "files-only",
        }),
        ..ViewWords::default()
    }
}

/// A Table Compare view's row filter as report words (a custom toggle set has no word).
pub fn table_view_words(filter: RowFilter) -> ViewWords {
    let display = match crate::table::display_word(filter) {
        "custom" => None,
        word => TABLE_DISPLAYS
            .iter()
            .find(|d| d.word.strip_prefix("display-") == Some(word))
            .map(|d| d.word),
    };
    ViewWords {
        display,
        ..ViewWords::default()
    }
}

/// A Hex Compare view's filter, context rows and *Show addresses* as report words.
pub fn hex_view_words(filter: Option<HexFilter>, context: u8, addresses: bool) -> ViewWords {
    let display = match filter {
        Some(HexFilter::All) => "display-all",
        Some(HexFilter::Differences) => "display-differences",
        Some(HexFilter::Same) => "display-same",
        None => "display-none",
    };
    let mut on = vec![ADDRESSES.words[usize::from(!addresses)]];
    on.extend(CONTEXT_WORDS.get(usize::from(context)).copied());
    ViewWords {
        display: Some(display),
        on,
        ..ViewWords::default()
    }
}

/// A Picture Compare view's mode and checkerboard as report words.
pub fn picture_view_words(mode: DiffMode, checkerboard: bool) -> ViewWords {
    let mode = match mode {
        DiffMode::Tolerance => "mode-tolerance",
        DiffMode::Range => "mode-range",
        DiffMode::Blend { .. } => "mode-blend",
    };
    let (mut on, mut off) = (vec![mode], Vec::new());
    if checkerboard {
        on.push("checkerboard");
    } else {
        off.push("checkerboard");
    }
    ViewWords {
        on,
        off,
        ..ViewWords::default()
    }
}
