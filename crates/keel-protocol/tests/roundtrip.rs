//! Round-trip tests for every envelope, command and event (spec 055 T012).

use keel_core::uuid::Uuid;
use keel_protocol::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(
    value: &T,
) -> serde_json::Value {
    let text = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&text).unwrap();
    assert_eq!(&back, value, "{text}");
    serde_json::from_str(&text).unwrap()
}

fn preview_id() -> PreviewId {
    PreviewId(Uuid::from_u128(0x0190_0000_0000_7000_8000_0000_0000_0001))
}

fn assertion() -> PasskeyAssertion {
    PasskeyAssertion {
        ceremony: CeremonyId(Uuid::from_u128(7)),
        credential: AssertionCredential {
            id: B64Url::new("Y3JlZA").unwrap(),
            raw_id: B64Url::new("Y3JlZA").unwrap(),
            response: AssertionResponse {
                authenticator_data: B64Url::new("YXV0aA").unwrap(),
                client_data_json: B64Url::new("Y2xpZW50").unwrap(),
                signature: B64Url::new("c2ln").unwrap(),
                user_handle: None,
            },
            kind: PublicKeyType::PublicKey,
            extensions: PasskeyExtensions::default(),
        },
    }
}

fn me() -> Me {
    Me {
        id: Uuid::from_u128(1),
        display_name: "Ada".into(),
        email: Some("ada@example.org".into()),
        instance_role: InstanceRole::Owner,
        grants: vec![MeGrant {
            workspace: Uuid::from_u128(2),
            workspace_slug: "infra".into(),
            role: TeamRole::WsAdmin,
            profiles: vec!["web".into()],
        }],
        verification: Verification {
            verified: false,
            at: None,
            window_secs: 300,
            enrolled: false,
            required: true,
            recovering: false,
        },
    }
}

#[test]
fn hello() {
    let h = Hello {
        protocol_version: PROTOCOL_VERSION,
        client: ClientName::new("keel-web/0.1.0").unwrap(),
        csrf_token: CsrfToken::new("abc-_123").unwrap(),
    };
    let v = round_trip(&h);
    assert_eq!(
        v,
        json!({"protocol_version": 1, "client": "keel-web/0.1.0", "csrf_token": "abc-_123"})
    );
}

#[test]
fn hello_refuses_unknown_fields_and_bad_values() {
    let ok = json!({"protocol_version": 1, "client": "x", "csrf_token": "t"});
    assert!(serde_json::from_value::<Hello>(ok.clone()).is_ok());
    let mut extra = ok.clone();
    extra["admin"] = json!(true);
    assert!(serde_json::from_value::<Hello>(extra).is_err());
    let mut bad_csrf = ok.clone();
    bad_csrf["csrf_token"] = json!("a b");
    assert!(serde_json::from_value::<Hello>(bad_csrf).is_err());
    let mut long_csrf = ok.clone();
    long_csrf["csrf_token"] = json!("a".repeat(129));
    assert!(serde_json::from_value::<Hello>(long_csrf).is_err());
    let mut bad_client = ok;
    bad_client["client"] = json!("\n");
    assert!(serde_json::from_value::<Hello>(bad_client).is_err());
}

#[test]
fn csrf_token_debug_is_redacted() {
    let t = CsrfToken::new("secret").unwrap();
    assert!(!format!("{t:?}").contains("secret"));
}

#[test]
fn cmd_envelope_get_me() {
    let env = CmdEnvelope {
        id: RequestId(42),
        cmd: Cmd::GetMe,
    };
    assert_eq!(
        round_trip(&env),
        json!({"id": 42, "cmd": {"type": "GetMe"}})
    );
}

#[test]
fn cmd_confirm_with_and_without_step_up() {
    let bare = CmdEnvelope {
        id: RequestId(1),
        cmd: Cmd::Confirm {
            preview_id: preview_id(),
            step_up: None,
        },
    };
    let v = round_trip(&bare);
    assert_eq!(v["cmd"]["type"], "Confirm");
    assert!(v["cmd"]["data"].get("step_up").is_none());

    let stepped = CmdEnvelope {
        id: RequestId(2),
        cmd: Cmd::Confirm {
            preview_id: preview_id(),
            step_up: Some(assertion()),
        },
    };
    let v = round_trip(&stepped);
    let cred = &v["cmd"]["data"]["step_up"]["credential"];
    assert_eq!(cred["type"], "public-key");
    assert_eq!(cred["response"]["clientDataJSON"], "Y2xpZW50");
}

#[test]
fn cmds_refuse_unknown_variants_and_fields() {
    assert!(
        serde_json::from_value::<CmdEnvelope>(json!({"id": 1, "cmd": {"type": "Nope"}})).is_err()
    );
    assert!(
        serde_json::from_value::<CmdEnvelope>(json!({"id": 1, "cmd": {"type": "GetMe"}, "x": 1}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<CmdEnvelope>(json!({
            "id": 1,
            "cmd": {"type": "Confirm", "data": {"preview_id": preview_id().0.to_string(), "force": true}}
        }))
        .is_err()
    );
}

#[test]
fn cmd_names_and_streaming() {
    assert_eq!(Cmd::GetMe.name(), "get_me");
    assert!(!Cmd::GetMe.is_streaming());
    assert_eq!(
        Cmd::Confirm {
            preview_id: preview_id(),
            step_up: None
        }
        .name(),
        "confirm"
    );
}

#[test]
fn event_envelope_me() {
    let env = EventEnvelope::reply(RequestId(42), Event::Me(me()));
    let v = round_trip(&env);
    assert_eq!(v["reply_to"], 42);
    assert_eq!(v["event"]["type"], "Me");
    assert_eq!(v["event"]["data"]["instance_role"], "owner");
    assert_eq!(v["event"]["data"]["grants"][0]["role"], "ws_admin");
}

#[test]
fn pushed_event_has_no_reply_to() {
    let v = round_trip(&EventEnvelope::push(Event::Revoked));
    assert_eq!(v, json!({"event": {"type": "Revoked"}}));
}

#[test]
fn preview() {
    let p = Event::Preview(Preview {
        preview_id: preview_id(),
        action: "workspace_create".into(),
        summary: "Create workspace infra".into(),
        affected: vec!["workspace infra".into()],
        step_up: true,
        expires_at: "2026-10-27T12:05:00Z".into(),
    });
    let v = round_trip(&p);
    assert_eq!(v["type"], "Preview");
    assert_eq!(v["data"]["step_up"], true);
}

#[test]
fn confirmed_denied_op_error_revoked() {
    round_trip(&Event::Confirmed {
        preview_id: preview_id(),
    });
    let v = round_trip(&Event::Denied {
        action: "get_me".into(),
        reason: "no_grant".into(),
    });
    assert_eq!(
        v,
        json!({"type": "Denied", "data": {"action": "get_me", "reason": "no_grant"}})
    );
    let v = round_trip(&Event::op_error("preview_expired", "The preview expired."));
    assert_eq!(v["data"]["code"], "preview_expired");
    assert_eq!(round_trip(&Event::Revoked), json!({"type": "Revoked"}));
}

// ---- Passkeys (T028): the same commands, events and shapes as depths-protocol (054) ----------

#[test]
fn passkey_commands_round_trip_as_in_depths() {
    let v = round_trip(&Cmd::GetVerification);
    assert_eq!(v, json!({"type": "GetVerification"}));

    let v = round_trip(&Cmd::PasskeyBegin {
        purpose: PasskeyPurpose::Register {
            name: PasskeyName::new("laptop").unwrap(),
        },
    });
    assert_eq!(
        v,
        json!({"type": "PasskeyBegin", "data": {"purpose": {"kind": "register", "name": "laptop"}}})
    );
    let v = round_trip(&Cmd::PasskeyBegin {
        purpose: PasskeyPurpose::Verify,
    });
    assert_eq!(
        v,
        json!({"type": "PasskeyBegin", "data": {"purpose": {"kind": "verify"}}})
    );

    assert_eq!(
        round_trip(&Cmd::ListPasskeys),
        json!({"type": "ListPasskeys"})
    );
    assert_eq!(
        round_trip(&Cmd::PreviewPasskeyRemove {
            id: B64Url::new("Y3JlZA").unwrap()
        }),
        json!({"type": "PreviewPasskeyRemove", "data": {"id": "Y3JlZA"}})
    );
    assert_eq!(
        round_trip(&Cmd::RecoverPasskeys {
            code: RecoveryCode::new("ABCDEFGHJK").unwrap()
        }),
        json!({"type": "RecoverPasskeys", "data": {"code": "ABCDEFGHJK"}})
    );
}

#[test]
fn passkey_finish_tells_attestation_from_assertion() {
    let reg = json!({
        "type": "PasskeyFinish",
        "data": {
            "ceremony": Uuid::from_u128(9).to_string(),
            "credential": {
                "id": "Y3JlZA",
                "rawId": "Y3JlZA",
                "response": {
                    "attestationObject": "YXR0",
                    "clientDataJSON": "Y2xpZW50",
                    "transports": ["internal", "hybrid"]
                },
                "type": "public-key",
                "extensions": {"credProps": {"rk": true}}
            }
        }
    });
    let cmd: Cmd = serde_json::from_value(reg.clone()).unwrap();
    let Cmd::PasskeyFinish(PasskeyResponse {
        credential: PasskeyCredential::Attestation(a),
        ..
    }) = &cmd
    else {
        panic!("not an attestation: {cmd:?}");
    };
    assert_eq!(a.response.transports.len(), 2);
    assert_eq!(round_trip(&cmd), reg);

    let cmd = Cmd::PasskeyFinish(PasskeyResponse {
        ceremony: CeremonyId(Uuid::from_u128(9)),
        credential: PasskeyCredential::Assertion(assertion().credential),
    });
    let v = round_trip(&cmd);
    assert_eq!(v["data"]["credential"]["response"]["signature"], "c2ln");
    assert_eq!(cmd.name(), "passkey_finish");
}

#[test]
fn passkey_inputs_are_bounded() {
    assert!(PasskeyName::new("").is_err());
    assert!(PasskeyName::new(" padded").is_err());
    assert!(PasskeyName::new("tab\there").is_err());
    assert!(PasskeyName::new("x".repeat(65)).is_err());
    assert!(PasskeyName::new("é".repeat(64)).is_ok());
    assert!(RecoveryCode::new("SHORT").is_err());
    assert!(RecoveryCode::new("ABCD-EFGH-JK").is_err());
    assert!(RecoveryCode::new("A".repeat(33)).is_err());
    let too_many = json!({
        "attestationObject": "YQ", "clientDataJSON": "YQ",
        "transports": ["usb", "usb", "usb", "usb", "usb", "usb", "usb", "usb", "usb"]
    });
    assert!(serde_json::from_value::<AttestationResponse>(too_many).is_err());
    let bad = json!({"attestationObject": "YQ", "clientDataJSON": "YQ", "transports": ["USB!"]});
    assert!(serde_json::from_value::<AttestationResponse>(bad).is_err());
    // The register purpose takes no extra fields.
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "PasskeyBegin",
            "data": {"purpose": {"kind": "register", "name": "a", "x": 1}}
        }))
        .is_err()
    );
}

#[test]
fn passkey_command_names() {
    assert_eq!(Cmd::GetVerification.name(), "get_verification");
    assert_eq!(
        Cmd::PasskeyBegin {
            purpose: PasskeyPurpose::Verify
        }
        .name(),
        "passkey_begin"
    );
    assert_eq!(Cmd::ListPasskeys.name(), "list_passkeys");
    assert_eq!(
        Cmd::PreviewPasskeyRemove {
            id: B64Url::new("YQ").unwrap()
        }
        .name(),
        "preview_passkey_remove"
    );
    assert_eq!(
        Cmd::RecoverPasskeys {
            code: RecoveryCode::new("ABCDEFGHJK").unwrap()
        }
        .name(),
        "recover_passkeys"
    );
    assert!(!Cmd::ListPasskeys.is_streaming());
}

#[test]
fn recovery_code_debug_is_redacted() {
    let code = RecoveryCode::new("ABCDEFGHJK").unwrap();
    assert!(!format!("{code:?}").contains("ABCDEFGHJK"));
}

#[test]
fn passkey_events_round_trip_as_in_depths() {
    let v = round_trip(&Event::PasskeyChallenge(PasskeyRequest {
        ceremony: CeremonyId(Uuid::from_u128(9)),
        options: json!({"challenge": "Y2g", "rpId": "keel.example.com"}),
    }));
    assert_eq!(v["type"], "PasskeyChallenge");
    assert_eq!(v["data"]["options"]["rpId"], "keel.example.com");

    let info = PasskeyInfo {
        id: "Y3JlZA".into(),
        name: "laptop".into(),
        created: "2026-10-27T12:00:00Z".into(),
        last_used: None,
    };
    let v = round_trip(&Event::PasskeyRegistered(info.clone()));
    assert_eq!(
        v,
        json!({"type": "PasskeyRegistered", "data": {
            "id": "Y3JlZA", "name": "laptop", "created": "2026-10-27T12:00:00Z", "last_used": null
        }})
    );
    let v = round_trip(&Event::Passkeys { items: vec![info] });
    assert_eq!(v["data"]["items"][0]["name"], "laptop");

    let v = round_trip(&Event::Verification(me().verification));
    assert_eq!(v["type"], "Verification");
    assert_eq!(v["data"]["window_secs"], 300);

    assert_eq!(
        round_trip(&Event::PasskeyEnrolmentRequired),
        json!({"type": "PasskeyEnrolmentRequired"})
    );
    for (reason, wire) in [
        (PasskeyRefusedReason::Replayed, "replayed"),
        (PasskeyRefusedReason::WrongOrigin, "wrong_origin"),
        (
            PasskeyRefusedReason::UnknownCredential,
            "unknown_credential",
        ),
        (PasskeyRefusedReason::Expired, "expired"),
        (PasskeyRefusedReason::Cancelled, "cancelled"),
        (PasskeyRefusedReason::RecoveryInvalid, "recovery_invalid"),
        (PasskeyRefusedReason::Invalid, "invalid"),
        (PasskeyRefusedReason::LastPasskey, "last_passkey"),
    ] {
        assert_eq!(
            round_trip(&Event::PasskeyRefused { reason }),
            json!({"type": "PasskeyRefused", "data": {"reason": wire}})
        );
    }
}
