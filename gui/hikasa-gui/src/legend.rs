//! The Legend (M2 W9a, FC-030; D-242): what the folder tree's glyphs, colours and centre marks mean. One entry per
//! row of D-236's tables (`folder::glyphs::GLYPH_TABLE` and the centre column) and per FC-031 colour class, so the
//! Legend cannot disagree with the tree. Toolkit-free: `ui/legend.rs` paints each swatch in the current palette.

use hikasa_core::{ContentKind, Side};
use hikasa_folder::{Category, ColourClass};

use crate::folder::glyphs::{
    CentreIcon, GLYPH_TABLE, GlyphView, content_mark, newer_mark, state_mark,
};

/// The help topic of the panel.
pub const LEGEND_TOPIC: &str = "panel-legend";

/// What an entry shows in front of its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swatch {
    /// A glyph of the name column, as the tree draws it.
    Glyph(GlyphView),
    /// The colour spot of a colour class (FC-031); the category tells the classes that share one apart.
    Class(ColourClass, Category),
    /// A centre column icon (FC-032) and the text mark it stands for.
    Mark(CentreIcon, &'static str),
    /// The Ignored badge (FC-033).
    Ignored,
    /// The Space mark's ring (D-238).
    Marked,
    /// A Folder Sync view's planned action (M2 W9b, FS-001); `true` for a conflict.
    SyncAction(hikasa_folder::sync::SyncAction, bool),
}

/// One line of the Legend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegendEntry {
    /// The swatch.
    pub swatch: Swatch,
    /// What it means.
    pub text: String,
}

/// A titled group of lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegendSection {
    /// The group's title.
    pub title: &'static str,
    /// Its lines.
    pub entries: Vec<LegendEntry>,
}

/// The FC-031 colour classes in the Legend's order, with the category whose colour each takes and its text.
pub const COLOUR_CLASSES: &[(ColourClass, Category, &str)] = &[
    (ColourClass::Same, Category::Same, "Same on both sides"),
    (
        ColourClass::NewerOrDifferent,
        Category::LeftNewer,
        "Newer than the other side, or a folder holding newer items",
    ),
    (
        ColourClass::OlderOrUnknown,
        Category::RightNewer,
        "Older than the other side, or a folder holding older items",
    ),
    (
        ColourClass::Orphan,
        Category::LeftOrphan,
        "On one side only (orphan), or a folder holding orphans",
    ),
    (
        ColourClass::NewerOrDifferent,
        Category::Different,
        "Different, neither side newer",
    ),
    (
        ColourClass::OlderOrUnknown,
        Category::Unknown,
        "Unknown: not compared yet",
    ),
    (ColourClass::Error, Category::Error, "The comparison failed"),
    (
        ColourClass::Filtered,
        Category::Filtered,
        "Filtered out (shown by Suppress Filters)",
    ),
];

/// The Legend of the Folder Compare view: the colours, the name column's glyphs, the centre column.
pub fn sections() -> Vec<LegendSection> {
    let colours = COLOUR_CLASSES
        .iter()
        .map(|&(class, category, text)| LegendEntry {
            swatch: Swatch::Class(class, category),
            text: text.to_owned(),
        })
        .collect();
    let glyphs = GLYPH_TABLE
        .iter()
        .map(|&(glyph, text)| LegendEntry {
            swatch: Swatch::Glyph(glyph),
            text: text.to_owned(),
        })
        .collect();
    let mut centre: Vec<LegendEntry> = Vec::new();
    for side in [Side::Left, Side::Right] {
        let (mark, title) = newer_mark(side);
        centre.push(LegendEntry {
            swatch: Swatch::Mark(CentreIcon::Newer(side), mark),
            text: title.to_owned(),
        });
    }
    for category in Category::ALL {
        if matches!(category, Category::LeftNewer | Category::RightNewer) {
            continue;
        }
        let (mark, title) = state_mark(category);
        centre.push(LegendEntry {
            swatch: Swatch::Mark(CentreIcon::State(category), mark),
            text: title.to_owned(),
        });
    }
    for kind in ContentKind::ALL {
        let (mark, title) = content_mark(kind);
        centre.push(LegendEntry {
            swatch: Swatch::Mark(CentreIcon::Content(kind), mark),
            text: title.to_owned(),
        });
    }
    centre.push(LegendEntry {
        swatch: Swatch::Ignored,
        text: "Ignored: the difference counts as same (FC-033)".to_owned(),
    });
    centre.push(LegendEntry {
        swatch: Swatch::Marked,
        text: "Marked with Space: Open and Compare To pair it with the selected item".to_owned(),
    });
    vec![
        LegendSection {
            title: "Colours",
            entries: colours,
        },
        LegendSection {
            title: "Name column",
            entries: glyphs,
        },
        LegendSection {
            title: "Centre column",
            entries: centre,
        },
    ]
}

/// The Legend of a Folder Sync view (§1.9.11: it adds the action icons): every section of [`sections`] and the actions.
pub fn sync_sections() -> Vec<LegendSection> {
    use hikasa_folder::sync::SyncAction;
    let mut out = sections();
    let mut entries: Vec<LegendEntry> = [
        SyncAction::CopyRight,
        SyncAction::CopyLeft,
        SyncAction::DeleteLeft,
        SyncAction::DeleteRight,
        SyncAction::Leave,
    ]
    .into_iter()
    .map(|action| LegendEntry {
        swatch: Swatch::SyncAction(action, false),
        text: crate::folder_view::sync::action_text(action).to_owned(),
    })
    .collect();
    entries.push(LegendEntry {
        swatch: Swatch::SyncAction(SyncAction::Leave, true),
        text: "A conflict: different on both sides, and the matrix leaves it".to_owned(),
    });
    out.push(LegendSection {
        title: "Sync actions",
        entries,
    });
    out
}

/// The Legend as text, one line per entry (tests, the help topic's table).
#[cfg(test)]
pub fn text() -> String {
    let mut out = String::new();
    for section in sections() {
        out.push_str(section.title);
        out.push('\n');
        for entry in section.entries {
            let swatch = match entry.swatch {
                Swatch::Glyph(g) => format!("{:?}", g.base),
                Swatch::Class(c, _) => format!("{c:?}"),
                Swatch::Mark(_, m) => m.to_owned(),
                Swatch::Ignored => crate::folder::glyphs::IGNORED_BADGE.to_owned(),
                Swatch::Marked => "( )".to_owned(),
                Swatch::SyncAction(action, conflict) => {
                    format!("{action:?}{}", if conflict { "!" } else { "" })
                }
            };
            out.push_str(&format!("  {swatch}\t{}\n", entry.text));
        }
    }
    out
}
