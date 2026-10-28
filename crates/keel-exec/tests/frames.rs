//! T061: every frame of contracts/runner.md round-trips through the codec, and the codec refuses
//! frames over 16 MiB.

use std::collections::BTreeMap;

use keel_exec::codec::{self, CodecError, MAX_FRAME_LEN};
use keel_exec::events::HostStats;
use keel_exec::frames::{
    Approval, Base, Direction, Frame, LogStream, Refusal, SecretItem, Subsystem,
};

fn every_frame() -> Vec<Frame> {
    let mut stats = BTreeMap::new();
    stats.insert(
        "web-1".to_string(),
        HostStats {
            ok: 3,
            changed: 1,
            failures: 0,
            unreachable: 0,
            skipped: 2,
            rescued: 0,
            ignored: 0,
        },
    );
    vec![
        Frame::Enroll {
            token: "keel-enroll-v1.s3cr3t.SHA256:abc".into(),
            inventory_name: "web-1".into(),
            allowed_profiles: vec!["base".into(), "web".into()],
            require_signed: true,
        },
        Frame::Enrolled {
            host_id: "h-1".into(),
            cert: "ssh-ed25519-cert-v01@openssh.com AAAA".into(),
        },
        Frame::Hello {
            host_id: "h-1".into(),
            version: "0.1.0".into(),
            ansible_core: "2.19.1".into(),
            python: "3.12.3".into(),
            allowed_profiles: vec!["base".into()],
            require_signed: false,
            override_quorum: 1,
            ledger_head: Some("sha256:00ff".into()),
            bases: vec![Base {
                job: "site".into(),
                commit: "a".repeat(40),
            }],
        },
        Frame::RenewCert {},
        Frame::Cert {
            cert: "ssh-ed25519-cert-v01@openssh.com AAAA".into(),
        },
        Frame::Krl {
            bytes: vec![0x53, 0x53, 0x48, 0x4b, 0, 255],
        },
        Frame::Assign {
            run_id: "r-1".into(),
            workspace: "00000000-0000-0000-0000-000000000001".into(),
            binding: "00000000-0000-0000-0000-000000000002".into(),
            job: "site".into(),
            base: None,
            target: "b".repeat(40),
            deadline: 1_900_000_000,
            pack_size: 1234,
            quorum: Some(2),
        },
        Frame::Pack {
            run_id: "r-1".into(),
            chunk: vec![1, 2, 3],
            last: false,
            sha256: None,
        },
        Frame::Pack {
            run_id: "r-1".into(),
            chunk: vec![],
            last: true,
            sha256: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into()),
        },
        Frame::Secrets {
            run_id: "r-1".into(),
            items: vec![SecretItem {
                name: "db_password".into(),
                kind: "password".into(),
                value: "hunter2".into(),
            }],
        },
        Frame::Approvals {
            run_id: "r-1".into(),
            items: vec![Approval {
                kind: "override".into(),
                approver: "alice".into(),
                credential_id: "Y3JlZA".into(),
                authenticator_data: "YXV0aA".into(),
                client_data_json: "e30".into(),
                signature: "c2ln".into(),
                expires_at: 1_900_000_000,
                comment: "hotfix reviewed".into(),
                hosts: vec!["web-1".into(), "web-2".into()],
            }],
        },
        Frame::Started {
            run_id: "r-1".into(),
        },
        Frame::Refused {
            run_id: "r-1".into(),
            reason: Refusal::BaseMismatch,
            detail: "ledger says c0ffee".into(),
        },
        Frame::Event {
            run_id: "r-1".into(),
            seq: 7,
            json: r#"{"v":1,"event":"task_start"}"#.into(),
        },
        Frame::Log {
            run_id: "r-1".into(),
            seq: 8,
            stream: LogStream::Stderr,
            bytes: b"PLAY [all]\n".to_vec(),
        },
        Frame::Cancel {
            run_id: "r-1".into(),
        },
        Frame::Finished {
            run_id: "r-1".into(),
            exit_code: 2,
            stats,
            ledger_line: "{\"seq\":1}".into(),
        },
        Frame::Ping,
        Frame::Pong,
        Frame::ReadLedger,
        Frame::Ledger {
            ledger_head: None,
            bases: vec![Base {
                job: "site".into(),
                commit: "c".repeat(40),
            }],
        },
    ]
}

#[tokio::test]
async fn every_frame_round_trips_through_the_codec() {
    for frame in every_frame() {
        let mut wire = Vec::new();
        codec::write_frame(&mut wire, &frame).await.unwrap();
        // u32 big-endian length prefix, then exactly that many bytes of JSON.
        let len = u32::from_be_bytes(wire[..4].try_into().unwrap()) as usize;
        assert_eq!(len, wire.len() - 4, "{}", frame.name());
        let mut reader = &wire[..];
        let back = codec::read_frame(&mut reader)
            .await
            .unwrap()
            .expect("one frame");
        assert_eq!(back, frame);
        assert!(
            codec::read_frame(&mut reader).await.unwrap().is_none(),
            "clean EOF after the frame"
        );
    }
}

#[test]
fn every_frame_names_a_direction_and_a_subsystem() {
    for frame in every_frame() {
        let name = frame.name();
        match name {
            "Enroll" | "Enrolled" => assert_eq!(frame.subsystem(), Some(Subsystem::Enroll)),
            "ReadLedger" | "Ledger" => assert_eq!(frame.subsystem(), None, "local socket only"),
            _ => assert_eq!(frame.subsystem(), Some(Subsystem::Runner), "{name}"),
        }
        let dir = frame.direction();
        match name {
            "Hello" | "Started" | "Refused" | "Event" | "Log" | "Finished" | "Enroll" => {
                assert_eq!(dir, Direction::RunnerToServer, "{name}")
            }
            "Assign" | "Pack" | "Secrets" | "Approvals" | "Cancel" | "Krl" | "Cert"
            | "Enrolled" => {
                assert_eq!(dir, Direction::ServerToRunner, "{name}")
            }
            "RenewCert" | "Ping" | "Pong" => assert_eq!(dir, Direction::Both, "{name}"),
            "ReadLedger" => assert_eq!(dir, Direction::RunnerToServer),
            "Ledger" => assert_eq!(dir, Direction::ServerToRunner),
            other => panic!("unexpected frame {other}"),
        }
    }
}

#[test]
fn wire_shape_is_tagged_json_with_base64_bytes() {
    let json = codec::encode_body(&Frame::Krl {
        bytes: b"SSHKRL\n\0".to_vec(),
    })
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["frame"], "Krl");
    assert_eq!(v["bytes"], "U1NIS1JMCgA=");

    let json = codec::encode_body(&Frame::Ping).unwrap();
    assert_eq!(std::str::from_utf8(&json).unwrap(), r#"{"frame":"Ping"}"#);

    let json = codec::encode_body(&Frame::Refused {
        run_id: "r".into(),
        reason: Refusal::VersionTooOld,
        detail: String::new(),
    })
    .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["reason"], "version_too_old");
}

#[test]
fn every_refusal_value_of_the_data_model_is_spelled_as_in_run_hosts() {
    let all = [
        (Refusal::Unsigned, "unsigned"),
        (Refusal::BadOverride, "bad_override"),
        (Refusal::BaseMismatch, "base_mismatch"),
        (Refusal::ProfileNotAllowed, "profile_not_allowed"),
        (Refusal::Downgrade, "downgrade"),
        (Refusal::Replay, "replay"),
        (Refusal::LedgerCorrupt, "ledger_corrupt"),
        (Refusal::VersionTooOld, "version_too_old"),
    ];
    for (r, s) in all {
        assert_eq!(
            serde_json::to_value(r).unwrap(),
            serde_json::Value::String(s.into())
        );
        assert_eq!(r.as_str(), s);
    }
}

#[test]
fn unknown_frames_and_unknown_fields_are_refused() {
    assert!(codec::decode_body(br#"{"frame":"Shell","cmd":"id"}"#).is_err());
    assert!(codec::decode_body(br#"{"frame":"Started","run_id":"r","extra":1}"#).is_err());
    assert!(codec::decode_body(br#"{"run_id":"r"}"#).is_err());
    assert!(codec::decode_body(b"not json").is_err());
    assert!(
        codec::decode_body(br#"{"frame":"Refused","run_id":"r","reason":"maybe","detail":""}"#)
            .is_err()
    );
}

#[test]
fn secrets_and_enroll_tokens_never_reach_debug_output() {
    let secrets = Frame::Secrets {
        run_id: "r-1".into(),
        items: vec![SecretItem {
            name: "db_password".into(),
            kind: "password".into(),
            value: "hunter2".into(),
        }],
    };
    let shown = format!("{secrets:?} {:#?}", secrets);
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(
        shown.contains("db_password"),
        "names are fine to show: {shown}"
    );

    let enroll = Frame::Enroll {
        token: "keel-enroll-v1.s3cr3t.SHA256:abc".into(),
        inventory_name: "web-1".into(),
        allowed_profiles: vec![],
        require_signed: false,
    };
    let shown = format!("{enroll:?} {:#?}", enroll);
    assert!(!shown.contains("s3cr3t"), "{shown}");

    let pack = Frame::Pack {
        run_id: "r".into(),
        chunk: vec![7; 4096],
        last: false,
        sha256: None,
    };
    assert!(
        format!("{pack:?}").len() < 200,
        "binary payloads are summarised, not dumped"
    );
}

#[tokio::test]
async fn the_codec_refuses_frames_over_16_mib() {
    assert_eq!(MAX_FRAME_LEN, 16 * 1024 * 1024);

    // Reading: a length prefix past the cap is refused before any body is read or allocated.
    let mut wire = ((MAX_FRAME_LEN + 1) as u32).to_be_bytes().to_vec();
    wire.extend_from_slice(b"{}");
    let err = codec::read_frame(&mut &wire[..]).await.unwrap_err();
    assert!(matches!(err, CodecError::TooLarge { .. }), "{err:?}");

    // Exactly at the cap the prefix is accepted (the body then fails to parse or is truncated).
    let wire = (MAX_FRAME_LEN as u32).to_be_bytes().to_vec();
    let err = codec::read_frame(&mut &wire[..]).await.unwrap_err();
    assert!(matches!(err, CodecError::Truncated), "{err:?}");

    // Writing: a frame whose JSON exceeds the cap is refused and nothing is written.
    let huge = Frame::Event {
        run_id: "r".into(),
        seq: 1,
        json: "x".repeat(MAX_FRAME_LEN),
    };
    let mut out = Vec::new();
    let err = codec::write_frame(&mut out, &huge).await.unwrap_err();
    assert!(matches!(err, CodecError::TooLarge { .. }), "{err:?}");
    assert!(out.is_empty());

    // A frame just under the cap still goes through.
    let overhead = codec::encode_body(&Frame::Event {
        run_id: "r".into(),
        seq: 1,
        json: String::new(),
    })
    .unwrap()
    .len();
    let fits = Frame::Event {
        run_id: "r".into(),
        seq: 1,
        json: "x".repeat(MAX_FRAME_LEN - overhead),
    };
    let mut out = Vec::new();
    codec::write_frame(&mut out, &fits).await.unwrap();
    assert_eq!(out.len(), MAX_FRAME_LEN + 4);
    assert_eq!(codec::read_frame(&mut &out[..]).await.unwrap(), Some(fits));
}

#[tokio::test]
async fn a_stream_cut_mid_frame_is_an_error_not_eof() {
    let mut wire = Vec::new();
    codec::write_frame(&mut wire, &Frame::Ping).await.unwrap();
    wire.truncate(wire.len() - 1);
    let err = codec::read_frame(&mut &wire[..]).await.unwrap_err();
    assert!(matches!(err, CodecError::Truncated), "{err:?}");

    let err = codec::read_frame(&mut &[0u8, 0][..]).await.unwrap_err();
    assert!(matches!(err, CodecError::Truncated), "{err:?}");
}

#[tokio::test]
async fn frames_stream_back_to_back() {
    let (mut a, mut b) = tokio::io::duplex(1024);
    let sent = every_frame();
    let to_send = sent.clone();
    let writer = tokio::spawn(async move {
        for f in &to_send {
            codec::write_frame(&mut a, f).await.unwrap();
        }
    });
    let mut got = Vec::new();
    while let Some(f) = codec::read_frame(&mut b).await.unwrap() {
        got.push(f);
        if got.len() == sent.len() {
            break;
        }
    }
    writer.await.unwrap();
    assert_eq!(got, sent);
}
