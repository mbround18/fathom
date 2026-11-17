//! Spec 067 T005: the `keel-relay-v1` control frames (contracts/relay.md "Control subsystem").

use keel_exec::codec::CodecError;
use keel_exec::relay::{
    self, CacheMissReason, Frame, MAX_CACHE_CHUNK, MAX_CONTROL_FRAME_LEN, decode_body, encode,
    read_frame, write_frame,
};

fn every_frame() -> Vec<Frame> {
    vec![
        Frame::Hello {
            version: "0.1.0".into(),
            relay_key_fingerprint: "SHA256:abc".into(),
            max_streams: 256,
            http_cache: true,
        },
        Frame::Ping { seq: 7 },
        Frame::Pong {
            seq: 7,
            version: "0.1.0".into(),
            streams: 3,
            uptime_s: 86_400,
        },
        Frame::CacheGet {
            req: 1,
            release: "0.2.0".into(),
            path: "runner/0.2.0/keel-runner-x86_64".into(),
        },
        Frame::CacheChunk {
            req: 1,
            offset: 524_288,
            bytes: vec![0, 1, 2, 255],
            last: true,
        },
        Frame::CacheMiss {
            req: 2,
            reason: CacheMissReason::Unavailable,
        },
        Frame::CacheMiss {
            req: 3,
            reason: CacheMissReason::NotFound,
        },
        Frame::CacheMiss {
            req: 4,
            reason: CacheMissReason::TooLarge,
        },
        Frame::Close {
            reason: "revoked".into(),
        },
    ]
}

#[tokio::test]
async fn every_control_frame_round_trips_through_the_codec() {
    let (mut a, mut b) = tokio::io::duplex(64 * 1024);
    for frame in every_frame() {
        write_frame(&mut a, &frame).await.unwrap();
        assert_eq!(read_frame(&mut b).await.unwrap(), Some(frame.clone()));

        let wire = encode(&frame).unwrap();
        let len = u32::from_be_bytes(wire[..4].try_into().unwrap()) as usize;
        assert_eq!(len, wire.len() - 4);
        assert_eq!(decode_body(&wire[4..]).unwrap(), frame);
    }
    drop(a);
    assert_eq!(
        read_frame(&mut b).await.unwrap(),
        None,
        "clean end of stream"
    );
}

#[test]
fn frames_are_tagged_t_with_contract_field_names() {
    let json = |f: &Frame| serde_json::to_value(f).unwrap();
    assert_eq!(
        json(&Frame::Ping { seq: 1 }),
        serde_json::json!({"t": "Ping", "seq": 1})
    );
    assert_eq!(
        json(&Frame::CacheChunk {
            req: 9,
            offset: 0,
            bytes: b"hi".to_vec(),
            last: false
        }),
        serde_json::json!({"t": "CacheChunk", "req": 9, "offset": 0, "bytes": "aGk=", "last": false})
    );
    assert_eq!(
        json(&Frame::CacheMiss {
            req: 9,
            reason: CacheMissReason::NotFound
        }),
        serde_json::json!({"t": "CacheMiss", "req": 9, "reason": "not_found"})
    );
}

#[test]
fn unknown_tags_and_unknown_fields_fail() {
    assert!(matches!(
        decode_body(br#"{"t":"Exec","cmd":"id"}"#),
        Err(CodecError::Malformed(_))
    ));
    assert!(matches!(
        decode_body(br#"{"frame":"Ping","seq":1}"#),
        Err(CodecError::Malformed(_))
    ));
    assert!(matches!(
        decode_body(br#"{"t":"Ping","seq":1,"extra":true}"#),
        Err(CodecError::Malformed(_))
    ));
    assert!(matches!(
        decode_body(br#"{"t":"CacheMiss","req":1,"reason":"gone"}"#),
        Err(CodecError::Malformed(_))
    ));
}

#[tokio::test]
async fn a_frame_over_one_mib_fails_both_ways() {
    assert_eq!(MAX_CONTROL_FRAME_LEN, 1024 * 1024);
    let big = Frame::Close {
        reason: "x".repeat(MAX_CONTROL_FRAME_LEN),
    };
    assert!(matches!(
        encode(&big),
        Err(CodecError::TooLarge { max, .. }) if max == MAX_CONTROL_FRAME_LEN
    ));
    let (mut a, mut b) = tokio::io::duplex(1024);
    assert!(write_frame(&mut a, &big).await.is_err());

    // A prefix over the cap is refused before any body byte is read.
    use tokio::io::AsyncWriteExt;
    a.write_all(&((MAX_CONTROL_FRAME_LEN as u32) + 1).to_be_bytes())
        .await
        .unwrap();
    assert!(matches!(
        read_frame(&mut b).await,
        Err(CodecError::TooLarge { .. })
    ));
}

#[test]
fn a_cache_chunk_over_512_kib_fails() {
    assert_eq!(MAX_CACHE_CHUNK, 512 * 1024);
    let at_cap = Frame::CacheChunk {
        req: 1,
        offset: 0,
        bytes: vec![7; MAX_CACHE_CHUNK],
        last: false,
    };
    let wire = encode(&at_cap).unwrap();
    assert_eq!(decode_body(&wire[4..]).unwrap(), at_cap);

    let over = Frame::CacheChunk {
        req: 1,
        offset: 0,
        bytes: vec![7; MAX_CACHE_CHUNK + 1],
        last: false,
    };
    assert!(encode(&over).is_err(), "never sent");
    let body = serde_json::to_vec(&over).unwrap();
    assert!(body.len() <= MAX_CONTROL_FRAME_LEN);
    assert!(decode_body(&body).is_err(), "never accepted");
}

#[test]
fn the_link_constants_match_the_contract() {
    assert_eq!(relay::RELAY_SUBSYSTEM, "keel-relay-v1");
    assert_eq!(relay::LINK_USER, "keel");
    assert_eq!(relay::RUNNER_CHANNEL_ADDR, ("keel-runner", 0));
}
