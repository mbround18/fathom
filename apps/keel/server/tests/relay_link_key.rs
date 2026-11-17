//! Keel's relay-link key (spec 067 T011): made once, sealed under the master key with its own
//! AAD, audited, and loaded on every start.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod repos;

use keel_server::relays;
use repos::Harness;

#[tokio::test]
async fn the_relay_link_key_is_created_once_sealed_and_audited() {
    let Some(h) = Harness::new("relay_link_key").await else {
        return;
    };
    assert!(h.app.relay_link.get().is_none(), "loaded before bootstrap");
    relays::bootstrap(&h.app).await.expect("bootstrap");
    let first = h.app.relay_link.get().expect("loaded");
    assert!(first.public_line.starts_with("ssh-ed25519 "));
    assert!(
        first.public_line.ends_with(" keel"),
        "{}",
        first.public_line
    );
    assert!(first.fingerprint.starts_with("SHA256:"));
    assert_eq!(
        keel_exec::ca::fingerprint(first.key().public_key().key_data()),
        first.fingerprint
    );
    assert!(!format!("{first:?}").contains("PRIVATE"));

    // A second start loads the same key and makes no new one.
    relays::bootstrap(&h.app).await.expect("second bootstrap");
    let second = h.app.relay_link.get().expect("loaded");
    assert_eq!(
        (second.id, &second.fingerprint),
        (first.id, &first.fingerprint)
    );
    let rows: Vec<(String, String)> =
        sqlx::query_as("select state, to_jsonb(k)::text from relay_link_keys k")
            .fetch_all(&h.db.owner)
            .await
            .expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "active");
    assert!(!rows[0].1.contains("OPENSSH PRIVATE KEY"), "not sealed");

    // Sealed under its own AAD: the bytes do not open as another row's.
    let master = h.app.repos.master_key.clone().expect("master key");
    let (sealed,): (Vec<u8>,) = sqlx::query_as("select sealed_private_key from relay_link_keys")
        .fetch_one(&h.db.owner)
        .await
        .expect("sealed");
    assert!(
        master
            .open(format!("relay_link_key:{}", first.id).as_bytes(), &sealed)
            .is_ok()
    );
    assert!(
        master
            .open(format!("ssh_ca:{}", first.id).as_bytes(), &sealed)
            .is_err()
    );

    let (audits,): (i64,) = sqlx::query_as(
        "select count(*) from audit_event where action = $1 and target->>'relay_link_key' = $2",
    )
    .bind(relays::ACTION_CREATE)
    .bind(first.id.to_string())
    .fetch_one(&h.db.owner)
    .await
    .expect("audit");
    assert_eq!(audits, 1);
    h.finish().await;
}
