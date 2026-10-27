//! T089: the deterministic generator for tests/vectors/.
//!
//! Every key comes from a fixed seed, every timestamp is fixed, and every signature scheme used is
//! deterministic (Ed25519 for OpenPGP and SSH, RFC 6979 ECDSA for ES256), so the output is the
//! same byte for byte on every run. The test `vectors_are_current` regenerates everything in
//! memory and compares it with the committed files; run it with `KEEL_TRUST_REGEN=1` to write
//! them instead:
//!
//! ```text
//! KEEL_TRUST_REGEN=1 cargo test -p keel-trust --test gen
//! ```
//!
//! The expected verdicts are written here by hand, per case, not computed by the library.
//!
//! Keys (T0 = 2026-01-01T00:00:00Z):
//! - OpenPGP A (trusted): certify-only primary created T0-365d; signing subkey A1 created
//!   T0-300d without expiry; signing subkey A2 created T0-100d, expiring 30 days later.
//! - OpenPGP C (trusted): signing primary created T0-200d, revoked "key compromised" at T0-10d.
//! - OpenPGP U: signing primary, not trusted.
//! - SSH E (trusted, `keel namespaces="git"`) and SSH X (not trusted), Ed25519.
//! - Approvers: alice (ES256) and bob (EdDSA).

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::fs;

use common::{CASES, CaseFile, PolicyFile, RepoFile, RunFile, git_sha1, vectors_dir};
use keel_trust::challenge::{ApprovalKind, OverrideScope, format_utc};
use keel_trust::ledger::{Auth, Entry, Ledger, Outcome};
use keel_trust::object::CommitId;
use keel_trust::override_verify::{Approval, Approver};
use p256::ecdsa::signature::Signer as _;
use pgp::bytes::Bytes;
use pgp::composed::{
    ArmorOptions, DetachedSignature, KeyType, SignedKeyDetails, SignedPublicKey, SignedPublicSubKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{
    KeyFlags, PubKeyInner, PublicKey, PublicSubkey, RevocationCode, SecretKey, SecretSubkey,
    Signature, SignatureConfig, SignatureType, Subpacket, SubpacketData, UserId,
};
use pgp::types::{
    Duration, KeyDetails, KeyVersion, Password, SignedUser, SigningKey, Tag, Timestamp,
};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ssh_key::{HashAlg, LineEnding, PrivateKey};
use web_passkeys::verify::{FLAG_UP, FLAG_UV};
use web_passkeys::{CoseKey, b64};

const T0: i64 = 1_767_225_600;
const DAY: i64 = 86_400;
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const ORIGIN: &str = "https://keel.example.test";
const RP_ID: &str = "keel.example.test";
const HOST: &str = "web-1";
const JOB: &str = "web-deploy";
const WORKSPACE: &str = "0b7f1c2e-3d4a-4b5c-8d6e-7f8091a2b3c4";
const BINDING: &str = "1c2d3e4f-5a6b-4c7d-8e9f-a0b1c2d3e4f5";
const PREVIOUS_RUN: &str = "5e5e5e5e-0000-4000-8000-000000000000";
const NOW: i64 = T0 + DAY;

fn ts(t: i64) -> Timestamp {
    Timestamp::from_secs(u32::try_from(t).unwrap())
}

fn pw() -> Password {
    Password::empty()
}

// ---------------------------------------------------------------------------------------------
// OpenPGP

struct Pgp {
    primary: SecretKey,
    subkeys: Vec<SecretSubkey>,
    cert: SignedPublicKey,
}

fn new_primary(rng: &mut ChaCha20Rng, created: i64) -> SecretKey {
    let (pp, sp) = KeyType::Ed25519Legacy.generate(&mut *rng).unwrap();
    let inner = PubKeyInner::new(
        KeyVersion::V4,
        PublicKeyAlgorithm::EdDSALegacy,
        ts(created),
        None,
        pp,
    )
    .unwrap();
    SecretKey::new(PublicKey::from_inner(inner).unwrap(), sp).unwrap()
}

fn new_subkey(rng: &mut ChaCha20Rng, created: i64) -> SecretSubkey {
    let (pp, sp) = KeyType::Ed25519Legacy.generate(&mut *rng).unwrap();
    let inner = PubKeyInner::new(
        KeyVersion::V4,
        PublicKeyAlgorithm::EdDSALegacy,
        ts(created),
        None,
        pp,
    )
    .unwrap();
    SecretSubkey::new(PublicSubkey::from_inner(inner).unwrap(), sp).unwrap()
}

/// A v4 signature config with creation time and issuer subpackets.
fn config(
    typ: SignatureType,
    issuer: &impl KeyDetails,
    created: i64,
    extra: Vec<SubpacketData>,
) -> SignatureConfig {
    let mut c = SignatureConfig::v4(typ, PublicKeyAlgorithm::EdDSALegacy, HashAlgorithm::Sha256);
    c.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(ts(created))).unwrap(),
        Subpacket::regular(SubpacketData::IssuerFingerprint(issuer.fingerprint())).unwrap(),
    ];
    for data in extra {
        c.hashed_subpackets.push(Subpacket::regular(data).unwrap());
    }
    c.unhashed_subpackets =
        vec![Subpacket::regular(SubpacketData::IssuerKeyId(issuer.legacy_key_id())).unwrap()];
    c
}

fn flags(sign: bool, certify: bool) -> SubpacketData {
    let mut f = KeyFlags::default();
    f.set_sign(sign);
    f.set_certify(certify);
    SubpacketData::KeyFlags(f)
}

struct SubSpec {
    created: i64,
    expires_after: Option<u32>,
}

fn pgp_key(
    seed: u8,
    uid: &str,
    created: i64,
    primary_signs: bool,
    subs: &[SubSpec],
    revoked_at: Option<i64>,
) -> Pgp {
    let mut rng = ChaCha20Rng::from_seed([seed; 32]);
    let primary = new_primary(&mut rng, created);
    let primary_pub = primary.public_key().clone();
    let uid = UserId::from_str(Default::default(), uid).unwrap();
    let cert_sig = config(
        SignatureType::CertPositive,
        &primary_pub,
        created,
        vec![flags(primary_signs, true)],
    )
    .sign_certification(&primary, &primary_pub, &pw(), Tag::UserId, &uid)
    .unwrap();

    let mut subkeys = Vec::new();
    let mut public_subkeys = Vec::new();
    for spec in subs {
        let sub = new_subkey(&mut rng, spec.created);
        let sub_pub = sub.public_key().clone();
        let backsig = config(SignatureType::KeyBinding, &sub_pub, spec.created, vec![])
            .sign_primary_key_binding(&sub, &sub_pub, &pw(), &primary_pub)
            .unwrap();
        let mut extra = vec![flags(true, false)];
        if let Some(secs) = spec.expires_after {
            extra.push(SubpacketData::KeyExpirationTime(Duration::from_secs(secs)));
        }
        extra.push(SubpacketData::EmbeddedSignature(Box::new(backsig)));
        let binding = config(
            SignatureType::SubkeyBinding,
            &primary_pub,
            spec.created,
            extra,
        )
        .sign_subkey_binding(&primary, &primary_pub, &pw(), &sub_pub)
        .unwrap();
        public_subkeys.push(SignedPublicSubKey::new(sub_pub, vec![binding]));
        subkeys.push(sub);
    }

    let revocations: Vec<Signature> = revoked_at
        .map(|at| {
            config(
                SignatureType::KeyRevocation,
                &primary_pub,
                at,
                vec![SubpacketData::RevocationReason(
                    RevocationCode::KeyCompromised,
                    Bytes::from_static(b"test"),
                )],
            )
            .sign_key(&primary, &pw(), &primary_pub)
            .unwrap()
        })
        .into_iter()
        .collect();

    let details = SignedKeyDetails::new(
        revocations,
        vec![],
        vec![SignedUser::new(uid, vec![cert_sig])],
        vec![],
    );
    let cert = SignedPublicKey::new(primary_pub, details, public_subkeys);
    Pgp {
        primary,
        subkeys,
        cert,
    }
}

fn pgp_sign(
    key: &impl SigningKey,
    issuer: &impl KeyDetails,
    created: i64,
    payload: &[u8],
) -> String {
    let sig = config(SignatureType::Binary, issuer, created, vec![])
        .sign(key, &pw(), payload)
        .unwrap();
    DetachedSignature::new(sig)
        .to_armored_string(ArmorOptions::default())
        .unwrap()
}

// ---------------------------------------------------------------------------------------------
// Commits

#[derive(Clone, Copy)]
enum By<'a> {
    Nobody,
    PgpPrimary(&'a Pgp),
    PgpSub(&'a Pgp, usize),
    Ssh(&'a PrivateKey),
}

#[derive(Default)]
struct Repo {
    file: RepoFile,
    tick: i64,
}

impl Repo {
    fn new() -> Self {
        let mut r = Self::default();
        r.file.trees.insert(EMPTY_TREE.into());
        r
    }

    /// Adds a commit and returns its id. Committer time advances one minute per commit.
    fn commit(&mut self, parents: &[&str], message: &str, by: By<'_>) -> String {
        self.tick += 1;
        let t = T0 + self.tick * 60;
        let mut head = format!("tree {EMPTY_TREE}\n");
        for p in parents {
            head.push_str(&format!("parent {p}\n"));
        }
        head.push_str(&format!(
            "author Keel Test <test@keel.example.test> {t} +0000\n"
        ));
        head.push_str(&format!(
            "committer Keel Test <test@keel.example.test> {t} +0000\n"
        ));
        let payload = format!("{head}\n{message}\n");
        let sig = match by {
            By::Nobody => None,
            By::PgpPrimary(k) => Some(pgp_sign(
                &k.primary,
                k.primary.public_key(),
                t,
                payload.as_bytes(),
            )),
            By::PgpSub(k, i) => Some(pgp_sign(
                &k.subkeys[i],
                k.subkeys[i].public_key(),
                t,
                payload.as_bytes(),
            )),
            By::Ssh(k) => Some(
                k.sign("git", HashAlg::Sha512, payload.as_bytes())
                    .unwrap()
                    .to_pem(LineEnding::LF)
                    .unwrap(),
            ),
        };
        let raw = match sig {
            None => payload,
            Some(sig) => {
                let lines: Vec<&str> = sig.trim_end().split('\n').collect();
                format!("{head}gpgsig {}\n\n{message}\n", lines.join("\n "))
            }
        };
        let id = git_sha1(raw.as_bytes());
        self.file.commits.insert(id.clone(), raw);
        id
    }
}

// ---------------------------------------------------------------------------------------------
// Approvers and assertions

enum ApproverKey {
    Es256(p256::ecdsa::SigningKey),
    EdDsa(ed25519_dalek::SigningKey),
}

struct Person {
    label: &'static str,
    credential_id: String,
    key: ApproverKey,
}

impl Person {
    fn approver(&self) -> Approver {
        let cose = match &self.key {
            ApproverKey::Es256(k) => {
                let p = k.verifying_key().to_encoded_point(false);
                CoseKey::Es256 {
                    x: (*p.x().unwrap()).into(),
                    y: (*p.y().unwrap()).into(),
                }
            }
            ApproverKey::EdDsa(k) => CoseKey::EdDsa {
                x: k.verifying_key().to_bytes(),
            },
        };
        Approver {
            label: self.label.into(),
            credential_id: self.credential_id.clone(),
            public_key_cose: b64::encode(cose.encode()),
            alg: cose.alg(),
        }
    }

    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        match &self.key {
            ApproverKey::Es256(k) => {
                let s: p256::ecdsa::Signature = k.sign(msg);
                s.to_der().as_bytes().to_vec()
            }
            ApproverKey::EdDsa(k) => {
                use ed25519_dalek::Signer as _;
                k.sign(msg).to_bytes().to_vec()
            }
        }
    }
}

struct ApprovalSpec<'a> {
    by: &'a Person,
    kind: ApprovalKind,
    hosts: &'a [&'a str],
    expires_at: i64,
    origin: &'a str,
    flags: u8,
}

fn approval(spec: ApprovalSpec<'_>, run: &str, base: Option<&str>, target: &str) -> Approval {
    let hosts: Vec<String> = spec.hosts.iter().map(|h| h.to_string()).collect();
    let expires_at = format_utc(spec.expires_at).unwrap();
    let scope = OverrideScope {
        kind: spec.kind,
        workspace: WORKSPACE.into(),
        binding: BINDING.into(),
        run: run.into(),
        job: JOB.into(),
        base: base.map(|b| CommitId::parse(b).unwrap()),
        target: CommitId::parse(target).unwrap(),
        hosts: hosts.clone(),
        expires_at: expires_at.clone(),
    };
    let challenge = scope.challenge().unwrap();
    let mut authenticator_data = Sha256::digest(RP_ID.as_bytes()).to_vec();
    authenticator_data.push(spec.flags);
    authenticator_data.extend_from_slice(&7u32.to_be_bytes());
    let client_data_json = format!(
        r#"{{"type":"webauthn.get","challenge":"{}","origin":"{}","crossOrigin":false}}"#,
        b64::encode(challenge),
        spec.origin
    )
    .into_bytes();
    let mut msg = authenticator_data.clone();
    msg.extend_from_slice(&Sha256::digest(&client_data_json));
    Approval {
        kind: spec.kind,
        approver: spec.by.label.into(),
        credential_id: spec.by.credential_id.clone(),
        signature: spec.by.sign(&msg),
        authenticator_data,
        client_data_json,
        expires_at,
        comment: "vector".into(),
        hosts,
    }
}

// ---------------------------------------------------------------------------------------------
// Ledgers

fn run_id(n: u32) -> String {
    format!("5e5e5e5e-0000-4000-8000-{n:012x}")
}

/// A ledger whose job base is `base` (one earlier baseline run that succeeded).
fn ledger_with_base(base: &str) -> (Ledger, String) {
    let mut l = Ledger::empty();
    let mut text = l
        .append(Entry::Start {
            at: format_utc(T0 - 3600).unwrap(),
            run: PREVIOUS_RUN.into(),
            job: JOB.into(),
            base: None,
            target: CommitId::parse(base).unwrap(),
            auth: Auth::Baseline {
                approvals: vec!["YWxpY2UtY3JlZGVudGlhbA".into()],
                approvers: vec!["alice".into()],
            },
        })
        .unwrap();
    text += &l
        .append(Entry::End {
            at: format_utc(T0 - 3000).unwrap(),
            run: PREVIOUS_RUN.into(),
            job: JOB.into(),
            outcome: Outcome::Succeeded,
            exit: Some(0),
        })
        .unwrap();
    (l, text)
}

// ---------------------------------------------------------------------------------------------
// Cases

struct Out {
    files: BTreeMap<String, String>,
}

impl Out {
    fn put(&mut self, path: &str, content: String) {
        self.files.insert(path.into(), content);
    }

    #[allow(clippy::too_many_arguments)]
    fn case(
        &mut self,
        name: &str,
        description: &str,
        repo: &Repo,
        ledger: &str,
        assumed_base: Option<&str>,
        target: &str,
        run: &str,
        override_quorum: u32,
        approvals: Vec<Approval>,
        expect_check: Value,
        expect: Value,
    ) {
        assert!(CASES.contains(&name), "{name} missing from CASES");
        let case = CaseFile {
            description: description.into(),
            ledger: ledger.into(),
            assumed_base: assumed_base.map(str::to_owned),
            target: target.into(),
            run: RunFile {
                workspace: WORKSPACE.into(),
                binding: BINDING.into(),
                run: run.into(),
                job: JOB.into(),
                assign_quorum: 0,
            },
            policy: PolicyFile {
                origin: ORIGIN.into(),
                rp_id: RP_ID.into(),
                override_quorum,
                max_override_age_secs: DAY,
                inventory_name: HOST.into(),
            },
            approvals,
            now: NOW,
            expect_check,
            expect,
        };
        self.put(
            &format!("{name}/repo.json"),
            serde_json::to_string_pretty(&repo.file).unwrap() + "\n",
        );
        self.put(
            &format!("{name}/case.json"),
            serde_json::to_string_pretty(&case).unwrap() + "\n",
        );
    }
}

fn generate() -> BTreeMap<String, String> {
    let a = pgp_key(
        0xa0,
        "Alice Signer <alice@keel.example.test>",
        T0 - 365 * DAY,
        false,
        &[
            SubSpec {
                created: T0 - 300 * DAY,
                expires_after: None,
            },
            SubSpec {
                created: T0 - 100 * DAY,
                expires_after: Some(30 * DAY as u32),
            },
        ],
        None,
    );
    let c = pgp_key(
        0xc0,
        "Carol Revoked <carol@keel.example.test>",
        T0 - 200 * DAY,
        true,
        &[],
        Some(T0 - 10 * DAY),
    );
    let u = pgp_key(
        0xf0,
        "Unknown <unknown@keel.example.test>",
        T0 - 200 * DAY,
        true,
        &[],
        None,
    );
    let mut e = PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[0xe0; 32]));
    e.set_comment("");
    let mut x = PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[0xe1; 32]));
    x.set_comment("");

    let id_a = keel_trust::signers::pgp_signer_id(&a.cert);
    let id_e = e.public_key().fingerprint(HashAlg::Sha256).to_string();

    let alice = Person {
        label: "alice",
        credential_id: b64::encode(b"alice-credential"),
        key: ApproverKey::Es256(p256::ecdsa::SigningKey::from_slice(&[0x11; 32]).unwrap()),
    };
    let bob = Person {
        label: "bob",
        credential_id: b64::encode(b"bob-credential"),
        key: ApproverKey::EdDsa(ed25519_dalek::SigningKey::from_bytes(&[0x22; 32])),
    };

    let mut out = Out {
        files: BTreeMap::new(),
    };

    // Shared trust files.
    let mut signers =
        String::from("# Keel trust vectors: SSH allowed_signers lines, then OpenPGP keys.\n");
    signers.push_str(&format!(
        "keel namespaces=\"git\" {}\n",
        e.public_key().to_openssh().unwrap()
    ));
    for k in [&a, &c] {
        signers.push_str(&k.cert.to_armored_string(ArmorOptions::default()).unwrap());
        if !signers.ends_with('\n') {
            signers.push('\n');
        }
    }
    out.put("signers", signers);
    out.put(
        "approvers.json",
        serde_json::to_string_pretty(&vec![alice.approver(), bob.approver()]).unwrap() + "\n",
    );

    let ok = |by: &Person, kind, run: &str, base: Option<&str>, target: &str| {
        approval(
            ApprovalSpec {
                by,
                kind,
                hosts: &[HOST],
                expires_at: NOW + 3600,
                origin: ORIGIN,
                flags: FLAG_UP | FLAG_UV,
            },
            run,
            base,
            target,
        )
    };
    let signed = |signers: &[&str]| json!({ "verdict": "signed", "signers": signers });
    let needs = |reason: &str, failures: Vec<(&str, &str)>| {
        let f: Vec<Value> = failures
            .iter()
            .map(|(c, p)| json!({ "commit": c, "problem": p }))
            .collect();
        json!({ "verdict": "needs_override", "reason": reason, "failures": f })
    };
    let refused_unsigned = |failures: Vec<(&str, &str)>| {
        let f: Vec<Value> = failures
            .iter()
            .map(|(c, p)| json!({ "commit": c, "problem": p }))
            .collect();
        json!({ "refused": "unsigned", "failures": f })
    };

    // signed_openpgp: base B0, then two commits by A's subkey A1.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "one", By::PgpSub(&a, 0));
        let c2 = r.commit(&[&c1], "two", By::PgpSub(&a, 0));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "signed_openpgp",
            "Two commits signed by a trusted OpenPGP signing subkey on top of the base.",
            &r,
            &ledger,
            Some(&b0),
            &c2,
            &run_id(1),
            1,
            vec![],
            signed(&[&id_a]),
            json!({ "authorized": { "kind": "signed", "signers": [id_a] } }),
        );
    }

    // signed_ssh: a merge of two branches, every commit signed by SSH key E.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let l1 = r.commit(&[&b0], "left", By::Ssh(&e));
        let r1 = r.commit(&[&b0], "right", By::Ssh(&e));
        let m = r.commit(&[&l1, &r1], "merge", By::Ssh(&e));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "signed_ssh",
            "A merge whose both sides and the merge itself are signed by a trusted SSH key (namespace git).",
            &r, &ledger, Some(&b0), &m, &run_id(2), 1, vec![],
            signed(&[&id_e]),
            json!({ "authorized": { "kind": "signed", "signers": [id_e] } }),
        );
    }

    // The unsigned-tip graph, shared by the override cases.
    let mut tip_repo = Repo::new();
    let tip_b0 = tip_repo.commit(&[], "base", By::Nobody);
    let tip_c1 = tip_repo.commit(&[&tip_b0], "signed", By::PgpSub(&a, 0));
    let tip_c2 = tip_repo.commit(&[&tip_c1], "unsigned tip", By::Nobody);
    let (_, tip_ledger) = ledger_with_base(&tip_b0);
    let tip_check = needs("unsigned", vec![(&tip_c2, "missing")]);

    out.case(
        "unsigned_tip",
        "The tip commit has no signature and no approval was given.",
        &tip_repo,
        &tip_ledger,
        Some(&tip_b0),
        &tip_c2,
        &run_id(3),
        1,
        vec![],
        tip_check.clone(),
        refused_unsigned(vec![(&tip_c2, "missing")]),
    );

    // unsigned_under_signed_merge: a signed merge whose second parent is unsigned.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let s1 = r.commit(&[&b0], "signed side", By::PgpSub(&a, 0));
        let x1 = r.commit(&[&b0], "unsigned side", By::Nobody);
        let m = r.commit(&[&s1, &x1], "signed merge", By::Ssh(&e));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "unsigned_under_signed_merge",
            "A signed merge brings in an unsigned commit; the walk includes merge parents.",
            &r,
            &ledger,
            Some(&b0),
            &m,
            &run_id(4),
            1,
            vec![],
            needs("unsigned", vec![(&x1, "missing")]),
            refused_unsigned(vec![(&x1, "missing")]),
        );
    }

    // unknown_key: one commit by an untrusted OpenPGP key, one by an untrusted SSH key.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "untrusted pgp", By::PgpPrimary(&u));
        let c2 = r.commit(&[&c1], "untrusted ssh", By::Ssh(&x));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "unknown_key",
            "Commits signed by an OpenPGP key and an SSH key that are not in the signers file.",
            &r,
            &ledger,
            Some(&b0),
            &c2,
            &run_id(5),
            1,
            vec![],
            needs("unsigned", vec![(&c2, "unknown_key"), (&c1, "unknown_key")]),
            refused_unsigned(vec![(&c2, "unknown_key"), (&c1, "unknown_key")]),
        );
    }

    // expired_pgp_subkey: signed by A2, which expired at T0-70d.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "expired subkey", By::PgpSub(&a, 1));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "expired_pgp_subkey",
            "Signed by a trusted key's subkey that had expired at the committer time.",
            &r,
            &ledger,
            Some(&b0),
            &c1,
            &run_id(6),
            1,
            vec![],
            needs("unsigned", vec![(&c1, "expired")]),
            refused_unsigned(vec![(&c1, "expired")]),
        );
    }

    // revoked_key: signed by C, revoked as compromised.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "revoked", By::PgpPrimary(&c));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "revoked_key",
            "Signed by a trusted key that carries a key-compromised revocation.",
            &r,
            &ledger,
            Some(&b0),
            &c1,
            &run_id(7),
            1,
            vec![],
            needs("unsigned", vec![(&c1, "revoked")]),
            refused_unsigned(vec![(&c1, "revoked")]),
        );
    }

    // downgrade: the target is on a sibling branch, not a descendant of the base.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "root", By::Nobody);
        let base = r.commit(&[&b0], "applied", By::PgpSub(&a, 0));
        let other = r.commit(&[&b0], "sibling", By::PgpSub(&a, 0));
        let (_, ledger) = ledger_with_base(&base);
        out.case(
            "downgrade",
            "Every commit is signed, but the target is not a descendant of the host's base.",
            &r,
            &ledger,
            Some(&base),
            &other,
            &run_id(8),
            1,
            vec![],
            needs("downgrade", vec![]),
            json!({ "refused": "downgrade" }),
        );
    }

    // reapply_base: the target is the base itself.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "reapply_base",
            "Re-running the base: no new commits, so nothing to verify.",
            &r,
            &ledger,
            Some(&b0),
            &b0,
            &run_id(9),
            1,
            vec![],
            signed(&[]),
            json!({ "authorized": { "kind": "signed", "signers": [] } }),
        );
    }

    // baseline / baseline_without_approval: an empty ledger.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "root", By::PgpSub(&a, 0));
        let c1 = r.commit(&[&b0], "first", By::PgpSub(&a, 0));
        let run = run_id(10);
        let ap = ok(&alice, ApprovalKind::Baseline, &run, None, &c1);
        out.case(
            "baseline",
            "First run on a host: a baseline approval authorizes it.",
            &r, "", None, &c1, &run, 1, vec![ap],
            needs("baseline", vec![]),
            json!({ "authorized": { "kind": "baseline", "approvals": [alice.credential_id], "approvers": ["alice"] } }),
        );
        out.case(
            "baseline_without_approval",
            "First run on a host with every commit signed but no baseline approval.",
            &r,
            "",
            None,
            &c1,
            &run_id(11),
            1,
            vec![],
            needs("baseline", vec![]),
            refused_unsigned(vec![]),
        );
    }

    // override_ok: two distinct approvers (ES256 and EdDSA) meet a quorum of 2.
    {
        let run = run_id(12);
        let approvals = vec![
            ok(&alice, ApprovalKind::Override, &run, Some(&tip_b0), &tip_c2),
            ok(&bob, ApprovalKind::Override, &run, Some(&tip_b0), &tip_c2),
        ];
        out.case(
            "override_ok",
            "An unsigned tip overridden by two distinct approvers, quorum 2.",
            &tip_repo, &tip_ledger, Some(&tip_b0), &tip_c2, &run, 2, approvals,
            tip_check.clone(),
            json!({ "authorized": { "kind": "override", "approvals": [alice.credential_id, bob.credential_id], "approvers": ["alice", "bob"] } }),
        );
    }

    let variant = |spec: ApprovalSpec<'_>, run: &str| approval(spec, run, Some(&tip_b0), &tip_c2);
    let base_spec = |by| ApprovalSpec {
        by,
        kind: ApprovalKind::Override,
        hosts: &[HOST],
        expires_at: NOW + 3600,
        origin: ORIGIN,
        flags: FLAG_UP | FLAG_UV,
    };

    {
        let run = run_id(13);
        let ap = variant(
            ApprovalSpec {
                hosts: &["web-2"],
                ..base_spec(&alice)
            },
            &run,
        );
        out.case(
            "override_other_host",
            "A valid approval for another host (web-2) does not cover web-1: step 1.",
            &tip_repo,
            &tip_ledger,
            Some(&tip_b0),
            &tip_c2,
            &run,
            1,
            vec![ap],
            tip_check.clone(),
            json!({ "refused": "bad_override", "index": 0, "step": 1 }),
        );
    }
    {
        let run = run_id(14);
        let ap = variant(
            ApprovalSpec {
                origin: "https://keel.evil.test",
                ..base_spec(&alice)
            },
            &run,
        );
        out.case(
            "override_wrong_origin",
            "The assertion was made at another origin: step 3.",
            &tip_repo,
            &tip_ledger,
            Some(&tip_b0),
            &tip_c2,
            &run,
            1,
            vec![ap],
            tip_check.clone(),
            json!({ "refused": "bad_override", "index": 0, "step": 3 }),
        );
    }
    {
        let run = run_id(15);
        let ap = variant(
            ApprovalSpec {
                flags: FLAG_UP,
                ..base_spec(&bob)
            },
            &run,
        );
        out.case(
            "override_uv_missing",
            "The authenticator did not verify the user (UV clear): step 4.",
            &tip_repo,
            &tip_ledger,
            Some(&tip_b0),
            &tip_c2,
            &run,
            1,
            vec![ap],
            tip_check.clone(),
            json!({ "refused": "bad_override", "index": 0, "step": 4 }),
        );
    }
    {
        let run = run_id(16);
        let ap = variant(
            ApprovalSpec {
                expires_at: NOW - 60,
                ..base_spec(&alice)
            },
            &run,
        );
        out.case(
            "override_expired",
            "A correctly signed approval whose expires_at has passed: step 6.",
            &tip_repo,
            &tip_ledger,
            Some(&tip_b0),
            &tip_c2,
            &run,
            1,
            vec![ap],
            tip_check.clone(),
            json!({ "refused": "bad_override", "index": 0, "step": 6 }),
        );
    }
    {
        let run = run_id(17);
        let ap = ok(&alice, ApprovalKind::Override, &run, Some(&tip_b0), &tip_c2);
        out.case(
            "override_quorum_short",
            "Quorum 2, but the only approver approved twice: one distinct approver.",
            &tip_repo,
            &tip_ledger,
            Some(&tip_b0),
            &tip_c2,
            &run,
            2,
            vec![ap.clone(), ap],
            tip_check.clone(),
            json!({ "refused": "bad_override", "have": 1, "need": 2 }),
        );
    }

    // replay: the run already has a start line.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "one", By::PgpSub(&a, 0));
        let (mut l, mut ledger) = ledger_with_base(&b0);
        let run = run_id(18);
        ledger += &l
            .append(Entry::Start {
                at: format_utc(T0 + 3600).unwrap(),
                run: run.clone(),
                job: JOB.into(),
                base: Some(CommitId::parse(&b0).unwrap()),
                target: CommitId::parse(&c1).unwrap(),
                auth: Auth::Signed {
                    signers: vec![id_a.clone()],
                },
            })
            .unwrap();
        out.case(
            "replay",
            "The same run is sent again after it started on this host.",
            &r,
            &ledger,
            Some(&b0),
            &c1,
            &run,
            1,
            vec![],
            signed(&[&id_a]),
            json!({ "refused": "replay" }),
        );
    }

    // ledger_corrupt: one value changed after hashing.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "one", By::PgpSub(&a, 0));
        let (_, ledger) = ledger_with_base(&b0);
        let broken = ledger.replacen("\"succeeded\"", "\"failed\"", 1);
        assert_ne!(broken, ledger);
        out.case(
            "ledger_corrupt",
            "The ledger's end line was edited after it was written, breaking the hash chain.",
            &r,
            &broken,
            Some(&b0),
            &c1,
            &run_id(19),
            1,
            vec![],
            Value::Null,
            json!({ "refused": "ledger_corrupt" }),
        );
    }

    // base_mismatch: Keel believes the host is at another commit.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "one", By::PgpSub(&a, 0));
        let c2 = r.commit(&[&c1], "two", By::PgpSub(&a, 0));
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "base_mismatch",
            "Assign says the base is c1, the host's ledger says b0.",
            &r,
            &ledger,
            Some(&c1),
            &c2,
            &run_id(20),
            1,
            vec![],
            signed(&[&id_a]),
            json!({ "refused": "base_mismatch", "host_base": b0 }),
        );
    }

    // pack_incomplete: a commit between base and target is missing.
    {
        let mut r = Repo::new();
        let b0 = r.commit(&[], "base", By::Nobody);
        let c1 = r.commit(&[&b0], "one", By::PgpSub(&a, 0));
        let c2 = r.commit(&[&c1], "two", By::PgpSub(&a, 0));
        r.file.commits.remove(&c1);
        let (_, ledger) = ledger_with_base(&b0);
        out.case(
            "pack_incomplete",
            "The pack lacks a commit between the base and the target.",
            &r,
            &ledger,
            Some(&b0),
            &c2,
            &run_id(21),
            1,
            vec![],
            json!({ "error": "pack_incomplete" }),
            json!({ "refused": "pack_incomplete" }),
        );
    }

    for name in CASES {
        assert!(
            out.files.contains_key(&format!("{name}/case.json")),
            "{name} not generated"
        );
    }
    out.files
}

#[test]
fn vectors_are_current() {
    let files = generate();
    let dir = vectors_dir();
    if std::env::var_os("KEEL_TRUST_REGEN").is_some_and(|v| v == "1") {
        for (path, content) in &files {
            let p = dir.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        return;
    }
    for (path, content) in &files {
        let on_disk = fs::read_to_string(dir.join(path))
            .unwrap_or_else(|_| panic!("{path} missing; run with KEEL_TRUST_REGEN=1"));
        assert!(
            on_disk == *content,
            "{path} is stale; run with KEEL_TRUST_REGEN=1"
        );
    }
}

#[test]
fn generation_is_deterministic() {
    assert_eq!(generate(), generate());
}
