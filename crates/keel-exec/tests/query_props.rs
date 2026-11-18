//! Spec 067 T064: property tests over the US5 query frames (`Query`, `QueryAnswer`,
//! `QueryRefused`, `ReadSmart`, `Smart`).
//!
//! Structured frames round-trip through the codec unchanged; arbitrary JSON-shaped noise and
//! raw bytes never panic the decoder, and whatever it accepts re-encodes to the same frame.
//! Query frames travel only on the runner subsystem (or the local socket), in the direction
//! the contract gives them.

use std::collections::BTreeMap;

use keel_exec::codec::{decode_body, encode_body};
use keel_exec::frames::{Direction, Frame, QueryRefusal, SmartDisk, Subsystem};
use proptest::prelude::*;
use serde_json::{Map, Value, json};

fn refusal() -> impl Strategy<Value = QueryRefusal> {
    prop_oneof![
        Just(QueryRefusal::NotAllowed),
        Just(QueryRefusal::BadParams),
        Just(QueryRefusal::Unavailable),
        Just(QueryRefusal::TooLarge),
        Just(QueryRefusal::Busy),
    ]
}

fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        ".{0,24}".prop_map(Value::String),
    ]
}

fn json_value() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(3, 32, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::btree_map(".{0,12}", inner, 0..6)
                .prop_map(|m| Value::Object(m.into_iter().collect::<Map<_, _>>())),
        ]
    })
}

fn query_frame() -> impl Strategy<Value = Frame> {
    prop_oneof![
        (
            ".{0,40}",
            ".{0,24}",
            prop::collection::btree_map(".{0,12}", ".{0,64}", 0..3),
            ".{0,64}"
        )
            .prop_map(|(query_id, kind, params, asked_by)| Frame::Query {
                query_id,
                kind,
                params,
                asked_by,
            }),
        (".{0,40}", json_value())
            .prop_map(|(query_id, answer)| Frame::QueryAnswer { query_id, answer }),
        (".{0,40}", refusal())
            .prop_map(|(query_id, reason)| Frame::QueryRefused { query_id, reason }),
        Just(Frame::ReadSmart),
        (
            any::<bool>(),
            prop::collection::vec(
                ("[a-z0-9_]{1,12}", any::<Option<bool>>())
                    .prop_map(|(name, smart_passed)| SmartDisk { name, smart_passed }),
                0..4
            )
        )
            .prop_map(|(installed, disks)| Frame::Smart { installed, disks }),
    ]
}

/// JSON-shaped noise around the query frames' tags and fields.
fn noise() -> impl Strategy<Value = Vec<u8>> {
    let tag = prop_oneof![
        Just("Query"),
        Just("QueryAnswer"),
        Just("QueryRefused"),
        Just("ReadSmart"),
        Just("Smart"),
        Just("query"),
    ];
    let field = prop_oneof![
        Just("query_id"),
        Just("kind"),
        Just("params"),
        Just("asked_by"),
        Just("answer"),
        Just("reason"),
        Just("installed"),
        Just("disks"),
        Just("extra"),
    ];
    (tag, prop::collection::btree_map(field, json_value(), 0..6)).prop_map(|(tag, fields)| {
        let mut obj: Map<String, Value> =
            fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        obj.insert("frame".into(), json!(tag));
        serde_json::to_vec(&Value::Object(obj)).expect("json")
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn query_frames_round_trip(frame in query_frame()) {
        let body = encode_body(&frame).expect("encode");
        prop_assert_eq!(decode_body(&body).expect("decode"), frame);
    }

    #[test]
    fn query_frames_keep_to_their_subsystem_and_direction(frame in query_frame()) {
        match &frame {
            Frame::Query { .. } => {
                prop_assert_eq!(frame.subsystem(), Some(Subsystem::Runner));
                prop_assert_eq!(frame.direction(), Direction::ServerToRunner);
            }
            Frame::QueryAnswer { .. } | Frame::QueryRefused { .. } => {
                prop_assert_eq!(frame.subsystem(), Some(Subsystem::Runner));
                prop_assert_eq!(frame.direction(), Direction::RunnerToServer);
            }
            // The SMART read is the runner's local-socket call to the executor, never on SSH.
            _ => prop_assert_eq!(frame.subsystem(), None),
        }
    }

    #[test]
    fn noise_never_panics_and_what_decodes_round_trips(body in noise()) {
        if let Ok(frame) = decode_body(&body) {
            let again = encode_body(&frame).expect("encode");
            prop_assert_eq!(&decode_body(&again).expect("decode"), &frame);
            // Unknown fields are refused, so nothing with fields decoded carried `extra`. A unit
            // frame (`ReadSmart`, like `Ping` and `ReadLedger`) has nothing to confuse and serde
            // ignores extra keys there; it carries nothing either way.
            let v: Value = serde_json::from_slice(&body).expect("json");
            prop_assert!(frame == Frame::ReadSmart || v.get("extra").is_none());
        }
    }

    #[test]
    fn raw_bytes_never_panic(body in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode_body(&body);
    }
}

#[test]
fn an_empty_params_map_is_left_out_and_read_back_empty() {
    let q = Frame::Query {
        query_id: "q".into(),
        kind: "facts".into(),
        params: BTreeMap::new(),
        asked_by: "ops".into(),
    };
    let body = encode_body(&q).expect("encode");
    let v: Value = serde_json::from_slice(&body).expect("json");
    assert!(v.get("params").is_none());
    assert_eq!(decode_body(&body).expect("decode"), q);
}
