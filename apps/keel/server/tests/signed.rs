//! Signed mode (spec 055 US8, T092): Keel's early check holds a run in `needs_review` at sync
//! time with the failing commits listed, grouping hosts by base; an override needs a passkey
//! assertion over its own challenge and a comment, and four-eyes refuses the requester; a
//! denial denies the run; trusted signers let signed commits through; turning signed mode off
//! needs step-up and is audited; and signed mode refuses a managed inventory.

mod common;
mod repos;

use std::path::Path;

use keel_protocol::{
    AssertionCredential, AttestationCredential, Cmd, DiffStatus, Event, OverrideChallenge,
    PasskeyAssertion, PasskeyCredential, PasskeyName, PasskeyPurpose, PasskeyResponse, RepoPath,
    ReviewComment, ReviewFlagKind, ReviewReason, ReviewState, SignatureStatus, SignerKey,
    SignerKind, governance::Label as SignerLabel,
};
use repos::{Harness, Source, bind, git, op_error, preview};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse, Url};

const ORIGIN: &str = repos::ORIGIN;
const INVENTORY: &str = "inventories/prod/hosts.yaml";
const HOSTS: &str = "web:\n  hosts:\n    web-1:\n    web-2:\n";
/// The `web` job runs on pushes under `playbooks/`, against the repository inventory `prod`.
const MANIFEST: &str = "version: 1\nprofiles: { debian: {} }\ninventories:\n  prod:\n    \
                        path: inventories/prod/hosts.yaml\njobs:\n  web:\n    \
                        playbook: playbooks/web.yaml\n    profiles: [debian]\n    \
                        inventory: prod\n    paths: ['playbooks/**']\n";

type Authenticator = WebauthnAuthenticator<SoftPasskey>;

struct Fixture {
    h: Harness,
    src: Source,
    binding: Uuid,
    hosts: Vec<(String, Uuid)>,
}

impl Fixture {
    /// A signed-mode binding whose `web` job reaches the enrolled hosts web-1 and web-2, under a
    /// policy that needs no approval (`four_eyes` as given).
    async fn new(test: &str, four_eyes: bool) -> Option<Self> {
        let h = Harness::new(test).await?;
        let src = Source::new(MANIFEST);
        src.commit(INVENTORY, HOSTS, "inventory");
        src.push();
        let binding = bind(&h, &src).await;
        let owner = &h.db.owner;
        let mut hosts = Vec::new();
        for name in ["web-1", "web-2"] {
            let id = Uuid::now_v7();
            sqlx::query(
                "insert into hosts (id, workspace_id, inventory_name, allowed_profiles) \
                 values ($1, $2, $3, '{debian}')",
            )
            .bind(id)
            .bind(h.workspace)
            .bind(name)
            .execute(owner)
            .await
            .expect("host");
            sqlx::query(
                "insert into host_profiles (host_id, profile_id) \
                 select $1, id from profiles where workspace_id = $2 and name = 'debian'",
            )
            .bind(id)
            .bind(h.workspace)
            .execute(owner)
            .await
            .expect("host profile");
            hosts.push((name.to_owned(), id));
        }
        sqlx::query(
            "insert into approval_policies (id, workspace_id, priority, requires_approval, \
             four_eyes) values ($1, $2, 1, false, $3)",
        )
        .bind(Uuid::now_v7())
        .bind(h.workspace)
        .bind(four_eyes)
        .execute(owner)
        .await
        .expect("policy");
        h.confirm(Cmd::PreviewSignedMode { binding, on: true })
            .await;
        Some(Self {
            h,
            src,
            binding,
            hosts,
        })
    }

    async fn sync(&self) {
        let out = self
            .h
            .cmd(Cmd::SyncBinding {
                binding: self.binding,
            })
            .await;
        assert!(
            matches!(out.as_slice(), [Event::BindingSynced { .. }]),
            "{out:?}"
        );
    }

    /// Commits `file`, pushes and syncs; returns the new head.
    async fn push(&self, file: &str, content: &str) -> String {
        let head = self.src.commit(file, content, "change");
        self.src.push();
        self.sync().await;
        head
    }

    /// The newest run: id, state and commit.
    async fn last_run(&self) -> (Uuid, String, String) {
        sqlx::query_as("select id, state, commit from runs order by created_at desc limit 1")
            .fetch_one(&self.h.db.owner)
            .await
            .expect("run")
    }

    async fn run_state(&self, run: Uuid) -> String {
        let (s,): (String,) = sqlx::query_as("select state from runs where id = $1")
            .bind(run)
            .fetch_one(&self.h.db.owner)
            .await
            .expect("run");
        s
    }

    /// The run's review groups: id, base, hosts, reason, by base (none last).
    async fn groups(&self, run: Uuid) -> Vec<(Uuid, Option<String>, Vec<String>, String)> {
        sqlx::query_as(
            "select id, base_commit, hosts, reason from review_groups where run_id = $1 \
             order by base_commit nulls last",
        )
        .bind(run)
        .fetch_all(&self.h.db.owner)
        .await
        .expect("groups")
    }

    /// Records that `host` applied `commit` for the job, as the run's ledger would.
    async fn applied(&self, host: &str, commit: &str, run: Uuid) {
        let id = self.host(host);
        sqlx::query(
            "insert into host_revisions (id, host_id, commit, run_id, authorization_kind, outcome) \
             values ($1, $2, $3, $4, 'signed', 'succeeded')",
        )
        .bind(Uuid::now_v7())
        .bind(id)
        .bind(commit)
        .bind(run)
        .execute(&self.h.db.owner)
        .await
        .expect("host revision");
    }

    fn host(&self, name: &str) -> Uuid {
        self.hosts
            .iter()
            .find(|h| h.0 == name)
            .map(|h| h.1)
            .expect("host")
    }

    async fn review(&self, group: Uuid) -> Event {
        let out = self.h.cmd(Cmd::GetReview { group }).await;
        match out.as_slice() {
            [e @ Event::Review { .. }] => e.clone(),
            other => panic!("review: {other:?}"),
        }
    }

    /// Registers the owner's first passkey (no check needed for the first).
    async fn passkey(&self) -> Authenticator {
        let mut auth = WebauthnAuthenticator::new(SoftPasskey::new(true));
        let out = self
            .h
            .cmd(Cmd::PasskeyBegin {
                purpose: PasskeyPurpose::Register {
                    name: PasskeyName::new("laptop").expect("name"),
                },
            })
            .await;
        let [Event::PasskeyChallenge(r)] = out.as_slice() else {
            panic!("challenge: {out:?}");
        };
        let credential = attest(&mut auth, &r.options);
        let out = self
            .h
            .cmd(Cmd::PasskeyFinish(PasskeyResponse {
                ceremony: r.ceremony,
                credential: PasskeyCredential::Attestation(credential),
            }))
            .await;
        assert!(
            !out.iter().any(|e| matches!(e, Event::OpError { .. })),
            "{out:?}"
        );
        auth
    }

    /// Previews an override of `group`: the preview and the challenge.
    async fn preview_override(&self, group: Uuid) -> (keel_protocol::Preview, OverrideChallenge) {
        let out = self
            .h
            .cmd(Cmd::PreviewOverride {
                group,
                comment: comment("Reviewed the diff with the on-call lead."),
            })
            .await;
        match out.as_slice() {
            [Event::Preview(p), Event::OverrideChallenge(c)] => (p.clone(), c.clone()),
            other => panic!("override preview: {other:?}"),
        }
    }
}

fn comment(text: &str) -> ReviewComment {
    ReviewComment::new(text).expect("comment")
}

fn browser_json(mut v: Value) -> Value {
    let obj = v.as_object_mut().expect("credential object");
    obj.remove("extensions");
    let response = obj["response"].as_object_mut().expect("response");
    response.retain(|_, f| !f.is_null());
    v
}

fn attest(auth: &mut Authenticator, options: &Value) -> AttestationCredential {
    let ccr: CreationChallengeResponse =
        serde_json::from_value(json!({ "publicKey": options })).expect("creation options");
    let reg = auth
        .do_registration(Url::parse(ORIGIN).unwrap(), ccr)
        .expect("softpasskey registers");
    serde_json::from_value(browser_json(serde_json::to_value(reg).unwrap()))
        .expect("protocol attestation")
}

fn assert_with(auth: &mut Authenticator, options: &Value) -> AssertionCredential {
    let rcr: RequestChallengeResponse =
        serde_json::from_value(json!({ "publicKey": options })).expect("request options");
    let cred = auth
        .do_authentication(Url::parse(ORIGIN).unwrap(), rcr)
        .expect("softpasskey asserts");
    serde_json::from_value(browser_json(serde_json::to_value(cred).unwrap()))
        .expect("protocol assertion")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An ed25519 SSH key in `dir`: the private key's path and the public key line.
fn ssh_key(dir: &Path) -> (String, String) {
    let key = dir.join("signer");
    let out = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "signer", "-f"])
        .arg(&key)
        .output()
        .expect("ssh-keygen runs");
    assert!(out.status.success(), "{out:?}");
    let public = std::fs::read_to_string(dir.join("signer.pub")).expect("public key");
    (key.display().to_string(), public.trim().to_owned())
}

#[tokio::test]
async fn the_early_check_holds_unsigned_commits_and_groups_hosts_by_base() {
    let Some(f) = Fixture::new("signed_early_check", false).await else {
        return;
    };
    // The first signed-mode run: no host has a base, so every host needs a baseline review.
    let first = f
        .push("playbooks/web.yaml", "- hosts: all\n  tasks: []\n")
        .await;
    let (run1, state, commit) = f.last_run().await;
    assert_eq!(
        (state.as_str(), commit.as_str()),
        ("needs_review", first.as_str())
    );
    let groups = f.groups(run1).await;
    assert_eq!(groups.len(), 1, "{groups:?}");
    assert_eq!(groups[0].1, None);
    assert_eq!(groups[0].2, ["web-1", "web-2"]);
    assert_eq!(groups[0].3, "baseline");
    let Event::Review {
        review, commits, ..
    } = f.review(groups[0].0).await
    else {
        unreachable!()
    };
    assert_eq!(review.reason, ReviewReason::Baseline);
    assert_eq!((review.state, review.quorum), (ReviewState::Open, 1));
    assert_eq!(commits[0].id, first);
    assert_eq!(commits[0].status, SignatureStatus::Missing);
    assert!(commits.iter().all(|c| c.signer.is_none()));

    // web-1 applied the first commit; web-2 never ran.
    f.applied("web-1", &first, run1).await;

    // Two unsigned commits: one touches the inventory's vars, the next a keel_runner_ variable.
    let vars = f.src.commit(
        "inventories/prod/group_vars/web.yml",
        "tier: front\n",
        "vars",
    );
    let second = f
        .push(
            "playbooks/web.yaml",
            "- hosts: all\n  vars:\n    keel_runner_user: root\n",
        )
        .await;
    let (run2, state, _) = f.last_run().await;
    assert_eq!(state, "needs_review");
    let groups = f.groups(run2).await;
    assert_eq!(groups.len(), 2, "{groups:?}");
    assert_eq!(
        (
            groups[0].1.as_deref(),
            groups[0].2.clone(),
            groups[0].3.as_str()
        ),
        (Some(first.as_str()), vec!["web-1".to_owned()], "unsigned")
    );
    assert_eq!(
        (
            groups[1].1.as_deref(),
            groups[1].2.clone(),
            groups[1].3.as_str()
        ),
        (None, vec!["web-2".to_owned()], "baseline")
    );
    let base: (Option<String>,) = sqlx::query_as(
        "select base_commit from run_hosts where run_id = $1 and host_name = 'web-1'",
    )
    .bind(run2)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("run host");
    assert_eq!(base.0.as_deref(), Some(first.as_str()));

    // The web-1 group lists only the commits after its base, and flags what they touch.
    let group = groups[0].0;
    let Event::Review {
        review,
        commits,
        flags,
        ..
    } = f.review(group).await
    else {
        unreachable!()
    };
    let ids: Vec<&str> = commits.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, [second.as_str(), vars.as_str()]);
    assert!(commits.iter().all(|c| c.status == SignatureStatus::Missing));
    assert!(
        flags.iter().any(|f| f.kind == ReviewFlagKind::Inventory
            && f.path == "inventories/prod/group_vars/web.yml"),
        "{flags:?}"
    );
    assert!(
        flags
            .iter()
            .any(|f| f.kind == ReviewFlagKind::RunnerVars && f.path == "playbooks/web.yaml"),
        "{flags:?}"
    );
    assert_eq!(review.flagged as usize, flags.len());

    // The diff, file by file and page by page.
    let out =
        f.h.cmd(Cmd::GetReviewDiff {
            group,
            path: None,
            page: None,
        })
        .await;
    let [Event::ReviewDiff { files, page, .. }] = out.as_slice() else {
        panic!("diff: {out:?}");
    };
    assert!(page.is_none());
    assert_eq!(files.len(), 2, "{files:?}");
    let playbook = files
        .iter()
        .find(|f| f.path == "playbooks/web.yaml")
        .expect("playbook");
    assert_eq!(playbook.status, DiffStatus::Modified);
    assert_eq!(playbook.flags, [ReviewFlagKind::RunnerVars]);
    let out =
        f.h.cmd(Cmd::GetReviewDiff {
            group,
            path: Some(RepoPath::new("playbooks/web.yaml").expect("path")),
            page: Some(0),
        })
        .await;
    let [
        Event::ReviewDiff {
            page: Some(page), ..
        },
    ] = out.as_slice()
    else {
        panic!("diff page: {out:?}");
    };
    assert_eq!((page.page, page.pages, page.binary), (0, 1, false));
    assert!(
        page.lines
            .iter()
            .any(|l| l.starts_with('+') && l.contains("keel_runner_user"))
    );
    let out =
        f.h.cmd(Cmd::GetReviewDiff {
            group,
            path: Some(RepoPath::new("playbooks/web.yaml").expect("path")),
            page: Some(1),
        })
        .await;
    assert_eq!(op_error(&out), "bad_page");
    let out =
        f.h.cmd(Cmd::GetReviewDiff {
            group,
            path: Some(RepoPath::new("README.md").expect("path")),
            page: None,
        })
        .await;
    assert_eq!(op_error(&out), "not_changed");

    // Lists, by workspace and state.
    let out =
        f.h.cmd(Cmd::ListReviews {
            workspace: Some(f.h.workspace),
            state: Some(ReviewState::Open),
        })
        .await;
    let [Event::Reviews { items }] = out.as_slice() else {
        panic!("list: {out:?}");
    };
    assert_eq!(items.len(), 3);
    f.h.finish().await;
}

#[tokio::test]
async fn an_override_needs_a_passkey_over_its_challenge_and_a_comment() {
    let Some(f) = Fixture::new("signed_override", false).await else {
        return;
    };
    assert!(ReviewComment::new("").is_err(), "a comment is required");
    assert!(
        ReviewComment::new("   ").is_err(),
        "a blank comment is refused"
    );
    f.push("playbooks/web.yaml", "- hosts: all\n  tasks: []\n")
        .await;
    let (run, _, _) = f.last_run().await;
    let group = f.groups(run).await[0].0;

    // Without a passkey there is nothing to sign with.
    let out =
        f.h.cmd(Cmd::PreviewOverride {
            group,
            comment: comment("ok"),
        })
        .await;
    assert!(
        matches!(out.as_slice(), [Event::PasskeyEnrolmentRequired]),
        "{out:?}"
    );

    // 058 A2 (T038): neither host has a base, so this is a baseline; web-1 was changed outside
    // Keel, and approving the baseline clears it.
    assert!(f.groups(run).await[0].1.is_none());
    sqlx::query("update hosts set changed_outside = true where id = $1")
        .bind(f.host("web-1"))
        .execute(&f.h.db.owner)
        .await
        .expect("flag");

    let mut auth = f.passkey().await;
    let (p, c) = f.preview_override(group).await;
    assert!(p.step_up);
    assert_eq!(c.preview_id, p.preview_id);
    assert!(c.canonical.contains(&run.to_string()), "{}", c.canonical);
    assert!(c.canonical.contains("web-1") && c.canonical.contains("web-2"));

    // A recent step-up does not stand in for the assertion.
    f.h.step_up(&f.h.owner).await;
    let out =
        f.h.cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert!(
        !matches!(out.first(), Some(Event::Confirmed { .. })),
        "{out:?}"
    );
    // A verify ceremony's assertion is over a random challenge, not the override's.
    let out =
        f.h.cmd(Cmd::PasskeyBegin {
            purpose: PasskeyPurpose::Verify,
        })
        .await;
    let [Event::PasskeyChallenge(r)] = out.as_slice() else {
        panic!("verify challenge: {out:?}");
    };
    let wrong = PasskeyAssertion {
        ceremony: r.ceremony,
        credential: assert_with(&mut auth, &r.options),
    };
    let out =
        f.h.cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: Some(wrong),
        })
        .await;
    assert!(
        !matches!(out.first(), Some(Event::Confirmed { .. })),
        "{out:?}"
    );
    assert_eq!(f.run_state(run).await, "needs_review");

    // The assertion over the override's own challenge approves it.
    let assertion = PasskeyAssertion {
        ceremony: c.request.ceremony,
        credential: assert_with(&mut auth, &c.request.options),
    };
    let signature = serde_json::to_value(&assertion.credential.response.signature)
        .expect("signature")
        .as_str()
        .expect("base64url")
        .to_owned();
    let out =
        f.h.cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: Some(assertion),
        })
        .await;
    let [
        Event::Confirmed { .. },
        Event::Review {
            review, decisions, ..
        },
    ] = out.as_slice()
    else {
        panic!("confirm: {out:?}");
    };
    assert_eq!((review.state, review.approvals), (ReviewState::Approved, 1));
    assert_eq!(decisions.len(), 1);
    assert!(decisions[0].approve);
    assert_eq!(
        decisions[0].comment.as_deref(),
        Some("Reviewed the diff with the on-call lead.")
    );
    assert!(decisions[0].expires_at.is_some());
    // Every group approved and no approval policy to satisfy: the run is queued.
    assert_eq!(f.run_state(run).await, "queued");

    // The approval keeps the challenge's hash and the assertion, for hosts to check.
    let (sha, assertion, kind): (String, Value, String) = sqlx::query_as(
        "select challenge_sha256, assertion, kind from approvals where review_group_id = $1",
    )
    .bind(group)
    .fetch_one(&f.h.db.owner)
    .await
    .expect("approval");
    let challenge = Sha256::digest(c.canonical.as_bytes());
    assert_eq!(sha, hex(&Sha256::digest(challenge)));
    assert_eq!(kind, "override");
    assert_eq!(assertion["signature"], signature.as_str());
    // The audit log records the approval but neither the challenge nor the assertion.
    let actions = f.h.audit_actions().await;
    assert!(
        actions.iter().any(|a| a.0 == "override_approved"),
        "{actions:?}"
    );
    assert!(
        actions.iter().any(|a| a.0 == "run_overridden"),
        "{actions:?}"
    );
    let (cleared,): (bool,) = sqlx::query_as("select not changed_outside from hosts where id = $1")
        .bind(f.host("web-1"))
        .fetch_one(&f.h.db.owner)
        .await
        .expect("host");
    assert!(cleared, "a baseline approval clears changed_outside");
    assert!(
        actions
            .iter()
            .any(|a| a.0 == "host_changed_outside_cleared"),
        "{actions:?}"
    );
    let audit = f.h.audit_text().await;
    assert!(!audit.contains(&signature));
    assert!(!audit.contains(&hex(&challenge)));
    assert!(!audit.contains(&sha));

    // A closed review cannot be overridden again.
    let out =
        f.h.cmd(Cmd::PreviewOverride {
            group,
            comment: comment("again"),
        })
        .await;
    assert_eq!(op_error(&out), "review_closed");
    f.h.finish().await;
}

#[tokio::test]
async fn four_eyes_refuses_the_requester_and_a_denial_denies_the_run() {
    let Some(f) = Fixture::new("signed_four_eyes", true).await else {
        return;
    };
    f.passkey().await;
    // The owner's sync made the run: they requested it.
    f.push("playbooks/web.yaml", "- hosts: all\n  tasks: []\n")
        .await;
    let (run, _, _) = f.last_run().await;
    let group = f.groups(run).await[0].0;
    let Event::Review { review, .. } = f.review(group).await else {
        unreachable!()
    };
    assert!(review.four_eyes);
    let out =
        f.h.cmd(Cmd::PreviewOverride {
            group,
            comment: comment("mine"),
        })
        .await;
    assert_eq!(op_error(&out), "four_eyes_requester");

    // Anyone with Approve may deny; the run is denied and goes nowhere.
    let (p, out) =
        f.h.confirm(Cmd::PreviewOverrideDeny {
            group,
            comment: comment("Unreviewed change to the web role."),
        })
        .await;
    assert!(!p.step_up);
    let [
        _,
        Event::Review {
            review, decisions, ..
        },
    ] = out.as_slice()
    else {
        panic!("deny: {out:?}");
    };
    assert_eq!(review.state, ReviewState::Denied);
    assert!(!decisions[0].approve);
    assert_eq!(f.run_state(run).await, "denied");
    let (cancelled,): (i64,) =
        sqlx::query_as("select count(*) from run_hosts where run_id = $1 and state = 'cancelled'")
            .bind(run)
            .fetch_one(&f.h.db.owner)
            .await
            .expect("run hosts");
    assert_eq!(cancelled, 2);
    let actions = f.h.audit_actions().await;
    assert!(
        actions.iter().any(|a| a.0 == "override_denied"),
        "{actions:?}"
    );
    f.h.finish().await;
}

#[tokio::test]
async fn trusted_signers_let_signed_commits_through() {
    let Some(f) = Fixture::new("signed_signers", false).await else {
        return;
    };
    let keys = tempfile::tempdir().expect("tempdir");
    let (private, public) = ssh_key(keys.path());

    // Adding a signer needs step-up and Administer; one key of the named kind.
    let out =
        f.h.cmd(Cmd::PreviewSignerAdd {
            workspace: f.h.workspace,
            kind: SignerKind::Openpgp,
            public_key: SignerKey::new(public.clone()).expect("key"),
            label: SignerLabel::new("ci").expect("label"),
            user: None,
        })
        .await;
    assert_eq!(op_error(&out), "invalid_signer_key");
    let add = Cmd::PreviewSignerAdd {
        workspace: f.h.workspace,
        kind: SignerKind::Ssh,
        public_key: SignerKey::new(public.clone()).expect("key"),
        label: SignerLabel::new("ci").expect("label"),
        user: Some(f.h.owner.user.id),
    };
    let (p, out) = f.h.confirm(add.clone()).await;
    assert!(p.step_up);
    let [_, Event::Signers { items, .. }] = out.as_slice() else {
        panic!("signers: {out:?}");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, SignerKind::Ssh);
    assert!(items[0].fingerprint.starts_with("SHA256:"), "{items:?}");
    let signer = items[0].id;
    let fingerprint = items[0].fingerprint.clone();
    let p = preview(&f.h.cmd(add.clone()).await);
    let out =
        f.h.cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert_eq!(op_error(&out[1..]), "signer_exists");

    // A signed commit, as `git commit -S` with an SSH key makes it.
    let sign = |file: &str, content: &str| -> String {
        std::fs::create_dir_all(f.src.work.join(file).parent().expect("dir")).expect("dir");
        std::fs::write(f.src.work.join(file), content).expect("write");
        git(&f.src.work, &["add", "-A"]);
        git(
            &f.src.work,
            &[
                "-c",
                "gpg.format=ssh",
                "-c",
                &format!("user.signingkey={private}"),
                "commit",
                "-q",
                "-S",
                "-m",
                "signed",
            ],
        );
        git(&f.src.work, &["rev-parse", "HEAD"])
    };
    let first = sign("playbooks/web.yaml", "- hosts: all\n  tasks: []\n");
    f.src.push();
    f.sync().await;
    let (run1, state, _) = f.last_run().await;
    // No base yet: still a baseline review, but the commit shows as signed by the signer.
    assert_eq!(state, "needs_review");
    let groups = f.groups(run1).await;
    let Event::Review { commits, .. } = f.review(groups[0].0).await else {
        unreachable!()
    };
    assert_eq!(commits[0].id, first);
    assert_eq!(commits[0].status, SignatureStatus::Good);
    assert_eq!(commits[0].signer.as_deref(), Some(fingerprint.as_str()));

    // With a base on every host, a signed commit needs no review.
    f.applied("web-1", &first, run1).await;
    f.applied("web-2", &first, run1).await;
    sign("playbooks/web.yaml", "- hosts: all\n  tasks: [] # two\n");
    f.src.push();
    f.sync().await;
    let (run2, state, _) = f.last_run().await;
    assert_ne!(run2, run1);
    assert_eq!(state, "queued");
    assert!(f.groups(run2).await.is_empty());

    // An unsigned commit on top does.
    f.push("playbooks/web.yaml", "- hosts: all\n  tasks: [] # three\n")
        .await;
    let (run3, state, _) = f.last_run().await;
    assert_eq!(state, "needs_review");
    let groups = f.groups(run3).await;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].2, ["web-1", "web-2"]);
    assert_eq!(groups[0].3, "unsigned");

    // Removing the signer needs step-up; then its commits are no longer trusted.
    let (p, out) = f.h.confirm(Cmd::PreviewSignerRemove { signer }).await;
    assert!(p.step_up);
    assert!(
        matches!(out.as_slice(), [_, Event::Signers { items, .. }] if items.is_empty()),
        "{out:?}"
    );
    // Re-adding a removed signer works.
    let (_, out) = f.h.confirm(add).await;
    assert!(
        matches!(out.as_slice(), [_, Event::Signers { items, .. }] if items.len() == 1),
        "{out:?}"
    );
    let actions = f.h.audit_actions().await;
    for a in ["signer_added", "signer_removed"] {
        assert!(actions.iter().any(|x| x.0 == a), "{a}: {actions:?}");
    }
    f.h.finish().await;
}

#[tokio::test]
async fn signed_mode_off_needs_step_up_and_managed_inventories_are_refused() {
    let Some(f) = Fixture::new("signed_mode_off", false).await else {
        return;
    };
    sqlx::query("update sessions set verified_at = null where id = $1")
        .bind(&f.h.owner.session)
        .execute(&f.h.db.owner)
        .await
        .expect("stale");
    let p = preview(
        &f.h.cmd(Cmd::PreviewSignedMode {
            binding: f.binding,
            on: false,
        })
        .await,
    );
    assert!(p.step_up);
    let out =
        f.h.cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert!(
        !matches!(out.first(), Some(Event::Confirmed { .. })),
        "{out:?}"
    );
    f.h.step_up(&f.h.owner).await;
    f.h.confirm(Cmd::PreviewSignedMode {
        binding: f.binding,
        on: false,
    })
    .await;
    let actions = f.h.audit_actions().await;
    assert!(
        actions
            .iter()
            .any(|a| a.0 == "signed_mode_off" && a.1 == "allowed"),
        "{actions:?}"
    );

    // A job on a managed inventory: signed mode stays off.
    let managed = Source::new(
        "version: 1\nprofiles: { debian: {} }\njobs:\n  web:\n    \
         playbook: playbooks/web.yaml\n    profiles: [debian]\n    inventory: staging\n",
    );
    let other = bind(&f.h, &managed).await;
    let out =
        f.h.cmd(Cmd::PreviewSignedMode {
            binding: other,
            on: true,
        })
        .await;
    assert_eq!(op_error(&out), "managed_inventory");
    f.h.finish().await;
}
