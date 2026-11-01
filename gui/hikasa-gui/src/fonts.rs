//! The theme's named fonts (M2 D-160, revises D-113 and D-133): which families the four slots of `theme.toml` name,
//! where their files come from, and when the window applies them. No toolkit type: `ui::fonts` turns the result into
//! egui's font definitions, so the whole flow runs headless with an injected [`FontSource`].
//!
//! A lookup enumerates the system's fonts, so it runs on a thread of its own and never on the UI thread (D-105),
//! one at a time. It runs only for a family a slot names: a theme of built-in fonts costs nothing. The enumeration is
//! dropped with the lookup; what stays is the mapped file of each family in use, and a family no slot names any more
//! is forgotten, so the memory follows the theme.

use std::collections::BTreeMap;
use std::sync::Arc;

use fontique::{
    Blob, Collection, CollectionOptions, FontStyle, FontWeight, FontWidth, SourceCache,
    SourceCacheOptions,
};
use hikasa_session::settings::Fonts;

use crate::options::theme_page::FontSlot;

/// The bytes of a font file: mapped, shared and never copied.
pub type FontBytes = Arc<dyn AsRef<[u8]> + Send + Sync>;

/// The regular face of a family.
#[derive(Clone)]
pub struct LoadedFont {
    /// The file.
    pub bytes: FontBytes,
    /// The face in the file.
    pub index: u32,
}

impl std::fmt::Debug for LoadedFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedFont")
            .field("bytes", &(*self.bytes).as_ref().len())
            .field("index", &self.index)
            .finish()
    }
}

/// Where fonts come from. Called off the UI thread.
pub trait FontSource: Send + Sync {
    /// One answer per family, in order; `None` when the family is not found.
    fn load(&self, families: &[String]) -> Vec<Option<LoadedFont>>;
}

/// The fonts installed on this system (fontique: fontconfig, DirectWrite or Core Text). A family name is matched as
/// the system knows it, whatever its case; the face closest to regular is used.
pub struct SystemFonts;

impl FontSource for SystemFonts {
    fn load(&self, families: &[String]) -> Vec<Option<LoadedFont>> {
        lookup(
            Collection::new(CollectionOptions {
                shared: false,
                system_fonts: true,
            }),
            families,
        )
    }
}

/// The units per em of face `index` of the font file `data`, from its `head` table. `None` when there is no such table.
fn units_per_em(data: &[u8], index: u32) -> Option<u16> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let face = if data.get(..4)? == b"ttcf" {
        u32_at(12 + 4 * index as usize)? as usize
    } else {
        0
    };
    let tables = u16_at(face.checked_add(4)?)?;
    (0..usize::from(tables)).find_map(|table| {
        let record = face + 12 + 16 * table;
        (data.get(record..record + 4)? == b"head")
            .then(|| u32_at(record + 8))
            .flatten()
            .and_then(|head| u16_at(head as usize + 18))
    })
}

/// The regular face of each family in `collection`. A file that cannot be read or parsed is a family not found, and so
/// is a font of no size: every length in a font is a share of its units per em, and the window divides by them.
fn lookup(mut collection: Collection, families: &[String]) -> Vec<Option<LoadedFont>> {
    let mut cache = SourceCache::new(SourceCacheOptions::default());
    families
        .iter()
        .map(|family| {
            let info = collection.family_by_name(family)?;
            let font = info
                .match_font(
                    FontWidth::NORMAL,
                    FontStyle::Normal,
                    FontWeight::NORMAL,
                    false,
                )
                .or_else(|| info.default_font())?;
            fontique::FontInfo::from_source(font.source().clone(), font.index())?;
            let blob: Blob<u8> = font.load(Some(&mut cache))?;
            let bytes = blob.into_raw_parts().0;
            units_per_em((*bytes).as_ref(), font.index()).filter(|&units| units > 0)?;
            Some(LoadedFont {
                bytes,
                index: font.index(),
            })
        })
        .collect()
}

/// The families of the four slots of a theme, `""` for the built-in font. `use_system` puts the folder views on the
/// built-in font, so their family is not looked up.
#[must_use]
pub fn named_families(fonts: &Fonts) -> [String; FontSlot::ALL.len()] {
    let family = |slot: FontSlot| match slot {
        FontSlot::Editor => fonts.editor.family.clone(),
        FontSlot::Listing => fonts.listing.family.clone(),
        FontSlot::Hex => fonts.hex.family.clone(),
        FontSlot::Folder if fonts.folder.use_system => String::new(),
        FontSlot::Folder => fonts.folder.family.clone(),
    };
    FontSlot::ALL.map(family)
}

/// What the core does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontStep {
    /// Nothing.
    Stay,
    /// Look these families up.
    Load(Vec<String>),
    /// The window's fonts changed: apply them once.
    Apply,
}

/// The fonts of the theme on its way to the window: one lookup at a time, the newest theme wins, one apply when what
/// the slots show changes (and none for a colour, or for a family that is not found).
pub struct FontLoader {
    source: Arc<dyn FontSource>,
    /// What the slots name.
    wanted: [String; FontSlot::ALL.len()],
    /// The answers for the families the slots name, and nothing else.
    known: BTreeMap<String, Option<LoadedFont>>,
    /// A lookup is on its way.
    running: bool,
    /// The family each slot shows in the window.
    applied: [String; FontSlot::ALL.len()],
    /// The families already reported as not found: one notice per family.
    reported: Vec<String>,
}

impl FontLoader {
    /// Every slot on the built-in font.
    pub fn new(source: Arc<dyn FontSource>) -> FontLoader {
        FontLoader {
            source,
            wanted: Default::default(),
            known: BTreeMap::new(),
            running: false,
            applied: Default::default(),
            reported: Vec::new(),
        }
    }

    /// The source, for the lookup's thread.
    pub fn source(&self) -> Arc<dyn FontSource> {
        Arc::clone(&self.source)
    }

    /// True while a lookup is on its way.
    pub fn running(&self) -> bool {
        self.running
    }

    /// The slots now name `families`.
    pub fn want(&mut self, families: [String; FontSlot::ALL.len()]) -> FontStep {
        if families == self.wanted {
            return FontStep::Stay;
        }
        self.wanted = families;
        self.known.retain(|family, _| self.wanted.contains(family));
        self.next()
    }

    /// A lookup ended with `found`, one answer per family of `asked`; answers for what no slot names any more go.
    pub fn finish(&mut self, asked: Vec<String>, found: Vec<Option<LoadedFont>>) -> FontStep {
        self.running = false;
        for (family, font) in asked.into_iter().zip(found) {
            if self.wanted.contains(&family) {
                self.known.insert(family, font);
            }
        }
        self.next()
    }

    fn next(&mut self) -> FontStep {
        let mut missing: Vec<String> = Vec::new();
        for family in &self.wanted {
            if !family.is_empty() && !self.known.contains_key(family) && !missing.contains(family) {
                missing.push(family.clone());
            }
        }
        if !missing.is_empty() {
            if self.running {
                return FontStep::Stay;
            }
            self.running = true;
            return FontStep::Load(missing);
        }
        if self.running {
            return FontStep::Stay;
        }
        let shown = self.shown();
        if shown == self.applied {
            FontStep::Stay
        } else {
            self.applied = shown;
            FontStep::Apply
        }
    }

    /// The family each slot would show: a family found, else `""`.
    fn shown(&self) -> [String; FontSlot::ALL.len()] {
        self.wanted.clone().map(|family| {
            if matches!(self.known.get(&family), Some(Some(_))) {
                family
            } else {
                String::new()
            }
        })
    }

    /// The families not found that were not reported yet; each is reported once.
    pub fn take_not_found(&mut self) -> Vec<String> {
        let mut report = Vec::new();
        for family in &self.wanted {
            if matches!(self.known.get(family), Some(None)) && !self.reported.contains(family) {
                self.reported.push(family.clone());
                report.push(family.clone());
            }
        }
        report
    }

    /// The family a slot shows and its font; `None` for the built-in font.
    pub fn font(&self, slot: FontSlot) -> Option<(&str, &LoadedFont)> {
        let family = &self.applied[slot as usize];
        let font = self.known.get(family)?.as_ref()?;
        Some((family, font))
    }

    /// The number of families held in memory.
    #[cfg(test)]
    pub fn held(&self) -> usize {
        self.known.len()
    }
}

/// A source with no fonts: every family is missing and nothing touches the system.
#[cfg(test)]
pub struct NoFonts;

#[cfg(test)]
impl FontSource for NoFonts {
    fn load(&self, families: &[String]) -> Vec<Option<LoadedFont>> {
        vec![None; families.len()]
    }
}

/// A source over font data given by the test, through the same lookup as the system's, and how often it was asked.
#[cfg(test)]
pub struct DataFonts {
    data: Vec<Vec<u8>>,
    /// The families asked for, per lookup.
    pub asked: parking_lot::Mutex<Vec<Vec<String>>>,
}

#[cfg(test)]
impl DataFonts {
    /// Fonts made of `data` (the families are the names inside the data).
    pub fn new(data: Vec<Vec<u8>>) -> DataFonts {
        DataFonts {
            data,
            asked: parking_lot::Mutex::new(Vec::new()),
        }
    }
}

#[cfg(test)]
impl FontSource for DataFonts {
    fn load(&self, families: &[String]) -> Vec<Option<LoadedFont>> {
        self.asked.lock().push(families.to_vec());
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        });
        for data in &self.data {
            collection.register_fonts(Blob::new(Arc::new(data.clone())), None);
        }
        lookup(collection, families)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A font the test harness has without the system's help: the one egui bundles, whose family is `Hack`.
    pub(crate) fn bundled() -> Vec<u8> {
        egui::FontDefinitions::default().font_data["Hack"]
            .bytes()
            .to_vec()
    }

    fn slots(editor: &str, listing: &str, hex: &str, folder: &str) -> [String; 4] {
        [editor, listing, hex, folder].map(str::to_owned)
    }

    fn font() -> LoadedFont {
        LoadedFont {
            bytes: Arc::new(vec![1u8, 2, 3]),
            index: 0,
        }
    }

    fn loader() -> FontLoader {
        FontLoader::new(Arc::new(NoFonts))
    }

    #[test]
    fn the_lookup_finds_a_family_in_any_case_and_misses_the_others() {
        let source = DataFonts::new(vec![bundled()]);
        let found = source.load(&["Hack".into(), "hack".into(), "Nowhere Mono".into()]);
        assert_eq!(found.len(), 3);
        let first = found[0].as_ref().expect("the family is found");
        assert_eq!((*first.bytes).as_ref(), bundled().as_slice());
        assert!(found[1].is_some(), "family names ignore case");
        assert!(found[2].is_none());
    }

    /// `bundled()` with its units per em set to `units`.
    pub(crate) fn with_units(units: u16) -> Vec<u8> {
        let mut data = bundled();
        let tables = usize::from(u16::from_be_bytes([data[4], data[5]]));
        let head = (0..tables)
            .map(|table| 12 + 16 * table)
            .find(|&record| &data[record..record + 4] == b"head")
            .expect("a head table");
        let at = u32::from_be_bytes(data[head + 8..head + 12].try_into().unwrap()) as usize;
        data[at + 18..at + 20].copy_from_slice(&units.to_be_bytes());
        data
    }

    #[test]
    fn a_font_of_no_size_is_no_family() {
        assert_eq!(units_per_em(&bundled(), 0), Some(2048));
        let source = DataFonts::new(vec![with_units(0)]);
        assert!(source.load(&["Hack".into()])[0].is_none());
    }

    #[test]
    fn data_that_is_no_font_is_no_family() {
        let source = DataFonts::new(vec![b"not a font".to_vec()]);
        assert!(source.load(&["Hack".into()])[0].is_none());
    }

    /// The cost of a lookup on this machine (docs/02): a measurement, so ignored; it reads the system's font list and
    /// nothing else.
    #[test]
    #[ignore = "reads the machine's fonts: run in release with --ignored --nocapture"]
    fn measure_a_system_lookup() {
        let family =
            std::env::var("HIKASA_PERF_FONT").unwrap_or_else(|_| "DejaVu Sans Mono".into());
        for run in 0..5 {
            let began = std::time::Instant::now();
            let found = SystemFonts.load(std::slice::from_ref(&family));
            let len = found[0].as_ref().map(|f| (*f.bytes).as_ref().len());
            eprintln!(
                "run {run}: {family}: {len:?} bytes in {:?}",
                began.elapsed()
            );
            assert!(len.is_some());
        }
    }

    #[test]
    fn built_in_fonts_never_look_anything_up() {
        let mut fonts = loader();
        assert_eq!(fonts.want(Default::default()), FontStep::Stay);
        assert!(!fonts.running());
    }

    #[test]
    fn a_family_of_two_slots_is_looked_up_once_and_applied_once() {
        let mut fonts = loader();
        let wanted = slots("A", "", "A", "");
        let step = fonts.want(wanted.clone());
        assert_eq!(step, FontStep::Load(vec!["A".into()]));
        assert!(fonts.running());
        assert_eq!(fonts.want(wanted), FontStep::Stay, "the same theme again");
        let step = fonts.finish(vec!["A".into()], vec![Some(font())]);
        assert_eq!(step, FontStep::Apply);
        assert!(fonts.font(FontSlot::Editor).is_some());
        assert!(fonts.font(FontSlot::Listing).is_none());
        assert!(fonts.font(FontSlot::Hex).is_some());
        assert_eq!(fonts.held(), 1);
    }

    #[test]
    fn a_family_not_found_is_reported_once_and_changes_nothing_on_screen() {
        let mut fonts = loader();
        fonts.want(slots("Gone", "", "Gone", ""));
        let step = fonts.finish(vec!["Gone".into()], vec![None]);
        assert_eq!(step, FontStep::Stay, "the built-in font is already shown");
        assert_eq!(fonts.take_not_found(), vec!["Gone".to_owned()]);
        assert!(fonts.take_not_found().is_empty());
        // Named again later: looked up again, not reported again.
        fonts.want(Default::default());
        fonts.want(slots("Gone", "", "", ""));
        fonts.finish(vec!["Gone".into()], vec![None]);
        assert!(fonts.take_not_found().is_empty());
    }

    #[test]
    fn the_newest_theme_wins_and_only_it_is_applied() {
        let mut fonts = loader();
        fonts.want(slots("A", "", "", ""));
        assert_eq!(
            fonts.want(slots("B", "", "", "")),
            FontStep::Stay,
            "one lookup at a time"
        );
        let step = fonts.finish(vec!["A".into()], vec![Some(font())]);
        assert_eq!(step, FontStep::Load(vec!["B".into()]));
        assert_eq!(fonts.held(), 0, "an answer no slot wants is dropped");
        let step = fonts.finish(vec!["B".into()], vec![Some(font())]);
        assert_eq!(step, FontStep::Apply);
        assert_eq!(fonts.font(FontSlot::Editor).map(|f| f.0), Some("B"));
    }

    #[test]
    fn a_family_no_slot_names_is_forgotten() {
        let mut fonts = loader();
        fonts.want(slots("A", "B", "", ""));
        fonts.finish(
            vec!["A".into(), "B".into()],
            vec![Some(font()), Some(font())],
        );
        assert_eq!(fonts.held(), 2);
        assert_eq!(
            fonts.want(slots("A", "", "", "")),
            FontStep::Apply,
            "no lookup needed"
        );
        assert_eq!(fonts.held(), 1);
        assert_eq!(fonts.want(Default::default()), FontStep::Apply);
        assert_eq!(fonts.held(), 0);
        assert!(fonts.font(FontSlot::Editor).is_none());
    }

    #[test]
    fn the_folder_family_waits_while_the_folder_uses_the_system_font() {
        let mut theme = Fonts::default();
        theme.folder.family = "A".into();
        theme.folder.use_system = true;
        assert_eq!(named_families(&theme)[FontSlot::Folder as usize], "");
        theme.folder.use_system = false;
        assert_eq!(named_families(&theme)[FontSlot::Folder as usize], "A");
    }
}
