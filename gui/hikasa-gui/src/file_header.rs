//! The File Info header of the hex (and picture) panes (M2 W13a, contract §1.13.8, D-360), toolkit-free: per pane the
//! path field (editable; Enter reopens that side; *Browse…*; drops land here too), the format picker (*Auto* or an
//! enabled format of the view's kind), and the facts of the opened side — size in bytes, modification time, how the
//! bytes are read, *Converted* — all from core (`HexSide`, `HexDocument::info`).

use hikasa_hex::HexLoad;
use hikasa_report::HexSide;
use hikasa_session::api::human::TimeZone;

/// The facts a header shows of an opened side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderFacts {
    /// The format's name (`None`: *Auto*, the raw bytes).
    pub format: Option<String>,
    /// The size (`12,345 bytes`).
    pub size: String,
    /// The modification time in the clock's zone (empty when unknown).
    pub modified: String,
    /// How the bytes are read.
    pub load: &'static str,
    /// A Hex format's load conversion produced the bytes.
    pub converted: bool,
}

/// The words of a load (HX-002).
pub fn load_text(load: HexLoad) -> &'static str {
    match load {
        HexLoad::Memory => "in memory",
        HexLoad::File => "kept open",
        HexLoad::TempCache => "from the temp cache",
    }
}

/// A count of bytes with thousands separators: `12,345 bytes`.
pub fn bytes_text(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 6);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out.push_str(if n == 1 { " byte" } else { " bytes" });
    out
}

/// The facts of an opened hex side.
pub fn hex_facts(side: &HexSide, zone: &TimeZone) -> HeaderFacts {
    HeaderFacts {
        format: side.format.as_ref().map(|f| f.name.clone()),
        size: bytes_text(side.size),
        modified: side
            .mtime
            .map(|t| {
                t.to_zoned(zone.clone())
                    .strftime("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_default(),
        load: load_text(side.load),
        converted: side.converted,
    }
}

impl HeaderFacts {
    /// The header's detail line: `12,345 bytes · 2026-11-18 10:00:00 · kept open · Converted`.
    pub fn line(&self) -> String {
        let mut parts = vec![self.size.clone()];
        if !self.modified.is_empty() {
            parts.push(self.modified.clone());
        }
        parts.push(self.load.to_owned());
        if self.converted {
            parts.push("Converted".to_owned());
        }
        parts.join(" · ")
    }
}
