//! Length-prefixed JSON framing: a `u32` big-endian length, then that many bytes of JSON.
//!
//! The body is capped at [`MAX_FRAME_LEN`] (16 MiB). A larger prefix is refused before any body
//! byte is read or allocated, and a larger frame is refused before anything is written.

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::frames::Frame;

/// The largest frame body accepted or sent, in bytes.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// The largest raw `Pack` chunk that, base64-encoded, keeps a frame well under the cap.
pub const MAX_PACK_CHUNK: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame of {len} bytes exceeds the {max}-byte limit")]
    TooLarge { len: usize, max: usize },
    #[error("stream ended inside a frame")]
    Truncated,
    #[error("malformed frame: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
}

/// Serialise a frame body (no prefix), refusing one over the cap.
pub fn encode_body(frame: &Frame) -> Result<Vec<u8>, CodecError> {
    encode_body_capped(frame, MAX_FRAME_LEN)
}

/// Parse a frame body (no prefix). Unknown frames and fields are errors.
pub fn decode_body(body: &[u8]) -> Result<Frame, CodecError> {
    decode_body_capped(body, MAX_FRAME_LEN)
}

/// Serialise a frame with its length prefix.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, CodecError> {
    encode_capped(frame, MAX_FRAME_LEN)
}

/// Write one frame and flush.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    frame: &Frame,
) -> Result<(), CodecError> {
    write_capped(w, frame, MAX_FRAME_LEN).await
}

/// Read one frame. `Ok(None)` is a clean end of stream at a frame boundary; an end of stream
/// anywhere else is [`CodecError::Truncated`].
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Frame>, CodecError> {
    read_capped(r, MAX_FRAME_LEN).await
}

/// [`read_frame`] with a smaller cap, for a stream whose first frame is small (an enroll or
/// join request): a larger prefix is refused before any body byte is read.
pub async fn read_frame_capped<R: AsyncRead + Unpin>(
    r: &mut R,
    max: usize,
) -> Result<Option<Frame>, CodecError> {
    read_capped(r, max.min(MAX_FRAME_LEN)).await
}

// The same framing for any frame type and cap (the relay control channel uses 1 MiB).

pub(crate) fn encode_body_capped<T: Serialize>(
    frame: &T,
    max: usize,
) -> Result<Vec<u8>, CodecError> {
    let body = serde_json::to_vec(frame)?;
    if body.len() > max {
        return Err(CodecError::TooLarge {
            len: body.len(),
            max,
        });
    }
    Ok(body)
}

pub(crate) fn decode_body_capped<T: DeserializeOwned>(
    body: &[u8],
    max: usize,
) -> Result<T, CodecError> {
    if body.len() > max {
        return Err(CodecError::TooLarge {
            len: body.len(),
            max,
        });
    }
    Ok(serde_json::from_slice(body)?)
}

pub(crate) fn encode_capped<T: Serialize>(frame: &T, max: usize) -> Result<Vec<u8>, CodecError> {
    let body = encode_body_capped(frame, max)?;
    let mut out = Vec::with_capacity(body.len() + 4);
    // Every cap is at most 16 MiB, so the length fits a u32.
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

pub(crate) async fn write_capped<W: AsyncWrite + Unpin, T: Serialize>(
    w: &mut W,
    frame: &T,
    max: usize,
) -> Result<(), CodecError> {
    let wire = encode_capped(frame, max)?;
    w.write_all(&wire).await?;
    w.flush().await?;
    Ok(())
}

pub(crate) async fn read_capped<R: AsyncRead + Unpin, T: DeserializeOwned>(
    r: &mut R,
    max: usize,
) -> Result<Option<T>, CodecError> {
    let mut prefix = [0u8; 4];
    let mut got = 0;
    while got < prefix.len() {
        let n = r.read(&mut prefix[got..]).await?;
        if n == 0 {
            return if got == 0 {
                Ok(None)
            } else {
                Err(CodecError::Truncated)
            };
        }
        got += n;
    }
    let len = u32::from_be_bytes(prefix) as usize;
    if len > max {
        return Err(CodecError::TooLarge { len, max });
    }
    let mut body = Vec::new();
    let read = r.take(len as u64).read_to_end(&mut body).await?;
    if read < len {
        return Err(CodecError::Truncated);
    }
    decode_body_capped(&body, max).map(Some)
}
