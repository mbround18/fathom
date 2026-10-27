//! Splits a run's pack into `Pack` frames (spec 055 T070, contracts/runner.md "Pack"): chunks of
//! at most [`PACK_CHUNK`] bytes, the last one flagged and carrying the whole pack's SHA-256 (hex),
//! so the host can check what it reassembled before indexing it.

use sha2::{Digest, Sha256};

use crate::codec::MAX_PACK_CHUNK;
use crate::frames::Frame;

/// The chunk size Keel sends: well under [`MAX_PACK_CHUNK`], so a chunk never waits behind a
/// large write and base64 keeps the frame far from the frame limit.
pub const PACK_CHUNK: usize = 1024 * 1024;

const _: () = assert!(PACK_CHUNK <= MAX_PACK_CHUNK);

/// The SHA-256 of `bytes`, lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The `Pack` frames for `pack`, in order. An empty pack still yields one (empty, last) frame.
#[must_use]
pub fn frames(run_id: &str, pack: &[u8]) -> Vec<Frame> {
    let sha = sha256_hex(pack);
    let chunks: Vec<&[u8]> = if pack.is_empty() {
        vec![&[]]
    } else {
        pack.chunks(PACK_CHUNK).collect()
    };
    let n = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let last = i + 1 == n;
            Frame::Pack {
                run_id: run_id.to_owned(),
                chunk: chunk.to_vec(),
                last,
                sha256: last.then(|| sha.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_flags_the_last_chunk_with_the_digest() {
        let pack: Vec<u8> = (0..(PACK_CHUNK * 2 + 10))
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let fs = frames("r1", &pack);
        assert_eq!(fs.len(), 3);
        let mut joined = Vec::new();
        for (i, f) in fs.iter().enumerate() {
            let Frame::Pack {
                run_id,
                chunk,
                last,
                sha256,
            } = f
            else {
                panic!("{f:?}");
            };
            assert_eq!(run_id, "r1");
            assert_eq!(*last, i == 2);
            assert_eq!(sha256.is_some(), i == 2);
            assert!(chunk.len() <= PACK_CHUNK);
            joined.extend_from_slice(chunk);
        }
        assert_eq!(joined, pack);
        let Frame::Pack { sha256, .. } = &fs[2] else {
            unreachable!()
        };
        assert_eq!(sha256.as_deref(), Some(sha256_hex(&pack).as_str()));
    }

    #[test]
    fn small_and_empty_packs_are_one_frame() {
        assert_eq!(frames("r", b"PACK").len(), 1);
        let fs = frames("r", b"");
        assert!(matches!(
            &fs[..],
            [Frame::Pack {
                last: true,
                sha256: Some(_),
                ..
            }]
        ));
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
