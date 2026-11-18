//! Spec 067 T064: property tests over the `keel-relay-v1` control-frame decoder.
//!
//! Arbitrary bytes, JSON-shaped noise and structured `Hello` frames go into `decode_body` and
//! `read_frame`. None may panic, none may make the decoder allocate more than the frame cap
//! (a counting allocator watches the largest single allocation), and anything the decoder
//! accepts must also pass an independent check of the contract: a JSON object with a known `t`,
//! exactly that frame's fields, and a clean round trip.
//!
//! The US5 query frames, which travel on the runner subsystem rather than the relay link, are
//! covered in `query_props.rs`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::BTreeSet;

use keel_exec::codec::CodecError;
use keel_exec::relay::{
    Frame, MAX_CACHE_CHUNK, MAX_CONTROL_FRAME_LEN, decode_body, encode, read_frame,
};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

// --- The largest single allocation, per thread ----------------------------------------------

struct Counting;

thread_local! {
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    // `try_with`: the slot can be gone while a thread is torn down.
    let _ = LARGEST.try_with(|l| l.set(l.get().max(size)));
}

// SAFETY: every call forwards to `System` unchanged; this only records sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// The largest single allocation `f` makes on this thread.
fn largest_alloc<T>(f: impl FnOnce() -> T) -> (T, usize) {
    LARGEST.with(|l| l.set(0));
    let out = f();
    (out, LARGEST.with(Cell::get))
}

/// What any decode of `input_len` bytes may allocate at most: never more than the frame cap,
/// and for small inputs nowhere near it.
fn alloc_budget(input_len: usize) -> usize {
    (64 * 1024 + 8 * input_len).min(MAX_CONTROL_FRAME_LEN)
}

// --- The contract, checked independently of serde ---------------------------------------------

/// Each frame's tag and exact field set (contracts/relay.md "Control subsystem").
fn contract(tag: &str) -> Option<&'static [&'static str]> {
    Some(match tag {
        "Hello" => &[
            "version",
            "relay_key_fingerprint",
            "max_streams",
            "http_cache",
        ],
        "Ping" => &["seq"],
        "Pong" => &["seq", "version", "streams", "uptime_s"],
        "CacheGet" => &["req", "release", "path"],
        "CacheChunk" => &["req", "offset", "bytes", "last"],
        "CacheMiss" => &["req", "reason"],
        "Close" => &["reason"],
        _ => return None,
    })
}

/// Panics unless `body`, which the decoder accepted as `frame`, is what the contract allows.
fn accepted_is_in_contract(body: &[u8], frame: &Frame) {
    let value: Value = serde_json::from_slice(body).expect("accepted, but not JSON");
    let obj = value.as_object().expect("accepted, but not an object");
    let tag = obj["t"]
        .as_str()
        .expect("accepted, but `t` is not a string");
    let fields = contract(tag).unwrap_or_else(|| panic!("accepted unknown tag {tag:?}"));
    let keys: BTreeSet<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "t")
        .collect();
    let want: BTreeSet<&str> = fields.iter().copied().collect();
    assert_eq!(keys, want, "accepted {tag} with other fields");
    if let Frame::CacheChunk { bytes, .. } = frame {
        assert!(bytes.len() <= MAX_CACHE_CHUNK);
    }
    let wire = encode(frame).expect("accepted, but does not encode");
    assert_eq!(&decode_body(&wire[4..]).unwrap(), frame, "no round trip");
}

fn decode_checked(body: &[u8]) -> Result<Frame, CodecError> {
    let (result, largest) = largest_alloc(|| decode_body(body));
    assert!(
        largest <= alloc_budget(body.len()),
        "decoding {} bytes allocated {largest} at once",
        body.len()
    );
    if let Ok(frame) = &result {
        accepted_is_in_contract(body, frame);
    }
    result
}

// --- Generators -----------------------------------------------------------------------------

/// A JSON value of the kinds the frames use, plus the wrong kinds.
fn any_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<u32>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_map(|f| json!(f)),
        "[ -~]{0,24}".prop_map(Value::from),
        Just(json!("aGVsbG8=")),
        Just(json!("not base64!")),
        Just(json!("unavailable")),
        Just(json!("not_found")),
        Just(json!("too_large")),
        Just(json!([])),
        Just(json!({})),
    ]
}

const KEYS: &[&str] = &[
    "t",
    "version",
    "relay_key_fingerprint",
    "max_streams",
    "http_cache",
    "seq",
    "streams",
    "uptime_s",
    "req",
    "release",
    "path",
    "offset",
    "bytes",
    "last",
    "reason",
    "extra",
];

const TAGS: &[&str] = &[
    "Hello",
    "Ping",
    "Pong",
    "CacheGet",
    "CacheChunk",
    "CacheMiss",
    "Close",
    "hello",
    "Query",
    "",
];

/// A JSON object shaped like a frame: a tag, then any of the frames' field names with values of
/// any kind. Many are valid frames; most are near misses.
fn frame_like() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::option::weighted(0.9, prop::sample::select(TAGS)),
        prop::collection::vec((prop::sample::select(KEYS), any_value()), 0..6),
    )
        .prop_map(|(tag, fields)| {
            let mut obj = Map::new();
            if let Some(tag) = tag {
                obj.insert("t".into(), json!(tag));
            }
            for (k, v) in fields {
                obj.insert(k.into(), v);
            }
            serde_json::to_vec(&Value::Object(obj)).unwrap()
        })
}

/// A valid frame's body, then one byte changed.
fn mutated_frame() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::sample::select(valid_bodies()),
        any::<prop::sample::Index>(),
        any::<u8>(),
    )
        .prop_map(|(mut body, at, byte)| {
            let i = at.index(body.len());
            body[i] = byte;
            body
        })
}

fn valid_bodies() -> Vec<Vec<u8>> {
    [
        json!({"t":"Hello","version":"0.1.0","relay_key_fingerprint":"SHA256:x","max_streams":256,"http_cache":true}),
        json!({"t":"Ping","seq":7}),
        json!({"t":"Pong","seq":7,"version":"0.1.0","streams":3,"uptime_s":60}),
        json!({"t":"CacheGet","req":1,"release":"0.2.0","path":"runner/a"}),
        json!({"t":"CacheChunk","req":1,"offset":0,"bytes":"aGVsbG8=","last":true}),
        json!({"t":"CacheMiss","req":1,"reason":"not_found"}),
        json!({"t":"Close","reason":"revoked"}),
    ]
    .iter()
    .map(|v| serde_json::to_vec(v).unwrap())
    .collect()
}

/// One `Hello` field: missing, valid, or the wrong kind.
#[derive(Debug, Clone)]
enum Field {
    Missing,
    Valid(Value),
    Wrong(Value),
}

fn string_field() -> impl Strategy<Value = Field> {
    prop_oneof![
        1 => Just(Field::Missing),
        6 => "[ -~]{0,16}".prop_map(|s| Field::Valid(json!(s))),
        2 => prop_oneof![
            any::<u32>().prop_map(Value::from),
            Just(Value::Null),
            any::<bool>().prop_map(Value::from),
            Just(json!(["a"])),
        ]
        .prop_map(Field::Wrong),
    ]
}

fn u32_field() -> impl Strategy<Value = Field> {
    prop_oneof![
        1 => Just(Field::Missing),
        6 => any::<u32>().prop_map(|n| Field::Valid(json!(n))),
        2 => prop_oneof![
            (u32::MAX as u64 + 1..=u64::MAX).prop_map(Value::from),
            (i64::MIN..0).prop_map(Value::from),
            Just(json!(1.5)),
            Just(json!("256")),
            Just(Value::Null),
        ]
        .prop_map(Field::Wrong),
    ]
}

fn bool_field() -> impl Strategy<Value = Field> {
    prop_oneof![
        1 => Just(Field::Missing),
        6 => any::<bool>().prop_map(|b| Field::Valid(json!(b))),
        2 => prop_oneof![Just(json!("true")), Just(json!(1)), Just(Value::Null)].prop_map(Field::Wrong),
    ]
}

fn hello() -> impl Strategy<Value = ([(&'static str, Field); 4], Option<Value>)> {
    (
        string_field(),
        string_field(),
        u32_field(),
        bool_field(),
        prop::option::of(any_value()),
    )
        .prop_map(|(v, f, m, h, extra)| {
            (
                [
                    ("version", v),
                    ("relay_key_fingerprint", f),
                    ("max_streams", m),
                    ("http_cache", h),
                ],
                extra,
            )
        })
}

/// A length prefix near the interesting edges, or anything.
fn prefix() -> impl Strategy<Value = u32> {
    let max = MAX_CONTROL_FRAME_LEN as u32;
    prop_oneof![
        0u32..64,
        Just(max - 1),
        Just(max),
        Just(max + 1),
        Just(u32::MAX),
        any::<u32>(),
    ]
}

fn config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

// --- Properties -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    #[test]
    fn arbitrary_bytes_never_panic_and_only_contract_frames_decode(
        body in prop::collection::vec(any::<u8>(), 0..512)
    ) {
        let _ = decode_checked(&body);
    }

    #[test]
    fn frame_shaped_json_decodes_only_within_the_contract(body in frame_like()) {
        let _ = decode_checked(&body);
    }

    #[test]
    fn a_valid_frame_with_one_byte_changed_decodes_only_within_the_contract(
        body in mutated_frame()
    ) {
        let _ = decode_checked(&body);
    }

    #[test]
    fn hello_decodes_exactly_when_every_field_is_present_and_right(
        (fields, extra) in hello()
    ) {
        let mut obj = Map::new();
        obj.insert("t".into(), json!("Hello"));
        let mut complete = true;
        for (name, field) in &fields {
            match field {
                Field::Missing => complete = false,
                Field::Valid(v) => {
                    obj.insert((*name).into(), v.clone());
                }
                Field::Wrong(v) => {
                    complete = false;
                    obj.insert((*name).into(), v.clone());
                }
            }
        }
        if let Some(v) = &extra {
            obj.insert("relay_note".into(), v.clone());
        }
        let body = serde_json::to_vec(&Value::Object(obj)).unwrap();
        let result = decode_checked(&body);
        let expected = complete && extra.is_none();
        prop_assert_eq!(result.is_ok(), expected, "{:?} -> {:?}", String::from_utf8_lossy(&body), result);
        if let Ok(Frame::Hello { version, relay_key_fingerprint, max_streams, http_cache }) = result {
            let get = |i: usize| match &fields[i].1 { Field::Valid(v) => v.clone(), _ => unreachable!() };
            prop_assert_eq!(json!(version), get(0));
            prop_assert_eq!(json!(relay_key_fingerprint), get(1));
            prop_assert_eq!(json!(max_streams), get(2));
            prop_assert_eq!(json!(http_cache), get(3));
        }
    }

    #[test]
    fn read_frame_refuses_over_the_cap_before_allocating_for_it(
        len in prefix(),
        body in prop::collection::vec(any::<u8>(), 0..2048),
        cut in 0usize..4,
    ) {
        let mut wire = len.to_be_bytes().to_vec();
        wire.extend_from_slice(&body);
        // Sometimes the stream ends inside the prefix.
        if body.is_empty() && cut > 0 {
            wire.truncate(cut);
        }
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let (result, largest) = largest_alloc(|| rt.block_on(read_frame(&mut wire.as_slice())));
        prop_assert!(
            largest <= alloc_budget(wire.len()),
            "a {}-byte stream announcing {len} allocated {largest} at once", wire.len()
        );
        let len = len as usize;
        match &result {
            _ if wire.len() < 4 => {
                prop_assert!(matches!(result, Err(CodecError::Truncated)), "{result:?}")
            }
            _ if len > MAX_CONTROL_FRAME_LEN => prop_assert!(
                matches!(result, Err(CodecError::TooLarge { len: l, .. }) if l == len),
                "{result:?}"
            ),
            _ if body.len() < len => {
                prop_assert!(matches!(result, Err(CodecError::Truncated)), "{result:?}")
            }
            Ok(Some(frame)) => accepted_is_in_contract(&body[..len], frame),
            Ok(None) => prop_assert!(false, "a non-empty stream read as its end"),
            Err(e) => prop_assert!(decode_body(&body[..len]).is_err(), "{e:?}"),
        }
    }
}

#[test]
fn an_empty_stream_is_a_clean_end() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert!(matches!(rt.block_on(read_frame(&mut &[][..])), Ok(None)));
}

#[test]
fn a_frame_announcing_the_full_cap_allocates_only_what_arrived() {
    // A peer that announces 1 MiB and sends a few bytes cannot make the reader reserve 1 MiB.
    let mut wire = (MAX_CONTROL_FRAME_LEN as u32).to_be_bytes().to_vec();
    wire.extend_from_slice(br#"{"t":"Ping","seq":1}"#);
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (result, largest) = largest_alloc(|| rt.block_on(read_frame(&mut wire.as_slice())));
    assert!(matches!(result, Err(CodecError::Truncated)), "{result:?}");
    // Above zero: the counter sees the reader's buffer, so the bound means something.
    assert!(largest > 0 && largest < 4096, "{largest}");
}
