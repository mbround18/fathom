//! T074: runner/host certificates, the KRL and CA rotation. Cross-checked against `ssh-keygen`
//! when it is installed (skipped otherwise).

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keel_exec::ca::{
    CaError, CaKind, CertRefusal, HostCertRequest, HostCertVerifier, RunnerCertRequest, RunnerCertVerifier, SshCa,
    TrustedCa,
};
use keel_exec::krl::Krl;
use russh::keys::ssh_key::certificate::{Builder, CertType};
use russh::keys::{Algorithm, Certificate, PrivateKey};

const DAY: Duration = Duration::from_secs(24 * 3600);

fn t0() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn key() -> PrivateKey {
    PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn runner_cert(ca: &SshCa, runner: &PrivateKey, now: SystemTime) -> Certificate {
    let assigned = strings(&["web", "base", "db"]);
    let allowed = strings(&["base", "web", "monitoring"]);
    ca.issue_runner_cert(&RunnerCertRequest {
        runner_id: "r-42",
        inventory_name: "web-1",
        assigned_profiles: &assigned,
        allowed_profiles: &allowed,
        public_key: runner.public_key().key_data(),
        ttl: DAY,
        now,
    })
    .unwrap()
}

#[test]
fn runner_certificates_have_the_contract_shape() {
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let runner = key();
    let cert = runner_cert(&ca, &runner, t0());

    assert_eq!(cert.cert_type(), CertType::User);
    assert_eq!(cert.key_id(), "runner:r-42");
    // host:<name>, then profile:<p> for assigned ∩ allowed (db is not allowed, monitoring not
    // assigned).
    assert_eq!(cert.valid_principals(), ["host:web-1", "profile:base", "profile:web"]);
    assert_ne!(cert.serial(), 0);
    assert_eq!(cert.valid_after(), unix(t0()));
    assert_eq!(cert.valid_before(), unix(t0()) + DAY.as_secs());
    assert!(cert.extensions().is_empty(), "no extensions: no pty, no forwarding, no agent");
    assert!(cert.critical_options().is_empty());
    assert_eq!(cert.public_key(), runner.public_key().key_data());
    assert_eq!(cert.signature_key(), ca.key_data());

    let verifier = RunnerCertVerifier::new(vec![TrustedCa::active(ca.key_data().clone())], Krl::default());
    let id = verifier.verify(&cert, t0() + Duration::from_secs(60)).unwrap();
    assert_eq!(id.runner_id, "r-42");
    assert_eq!(id.inventory_name, "web-1");
    assert_eq!(id.profiles, ["base", "web"]);
    assert_eq!(id.serial, cert.serial());
    assert_eq!(id.ca_fingerprint, ca.fingerprint());
}

#[test]
fn serials_are_unique() {
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let runner = key();
    let serials: HashSet<u64> = (0..500).map(|_| runner_cert(&ca, &runner, t0()).serial()).collect();
    assert_eq!(serials.len(), 500);
}

#[test]
fn validity_follows_the_ttl() {
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let cert = runner_cert(&ca, &key(), t0());
    let v = RunnerCertVerifier::new(vec![TrustedCa::active(ca.key_data().clone())], Krl::default());
    assert_eq!(v.verify(&cert, t0() - Duration::from_secs(1)), Err(CertRefusal::NotYetValid));
    assert!(v.verify(&cert, t0() + DAY - Duration::from_secs(1)).is_ok());
    assert_eq!(v.verify(&cert, t0() + DAY), Err(CertRefusal::Expired));
}

#[test]
fn certificates_of_the_wrong_kind_or_ca_are_refused() {
    let runner_ca = SshCa::generate(CaKind::Runner).unwrap();
    let host_ca = SshCa::generate(CaKind::Host).unwrap();
    let k = key();
    let v = RunnerCertVerifier::new(vec![TrustedCa::active(runner_ca.key_data().clone())], Krl::default());

    // A CA refuses to issue the other kind.
    let names = strings(&["keel.example"]);
    let host_req = HostCertRequest { host_names: &names, public_key: k.public_key().key_data(), ttl: DAY, now: t0() };
    assert!(matches!(runner_ca.issue_host_cert(&host_req), Err(CaError::WrongKind { .. })));
    assert!(matches!(
        host_ca.issue_runner_cert(&RunnerCertRequest {
            runner_id: "r",
            inventory_name: "h",
            assigned_profiles: &[],
            allowed_profiles: &[],
            public_key: k.public_key().key_data(),
            ttl: DAY,
            now: t0(),
        }),
        Err(CaError::WrongKind { .. })
    ));

    // A host certificate is not a runner certificate.
    let host_cert = host_ca.issue_host_cert(&host_req).unwrap();
    let v_host = RunnerCertVerifier::new(vec![TrustedCa::active(host_ca.key_data().clone())], Krl::default());
    assert_eq!(v_host.verify(&host_cert, t0()), Err(CertRefusal::WrongType));

    // Untrusted CA.
    let other = SshCa::generate(CaKind::Runner).unwrap();
    assert_eq!(v.verify(&runner_cert(&other, &k, t0()), t0()), Err(CertRefusal::UntrustedCa));

    // Non-Ed25519 subject keys are refused at issue.
    let ecdsa = PrivateKey::random(
        &mut rand::rng(),
        Algorithm::Ecdsa { curve: russh::keys::EcdsaCurve::NistP256 },
    )
    .unwrap();
    assert!(matches!(
        runner_ca.issue_runner_cert(&RunnerCertRequest {
            runner_id: "r",
            inventory_name: "h",
            assigned_profiles: &[],
            allowed_profiles: &[],
            public_key: ecdsa.public_key().key_data(),
            ttl: DAY,
            now: t0(),
        }),
        Err(CaError::NotEd25519(_))
    ));
}

/// Certificates signed by a trusted CA key but shaped differently from what Keel issues.
#[test]
fn foreign_shaped_certificates_are_refused() {
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let ca_key = PrivateKey::from_openssh(ca.to_openssh().unwrap().expose()).unwrap();
    let v = RunnerCertVerifier::new(vec![TrustedCa::active(ca.key_data().clone())], Krl::default());
    let subject = key();
    let now = unix(t0());

    let build = |f: &dyn Fn(&mut Builder)| {
        let mut b = Builder::new_with_random_nonce(
            &mut rand::rng(),
            subject.public_key().key_data().clone(),
            now,
            now + 3600,
        )
        .unwrap();
        b.serial(7).unwrap();
        b.cert_type(CertType::User).unwrap();
        f(&mut b);
        b.sign(&ca_key).unwrap()
    };

    let with_ext = build(&|b| {
        b.key_id("runner:r").unwrap();
        b.valid_principal("host:h").unwrap();
        b.extension("permit-pty", "").unwrap();
    });
    assert_eq!(v.verify(&with_ext, t0()), Err(CertRefusal::Extensions));

    let with_opt = build(&|b| {
        b.key_id("runner:r").unwrap();
        b.valid_principal("host:h").unwrap();
        b.critical_option("force-command", "/bin/sh").unwrap();
    });
    assert_eq!(v.verify(&with_opt, t0()), Err(CertRefusal::CriticalOptions));

    let bad_id = build(&|b| {
        b.key_id("admin").unwrap();
        b.valid_principal("host:h").unwrap();
    });
    assert_eq!(v.verify(&bad_id, t0()), Err(CertRefusal::BadKeyId));

    let any_principal = build(&|b| {
        b.key_id("runner:r").unwrap();
        b.all_principals_valid().unwrap();
    });
    assert_eq!(v.verify(&any_principal, t0()), Err(CertRefusal::BadPrincipals));

    let two_hosts = build(&|b| {
        b.key_id("runner:r").unwrap();
        b.valid_principal("host:a").unwrap();
        b.valid_principal("host:b").unwrap();
    });
    assert_eq!(v.verify(&two_hosts, t0()), Err(CertRefusal::BadPrincipals));

    let stray = build(&|b| {
        b.key_id("runner:r").unwrap();
        b.valid_principal("host:a").unwrap();
        b.valid_principal("root").unwrap();
    });
    assert_eq!(v.verify(&stray, t0()), Err(CertRefusal::BadPrincipals));
}

#[test]
fn a_revoked_serial_in_the_krl_is_refused() {
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let runner = key();
    let revoked = runner_cert(&ca, &runner, t0());
    let fine = runner_cert(&ca, &runner, t0());

    let mut krl = Krl::new(1, unix(t0()));
    krl.revoke_serial(ca.key_data(), revoked.serial());
    // The KRL survives the wire (`Krl { bytes }`).
    let krl = Krl::from_bytes(&krl.to_bytes()).unwrap();

    let v = RunnerCertVerifier::new(vec![TrustedCa::active(ca.key_data().clone())], krl.clone());
    assert_eq!(v.verify(&revoked, t0()), Err(CertRefusal::Revoked));
    assert!(v.verify(&fine, t0()).is_ok());

    // The same serial from another CA is not revoked by this CA's section.
    let other = SshCa::generate(CaKind::Runner).unwrap();
    assert!(!krl.is_certificate_revoked(&runner_cert(&other, &runner, t0())));

    // Revoking the enrolled key revokes every certificate for it, and the bare key.
    let mut krl = Krl::new(2, unix(t0()));
    krl.revoke_key(runner.public_key().key_data());
    assert!(krl.is_certificate_revoked(&fine));
    assert!(krl.is_key_revoked(runner.public_key().key_data()));
    assert!(!krl.is_key_revoked(key().public_key().key_data()));

    // Ranges and key ids.
    let mut krl = Krl::new(3, 0);
    krl.revoke_serial_range(ca.key_data(), fine.serial().saturating_sub(1), fine.serial().saturating_add(1));
    assert!(krl.is_certificate_revoked(&fine));
    let mut krl = Krl::new(4, 0);
    krl.revoke_key_id(ca.key_data(), "runner:r-42");
    assert!(Krl::from_bytes(&krl.to_bytes()).unwrap().is_certificate_revoked(&fine));
}

#[test]
fn a_retiring_ca_stays_trusted_until_its_certificates_expire() {
    let old = SshCa::generate(CaKind::Runner).unwrap();
    let new = SshCa::generate(CaKind::Runner).unwrap();
    let runner = key();
    let old_cert = runner_cert(&old, &runner, t0());
    // Rotation one hour later: the old CA retires once its last certificate expires.
    let rotated = t0() + Duration::from_secs(3600);
    let new_cert = runner_cert(&new, &runner, rotated);
    let v = RunnerCertVerifier::new(
        vec![
            TrustedCa::active(new.key_data().clone()),
            TrustedCa::retiring(old.key_data().clone(), old_cert.valid_before()),
        ],
        Krl::default(),
    );
    assert!(v.verify(&old_cert, rotated).is_ok(), "old certificates keep working after rotation");
    assert!(v.verify(&new_cert, rotated).is_ok());

    // A certificate the old CA signed with a longer validity is not trusted past the
    // retirement point.
    let old_key = PrivateKey::from_openssh(old.to_openssh().unwrap().expose()).unwrap();
    let mut b = Builder::new_with_random_nonce(
        &mut rand::rng(),
        runner.public_key().key_data().clone(),
        unix(t0()),
        unix(t0()) + 10 * DAY.as_secs(),
    )
    .unwrap();
    b.serial(9).unwrap();
    b.cert_type(CertType::User).unwrap();
    b.key_id("runner:r-42").unwrap();
    b.valid_principal("host:web-1").unwrap();
    let long = b.sign(&old_key).unwrap();
    let after = UNIX_EPOCH + Duration::from_secs(old_cert.valid_before());
    assert_eq!(v.verify(&long, after), Err(CertRefusal::RetiredCa));
    assert!(v.verify(&new_cert, after).is_ok());
}

#[test]
fn host_certificates_and_the_runner_side_check() {
    let host_ca = SshCa::generate(CaKind::Host).unwrap();
    let server_key = key();
    let names = strings(&["keel.example", "10.0.0.5"]);
    let cert = host_ca
        .issue_host_cert(&HostCertRequest {
            host_names: &names,
            public_key: server_key.public_key().key_data(),
            ttl: 30 * DAY,
            now: t0(),
        })
        .unwrap();
    assert_eq!(cert.cert_type(), CertType::Host);
    assert_eq!(cert.key_id(), "keel-host:keel.example");
    assert_eq!(cert.valid_principals(), names.as_slice());
    assert!(cert.valid_after() < unix(t0()), "backdated for clock skew");

    let trusted = vec![TrustedCa::active(host_ca.key_data().clone())];
    assert!(HostCertVerifier::new(trusted.clone(), "keel.example").verify(&cert, t0()).is_ok());
    assert_eq!(
        HostCertVerifier::new(trusted, "evil.example").verify(&cert, t0()),
        Err(CertRefusal::BadPrincipals)
    );
    let other = SshCa::generate(CaKind::Host).unwrap();
    assert_eq!(
        HostCertVerifier::new(vec![TrustedCa::active(other.key_data().clone())], "keel.example").verify(&cert, t0()),
        Err(CertRefusal::UntrustedCa)
    );

    let line = host_ca.cert_authority_line("*");
    assert!(line.starts_with("@cert-authority * ssh-ed25519 AAAA"), "{line}");
    assert!(host_ca.fingerprint().starts_with("SHA256:"));
    assert!(!format!("{host_ca:?}").contains("PRIVATE"), "Debug never shows the key");
}

#[test]
fn ca_keys_round_trip_through_openssh_pem() {
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let pem = ca.to_openssh().unwrap();
    assert!(!format!("{pem:?}").contains("PRIVATE"));
    let back = SshCa::from_openssh(CaKind::Runner, pem.expose()).unwrap();
    assert_eq!(back.fingerprint(), ca.fingerprint());
}

// ---- ssh-keygen cross-checks ----

fn ssh_keygen(args: &[&str], dir: &Path) -> Option<std::process::Output> {
    match Command::new("ssh-keygen").args(args).current_dir(dir).output() {
        Ok(out) => Some(out),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping: ssh-keygen is not installed");
            None
        }
        Err(e) => panic!("ssh-keygen: {e}"),
    }
}

fn write_cert(dir: &Path, name: &str, cert: &Certificate) {
    std::fs::write(dir.join(name), cert.to_openssh().unwrap() + "\n").unwrap();
}

#[test]
fn ssh_keygen_reads_our_certificates_and_krl() {
    let dir = tempfile::tempdir().unwrap();
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    let runner = key();
    let revoked = runner_cert(&ca, &runner, SystemTime::now());
    let fine = runner_cert(&ca, &runner, SystemTime::now());
    write_cert(dir.path(), "revoked-cert.pub", &revoked);
    write_cert(dir.path(), "fine-cert.pub", &fine);

    let Some(out) = ssh_keygen(&["-L", "-f", "revoked-cert.pub"], dir.path()) else { return };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shown = String::from_utf8_lossy(&out.stdout);
    assert!(shown.contains("Key ID: \"runner:r-42\""), "{shown}");
    assert!(shown.contains(&format!("Serial: {}", revoked.serial())), "{shown}");
    assert!(shown.contains("Critical Options: (none)"), "{shown}");
    assert!(shown.contains("Extensions: (none)"), "{shown}");
    assert!(shown.contains("host:web-1") && shown.contains("profile:web"), "{shown}");

    let mut krl = Krl::new(1, 0);
    krl.revoke_serial(ca.key_data(), revoked.serial());
    krl.revoke_key_id(ca.key_data(), "runner:gone");
    krl.revoke_key(key().public_key().key_data());
    std::fs::write(dir.path().join("keel.krl"), krl.to_bytes()).unwrap();

    let out = ssh_keygen(&["-Q", "-f", "keel.krl", "revoked-cert.pub"], dir.path()).unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && text.contains("REVOKED"), "{text}");
    let out = ssh_keygen(&["-Q", "-f", "keel.krl", "fine-cert.pub"], dir.path()).unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
}

#[test]
fn we_read_krls_written_by_ssh_keygen() {
    let dir = tempfile::tempdir().unwrap();
    let ca = SshCa::generate(CaKind::Runner).unwrap();
    std::fs::write(dir.path().join("ca.pub"), ca.public_line() + "\n").unwrap();
    let bare = key();
    let hashed = key();
    let hashed_fp = keel_exec::ca::fingerprint(hashed.public_key().key_data());

    // A mix that makes ssh-keygen choose lists, ranges and bitmaps.
    let mut spec = String::new();
    spec.push_str("serial: 7\n");
    spec.push_str("serial: 1000-2000\n");
    for s in (5000..5200).step_by(3) {
        spec.push_str(&format!("serial: {s}\n"));
    }
    spec.push_str("id: runner:stolen\n");
    spec.push_str(&format!("key: {}\n", bare.public_key().to_openssh().unwrap()));
    // `hash:` takes a SHA256 fingerprint and yields a FINGERPRINT_SHA256 section.
    spec.push_str(&format!("hash: {hashed_fp}\n"));
    std::fs::write(dir.path().join("spec"), spec).unwrap();

    let Some(out) = ssh_keygen(&["-k", "-f", "theirs.krl", "-s", "ca.pub", "spec"], dir.path()) else { return };
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let krl = Krl::from_bytes(&std::fs::read(dir.path().join("theirs.krl")).unwrap()).unwrap();

    let ca_key = PrivateKey::from_openssh(ca.to_openssh().unwrap().expose()).unwrap();
    let cert = |serial: u64, id: &str| {
        let mut b = Builder::new_with_random_nonce(&mut rand::rng(), key().public_key().key_data().clone(), 0, u64::MAX)
            .unwrap();
        b.serial(serial).unwrap();
        b.cert_type(CertType::User).unwrap();
        b.key_id(id).unwrap();
        b.valid_principal("host:h").unwrap();
        b.sign(&ca_key).unwrap()
    };
    for s in [7, 1000, 1500, 2000, 5000, 5003, 5198] {
        assert!(krl.is_certificate_revoked(&cert(s, "runner:x")), "serial {s}");
    }
    for s in [6, 8, 999, 2001, 5001, 5002, 5199, 9999] {
        assert!(!krl.is_certificate_revoked(&cert(s, "runner:x")), "serial {s}");
    }
    assert!(krl.is_certificate_revoked(&cert(42, "runner:stolen")));
    assert!(krl.is_key_revoked(bare.public_key().key_data()));
    assert!(krl.is_key_revoked(hashed.public_key().key_data()));
    assert!(!krl.is_key_revoked(key().public_key().key_data()));

    // And our re-encoding of it is still read by ssh-keygen as revoking the same certificate.
    std::fs::write(dir.path().join("ours.krl"), krl.to_bytes()).unwrap();
    write_cert(dir.path(), "c.pub", &cert(1500, "runner:x"));
    let out = ssh_keygen(&["-Q", "-f", "ours.krl", "c.pub"], dir.path()).unwrap();
    assert!(!out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
}
