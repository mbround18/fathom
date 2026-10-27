//! Runs the committed vectors (tests/vectors/, written by tests/gen) through the commit check and
//! `authorize`, and compares with each case's expected verdict. The server's early check and the
//! executor link this same code, so passing here is the shared-verdict guarantee.

mod common;

use std::fs;

use common::{CASES, CaseFile, MemStore, RepoFile, summarize, vectors_dir};
use keel_trust::authorize::{RunRequest, authorize};
use keel_trust::commit_check::{CheckError, check};
use keel_trust::ledger::Ledger;
use keel_trust::object::CommitId;
use keel_trust::override_verify::parse_approvers;
use keel_trust::signers::Signers;
use serde_json::{Value, json};

fn id(s: &str) -> CommitId {
    CommitId::parse(s).expect("vector commit id")
}

#[test]
fn case_directories_match_the_list() {
    let mut found: Vec<String> = fs::read_dir(vectors_dir())
        .unwrap()
        .filter_map(|e| {
            let e = e.unwrap();
            e.file_type()
                .unwrap()
                .is_dir()
                .then(|| e.file_name().into_string().unwrap())
        })
        .collect();
    found.sort();
    let mut want: Vec<String> = CASES.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(found, want);
}

#[test]
fn every_vector_reaches_its_expected_verdict() {
    let dir = vectors_dir();
    let signers = Signers::parse(&fs::read_to_string(dir.join("signers")).unwrap()).unwrap();
    assert_eq!(signers.ssh.len(), 1);
    assert_eq!(signers.pgp.len(), 2);
    let approvers =
        parse_approvers(&fs::read_to_string(dir.join("approvers.json")).unwrap()).unwrap();

    for name in CASES {
        let case_dir = dir.join(name);
        let repo: RepoFile =
            serde_json::from_str(&fs::read_to_string(case_dir.join("repo.json")).unwrap()).unwrap();
        let case: CaseFile =
            serde_json::from_str(&fs::read_to_string(case_dir.join("case.json")).unwrap()).unwrap();
        let store = MemStore(repo);
        let target = id(&case.target);

        // The commit check alone, from the host's ledger base.
        let check_result = match Ledger::parse(&case.ledger) {
            Err(_) => Value::Null,
            Ok(ledger) => match check(
                &store,
                &signers,
                ledger.base(&case.run.job).as_ref(),
                &target,
            ) {
                Ok(v) => serde_json::to_value(v).unwrap(),
                Err(CheckError::PackIncomplete { .. }) => json!({ "error": "pack_incomplete" }),
                Err(e) => panic!("{name}: store error {e}"),
            },
        };
        assert_eq!(check_result, case.expect_check, "{name}: commit check");

        // The full decision.
        let assumed_base = case.assumed_base.as_deref().map(id);
        let run = case.run.context();
        let policy = case.policy.policy();
        let result = authorize(&RunRequest {
            ledger: &case.ledger,
            store: &store,
            signers: &signers,
            approvers: &approvers,
            policy: &policy,
            run: &run,
            assumed_base: assumed_base.as_ref(),
            target: &target,
            approvals: &case.approvals,
            now: case.now,
        });
        assert_eq!(summarize(&result), case.expect, "{name}: authorize");
    }
}

#[test]
fn an_authorized_run_appends_a_start_line_that_blocks_a_replay() {
    let dir = vectors_dir();
    let signers = Signers::parse(&fs::read_to_string(dir.join("signers")).unwrap()).unwrap();
    let approvers =
        parse_approvers(&fs::read_to_string(dir.join("approvers.json")).unwrap()).unwrap();
    let case_dir = dir.join("signed_openpgp");
    let store = MemStore(
        serde_json::from_str(&fs::read_to_string(case_dir.join("repo.json")).unwrap()).unwrap(),
    );
    let case: CaseFile =
        serde_json::from_str(&fs::read_to_string(case_dir.join("case.json")).unwrap()).unwrap();
    let target = id(&case.target);
    let assumed = case.assumed_base.as_deref().map(id);
    let run = case.run.context();
    let policy = case.policy.policy();
    let request = |ledger: &str| {
        authorize(&RunRequest {
            ledger,
            store: &store,
            signers: &signers,
            approvers: &approvers,
            policy: &policy,
            run: &run,
            assumed_base: assumed.as_ref(),
            target: &target,
            approvals: &[],
            now: case.now,
        })
    };
    let mut authorized = request(&case.ledger).unwrap();
    let line = authorized
        .ledger
        .append(keel_trust::ledger::Entry::Start {
            at: "2026-01-02T00:00:00Z".into(),
            run: run.run.clone(),
            job: run.job.clone(),
            base: authorized.base.clone(),
            target: target.clone(),
            auth: authorized.auth.clone(),
        })
        .unwrap();
    let ledger = format!("{}{line}", case.ledger);
    assert_eq!(summarize(&request(&ledger)), json!({ "refused": "replay" }));
}

/// A signed commit whose message was edited after signing (the signature kept) must fail with
/// `bad_signature`, for both OpenPGP and SSH. This shows the vectors pass because signatures
/// verify, not because the check ignores them.
#[test]
fn an_edited_signed_commit_fails_its_signature() {
    let dir = vectors_dir();
    let signers = Signers::parse(&fs::read_to_string(dir.join("signers")).unwrap()).unwrap();
    for name in ["signed_openpgp", "signed_ssh"] {
        let case_dir = dir.join(name);
        let mut repo: RepoFile =
            serde_json::from_str(&fs::read_to_string(case_dir.join("repo.json")).unwrap()).unwrap();
        let case: CaseFile =
            serde_json::from_str(&fs::read_to_string(case_dir.join("case.json")).unwrap()).unwrap();
        let raw = repo.commits.remove(&case.target).unwrap();
        assert!(raw.contains("\ngpgsig "), "{name}: tip is signed");
        let edited = format!("{raw}tampered\n");
        let tampered = common::git_sha1(edited.as_bytes());
        repo.commits.insert(tampered.clone(), edited);
        let store = MemStore(repo);
        let ledger = Ledger::parse(&case.ledger).unwrap();
        let verdict = check(
            &store,
            &signers,
            ledger.base(&case.run.job).as_ref(),
            &id(&tampered),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(verdict).unwrap(),
            json!({
                "verdict": "needs_override",
                "reason": "unsigned",
                "failures": [{ "commit": tampered, "problem": "bad_signature" }],
            }),
            "{name}"
        );
    }
}
