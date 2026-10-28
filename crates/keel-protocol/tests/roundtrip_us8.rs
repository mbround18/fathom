//! Round-trip tests for the signed-mode review, override, baseline and signer commands and
//! events (spec 055 T094).

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

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn comment(s: &str) -> ReviewComment {
    ReviewComment::new(s).unwrap()
}

fn sha(c: char) -> String {
    c.to_string().repeat(40)
}

fn review_info() -> ReviewInfo {
    ReviewInfo {
        id: id(1),
        run: id(2),
        workspace: id(3),
        binding: Some(id(4)),
        job_name: "site".into(),
        base: Some(sha('a')),
        target: sha('b'),
        hosts: vec!["web-1".into(), "web-2".into()],
        reason: ReviewReason::Unsigned,
        state: ReviewState::Open,
        approvals: 0,
        quorum: 2,
        four_eyes: true,
        flagged: 1,
        created_at: "2026-10-27T10:00:00Z".into(),
    }
}

fn all_commands() -> Vec<Cmd> {
    vec![
        Cmd::ListReviews {
            workspace: Some(id(3)),
            state: Some(ReviewState::Open),
        },
        Cmd::ListReviews {
            workspace: None,
            state: None,
        },
        Cmd::GetReview { group: id(1) },
        Cmd::GetReviewDiff {
            group: id(1),
            path: None,
            page: None,
        },
        Cmd::GetReviewDiff {
            group: id(1),
            path: Some(RepoPath::new(".keel/manifest.yml").unwrap()),
            page: Some(2),
        },
        Cmd::PreviewOverride {
            group: id(1),
            comment: comment("Hotfix, reviewed in person.\nTicket OPS-1"),
        },
        Cmd::PreviewOverrideDeny {
            group: id(1),
            comment: comment("Not reviewed"),
        },
        Cmd::PreviewBaseline {
            binding: id(4),
            job: id(5),
            hosts: BaselineHosts(vec![
                HostName::new("web-1").unwrap(),
                HostName::new("web-2").unwrap(),
            ]),
            commit: CommitSha::new(sha('c')).unwrap(),
            comment: comment("First run"),
        },
        Cmd::ListSigners { workspace: id(3) },
        Cmd::PreviewSignerAdd {
            workspace: id(3),
            kind: SignerKind::Ssh,
            public_key: SignerKey::new("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA").unwrap(),
            label: Label::new("Ada's laptop").unwrap(),
            user: Some(id(6)),
        },
        Cmd::PreviewSignerRemove { signer: id(7) },
    ]
}

#[test]
fn review_and_signer_commands() {
    for cmd in &all_commands() {
        round_trip(cmd);
    }
    let v = round_trip(&Cmd::PreviewOverride {
        group: id(1),
        comment: comment("ok"),
    });
    assert_eq!(
        v,
        json!({"type": "PreviewOverride", "data": {"group": id(1).to_string(), "comment": "ok"}})
    );
    // Optional fields may be left out.
    let c: Cmd = serde_json::from_value(json!({"type": "ListReviews", "data": {}})).unwrap();
    assert_eq!(
        c,
        Cmd::ListReviews {
            workspace: None,
            state: None
        }
    );
    let c: Cmd = serde_json::from_value(
        json!({"type": "GetReviewDiff", "data": {"group": id(1).to_string()}}),
    )
    .unwrap();
    assert_eq!(
        c,
        Cmd::GetReviewDiff {
            group: id(1),
            path: None,
            page: None
        }
    );
    let c: Cmd = serde_json::from_value(json!({"type": "PreviewSignerAdd", "data": {
        "workspace": id(3).to_string(), "kind": "openpgp",
        "public_key": "-----BEGIN PGP PUBLIC KEY BLOCK-----", "label": "ci"
    }}))
    .unwrap();
    assert!(matches!(
        c,
        Cmd::PreviewSignerAdd {
            user: None,
            kind: SignerKind::Openpgp,
            ..
        }
    ));
    // Signed-mode toggling stays the US1 command.
    let v = round_trip(&Cmd::PreviewSignedMode {
        binding: id(4),
        on: false,
    });
    assert_eq!(
        v,
        json!({"type": "PreviewSignedMode", "data": {"binding": id(4).to_string(), "on": false}})
    );
}

#[test]
fn only_reads_are_token_actions() {
    for name in [
        "list_reviews",
        "get_review",
        "get_review_diff",
        "list_signers",
    ] {
        assert!(TOKEN_ACTIONS.contains(&name), "{name}");
    }
    for c in &all_commands() {
        assert!(!c.is_streaming(), "{}", c.name());
        if c.name().starts_with("preview_") {
            assert!(!TOKEN_ACTIONS.contains(&c.name()), "{}", c.name());
        }
    }
    let names: Vec<&str> = all_commands().iter().map(Cmd::name).collect();
    for n in [
        "preview_override",
        "preview_override_deny",
        "preview_baseline",
        "preview_signer_add",
        "preview_signer_remove",
    ] {
        assert!(names.contains(&n), "{n}");
    }
}

#[test]
fn inputs_are_bounded_and_unknown_fields_refused() {
    // Comments are required and bounded.
    assert!(ReviewComment::new("").is_err());
    assert!(ReviewComment::new("   \n").is_err());
    assert!(ReviewComment::new("a\u{7}b").is_err());
    assert!(ReviewComment::new("x".repeat(MAX_REVIEW_COMMENT_CHARS + 1)).is_err());
    assert!(ReviewComment::new("x".repeat(MAX_REVIEW_COMMENT_CHARS)).is_ok());
    let over = |c: serde_json::Value| {
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewOverride", "data": {"group": id(1).to_string(), "comment": c}
        }))
    };
    assert!(over(json!("fine")).is_ok());
    assert!(over(json!("")).is_err());
    assert!(over(json!(null)).is_err());
    assert!(
        serde_json::from_value::<Cmd>(
            json!({"type": "PreviewOverride", "data": {"group": id(1).to_string()}})
        )
        .is_err(),
        "comment missing"
    );
    // Commits are full ids.
    assert!(CommitSha::new("abc").is_err());
    assert!(CommitSha::new("A".repeat(40)).is_err());
    assert!(CommitSha::new("a".repeat(41)).is_err());
    assert!(CommitSha::new("a".repeat(64)).is_ok());
    // Paths and keys.
    assert!(RepoPath::new("").is_err());
    assert!(RepoPath::new("a\nb").is_err());
    assert!(RepoPath::new("x".repeat(MAX_REPO_PATH_BYTES + 1)).is_err());
    assert!(SignerKey::new(" ").is_err());
    assert!(SignerKey::new("k".repeat(MAX_SIGNER_KEY_BYTES + 1)).is_err());
    // Baseline hosts: no duplicates, bounded.
    let baseline = |hosts: serde_json::Value| {
        serde_json::from_value::<Cmd>(json!({"type": "PreviewBaseline", "data": {
            "binding": id(4).to_string(), "job": id(5).to_string(), "hosts": hosts,
            "commit": sha('c'), "comment": "first"
        }}))
    };
    assert!(baseline(json!(["web-1"])).is_ok());
    assert!(baseline(json!(["web-1", "web-1"])).is_err());
    assert!(baseline(json!(["bad host"])).is_err());
    let many: Vec<String> = (0..=MAX_BASELINE_HOSTS).map(|i| format!("h{i}")).collect();
    assert!(baseline(json!(many)).is_err());
    // Unknown fields and values.
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "GetReview", "data": {"group": id(1).to_string(), "extra": 1}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "ListReviews", "data": {"state": "pending"}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Cmd>(json!({
            "type": "PreviewSignerAdd", "data": {
                "workspace": id(3).to_string(), "kind": "x509", "public_key": "k", "label": "l"
            }
        }))
        .is_err()
    );
    // The key is never printed.
    let key = SignerKey::new("ssh-ed25519 AAAASECRETLOOKING").unwrap();
    assert!(!format!("{key:?}").contains("AAAA"));
}

#[test]
fn review_events() {
    let events = vec![
        Event::Reviews {
            items: vec![review_info()],
        },
        Event::Review {
            review: ReviewInfo {
                base: None,
                reason: ReviewReason::Baseline,
                state: ReviewState::Approved,
                approvals: 2,
                ..review_info()
            },
            commits: vec![
                ReviewCommit {
                    id: sha('b'),
                    author: "Ada <ada@example.com>".into(),
                    committer: "Ada <ada@example.com>".into(),
                    summary: "Open port 443".into(),
                    status: SignatureStatus::Missing,
                    signer: None,
                },
                ReviewCommit {
                    id: sha('d'),
                    author: "Bob <bob@example.com>".into(),
                    committer: "Bob <bob@example.com>".into(),
                    summary: "Signed change".into(),
                    status: SignatureStatus::Good,
                    signer: Some("bob@example.com".into()),
                },
            ],
            flags: vec![
                ReviewFlag {
                    kind: ReviewFlagKind::KeelConfig,
                    path: ".keel/manifest.yml".into(),
                },
                ReviewFlag {
                    kind: ReviewFlagKind::RunnerVars,
                    path: "group_vars/all.yml".into(),
                },
            ],
            decisions: vec![ReviewDecision {
                approver: "carol@example.com".into(),
                approve: true,
                comment: Some("checked".into()),
                at: "2026-10-27T10:05:00Z".into(),
                expires_at: Some("2026-10-28T10:05:00Z".into()),
            }],
        },
        Event::ReviewDiff {
            group: id(1),
            files: vec![DiffFile {
                path: "inventory/hosts.yml".into(),
                status: DiffStatus::Modified,
                flags: vec![ReviewFlagKind::Inventory],
            }],
            page: Some(DiffPage {
                path: "inventory/hosts.yml".into(),
                page: 0,
                pages: 3,
                lines: vec!["@@ -1,2 +1,2 @@".into(), "-a".into(), "+b".into()],
                binary: false,
            }),
        },
        Event::ReviewDiff {
            group: id(1),
            files: vec![],
            page: None,
        },
        Event::OverrideChallenge(OverrideChallenge {
            preview_id: PreviewId(id(9)),
            request: PasskeyRequest {
                ceremony: CeremonyId(id(10)),
                options: json!({"challenge": "abc", "rpId": "keel.example"}),
            },
            canonical: "{\"v\":\"keel-override-v1\"}".into(),
            expires_at: "2026-10-28T10:00:00Z".into(),
        }),
        Event::ReviewRequested {
            group: id(1),
            run: id(2),
            workspace: id(3),
        },
        Event::Signers {
            workspace: id(3),
            items: vec![SignerInfo {
                id: id(7),
                kind: SignerKind::Openpgp,
                fingerprint: "0123456789ABCDEF0123456789ABCDEF01234567".into(),
                label: "release key".into(),
                user: Some("ada@example.com".into()),
                added_by: Some("admin@example.com".into()),
                added_at: "2026-10-27T09:00:00Z".into(),
            }],
        },
    ];
    for e in &events {
        round_trip(e);
    }
    let v = round_trip(&Event::ReviewRequested {
        group: id(1),
        run: id(2),
        workspace: id(3),
    });
    assert_eq!(
        v,
        json!({"type": "ReviewRequested", "data": {
            "group": id(1).to_string(), "run": id(2).to_string(), "workspace": id(3).to_string()
        }})
    );
    let v = round_trip(&events[4]);
    assert_eq!(v["type"], "OverrideChallenge");
    assert_eq!(v["data"]["preview_id"], id(9).to_string());
    assert_eq!(
        round_trip(&SignatureStatus::UnknownKey),
        json!("unknown_key")
    );
    assert_eq!(
        round_trip(&ReviewReason::CheckFailed),
        json!("check_failed")
    );
}
