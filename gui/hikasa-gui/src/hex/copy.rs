//! Copy and Copy as Text of a hex view (M2 W13a, contract §1.13.5, D-357), toolkit-free: the form of the copied text
//! and the body of the copy job when the bytes are not cached. The text itself is core's
//! (`hikasa_hex::copy::selection_text`).

use std::ops::Range;

use hikasa_core::Side;
use hikasa_hex::CharSet;
use hikasa_hex::copy::{COPY_LIMIT, CopyForm, selection_text};
use hikasa_session::ApiError;

use super::screen::Source;

/// The form Copy writes: characters (`chars`: the characters part, or Copy as Text) in the layout's character set,
/// else hex pairs.
pub fn form(chars: bool, set: CharSet) -> CopyForm {
    if chars {
        CopyForm::Chars(set)
    } else {
        CopyForm::Hex
    }
}

/// The copy job's body: the selection's text read through the page cache (at most 16 MiB).
///
/// # Errors
///
/// The core's message (*Copy takes at most 16 MiB; select less*, a read error).
pub fn copy_in_job(
    src: &Source,
    side: Side,
    columns: Range<u64>,
    form: CopyForm,
) -> Result<String, ApiError> {
    let view = src.view();
    let doc = if side == Side::Right {
        &src.opened.documents.right
    } else {
        &src.opened.documents.left
    };
    selection_text(doc, view.segments(), columns, side, form, COPY_LIMIT)
        .map_err(|e| ApiError::InvalidArgument(e.to_string()))
}
