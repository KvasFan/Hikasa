//! The glyphs and the centre column of a folder row (M2 W9a, FC-012, FC-013, FC-015, FC-032, FC-033; D-236): one
//! normative table feeds the tree and the Legend (`legend.rs`), so the two cannot disagree. Toolkit-free: `ui/` maps a
//! [`Glyph`] and a [`GlyphColour`] to an icon and a palette colour.

use hikasa_core::{ContentKind, Side};
use hikasa_folder::{Category, FolderRules, NodeFlags, NodeKind, NodeView, ScanStatus};

/// The base glyph in front of a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    /// None: the colour spot alone (files, links, other kinds).
    Spot,
    /// A scanned folder.
    Folder,
    /// A folder not scanned while its comparison runs, after a stop, or an orphan folder left unscanned.
    FolderHollow,
    /// A folder not opened yet because subfolders are not scanned automatically (FC-012).
    FolderFilled,
    /// An archive row (FC-015): the zipper instead of the folder.
    Archive,
}

/// The colour a glyph takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphColour {
    /// The FC-031 class of its side.
    Class,
    /// `[folder] unknown`.
    Unknown,
    /// `[folder] unopened_folder`.
    Unopened,
    /// The palette's error colour.
    Error,
}

/// A glyph with its overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphView {
    /// The base glyph.
    pub base: Glyph,
    /// Its colour.
    pub colour: GlyphColour,
    /// The red X: the folder's listing failed on this side.
    pub error: bool,
    /// The arrow: a symbolic link or junction, followed or not (FC-013).
    pub link: bool,
}

/// What [`glyph_of`] reads besides the node.
#[derive(Debug, Clone, Copy)]
pub struct GlyphInput<'a> {
    /// The comparison's rules (`auto_scan_subfolders`, `scan_orphan_folders`).
    pub rules: &'a FolderRules,
    /// The comparison is still running.
    pub running: bool,
    /// The node has an error on this side (its listing failed).
    pub failed_here: bool,
}

/// The glyph of `node` on `side` (D-236's table).
pub fn glyph_of(node: &NodeView<'_>, side: Side, input: &GlyphInput<'_>) -> GlyphView {
    let entry = node.side(side);
    let link = node.kind == NodeKind::Symlink || entry.is_some_and(|e| e.followed);
    let archive = node.flags.contains(NodeFlags::ARCHIVE);
    if !node.is_folder() {
        return GlyphView {
            base: if archive { Glyph::Archive } else { Glyph::Spot },
            colour: GlyphColour::Class,
            error: false,
            link,
        };
    }
    let orphan = node.left.is_none() || node.right.is_none();
    let (base, colour, error) = match node.scan {
        ScanStatus::Failed if input.failed_here => (Glyph::Folder, GlyphColour::Error, true),
        ScanStatus::Scanned | ScanStatus::NotApplicable | ScanStatus::Failed => {
            (Glyph::Folder, GlyphColour::Class, false)
        }
        ScanStatus::NotScanned => {
            if input.running
                || (orphan && !input.rules.handling.scan_orphan_folders)
                || input.rules.handling.auto_scan_subfolders
            {
                (Glyph::FolderHollow, GlyphColour::Unknown, false)
            } else {
                (Glyph::FolderFilled, GlyphColour::Unopened, false)
            }
        }
    };
    GlyphView {
        base: if archive { Glyph::Archive } else { base },
        colour,
        error,
        link,
    }
}

/// What the centre column shows (FC-032).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CentreIcon {
    /// A content test's result.
    Content(ContentKind),
    /// The newer side of a pair: left newer → an arrow pointing right.
    Newer(Side),
    /// The M1 state mark of the category.
    State(Category),
}

/// The centre cell of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Centre {
    /// The icon.
    pub icon: CentreIcon,
    /// The text mark (`=`, `≠`, …) the icon stands for.
    pub mark: &'static str,
    /// The Ignored badge (FC-033).
    pub ignored: bool,
    /// The Space mark ring (D-238).
    pub marked: bool,
    /// The tooltip.
    pub tooltip: String,
}

/// The state mark and its title of a category (M1).
pub fn state_mark(category: Category) -> (&'static str, &'static str) {
    match category {
        Category::Same => ("=", "Same"),
        Category::Different => ("≠", "Different"),
        Category::LeftNewer => ("◀", "Left newer"),
        Category::RightNewer => ("▶", "Right newer"),
        Category::LeftOrphan => ("◧", "Left orphan"),
        Category::RightOrphan => ("◨", "Right orphan"),
        Category::Unknown => ("?", "Unknown (not compared yet)"),
        Category::Error => ("!", "Error"),
        Category::Filtered => ("·", "Filtered"),
    }
}

/// The mark and title of a content result.
pub fn content_mark(kind: ContentKind) -> (&'static str, &'static str) {
    match kind {
        ContentKind::BinarySame => ("=", "Binary: same"),
        ContentKind::BinaryDifferent => ("≠", "Binary: different"),
        ContentKind::RulesSame => ("≈", "Rules-based: same"),
        ContentKind::UnimportantOnly => ("~", "Rules-based: unimportant differences only"),
        ContentKind::ImportantDifferences => ("!", "Rules-based: important differences"),
        ContentKind::Error => ("!", "Content comparison failed"),
    }
}

/// The centre cell of `node` (FC-032): the content result when a test ran, else the newer side's arrow, else the
/// state mark; the Ignored badge and the Space mark on top.
pub fn centre_of(node: &NodeView<'_>, category: Category, marked: bool) -> Centre {
    let ignored = node.flags.contains(NodeFlags::IGNORED);
    let (icon, mark, title) = if let Some(content) = node.content.filter(|_| !node.is_folder()) {
        let (mark, title) = content_mark(content.kind);
        (CentreIcon::Content(content.kind), mark, title)
    } else {
        match category {
            Category::LeftNewer => (CentreIcon::Newer(Side::Left), "▶", "Left is newer"),
            Category::RightNewer => (CentreIcon::Newer(Side::Right), "◀", "Right is newer"),
            other => {
                let (mark, title) = state_mark(other);
                (CentreIcon::State(other), mark, title)
            }
        }
    };
    let mut tooltip = title.to_owned();
    if ignored {
        tooltip.push_str(" · Ignored");
    }
    if marked {
        tooltip.push_str(" · Marked");
    }
    Centre {
        icon,
        mark,
        ignored,
        marked,
        tooltip,
    }
}

/// One row of the glyph table: a glyph and what it means (the Legend lists these, D-242).
pub const GLYPH_TABLE: &[(GlyphView, &str)] = &[
    (
        GlyphView {
            base: Glyph::Spot,
            colour: GlyphColour::Class,
            error: false,
            link: false,
        },
        "File: the colour spot only",
    ),
    (
        GlyphView {
            base: Glyph::Folder,
            colour: GlyphColour::Class,
            error: false,
            link: false,
        },
        "Folder, scanned",
    ),
    (
        GlyphView {
            base: Glyph::FolderHollow,
            colour: GlyphColour::Unknown,
            error: false,
            link: false,
        },
        "Folder not scanned (comparing, stopped, or an orphan folder left unscanned)",
    ),
    (
        GlyphView {
            base: Glyph::FolderFilled,
            colour: GlyphColour::Unopened,
            error: false,
            link: false,
        },
        "Folder not opened yet (subfolders are not scanned automatically)",
    ),
    (
        GlyphView {
            base: Glyph::Folder,
            colour: GlyphColour::Error,
            error: true,
            link: false,
        },
        "Folder whose listing failed",
    ),
    (
        GlyphView {
            base: Glyph::Spot,
            colour: GlyphColour::Class,
            error: false,
            link: true,
        },
        "Symbolic link or junction, followed or not",
    ),
    (
        GlyphView {
            base: Glyph::Archive,
            colour: GlyphColour::Class,
            error: false,
            link: false,
        },
        "Archive",
    ),
];
