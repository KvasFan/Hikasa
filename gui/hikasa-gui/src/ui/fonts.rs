//! The theme's fonts in egui (D-160). Each slot of `theme.toml` is one named family: the slot's font first, then the
//! built-in chain of monospace or proportional, so a glyph the named font lacks falls back as before: icons and emoji
//! to the bundled fonts, CJK and the rest to eframe's system font fallback, which egui asks after a family's chain.
//! `ui::theme` names the colours; this names the fonts.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

use crate::fonts::{FontLoader, LoadedFont};
use crate::options::theme_page::FontSlot;

/// The egui family of a slot.
fn slot_family(slot: FontSlot) -> FontFamily {
    FontFamily::Name(Arc::from(match slot {
        FontSlot::Editor => "hikasa-editor",
        FontSlot::Listing => "hikasa-listing",
        FontSlot::Hex => "hikasa-hex",
        FontSlot::Folder => "hikasa-folder",
    }))
}

/// The family of a slot while it shows the built-in font.
fn built_in(slot: FontSlot) -> FontFamily {
    match slot {
        FontSlot::Editor | FontSlot::Listing | FontSlot::Hex => FontFamily::Monospace,
        FontSlot::Folder => FontFamily::Proportional,
    }
}

/// egui's definitions for what `loader` holds. Every slot's family is bound, to the built-in chain when it names no
/// font, so that a family drawn with in the frame that replaces them is never unbound. The font files are shared.
#[must_use]
pub fn definitions(loader: &FontLoader) -> FontDefinitions {
    let mut definitions = FontDefinitions::default();
    for slot in FontSlot::ALL {
        let mut chain = definitions.families[&built_in(slot)].clone();
        if let Some((family, font)) = loader.font(slot) {
            let name = format!("theme:{family}");
            definitions
                .font_data
                .entry(name.clone())
                .or_insert_with(|| {
                    Arc::new(FontData::from_blob(Arc::clone(&font.bytes), font.index))
                });
            chain.insert(0, name);
        }
        definitions.families.insert(slot_family(slot), chain);
    }
    definitions
}

/// True when `font` draws: a few glyphs of its own in a throwaway egui context, as the window would draw them. egui
/// parses a font, and panics over one that it cannot take, on the thread that draws with it. So this runs on the fonts'
/// thread under the panic boundary, and a font that fails stays away from the window. Glyphs not drawn here can still
/// fail in the window: a file can be broken at any glyph, and only its first use tells (D-160).
#[must_use]
pub fn draws(font: &LoadedFont) -> bool {
    const SAMPLE: &str =
        "The quick brown fox jumps 0123456789 {}[]()<>/\\|_-+=*&%$#@!?;:'\",.~^ ÄÖüßéñ€…→ fi fl";
    let name = "sample".to_owned();
    let family = FontFamily::Name(Arc::from("sample"));
    let mut definitions = FontDefinitions::empty();
    definitions.font_data.insert(
        name.clone(),
        Arc::new(FontData::from_blob(Arc::clone(&font.bytes), font.index)),
    );
    definitions
        .families
        .insert(family.clone(), vec![name.clone()]);
    definitions
        .families
        .insert(FontFamily::Proportional, vec![name.clone()]);
    definitions
        .families
        .insert(FontFamily::Monospace, vec![name]);
    let ctx = egui::Context::default();
    ctx.set_fonts(definitions);
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
    out.textures_delta.clear();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        for size in [9.0, 14.0, 21.5] {
            let id = egui::FontId::new(size, family.clone());
            ui.painter().text(
                egui::pos2(1.0, 1.0),
                egui::Align2::LEFT_TOP,
                SAMPLE,
                id.clone(),
                egui::Color32::WHITE,
            );
            ui.label(egui::RichText::new(SAMPLE).font(id));
        }
    });
    out.textures_delta.clear();
    true
}

/// The family each slot draws with now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlotFonts {
    /// The slots with a font of their own.
    named: [bool; FontSlot::ALL.len()],
}

impl SlotFonts {
    /// What `loader` holds.
    #[must_use]
    pub fn of(loader: &FontLoader) -> SlotFonts {
        SlotFonts {
            named: FontSlot::ALL.map(|slot| loader.font(slot).is_some()),
        }
    }

    /// The family to draw `slot` with.
    #[must_use]
    pub fn family(self, slot: FontSlot) -> FontFamily {
        if self.named[slot as usize] {
            slot_family(slot)
        } else {
            built_in(slot)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use egui::epaint::text::{FallbackRequest, FontInsert};
    use egui::{FontId, RawInput};

    use super::*;
    use crate::fonts::{FontSource as _, FontStep, LoadedFont};

    /// A loader whose editor and folder slots show the bundled `Hack`.
    fn with_hack() -> FontLoader {
        let bytes = FontDefinitions::default().font_data["Hack"]
            .bytes()
            .to_vec();
        let mut loader = FontLoader::new(Arc::new(crate::fonts::NoFonts));
        let family = "Hack".to_owned();
        loader.want([family.clone(), String::new(), String::new(), family.clone()]);
        let font = LoadedFont {
            bytes: Arc::new(bytes),
            index: 0,
        };
        assert_eq!(
            loader.finish(vec![family], vec![Some(font)]),
            FontStep::Apply
        );
        loader
    }

    fn pass(ctx: &egui::Context) {
        let mut out = ctx.run_ui(RawInput::default(), |_| {});
        out.textures_delta.clear();
    }

    /// The fonts of a broken disk: truncated, with bytes changed, with a table's offset or length out of reach. A font
    /// the lookup takes and `draws` passes then draws in a window's definitions; one that fails does so inside the
    /// boundary. A panic in this test is a font that got past both.
    #[test]
    fn a_mutated_font_stops_at_the_lookup_or_at_the_throwaway_drawing() {
        let base = crate::fonts::tests::bundled();
        let directory_end = 12 + 16 * usize::from(u16::from_be_bytes([base[4], base[5]]));
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut random = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state >> 16).unwrap()
        };
        let mut drawn = 0;
        for round in 0..60 {
            let mut data = base.clone();
            match round % 4 {
                0 => data.truncate((random() % data.len()).max(directory_end)),
                1 => {
                    for _ in 0..200 {
                        let at = random() % data.len();
                        data[at] = random() as u8;
                    }
                }
                2 => {
                    for _ in 0..8 {
                        let at = 12 + random() % (directory_end - 12);
                        data[at] = random() as u8;
                    }
                }
                _ => {
                    let table = 12 + 16 * (random() % ((directory_end - 12) / 16));
                    data[table + 8 + random() % 8] = 0xff;
                }
            }
            let source = crate::fonts::DataFonts::new(vec![data]);
            let family = "Hack".to_owned();
            let found = source.load(std::slice::from_ref(&family)).remove(0);
            let Some(font) = found else { continue };
            if !hikasa_session::run_guarded(|| Ok(draws(&font))).unwrap_or(false) {
                continue;
            }
            drawn += 1;
            let mut loader = FontLoader::new(Arc::new(crate::fonts::NoFonts));
            loader.want([family.clone(), String::new(), String::new(), String::new()]);
            loader.finish(vec![family], vec![Some(font)]);
            let ctx = egui::Context::default();
            ctx.set_fonts(definitions(&loader));
            pass(&ctx);
            let mut out = ctx.run_ui(RawInput::default(), |ui| {
                let id = FontId::new(14.0, slot_family(FontSlot::Editor));
                ui.label(egui::RichText::new("The quick brown fox").font(id));
            });
            out.textures_delta.clear();
        }
        assert!(drawn > 0, "some of them are still fonts");
    }

    #[test]
    fn the_bundled_font_draws() {
        let font = LoadedFont {
            bytes: Arc::new(crate::fonts::tests::bundled()),
            index: 0,
        };
        assert!(draws(&font));
    }

    #[test]
    fn a_slot_draws_with_its_font_and_keeps_the_built_in_fallback() {
        let loader = with_hack();
        let definitions = definitions(&loader);
        let folder = &definitions.families[&slot_family(FontSlot::Folder)];
        assert_eq!(folder[0], "theme:Hack");
        assert_eq!(
            folder[1..],
            definitions.families[&FontFamily::Proportional][..],
            "the whole built-in chain follows"
        );
        assert_eq!(
            definitions.families[&slot_family(FontSlot::Hex)],
            definitions.families[&FontFamily::Monospace],
            "a slot without a font is the built-in chain"
        );
        let shared = definitions
            .font_data
            .keys()
            .filter(|n| n.starts_with("theme:"));
        assert_eq!(shared.count(), 1, "two slots share one entry");

        let fonts = SlotFonts::of(&loader);
        assert_eq!(
            fonts.family(FontSlot::Editor),
            slot_family(FontSlot::Editor)
        );
        assert_eq!(fonts.family(FontSlot::Hex), FontFamily::Monospace);
        assert_eq!(
            SlotFonts::default().family(FontSlot::Folder),
            FontFamily::Proportional
        );

        // In egui: the folder's text now has Hack's metrics, where the built-in proportional font differs.
        let ctx = egui::Context::default();
        ctx.set_fonts(definitions);
        pass(&ctx);
        let (named, built_in, mono, icon) = ctx.fonts_mut(|f| {
            let named = FontId::new(14.0, fonts.family(FontSlot::Folder));
            (
                f.glyph_width(&named, 'i'),
                f.glyph_width(&FontId::proportional(14.0), 'i'),
                f.glyph_width(&FontId::monospace(14.0), 'i'),
                f.has_glyph(&named, egui::special_emojis::GITHUB),
            )
        });
        assert_eq!(named, mono);
        assert_ne!(named, built_in);
        assert!(
            icon,
            "a glyph the named font lacks comes from the built-in chain"
        );
    }

    /// A character none of the fonts has reaches the system fallback with the slot's family, as with a built-in one:
    /// this is the path of CJK.
    #[test]
    fn a_glyph_nobody_has_reaches_the_system_fallback() {
        let asked = Arc::new(AtomicBool::new(false));
        let provider = {
            let asked = Arc::clone(&asked);
            move |request: &FallbackRequest<'_>| -> Option<FontInsert> {
                if request.cluster.starts_with('日')
                    && *request.family == slot_family(FontSlot::Editor)
                {
                    asked.store(true, Ordering::Relaxed);
                }
                None
            }
        };
        let ctx = egui::Context::default();
        ctx.add_font_provider(Arc::new(provider));
        let loader = with_hack();
        ctx.set_fonts(definitions(&loader));
        pass(&ctx);
        let family = SlotFonts::of(&loader).family(FontSlot::Editor);
        ctx.fonts_mut(|f| f.has_glyph(&FontId::new(14.0, family), '日'));
        assert!(asked.load(Ordering::Relaxed));
    }
}
