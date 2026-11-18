//! The Hex Compare view's typed calls (W13a, contract §1.13.20; D-355…D-360): [`Service::hex_address`] (Go To),
//! [`Service::hex_finder`] with [`HexFindQuery`] (the Find panel), [`Service::hex_response`] (the verdict, Info and
//! reports of a landed alignment), [`Service::hex_raw_same`] (the raw comparison of a converted pair), and the view
//! state keys of a hex session's `[view]` ([`HexViewState`], D-370). Every word is parsed by core's parsers here
//! (check 7): the GUI passes the words as typed.

use std::sync::Arc;

use hikasa_core::{Pair, Side, Warning};
use hikasa_hex::nav::AddressQuery;
use hikasa_hex::{
    DisplayFilter, HexAlignOptions, HexAlignment, HexFinder, HexLayout, HexPattern, Radix,
    TextEncoding,
};
use hikasa_vfs::{VfsSource, compare_sources};
use serde::{Deserialize, Serialize};

use super::hex::{HexCompareRequest, HexCompareResponse, HexOpened, hex_warning};
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

/// The Find panel's query (D-359): the pattern as typed, hex bytes or text, the text's encoding word and its case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HexFindQuery {
    /// The pattern as typed.
    pub pattern: String,
    /// Hex bytes (`DE AD ?? EF`) instead of text.
    pub hex: bool,
    /// The text's encoding word (`utf-8`, `latin1`, `utf-16le`, `utf-16be`; `None`: UTF-8). Ignored for hex bytes.
    pub encoding: Option<String>,
    /// Match the case of ASCII letters (text only).
    pub match_case: bool,
}

/// What a hex session's `[view]` keeps beyond the layout words (D-370, W13a; view state, never dirty): the address
/// column, the thumbnail and the value panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexViewState {
    /// `addresses`: the address column is shown (default true).
    pub addresses: bool,
    /// `thumbnail`: the thumbnail strip is shown (default true).
    pub thumbnail: bool,
    /// `values`: the value panel's blocks (`none`, `little`, `big`, `both`; default `none`).
    pub values: HexValues,
    /// `values_radix`: the value panel's integers (`decimal`, `hex`; default decimal).
    pub values_radix: Radix,
}

impl Default for HexViewState {
    fn default() -> HexViewState {
        HexViewState {
            addresses: true,
            thumbnail: true,
            values: HexValues::None,
            values_radix: Radix::Decimal,
        }
    }
}

/// The value panel's blocks (`[view] values`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HexValues {
    /// The panel is hidden.
    None,
    /// Little endian only.
    Little,
    /// Big endian only.
    Big,
    /// Both.
    Both,
}

impl HexValues {
    /// Every choice.
    pub const ALL: [HexValues; 4] = [
        HexValues::None,
        HexValues::Little,
        HexValues::Big,
        HexValues::Both,
    ];

    /// The `[view] values` word.
    pub const fn word(self) -> &'static str {
        match self {
            HexValues::None => "none",
            HexValues::Little => "little",
            HexValues::Big => "big",
            HexValues::Both => "both",
        }
    }

    /// Parses a `[view] values` word.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for another word.
    pub fn parse(word: &str) -> Result<HexValues, ApiError> {
        HexValues::ALL
            .into_iter()
            .find(|v| v.word().eq_ignore_ascii_case(word.trim()))
            .ok_or_else(|| {
                ApiError::InvalidArgument(format!(
                    "invalid view.values {word:?}: expected none, little, big, both"
                ))
            })
    }

    /// The little endian block is shown.
    pub const fn little(self) -> bool {
        matches!(self, HexValues::Little | HexValues::Both)
    }

    /// The big endian block is shown.
    pub const fn big(self) -> bool {
        matches!(self, HexValues::Big | HexValues::Both)
    }

    /// The choice with the little (`little`) or big block toggled.
    pub const fn toggled(self, little: bool) -> HexValues {
        let (l, b) = (self.little(), self.big());
        let (l, b) = if little { (!l, b) } else { (l, !b) };
        match (l, b) {
            (false, false) => HexValues::None,
            (true, false) => HexValues::Little,
            (false, true) => HexValues::Big,
            (true, true) => HexValues::Both,
        }
    }
}

/// The view state of a hex session's `[view]` (D-370): the keys `addresses`, `thumbnail`, `values` and
/// `values_radix`; an absent key is its default, a pre-W13 session has none.
///
/// # Errors
///
/// `InvalidArgument` for a value of the wrong type or word (`session edit --set` checks them).
pub fn hex_view_state_of(view: &toml::Table) -> Result<HexViewState, ApiError> {
    let mut state = HexViewState::default();
    let flag = |key: &str| -> Result<Option<bool>, ApiError> {
        match view.get(key) {
            None => Ok(None),
            Some(toml::Value::Boolean(b)) => Ok(Some(*b)),
            Some(other) => Err(ApiError::InvalidArgument(format!(
                "invalid view.{key} {other}: expected true or false"
            ))),
        }
    };
    if let Some(b) = flag("addresses")? {
        state.addresses = b;
    }
    if let Some(b) = flag("thumbnail")? {
        state.thumbnail = b;
    }
    if let Some(v) = super::hex::view_word(view, "values") {
        state.values = HexValues::parse(&v)?;
    }
    if let Some(v) = super::hex::view_word(view, "values_radix") {
        state.values_radix = Radix::parse(&v)
            .map_err(|e| ApiError::InvalidArgument(format!("invalid view.values_radix: {e}")))?;
    }
    Ok(state)
}

impl Service {
    /// Go To's address (D-358): `parse_address` of the dialog's text with its default radix (every spelling of D-333:
    /// `0x1F`, `1Fh`, `$1F`, `#31`, `31d`, digits, `+n`, `-n`, `end`).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` with the core's message for a text it refuses.
    pub fn hex_address(&self, text: &str, radix: Radix) -> Result<AddressQuery, ApiError> {
        hikasa_hex::nav::parse_address(text, radix)
            .map_err(|e| ApiError::InvalidArgument(e.to_string()))
    }

    /// The Find panel's finder (D-359): hex bytes by `parse_hex_bytes` (encoding and case ignored), text in the
    /// encoding of `TextEncoding::parse`; compiled by `HexFinder::new`.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` with the core's message: malformed hex, an unknown encoding, an empty pattern, one over 4,096
    /// bytes, all wildcards, text the encoding cannot write.
    pub fn hex_finder(&self, query: &HexFindQuery) -> Result<HexFinder, ApiError> {
        let invalid = |e: hikasa_hex::VocabError| ApiError::InvalidArgument(e.to_string());
        let pattern = if query.hex {
            HexPattern::Bytes {
                bytes: hikasa_hex::find::parse_hex_bytes(&query.pattern).map_err(invalid)?,
            }
        } else {
            HexPattern::Text {
                text: query.pattern.clone(),
                encoding: match &query.encoding {
                    Some(word) => TextEncoding::parse(word).map_err(invalid)?,
                    None => TextEncoding::Utf8,
                },
                match_case: query.match_case,
            }
        };
        HexFinder::new(&pattern).map_err(invalid)
    }

    /// The response of a landed alignment as `hex compare` builds it (D-331; Info, the outcome, the reports): the
    /// verdict of the alignment (a converted pair: `converted_verdict` with the raw inputs taken as different until
    /// [`HexCompareResponse::with_raw_same`] says otherwise), the layout, the filter, the alignment's warnings; the
    /// options of `request` (else the defaults with the landed mode). Pure: nothing is read.
    pub fn hex_response(
        &self,
        opened: &HexOpened,
        alignment: &Arc<HexAlignment>,
        layout: HexLayout,
        filter: DisplayFilter,
    ) -> HexCompareResponse {
        let verdict = if opened.converted() {
            hikasa_hex::converted_verdict(false, alignment)
        } else {
            hikasa_hex::verdict(alignment)
        };
        HexCompareResponse {
            opened: opened.clone(),
            alignment: Arc::clone(alignment),
            options: HexAlignOptions::with_mode(alignment.mode),
            kind: verdict.kind,
            exit_code: verdict.exit_code,
            layout,
            display: Some(filter),
            context: 0,
            max_sections: HexCompareRequest::DEFAULT_MAX_SECTIONS,
            preview: HexCompareRequest::DEFAULT_PREVIEW,
            stale: false,
            warnings: alignment.warnings.iter().map(hex_warning).collect(),
        }
    }

    /// The raw comparison of a converted pair's inputs (D-331's first conversion row: raw bytes identical is
    /// `binary_same`), `None` when no side was converted. Reads both inputs: run it in a job.
    ///
    /// # Errors
    ///
    /// A read error, cancellation.
    pub fn hex_raw_same(
        &self,
        opened: &HexOpened,
        job: &JobControl,
    ) -> Result<Option<bool>, ApiError> {
        if !opened.converted() {
            return Ok(None);
        }
        run_guarded(|| {
            let router = self.router();
            let source = |side: Side| -> Result<VfsSource<'_>, ApiError> {
                let spec = match side {
                    Side::Right => &opened.specs.right,
                    _ => &opened.specs.left,
                };
                let entry = self.stat_input(side, spec, job)?;
                Ok(VfsSource::new(&*router, spec.clone(), entry))
            };
            let (l, r) = (source(Side::Left)?, source(Side::Right)?);
            let same =
                compare_sources(&l, &r, &self.stream_options(), &job.ctl()).map_err(|e| {
                    if e.is_cancelled() {
                        ApiError::Cancelled
                    } else {
                        ApiError::from_vfs(e)
                    }
                })? == hikasa_vfs::BinaryOutcome::Same;
            Ok(Some(same))
        })
    }

    /// The `input_changed` warnings of an opened pair after a job (D-118): each document's stamp re-checked
    /// (`check_unchanged`, a stat per side). Run it in the job that read.
    pub fn hex_changed(&self, opened: &HexOpened) -> Vec<Warning> {
        let router = self.router();
        let docs = Pair::new(&opened.documents.left, &opened.documents.right);
        [docs.left, docs.right]
            .into_iter()
            .filter_map(|doc| doc.check_unchanged(&*router).err())
            .map(|changed| hex_warning(&changed.warning()))
            .collect()
    }
}

impl HexCompareResponse {
    /// The response with the raw comparison of a converted pair (`Service::hex_raw_same`): its verdict by
    /// `converted_verdict` (D-331). Without a conversion it is unchanged.
    pub fn with_raw_same(mut self, raw_same: Option<bool>) -> HexCompareResponse {
        if let Some(raw) = raw_same
            && self.opened.converted()
        {
            let verdict = hikasa_hex::converted_verdict(raw, &self.alignment);
            self.kind = verdict.kind;
            self.exit_code = verdict.exit_code;
        }
        self
    }
}
