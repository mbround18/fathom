//! Recovery codes: only the hash is stored, they expire, and they work once.

mod common;

use common::MemStore;
use web_passkeys::PasskeyStore;
use web_passkeys::recovery::{
    self, RECOVERY_CODE_LEN, RECOVERY_TTL_SECS, RecoveryError, hash_code, issue_from_entropy,
    matches, normalize,
};

const NOW: i64 = 1_800_000_000;

#[test]
fn a_code_is_crockford_and_only_its_hash_is_kept() {
    let issued = issue_from_entropy("alice", NOW, [0, 1, 2, 31, 32, 63, 200, 255, 17, 9]);
    assert_eq!(issued.code, "012Z0Z8ZH9");
    assert_eq!(issued.code.len(), RECOVERY_CODE_LEN);
    let r = &issued.record;
    assert_eq!(r.user, "alice");
    assert_eq!(r.expires, NOW + RECOVERY_TTL_SECS);
    assert!(!r.used);
    assert_eq!(r.code_sha256.len(), 64);
    assert_eq!(r.code_sha256, hash_code("012Z0Z8ZH9"));
    // The stored record never contains the code.
    let json = serde_json::to_string(r).unwrap();
    assert!(!json.contains("012Z0Z8ZH9"));
    // Neither does Debug output.
    assert!(!format!("{issued:?}").contains("012Z0Z8ZH9"));
}

#[test]
fn codes_are_normalized_before_hashing() {
    assert_eq!(normalize("o1i-l 2z"), "01112Z");
    assert_eq!(hash_code("012z-0z8z h9"), hash_code("012Z0Z8ZH9"));
    assert_eq!(hash_code("O12Z0Z8ZH9"), hash_code("012Z0Z8ZH9"));
    assert_ne!(hash_code("012Z0Z8ZH8"), hash_code("012Z0Z8ZH9"));
}

#[test]
fn matches_checks_hash_use_and_expiry() {
    let issued = issue_from_entropy("alice", NOW, [5; RECOVERY_CODE_LEN]);
    let r = issued.record.clone();
    assert!(matches(&r, &issued.code, NOW));
    assert!(matches(&r, &issued.code, NOW + RECOVERY_TTL_SECS - 1));
    assert!(!matches(&r, &issued.code, NOW + RECOVERY_TTL_SECS));
    assert!(!matches(&r, "0000000000", NOW));
    let used = recovery::RecoveryRecord { used: true, ..r };
    assert!(!matches(&used, &issued.code, NOW));
}

#[tokio::test]
async fn redeem_works_once() {
    let store = MemStore::default();
    let issued = recovery::issue(&store, "alice", NOW).await.unwrap();
    assert_eq!(issued.code.len(), RECOVERY_CODE_LEN);
    let stored = store.recovery("alice").await.unwrap().unwrap();
    assert_eq!(stored, issued.record);

    // Wrong code first: refused, and the right code still works afterwards.
    let wrong = if issued.code.starts_with('0') {
        "1111111111"
    } else {
        "0000000000"
    };
    assert!(matches!(
        recovery::redeem(&store, "alice", wrong, NOW + 1).await,
        Err(RecoveryError::Invalid)
    ));
    recovery::redeem(&store, "alice", &issued.code, NOW + 2)
        .await
        .unwrap();
    assert!(store.recovery("alice").await.unwrap().unwrap().used);

    // Single use.
    assert!(matches!(
        recovery::redeem(&store, "alice", &issued.code, NOW + 3).await,
        Err(RecoveryError::Invalid)
    ));
}

#[tokio::test]
async fn redeem_refuses_expired_replaced_and_other_users_codes() {
    let store = MemStore::default();
    let first = issue_from_entropy("alice", NOW, [1; RECOVERY_CODE_LEN]);
    store.put_recovery(first.record.clone()).await.unwrap();

    // Expired.
    assert!(matches!(
        recovery::redeem(&store, "alice", &first.code, NOW + RECOVERY_TTL_SECS).await,
        Err(RecoveryError::Invalid)
    ));

    // Bob has no code; alice's code does not work for bob.
    assert!(matches!(
        recovery::redeem(&store, "bob", &first.code, NOW).await,
        Err(RecoveryError::Invalid)
    ));

    // A new code replaces the old one.
    let second = issue_from_entropy("alice", NOW, [2; RECOVERY_CODE_LEN]);
    store.put_recovery(second.record.clone()).await.unwrap();
    assert!(matches!(
        recovery::redeem(&store, "alice", &first.code, NOW).await,
        Err(RecoveryError::Invalid)
    ));
    recovery::redeem(&store, "alice", &second.code, NOW)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_lost_compare_and_set_refuses() {
    // Two redemptions race: the store's compare-and-set lets only one through.
    let store = MemStore::default();
    let issued = issue_from_entropy("alice", NOW, [3; RECOVERY_CODE_LEN]);
    store.put_recovery(issued.record.clone()).await.unwrap();
    assert!(
        store
            .consume_recovery("alice", &issued.record.code_sha256)
            .await
            .unwrap()
    );
    assert!(
        !store
            .consume_recovery("alice", &issued.record.code_sha256)
            .await
            .unwrap()
    );
    assert!(matches!(
        recovery::redeem(&store, "alice", &issued.code, NOW).await,
        Err(RecoveryError::Invalid)
    ));
}
