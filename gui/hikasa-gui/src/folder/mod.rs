//! The folder views' toolkit-free parts (M2 W9a, D-233): columns and their layout memory, the CRC-on-demand schedule, the cell texts, the glyph
//! table and the centre column, the selection model. `folder_view.rs` holds the view model that uses them; `ui/`
//! draws them.

pub mod cells;
pub mod columns;
pub mod crc;
pub mod glyphs;
pub mod selection;
