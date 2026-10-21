//! Length-prefixed JSON framing and the two-phase decode (§4.4.1, §4.4.3; D-46): [`write_frame`],
//! [`read_frame`], [`send`], [`recv_request`], [`recv_response`].
//!
//! A frame is a `u32` little-endian payload length `N` (1 ≤ N ≤ [`MAX_FRAME_LEN`]) followed by `N` bytes of
//! UTF-8 JSON. EOF at a frame boundary is a clean close (`Ok(None)`); EOF inside a frame is
//! [`IpcError::ConnectionLost`]. Decoding is two-phase: the payload is parsed as a `serde_json::Value`, the `id`
//! is read from it (missing or not a `u64` → [`IpcError::Protocol`]), and only then is the body decoded, so that
//! a body that does not decode can be answered with its own id ([`IpcError::BadMessage`]) and an unknown `type`
//! decodes to the `Unknown` variant.

use std::io::{Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::MAX_FRAME_LEN;
use super::error::IpcError;
use super::protocol::{Envelope, Request, Response};

/// Writes one frame: u32 little-endian payload length, then the UTF-8 JSON payload.
pub fn write_frame(w: &mut dyn Write, payload: &[u8]) -> Result<(), IpcError> {
    let len = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge {
        len: u32::MAX,
        max: MAX_FRAME_LEN,
    })?;
    if len == 0 {
        return Err(IpcError::Protocol("empty frame".into()));
    }
    if len > MAX_FRAME_LEN {
        return Err(IpcError::FrameTooLarge {
            len,
            max: MAX_FRAME_LEN,
        });
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(payload);
    w.write_all(&frame).map_err(IpcError::from_transport)?;
    w.flush().map_err(IpcError::from_transport)
}

/// Reads one frame. `Ok(None)` = clean EOF at a frame boundary. EOF inside a frame = `ConnectionLost`.
///
/// A zero length is a protocol error and a length above `max` is [`IpcError::FrameTooLarge`]; the payload is
/// read incrementally, so an announced length is never allocated up front.
pub fn read_frame(r: &mut dyn Read, max: u32) -> Result<Option<Vec<u8>>, IpcError> {
    let mut header = [0u8; 4];
    let mut got = 0;
    while got < header.len() {
        let n = match r.read(&mut header[got..]) {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(IpcError::from_transport(e)),
        };
        if n == 0 {
            return if got == 0 {
                Ok(None)
            } else {
                Err(IpcError::ConnectionLost)
            };
        }
        got += n;
    }
    let len = u32::from_le_bytes(header);
    if len == 0 {
        return Err(IpcError::Protocol("empty frame".into()));
    }
    if len > max {
        return Err(IpcError::FrameTooLarge { len, max });
    }
    let mut payload = Vec::with_capacity(usize::try_from(len.min(64 * 1024)).unwrap_or(0));
    let read = r
        .take(u64::from(len))
        .read_to_end(&mut payload)
        .map_err(IpcError::from_transport)?;
    if read != len as usize {
        return Err(IpcError::ConnectionLost);
    }
    Ok(Some(payload))
}

/// Serialises an envelope (field order: `id`, `type`, body fields) and writes it.
pub fn send<T: Serialize>(w: &mut dyn Write, msg: &Envelope<T>) -> Result<(), IpcError> {
    let payload = serde_json::to_vec(msg)
        .map_err(|e| IpcError::Protocol(format!("cannot serialise a message: {e}")))?;
    write_frame(w, &payload)
}

/// Reads a request with the two-phase decode of §4.4.3 (D-46): `Ok(None)` at clean EOF; an envelope whose body
/// does not decode gives `Err(IpcError::BadMessage { id, .. })` so that the caller can answer with that id.
pub fn recv_request(r: &mut dyn Read) -> Result<Option<Envelope<Request>>, IpcError> {
    Ok(recv_typed(r)?.map(|(envelope, _)| envelope))
}

/// Reads a response (same two-phase decode; unknown `type` values become `Response::Unknown`).
pub fn recv_response(r: &mut dyn Read) -> Result<Option<Envelope<Response>>, IpcError> {
    Ok(recv_typed(r)?.map(|(envelope, _)| envelope))
}

/// Reads one frame and decodes it in two phases; also returns the frame's `type` string (for the
/// `unsupported_request` message of an unknown type).
pub(crate) fn recv_typed<T: DeserializeOwned>(
    r: &mut dyn Read,
) -> Result<Option<(Envelope<T>, String)>, IpcError> {
    match read_frame(r, MAX_FRAME_LEN)? {
        Some(payload) => decode(&payload).map(Some),
        None => Ok(None),
    }
}

/// The two-phase decode of one payload (§4.4.3 rule 3).
pub(crate) fn decode<T: DeserializeOwned>(
    payload: &[u8],
) -> Result<(Envelope<T>, String), IpcError> {
    let text = std::str::from_utf8(payload)
        .map_err(|_| IpcError::Protocol("the frame is not valid UTF-8".into()))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| IpcError::Protocol(format!("the frame is not valid JSON: {e}")))?;
    let Value::Object(object) = value else {
        return Err(IpcError::Protocol(
            "the message is not a JSON object".into(),
        ));
    };
    let id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| IpcError::Protocol("the message has no numeric `id`".into()))?;
    let type_name = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let body =
        serde_json::from_value::<T>(Value::Object(object)).map_err(|e| IpcError::BadMessage {
            id,
            message: e.to_string(),
        })?;
    Ok((Envelope { id, body }, type_name))
}
