//! The frames the server and the volume helper exchange over the helper's stdin/stdout
//! (contracts/helper.md).
//!
//! ```text
//! frame  = len:u32be kind:u8 body[len]
//! kind   = 0 Hello | 1 Op (JSON) | 2 Reply (JSON) | 3 Data (bytes) | 4 End | 5 Error (JSON)
//! max len = 1 MiB
//! ```
//!
//! Blocking `Read`/`Write` only: the helper has no runtime, and the server drives it from a
//! blocking task. [`write_frame`] does not flush; a caller that waits for an answer flushes first.

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Largest body a frame may carry.
pub const MAX_FRAME_LEN: usize = 1024 * 1024;

/// What a frame's body holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameKind {
    /// JSON [`crate::op::Hello`], the helper's first frame.
    Hello = 0,
    /// JSON [`crate::op::Op`], server to helper.
    Op = 1,
    /// JSON [`crate::op::Reply`], helper to server.
    Reply = 2,
    /// Raw bytes of a file or a tar stream, either way.
    Data = 3,
    /// The end of a run of Data frames. Empty body.
    End = 4,
    /// JSON [`crate::op::HelperError`], helper to server.
    Error = 5,
}

impl TryFrom<u8> for FrameKind {
    type Error = FrameError;
    fn try_from(value: u8) -> Result<Self, FrameError> {
        Ok(match value {
            0 => Self::Hello,
            1 => Self::Op,
            2 => Self::Reply,
            3 => Self::Data,
            4 => Self::End,
            5 => Self::Error,
            other => return Err(FrameError::UnknownKind(other)),
        })
    }
}

/// Why a frame could not be read, written or parsed. Every one of them is a protocol error: the
/// helper exits 70 and the server kills it.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame i/o: {0}")]
    Io(#[from] io::Error),
    #[error("frame length {0} is over the 1 MiB limit")]
    TooLong(u64),
    #[error("unknown frame kind {0}")]
    UnknownKind(u8),
    #[error("the stream ended inside a frame")]
    Truncated,
    #[error("expected a {expected:?} frame, got {got:?}")]
    UnexpectedKind { expected: FrameKind, got: FrameKind },
    #[error("frame body is not the expected JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// One frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: FrameKind,
    pub body: Vec<u8>,
}

impl Frame {
    pub fn new(kind: FrameKind, body: Vec<u8>) -> Self {
        Self { kind, body }
    }

    /// A frame whose body is `value` as JSON.
    pub fn json<T: Serialize>(kind: FrameKind, value: &T) -> Result<Self, FrameError> {
        Ok(Self::new(kind, serde_json::to_vec(value)?))
    }

    pub fn data(bytes: Vec<u8>) -> Self {
        Self::new(FrameKind::Data, bytes)
    }

    pub fn end() -> Self {
        Self::new(FrameKind::End, Vec::new())
    }

    /// The body as JSON of type `T`, after checking the frame is of `kind`.
    pub fn parse<T: DeserializeOwned>(&self, kind: FrameKind) -> Result<T, FrameError> {
        self.expect(kind)?;
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// Errors unless this frame is of `kind`.
    pub fn expect(&self, kind: FrameKind) -> Result<(), FrameError> {
        if self.kind == kind {
            Ok(())
        } else {
            Err(FrameError::UnexpectedKind { expected: kind, got: self.kind })
        }
    }

    pub fn write_to<W: Write>(&self, w: &mut W) -> Result<(), FrameError> {
        write_frame(w, self.kind, &self.body)
    }
}

/// Reads one frame. `Ok(None)` on a clean EOF before a header; EOF anywhere else is
/// [`FrameError::Truncated`]. The length is checked before the body is allocated.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<Frame>, FrameError> {
    let mut header = [0u8; 5];
    let mut got = 0;
    while got < header.len() {
        match r.read(&mut header[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(FrameError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLong(len as u64));
    }
    let kind = FrameKind::try_from(header[4])?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => FrameError::Truncated,
        _ => FrameError::Io(e),
    })?;
    Ok(Some(Frame { kind, body }))
}

/// Writes one frame. A body over [`MAX_FRAME_LEN`] is refused before anything is written.
pub fn write_frame<W: Write>(w: &mut W, kind: FrameKind, body: &[u8]) -> Result<(), FrameError> {
    if body.len() > MAX_FRAME_LEN {
        return Err(FrameError::TooLong(body.len() as u64));
    }
    let mut header = [0u8; 5];
    header[..4].copy_from_slice(&(body.len() as u32).to_be_bytes());
    header[4] = kind as u8;
    w.write_all(&header)?;
    w.write_all(body)?;
    Ok(())
}

/// Writes `value` as a JSON frame of `kind`.
pub fn write_json<W: Write, T: Serialize>(
    w: &mut W,
    kind: FrameKind,
    value: &T,
) -> Result<(), FrameError> {
    write_frame(w, kind, &serde_json::to_vec(value)?)
}

/// Writes `bytes` as Data frames of at most [`MAX_FRAME_LEN`] each (none for empty input). The
/// End frame is the caller's, as a stream may span several calls.
pub fn write_data<W: Write>(w: &mut W, bytes: &[u8]) -> Result<(), FrameError> {
    for chunk in bytes.chunks(MAX_FRAME_LEN) {
        write_frame(w, FrameKind::Data, chunk)?;
    }
    Ok(())
}

/// Reads Data frames up to the End frame and returns their bytes joined, refusing more than
/// `max` bytes in total ([`FrameError::TooLong`]) and any other frame kind.
pub fn read_data<R: Read>(r: &mut R, max: u64) -> Result<Vec<u8>, FrameError> {
    let mut out = Vec::new();
    loop {
        let frame = read_frame(r)?.ok_or(FrameError::Truncated)?;
        match frame.kind {
            FrameKind::End => return Ok(out),
            FrameKind::Data => {
                let total = out.len() as u64 + frame.body.len() as u64;
                if total > max {
                    return Err(FrameError::TooLong(total));
                }
                out.extend_from_slice(&frame.body);
            }
            got => return Err(FrameError::UnexpectedKind { expected: FrameKind::Data, got }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [FrameKind; 6] = [
        FrameKind::Hello,
        FrameKind::Op,
        FrameKind::Reply,
        FrameKind::Data,
        FrameKind::End,
        FrameKind::Error,
    ];

    #[test]
    fn round_trip_each_kind() {
        let mut buf = Vec::new();
        for (i, kind) in KINDS.into_iter().enumerate() {
            write_frame(&mut buf, kind, &vec![i as u8; i * 3]).unwrap();
        }
        let mut r = buf.as_slice();
        for (i, kind) in KINDS.into_iter().enumerate() {
            let frame = read_frame(&mut r).unwrap().unwrap();
            assert_eq!(frame, Frame::new(kind, vec![i as u8; i * 3]));
        }
        assert!(read_frame(&mut r).unwrap().is_none(), "clean EOF");
    }

    #[test]
    fn header_layout_is_len_be_then_kind() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FrameKind::Reply, b"{}").unwrap();
        assert_eq!(buf, [0, 0, 0, 2, 2, b'{', b'}']);
    }

    #[test]
    fn max_length_is_accepted_and_one_more_refused() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FrameKind::Data, &vec![7; MAX_FRAME_LEN]).unwrap();
        let frame = read_frame(&mut buf.as_slice()).unwrap().unwrap();
        assert_eq!(frame.body.len(), MAX_FRAME_LEN);

        let mut out = Vec::new();
        assert!(matches!(
            write_frame(&mut out, FrameKind::Data, &vec![0; MAX_FRAME_LEN + 1]),
            Err(FrameError::TooLong(_))
        ));
        assert!(out.is_empty(), "nothing written");
    }

    #[test]
    fn oversized_length_is_refused_before_reading_the_body() {
        let mut header = ((MAX_FRAME_LEN + 1) as u32).to_be_bytes().to_vec();
        header.push(3);
        assert!(matches!(
            read_frame(&mut header.as_slice()),
            Err(FrameError::TooLong(n)) if n == MAX_FRAME_LEN as u64 + 1
        ));
        let huge = [0xff, 0xff, 0xff, 0xff, 3];
        assert!(matches!(read_frame(&mut huge.as_slice()), Err(FrameError::TooLong(_))));
    }

    #[test]
    fn truncation_is_an_error() {
        let mut buf = Vec::new();
        write_frame(&mut buf, FrameKind::Data, b"hello").unwrap();
        for cut in 1..buf.len() {
            assert!(
                matches!(read_frame(&mut &buf[..cut]), Err(FrameError::Truncated)),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn unknown_kind_is_an_error() {
        let bytes = [0, 0, 0, 0, 6];
        assert!(matches!(read_frame(&mut bytes.as_slice()), Err(FrameError::UnknownKind(6))));
        let bytes = [0, 0, 0, 0, 255];
        assert!(matches!(read_frame(&mut bytes.as_slice()), Err(FrameError::UnknownKind(255))));
    }

    #[test]
    fn json_frames_check_their_kind() {
        let frame = Frame::json(FrameKind::Reply, &serde_json::json!({"a": 1})).unwrap();
        let value: serde_json::Value = frame.parse(FrameKind::Reply).unwrap();
        assert_eq!(value["a"], 1);
        assert!(matches!(
            frame.parse::<serde_json::Value>(FrameKind::Error),
            Err(FrameError::UnexpectedKind { expected: FrameKind::Error, got: FrameKind::Reply })
        ));
        let bad = Frame::new(FrameKind::Reply, b"not json".to_vec());
        assert!(matches!(bad.parse::<serde_json::Value>(FrameKind::Reply), Err(FrameError::Json(_))));
    }

    #[test]
    fn data_runs_split_and_join() {
        let content: Vec<u8> = (0..(MAX_FRAME_LEN * 2 + 10)).map(|i| i as u8).collect();
        let mut buf = Vec::new();
        write_data(&mut buf, &content).unwrap();
        Frame::end().write_to(&mut buf).unwrap();
        assert_eq!(read_data(&mut buf.as_slice(), content.len() as u64).unwrap(), content);
        assert!(matches!(
            read_data(&mut buf.as_slice(), content.len() as u64 - 1),
            Err(FrameError::TooLong(_))
        ));

        let mut empty = Vec::new();
        write_data(&mut empty, b"").unwrap();
        assert!(empty.is_empty());
        Frame::end().write_to(&mut empty).unwrap();
        assert_eq!(read_data(&mut empty.as_slice(), 0).unwrap(), b"");
    }

    #[test]
    fn a_data_run_must_end_with_end() {
        let mut buf = Vec::new();
        write_data(&mut buf, b"abc").unwrap();
        assert!(matches!(read_data(&mut buf.as_slice(), 10), Err(FrameError::Truncated)));
        write_frame(&mut buf, FrameKind::Reply, b"{}").unwrap();
        assert!(matches!(
            read_data(&mut buf.as_slice(), 10),
            Err(FrameError::UnexpectedKind { got: FrameKind::Reply, .. })
        ));
    }
}
