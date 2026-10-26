//! [`NameFilter`]: the Name Filters of a folder comparison (FC-050, FC-051, FC-054; algorithm §2.3; D-09).
//!
//! Evaluation is top-down and **per side**: the scanner keeps one [`FolderVerdict`] chain per side and asks
//! [`NameFilter::folder`] / [`NameFilter::file`] once per listed entry. No joined path is ever built, and the
//! common case (ASCII or NFC names) allocates nothing.
//!
//! ```
//! use hikasa_core::filter::{FolderVerdict, NameFilter};
//! use hikasa_core::mask::{CaseSensitivity, MaskOptions};
//! use hikasa_core::path::RelPath;
//!
//! let filter = NameFilter::parse(r"*.c;*.h;-*.bak;-obj\;src\", MaskOptions { case: CaseSensitivity::Sensitive })
//!     .unwrap();
//! assert_eq!(filter.to_filter_string(), r"*.c;*.h;src\;-*.bak;-obj\");
//! let root = RelPath::root();
//! assert_eq!(filter.folder(&root, "src", FolderVerdict::Base), FolderVerdict::Included);
//! assert_eq!(filter.folder(&root, "obj", FolderVerdict::Base), FolderVerdict::Excluded);
//! assert!(filter.file(&root, "main.c", FolderVerdict::Base));
//! assert!(!filter.file(&root, "main.bak", FolderVerdict::Base));
//! ```

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::case::nfc;
use crate::mask::{Mask, MaskError, MaskList, MaskOptions, MaskTarget, Name, Polarity, trim_item};
use crate::path::RelPath;

/// The four persisted lists of the Name Filters tab (FC-050). Order is preserved.
///
/// Items are written without the `-` prefix (the list says whether they exclude) and folder items carry their
/// trailing separator (`obj\`), exactly as [`NameFilter::to_lists`] produces them. Items are trimmed of spaces
/// and tabs, so a mask that starts with a blank writes it as a set (`[ ]x` for the name ` x`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NameFilterLists {
    /// Include-file masks (empty = all files).
    pub include_files: Vec<String>,
    /// Exclude-file masks.
    pub exclude_files: Vec<String>,
    /// Include-folder masks (empty = all folders).
    pub include_folders: Vec<String>,
    /// Exclude-folder masks.
    pub exclude_folders: Vec<String>,
}

/// Verdict for a folder, computed top-down per side (the parent's verdict is an input; the base is `Base`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderVerdict {
    /// The base folder itself: its files are subject to file masks; it does not propagate subtree inclusion.
    Base,
    /// Scanned; its files are subject to file masks; its subfolders inherit inclusion (D-09).
    Included,
    /// Scanned only because it is a prefix of an anchored include-folder mask; its files are excluded.
    PassThrough,
    /// Not scanned; the whole subtree is excluded.
    Excluded,
}

#[derive(Debug)]
struct Lists {
    include_files: MaskList,
    exclude_files: MaskList,
    include_folders: MaskList,
    exclude_folders: MaskList,
    opts: MaskOptions,
}

/// Compiled name filter. Pure, `Send + Sync`, cheap to clone (Arc inside).
#[derive(Debug, Clone)]
pub struct NameFilter {
    inner: Arc<Lists>,
}

/// `body` with each leading ASCII space or tab written as a one-character set (`" \tx"` → `"[ ][\t]x"`), so
/// that trimming the item (§2.2.1) cannot drop them. The set matches exactly the same char.
fn protect_leading_blanks(body: &str) -> String {
    let rest = body.trim_start_matches([' ', '\t']);
    let blanks = body.len() - rest.len();
    let mut out = String::with_capacity(body.len() + 2 * blanks);
    for blank in body.chars().take_while(|c| matches!(c, ' ' | '\t')) {
        out.push('[');
        out.push(blank);
        out.push(']');
    }
    out.push_str(rest);
    out
}

/// Sorts parsed masks into the four lists by their own polarity and target.
fn sort_masks(masks: Vec<Mask>, opts: MaskOptions) -> NameFilter {
    let mut include_files = Vec::new();
    let mut exclude_files = Vec::new();
    let mut include_folders = Vec::new();
    let mut exclude_folders = Vec::new();
    for mask in masks {
        match (mask.polarity(), mask.target()) {
            (Polarity::Include, MaskTarget::Files) => include_files.push(mask),
            (Polarity::Exclude, MaskTarget::Files) => exclude_files.push(mask),
            (Polarity::Include, MaskTarget::Folders) => include_folders.push(mask),
            (Polarity::Exclude, MaskTarget::Folders) => exclude_folders.push(mask),
        }
    }
    NameFilter {
        inner: Arc::new(Lists {
            include_files: MaskList::from_masks(include_files, opts),
            exclude_files: MaskList::from_masks(exclude_files, opts),
            include_folders: MaskList::from_masks(include_folders, opts),
            exclude_folders: MaskList::from_masks(exclude_folders, opts),
            opts,
        }),
    }
}

impl NameFilter {
    /// Matches everything.
    pub fn match_all(opts: MaskOptions) -> NameFilter {
        sort_masks(Vec::new(), opts)
    }

    /// Parses the toolbar syntax `*.c;*.h;-*.bak;-obj\;src\` (FC-050): `;`-separated items, `-` excludes, a
    /// trailing separator makes a folder mask.
    pub fn parse(filter: &str, opts: MaskOptions) -> Result<NameFilter, MaskError> {
        let mut masks = Vec::new();
        for item in filter.split(';') {
            if trim_item(item).is_empty() {
                continue;
            }
            masks.push(Mask::parse(item, opts)?);
        }
        Ok(sort_masks(masks, opts))
    }

    /// From the four lists. Each list item may itself hold several `;`-separated masks. Items of the exclude
    /// lists are parsed with a `-` prefix and items of the folder lists get a trailing `\` when they have no
    /// trailing separator; every mask is then placed by its own syntax (so an include-files item written as
    /// `-x` or `x\` is not lost: it lands in the exclude-files or include-folders list).
    pub fn from_lists(lists: &NameFilterLists, opts: MaskOptions) -> Result<NameFilter, MaskError> {
        let mut masks = Vec::new();
        let groups: [(&[String], bool, bool); 4] = [
            (&lists.include_files, false, false),
            (&lists.include_folders, false, true),
            (&lists.exclude_files, true, false),
            (&lists.exclude_folders, true, true),
        ];
        for (items, exclude, folder) in groups {
            for item in items.iter().flat_map(|item| item.split(';')) {
                let body = trim_item(item);
                if body.is_empty() {
                    continue;
                }
                let mut text = String::with_capacity(body.len() + 2);
                if exclude {
                    text.push('-');
                }
                text.push_str(body);
                if folder && !body.ends_with(['\\', '/']) {
                    text.push('\\');
                }
                masks.push(Mask::parse(&text, opts)?);
            }
        }
        Ok(sort_masks(masks, opts))
    }

    /// Back to the four lists (exclude items without their `-`; folder items keep their separator).
    ///
    /// The lists are lossless: `from_lists(&f.to_lists(), opts)` accepts exactly the items `f` accepts. An
    /// exclude item whose body starts with a space or tab (`- x` excludes the name ` x`, §2.2.1) is written with
    /// those blanks as one-character sets (`[ ]x`), because [`NameFilter::from_lists`] trims its items.
    pub fn to_lists(&self) -> NameFilterLists {
        let bare = |list: &MaskList| -> Vec<String> {
            list.masks()
                .iter()
                .map(|mask| {
                    let source = mask.source();
                    match mask.polarity() {
                        Polarity::Exclude => {
                            protect_leading_blanks(source.strip_prefix('-').unwrap_or(source))
                        }
                        Polarity::Include => source.to_owned(),
                    }
                })
                .collect()
        };
        let lists = &self.inner;
        NameFilterLists {
            include_files: bare(&lists.include_files),
            exclude_files: bare(&lists.exclude_files),
            include_folders: bare(&lists.include_folders),
            exclude_folders: bare(&lists.exclude_folders),
        }
    }

    /// Canonical toolbar string: include files, include folders, exclude files, exclude folders.
    pub fn to_filter_string(&self) -> String {
        let lists = &self.inner;
        let mut out = String::new();
        for list in [
            &lists.include_files,
            &lists.include_folders,
            &lists.exclude_files,
            &lists.exclude_folders,
        ] {
            for mask in list.masks() {
                if !out.is_empty() {
                    out.push(';');
                }
                out.push_str(mask.source());
            }
        }
        out
    }

    /// Verdict for the folder named `name` inside `parent_path`, given the parent folder's verdict (§2.3).
    /// Allocation-free; called once per listed folder entry and side.
    ///
    /// 1. parent `Excluded` → `Excluded`; 2. an exclude-folder mask matches → `Excluded` (exclusion always
    ///    wins); 3. no include-folder masks → `Included`; 4. parent `Included` → `Included` (subtree rule, D-09);
    ///    5. an include-folder mask matches → `Included`; 6. an include-folder mask is unanchored or may match
    ///    below this folder → `PassThrough`; 7. otherwise `Excluded`.
    pub fn folder(
        &self,
        parent_path: &RelPath,
        name: &str,
        parent: FolderVerdict,
    ) -> FolderVerdict {
        if parent == FolderVerdict::Excluded {
            return FolderVerdict::Excluded;
        }
        let lists = &self.inner;
        let parent_text = parent_path.as_str();
        let name = nfc(name);
        let name = Name::new(&name);
        if lists
            .exclude_folders
            .any_matches_prepared(parent_text, name)
        {
            return FolderVerdict::Excluded;
        }
        let includes = &lists.include_folders;
        if includes.is_empty() || parent == FolderVerdict::Included {
            return FolderVerdict::Included;
        }
        if includes.any_matches_prepared(parent_text, name) {
            return FolderVerdict::Included;
        }
        if includes
            .masks()
            .iter()
            .any(|m| !m.is_anchored() || m.may_match_below_prepared(parent_text, name))
        {
            return FolderVerdict::PassThrough;
        }
        FolderVerdict::Excluded
    }

    /// Whether the file named `name` inside `parent_path` is included, given the parent folder's verdict
    /// (§2.3). Allocation-free; called once per listed file entry and side.
    ///
    /// A file in an `Excluded` or `PassThrough` folder is excluded; an exclude-file mask excludes it; otherwise it
    /// is included when there are no include-file masks or one of them matches.
    pub fn file(&self, parent_path: &RelPath, name: &str, parent: FolderVerdict) -> bool {
        if matches!(parent, FolderVerdict::Excluded | FolderVerdict::PassThrough) {
            return false;
        }
        let lists = &self.inner;
        let parent_text = parent_path.as_str();
        let name = nfc(name);
        let name = Name::new(&name);
        if lists.exclude_files.any_matches_prepared(parent_text, name) {
            return false;
        }
        let includes = &lists.include_files;
        includes.is_empty() || includes.any_matches_prepared(parent_text, name)
    }

    /// True when the filter cannot exclude anything (fast path): no exclude masks, and each include list is
    /// empty or contains a mask that matches every name (`*`, `*.*`).
    pub fn is_match_all(&self) -> bool {
        let lists = &self.inner;
        let open = |list: &MaskList| list.is_empty() || list.masks().iter().any(Mask::is_universal);
        lists.exclude_files.is_empty()
            && lists.exclude_folders.is_empty()
            && open(&lists.include_files)
            && open(&lists.include_folders)
    }

    /// Options used.
    pub fn options(&self) -> MaskOptions {
        self.inner.opts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::CaseSensitivity;

    const S: MaskOptions = MaskOptions {
        case: CaseSensitivity::Sensitive,
    };

    #[test]
    fn toolbar_parsing_example() {
        let f = NameFilter::parse(r"*.c;*.h;-*.bak;-obj\;src\", S).unwrap();
        let lists = f.to_lists();
        assert_eq!(lists.include_files, ["*.c", "*.h"]);
        assert_eq!(lists.exclude_files, ["*.bak"]);
        assert_eq!(lists.exclude_folders, [r"obj\"]);
        assert_eq!(lists.include_folders, [r"src\"]);
        assert_eq!(f.to_filter_string(), r"*.c;*.h;src\;-*.bak;-obj\");
        let again = NameFilter::from_lists(&lists, S).unwrap();
        assert_eq!(again.to_lists(), lists);
        assert_eq!(
            NameFilter::parse(&f.to_filter_string(), S)
                .unwrap()
                .to_lists(),
            lists
        );
    }

    #[test]
    fn from_lists_places_items_by_syntax() {
        let lists = NameFilterLists {
            include_files: vec!["*.c;*.h".into(), "x\\".into(), "-y".into()],
            exclude_files: vec!["*.bak".into()],
            include_folders: vec!["src".into()],
            exclude_folders: vec!["obj/".into(), "-lit".into()],
        };
        let f = NameFilter::from_lists(&lists, S).unwrap();
        let out = f.to_lists();
        assert_eq!(out.include_files, ["*.c", "*.h"]);
        assert_eq!(out.include_folders, ["x\\", "src\\"]);
        assert_eq!(out.exclude_files, ["y", "*.bak"]);
        assert_eq!(out.exclude_folders, ["obj/", "-lit\\"]);
    }

    /// `- x` excludes the name ` x` (the body keeps its blank, §2.2.1); the lists must say the same, although
    /// `from_lists` trims its items (§3.3 sends `--filters` through the lists).
    #[test]
    fn lists_keep_leading_blanks_of_exclude_bodies() {
        let root = RelPath::root();
        let names = [
            "x.bak", " x.bak", "\tx.bak", "x", " x", "\tx", " ", "  \ty", "y", " \\",
        ];
        for toolbar in [
            "- *.bak",
            "-\t*.bak",
            "- x",
            "- \\",
            "-  \ty\\",
            "- x;-\tx;x",
        ] {
            let direct = NameFilter::parse(toolbar, S).unwrap();
            let lists = direct.to_lists();
            let via = NameFilter::from_lists(&lists, S).unwrap();
            assert_eq!(via.to_lists(), lists, "{toolbar:?}: the lists are stable");
            for name in names {
                for verdict in [FolderVerdict::Base, FolderVerdict::Included] {
                    assert_eq!(
                        direct.file(&root, name, verdict),
                        via.file(&root, name, verdict),
                        "{toolbar:?} file {name:?}"
                    );
                    assert_eq!(
                        direct.folder(&root, name, verdict),
                        via.folder(&root, name, verdict),
                        "{toolbar:?} folder {name:?}"
                    );
                }
            }
        }
        let lists = NameFilter::parse("- x;-\t*.bak;- \\", S)
            .unwrap()
            .to_lists();
        assert_eq!(lists.exclude_files, ["[ ]x", "[\t]*.bak"]);
        assert_eq!(lists.exclude_folders, ["[ ]\\"]);
        assert_eq!(protect_leading_blanks(" \t x"), "[ ][\t][ ]x");
        assert_eq!(protect_leading_blanks("x "), "x ");
    }

    #[test]
    fn match_all_detection() {
        assert!(NameFilter::match_all(S).is_match_all());
        assert!(NameFilter::parse("*;*\\", S).unwrap().is_match_all());
        assert!(NameFilter::parse("*.*", S).unwrap().is_match_all());
        assert!(!NameFilter::parse("*.c", S).unwrap().is_match_all());
        assert!(!NameFilter::parse("-x", S).unwrap().is_match_all());
        assert_eq!(NameFilter::match_all(S).options(), S);
    }
}
