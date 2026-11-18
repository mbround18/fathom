//! Relay revocation and quarantine release (spec 067 T027, T059): revoking closes the link and
//! every runner stream it carries at once, and the relay is never dialled again; releasing a
//! quarantine puts the relay back into service. Both are audited in the same transaction as the
//! row change.
//!
//! Spec 067 US3 (T030-T032): zones, relays and the relay-link key over the protocol, host reach
//! pushes, and the run-preview warning for a zone's only relay.
//!
//! Needs the test cluster (`KEEL_TEST_DATABASE_URL`); skips without it.

mod common;
mod relay_kit;
mod repos;

use keel_exec::codec::read_frame;
use keel_exec::relay::Frame;
use keel_protocol::{
    Cmd, Event, HostReach, RelayLinkKeyState, RelayState as WireState, ZoneDescription, ZoneName,
};
use keel_server::audit::{self, Actor, Decision, Entry};
use keel_server::relays::{
    self, RelayActionError, RelayState, RelaySupervisor, release_relay, revoke_relay,
};
use keel_server::runner_listener::RunnerHost;
use relay_kit::{
    TestRelay, WAIT, detached, harness, key, opts, register, runner_session, wait_live,
};
use repos::{op_error, preview};
use uuid::Uuid;

async fn actions(h: &repos::Harness, action: &str) -> Vec<serde_json::Value> {
    let rows: Vec<(serde_json::Value,)> =
        sqlx::query_as("select context from audit_event where action = $1 order by seq")
            .bind(action)
            .fetch_all(&h.db.owner)
            .await
            .expect("audit");
    rows.into_iter().map(|r| r.0).collect()
}

#[tokio::test]
async fn revoking_a_relay_closes_its_link_and_every_carried_stream() {
    let Some((h, zone)) = harness("relays_revoke").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");
    let mut side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    let (host_id, _ssh, mut stream) = runner_session(&h, &side.handle, "web-1", 40001).await;
    assert_eq!(h.app.runs.path(host_id).and_then(|p| p.relay()), Some(id));

    // Not this workspace's relay: nothing happens.
    assert!(matches!(
        revoke_relay(&h.app, &Actor::System, Uuid::now_v7(), id).await,
        Err(RelayActionError::NotFound)
    ));
    assert!(h.app.runs.path(host_id).is_some());

    revoke_relay(&h.app, &Actor::System, h.workspace, id)
        .await
        .expect("revoke");

    // The relay is told why, …
    let close = tokio::time::timeout(WAIT, side.seen.recv())
        .await
        .expect("no Close")
        .expect("control channel ended without Close");
    assert!(
        matches!(&close, Frame::Close { reason } if reason == "revoked"),
        "{close:?}"
    );
    // … the runner's stream over it ends (its session follows the lost-runner path) …
    let end = tokio::time::timeout(WAIT, async {
        loop {
            match read_frame(&mut stream).await {
                Ok(Some(_)) => {}
                other => return other.map(|_| ()),
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the carried stream stayed open");
    detached(&h, host_id).await;

    // … the row and its audit landed together, …
    let (revoked,): (bool,) =
        sqlx::query_as("select revoked_at is not null from relays where id = $1")
            .bind(id)
            .fetch_one(&h.db.owner)
            .await
            .expect("relay");
    assert!(revoked);
    let rows = actions(&h, relays::ACTION_REVOKE).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["quarantine_reason"], serde_json::Value::Null);

    // … and the relay is never dialled again.
    sup.sync().await.expect("sync");
    assert!(h.app.relay_live.get(id).is_none());
    relay.no_link().await;

    // Revocation is final.
    assert!(matches!(
        revoke_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::Revoked)
    ));
    assert!(matches!(
        release_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::Revoked)
    ));
    assert_eq!(actions(&h, relays::ACTION_REVOKE).await.len(), 1);
    sup.shutdown();
    h.finish().await;
}

#[tokio::test]
async fn releasing_a_quarantined_relay_puts_it_back_in_service() {
    let Some((h, zone)) = harness("relays_release").await else {
        return;
    };
    let mut relay = TestRelay::start(&h).await;
    let id = register(
        &h,
        zone,
        "edge-1",
        &relay.addr.to_string(),
        &relay.host_key_data(),
    )
    .await;
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());

    // Not quarantined: nothing to release.
    assert!(matches!(
        release_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::NotQuarantined)
    ));

    sqlx::query(
        "update relays set quarantined_at = now(), quarantine_reason = 'bad_frame' \
         where id = $1",
    )
    .bind(id)
    .execute(&h.db.owner)
    .await
    .expect("quarantine");
    sup.sync().await.expect("sync");
    wait_live(&h, id, |l| l.state == RelayState::Quarantined).await;
    relay.no_link().await;

    assert!(matches!(
        release_relay(&h.app, &Actor::System, Uuid::now_v7(), id).await,
        Err(RelayActionError::NotFound)
    ));
    release_relay(&h.app, &Actor::System, h.workspace, id)
        .await
        .expect("release");
    let (reason, at): (Option<String>, Option<time::OffsetDateTime>) =
        sqlx::query_as("select quarantine_reason, quarantined_at from relays where id = $1")
            .bind(id)
            .fetch_one(&h.db.owner)
            .await
            .expect("relay");
    assert_eq!((reason, at), (None, None));
    let rows = actions(&h, relays::ACTION_RELEASE).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["quarantine_reason"], "bad_frame");

    // Dialled again.
    sup.sync().await.expect("sync");
    let _side = relay.next_link().await.answer(relay.fingerprint());
    wait_live(&h, id, |l| l.state == RelayState::Connected).await;
    assert!(matches!(
        release_relay(&h.app, &Actor::System, h.workspace, id).await,
        Err(RelayActionError::NotQuarantined)
    ));
    sup.shutdown();
    h.finish().await;
}

// ---- spec 067 US3 over the protocol (T030-T032) ----------------------------------------------

fn zone_name(s: &str) -> ZoneName {
    ZoneName::new(s).expect("name")
}

fn desc(s: &str) -> ZoneDescription {
    ZoneDescription::new(s).expect("description")
}

fn ssh_line() -> String {
    key().public_key().to_openssh().expect("line")
}

async fn add_host(h: &repos::Harness, name: &str, zone: Option<Uuid>) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "insert into hosts (id, workspace_id, inventory_name, zone_id) values ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(h.workspace)
    .bind(name)
    .bind(zone)
    .execute(&h.db.owner)
    .await
    .expect("host");
    id
}

fn zones_of(events: &[Event]) -> Vec<keel_protocol::ZoneInfo> {
    match events {
        [Event::Confirmed { .. }, Event::Zones { items }] | [Event::Zones { items }] => {
            items.clone()
        }
        other => panic!("expected zones, got {other:?}"),
    }
}

fn relays_of(events: &[Event]) -> Vec<keel_protocol::RelayInfo> {
    match events {
        [Event::Confirmed { .. }, Event::Relays { items }] | [Event::Relays { items }] => {
            items.clone()
        }
        other => panic!("expected relays, got {other:?}"),
    }
}

fn keys_of(events: &[Event]) -> Vec<keel_protocol::RelayLinkKey> {
    match events {
        [Event::Confirmed { .. }, Event::RelayLinkKeys { items }]
        | [Event::RelayLinkKeys { items }] => items.clone(),
        other => panic!("expected link keys, got {other:?}"),
    }
}

#[tokio::test]
async fn zones_are_created_renamed_and_removed_only_when_empty() {
    let Some((h, edge)) = harness("relays_zone_crud").await else {
        return;
    };
    let ws = h.workspace;
    let (p, out) = h
        .confirm(Cmd::PreviewZoneCreate {
            workspace: ws,
            name: zone_name("dmz"),
            description: desc("The DMZ"),
        })
        .await;
    assert!(!p.step_up);
    let zones = zones_of(&out);
    assert_eq!(
        zones.iter().map(|z| z.name.as_str()).collect::<Vec<_>>(),
        ["dmz", "edge"]
    );
    let dmz = zones.iter().find(|z| z.name.as_str() == "dmz").unwrap().id;

    // A taken name is refused at preview.
    let out = h
        .cmd(Cmd::PreviewZoneCreate {
            workspace: ws,
            name: zone_name("edge"),
            description: desc(""),
        })
        .await;
    assert_eq!(op_error(&out), "zone_exists");
    let out = h
        .cmd(Cmd::PreviewZoneUpdate {
            zone: dmz,
            name: zone_name("edge"),
            description: desc(""),
        })
        .await;
    assert_eq!(op_error(&out), "zone_exists");

    let (_, out) = h
        .confirm(Cmd::PreviewZoneUpdate {
            zone: dmz,
            name: zone_name("dmz-2"),
            description: desc("Renamed"),
        })
        .await;
    let z = zones_of(&out)
        .into_iter()
        .find(|z| z.id == dmz)
        .expect("dmz");
    assert_eq!(
        (z.name.as_str(), z.description.as_str()),
        ("dmz-2", "Renamed")
    );

    // A host in it: the preview lists it and confirming is refused.
    let host = add_host(&h, "web-1", Some(dmz)).await;
    let listed = zones_of(&h.cmd(Cmd::ListZones { workspace: ws }).await);
    assert_eq!(listed.iter().find(|z| z.id == dmz).unwrap().hosts, 1);
    let p = preview(&h.cmd(Cmd::PreviewZoneRemove { zone: dmz }).await);
    assert!(p.affected.contains(&"host web-1".to_owned()), "{p:?}");
    assert!(!p.warnings.is_empty());
    let out = h
        .cmd(Cmd::Confirm {
            preview_id: p.preview_id,
            step_up: None,
        })
        .await;
    assert_eq!(op_error(&out[1..]), "zone_in_use");

    // Moved out (back to direct), the zone goes.
    let (p, out) = h.confirm(Cmd::PreviewHostZone { host, zone: None }).await;
    assert!(p.summary.contains("to zone direct"), "{}", p.summary);
    assert!(
        matches!(out.as_slice(), [Event::Confirmed { .. }, Event::Host(_)]),
        "{out:?}"
    );
    let (_, out) = h.confirm(Cmd::PreviewZoneRemove { zone: dmz }).await;
    assert!(zones_of(&out).iter().all(|z| z.id != dmz));

    // Another workspace's zone, or none, cannot take a host.
    let out = h
        .cmd(Cmd::PreviewHostZone {
            host,
            zone: Some(Uuid::now_v7()),
        })
        .await;
    assert_eq!(op_error(&out), "unknown_zone");
    // A zone with no relay: the move warns.
    let p = preview(
        &h.cmd(Cmd::PreviewHostZone {
            host,
            zone: Some(edge),
        })
        .await,
    );
    assert!(p.warnings.iter().any(|w| w.contains("no relay")), "{p:?}");

    // A zone that ever had a relay stays, even once the relay is revoked.
    let relay = register(
        &h,
        edge,
        "edge-1",
        "relay.edge:7832",
        &key().public_key().key_data().clone(),
    )
    .await;
    revoke_relay(&h.app, &Actor::System, ws, relay)
        .await
        .expect("revoke");
    let p = preview(&h.cmd(Cmd::PreviewZoneRemove { zone: edge }).await);
    assert!(
        p.affected.contains(&"relay edge-1 (revoked)".to_owned()),
        "{p:?}"
    );

    let audited: Vec<String> = h.audit_actions().await.into_iter().map(|a| a.0).collect();
    for want in ["zone.create", "zone.update", "zone.remove", "host.zone_set"] {
        assert!(audited.iter().any(|a| a == want), "{want}: {audited:?}");
    }
    h.finish().await;
}

#[tokio::test]
async fn relays_are_registered_updated_revoked_and_released_by_command() {
    let Some((h, zone)) = harness("relays_commands").await else {
        return;
    };
    let ws = h.workspace;
    let line = ssh_line();
    let register = |name: &str, line: &str| Cmd::PreviewRelayRegister {
        workspace: ws,
        zone,
        name: zone_name(name),
        dial_address: "relay.edge:7832".into(),
        host_key: line.to_owned(),
        host: None,
    };

    let out = h.cmd(register("edge-1", "ssh-ed25519 nope")).await;
    assert_eq!(op_error(&out), "invalid_host_key");
    let (p, out) = h.confirm(register("edge-1", &line)).await;
    assert!(p.step_up);
    let link = h.app.relay_link.get().expect("link key");
    assert!(
        p.warnings.iter().any(|w| w.contains(&link.public_line)),
        "{p:?}"
    );
    let relays = relays_of(&out);
    assert_eq!(relays.len(), 1);
    let r = &relays[0];
    assert_eq!(
        (r.name.as_str(), r.state),
        ("edge-1", WireState::Connecting)
    );
    let id = r.id;

    // The same host key twice is refused.
    let out = h.cmd(register("edge-2", &line)).await;
    assert_eq!(op_error(&out), "host_key_taken");

    // Update: a new address and a new key, warned about.
    let line2 = ssh_line();
    let (p, out) = h
        .confirm(Cmd::PreviewRelayUpdate {
            relay: id,
            zone,
            name: zone_name("edge-1"),
            dial_address: "relay2.edge:7832".into(),
            host_key: line2,
            host: None,
        })
        .await;
    assert!(
        p.warnings.iter().any(|w| w.contains("host key changes")),
        "{p:?}"
    );
    assert_eq!(relays_of(&out)[0].dial_address, "relay2.edge:7832");

    // Release: refused unless quarantined; the preview says why, when, and what it tried since.
    let out = h.cmd(Cmd::PreviewRelayRelease { relay: id }).await;
    assert_eq!(op_error(&out), "not_quarantined");
    sqlx::query(
        "update relays set quarantined_at = now() - interval '1 minute', \
         quarantine_reason = 'forbidden_channel' where id = $1",
    )
    .bind(id)
    .execute(&h.db.owner)
    .await
    .expect("quarantine");
    let mut tx = h.app.db.begin().await.expect("tx");
    for count in [3, 4] {
        audit::record(
            &mut tx,
            Entry {
                actor: &Actor::System,
                action: relays::ACTION_REFUSED,
                workspace: None,
                target: serde_json::json!({ "relay": id }),
                decision: Decision::Denied,
                reason: Some("forbidden_channel"),
                context: serde_json::json!({ "count": count }),
            },
        )
        .await
        .expect("audit");
    }
    tx.commit().await.expect("commit");
    let listed = relays_of(&h.cmd(Cmd::ListRelays { workspace: ws }).await);
    assert_eq!(listed[0].state, WireState::Quarantined);
    assert_eq!(listed[0].last_error.as_deref(), Some("forbidden_channel"));
    let (p, out) = h.confirm(Cmd::PreviewRelayRelease { relay: id }).await;
    assert!(p.step_up);
    assert!(p.summary.contains("forbidden_channel"), "{}", p.summary);
    assert!(p.summary.contains("7 refused attempt(s)"), "{}", p.summary);
    assert_eq!(relays_of(&out)[0].state, WireState::Connecting);

    // Revoke: no step-up; final.
    let (p, out) = h.confirm(Cmd::PreviewRelayRevoke { relay: id }).await;
    assert!(!p.step_up);
    assert!(
        p.summary.contains("carries no runner session"),
        "{}",
        p.summary
    );
    assert_eq!(relays_of(&out)[0].state, WireState::Revoked);
    let out = h.cmd(Cmd::PreviewRelayRevoke { relay: id }).await;
    assert_eq!(op_error(&out), "relay_revoked");
    let out = h
        .cmd(Cmd::PreviewRelayUpdate {
            relay: id,
            zone,
            name: zone_name("edge-1"),
            dial_address: "relay.edge:7832".into(),
            host_key: ssh_line(),
            host: None,
        })
        .await;
    assert_eq!(op_error(&out), "relay_revoked");

    let audited: Vec<String> = h.audit_actions().await.into_iter().map(|a| a.0).collect();
    for want in [
        "relay.register",
        "relay.update",
        "relay.release",
        "relay.revoke",
    ] {
        assert!(audited.iter().any(|a| a == want), "{want}: {audited:?}");
    }
    h.finish().await;
}

#[tokio::test]
async fn the_relay_link_key_rotates_through_pending() {
    let Some((h, _zone)) = harness("relays_link_key").await else {
        return;
    };
    let keys = keys_of(&h.cmd(Cmd::GetRelayLinkKeys).await);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].state, RelayLinkKeyState::Active);
    let first = keys[0].fingerprint.clone();

    let (p, out) = h.confirm(Cmd::PreviewRelayLinkKeyRotate).await;
    assert!(p.step_up);
    let pending = keys_of(&out)
        .into_iter()
        .find(|k| k.state == RelayLinkKeyState::Pending)
        .expect("pending")
        .fingerprint;
    // Still dialling with the first key; one pending at a time.
    assert_eq!(h.app.relay_link.get().unwrap().fingerprint, first);
    let out = h.cmd(Cmd::PreviewRelayLinkKeyRotate).await;
    assert_eq!(op_error(&out), "link_key_pending");
    let out = h
        .cmd(Cmd::PreviewRelayLinkKeyActivate {
            fingerprint: first.clone(),
        })
        .await;
    assert_eq!(op_error(&out), "link_key_not_pending");

    let (p, out) = h
        .confirm(Cmd::PreviewRelayLinkKeyActivate {
            fingerprint: pending.clone(),
        })
        .await;
    assert!(p.warnings.iter().any(|w| w.contains("hub_keys")), "{p:?}");
    let keys = keys_of(&out);
    let state = |fp: &str| keys.iter().find(|k| k.fingerprint == fp).unwrap().state;
    assert_eq!(state(&first), RelayLinkKeyState::Retired);
    assert_eq!(state(&pending), RelayLinkKeyState::Active);
    assert_eq!(h.app.relay_link.get().unwrap().fingerprint, pending);

    // A pending key can also be dropped.
    let (_, out) = h.confirm(Cmd::PreviewRelayLinkKeyRotate).await;
    let next = keys_of(&out)
        .into_iter()
        .find(|k| k.state == RelayLinkKeyState::Pending)
        .expect("pending")
        .fingerprint;
    let (_, out) = h
        .confirm(Cmd::PreviewRelayLinkKeyRetire {
            fingerprint: next.clone(),
        })
        .await;
    assert!(
        keys_of(&out)
            .iter()
            .all(|k| k.state != RelayLinkKeyState::Pending)
    );
    assert_eq!(h.app.relay_link.get().unwrap().fingerprint, pending);

    let audited: Vec<String> = h.audit_actions().await.into_iter().map(|a| a.0).collect();
    for want in [
        relays::ACTION_CREATE,
        relays::ACTION_KEY_ACTIVATE,
        relays::ACTION_KEY_RETIRE,
    ] {
        assert!(audited.iter().any(|a| a == want), "{want}: {audited:?}");
    }
    h.finish().await;
}

#[tokio::test]
async fn a_host_behind_an_unreachable_relay_is_pushed_relay_unreachable() {
    let Some((h, zone)) = harness("relays_reach_push").await else {
        return;
    };
    // Nothing listens here.
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = dead.local_addr().expect("addr");
    drop(dead);
    let relay = register(
        &h,
        zone,
        "edge-1",
        &addr.to_string(),
        &key().public_key().key_data().clone(),
    )
    .await;
    let host = add_host(&h, "web-1", Some(zone)).await;
    let mut events = h.app.repos.events.subscribe();
    let sup = RelaySupervisor::new(h.app.clone(), RunnerHost::new().expect("host"), opts());
    sup.sync().await.expect("sync");

    let (mut relay_seen, mut reach) = (false, None);
    tokio::time::timeout(WAIT, async {
        while reach.is_none() || !relay_seen {
            let Ok((_, e)) = events.recv().await else {
                continue;
            };
            match e {
                Event::RelayChanged(r) if r.id == relay && r.state == WireState::Unreachable => {
                    relay_seen = true;
                }
                Event::HostReachChanged {
                    host: h2,
                    reach: r @ HostReach::RelayUnreachable { .. },
                } if h2 == host => reach = Some(r),
                _ => {}
            }
        }
    })
    .await
    .expect("pushed");
    assert_eq!(
        reach,
        Some(HostReach::RelayUnreachable {
            relays: vec![relay]
        })
    );
    // And the host list says so too.
    let out = h
        .cmd(Cmd::ListHosts {
            workspace: h.workspace,
            profile: None,
            drift: None,
            changed_outside: None,
        })
        .await;
    let Some(Event::Hosts { items }) = out.first() else {
        panic!("{out:?}");
    };
    let info = items.iter().find(|i| i.id == host).expect("host");
    assert_eq!(info.zone, Some(zone));
    assert!(
        matches!(info.reach, HostReach::RelayUnreachable { .. }),
        "{:?}",
        info.reach
    );
    sup.shutdown();
    h.finish().await;
}

#[tokio::test]
async fn a_run_reaching_a_zones_only_relay_host_warns() {
    let Some((h, zone)) = harness("relays_run_warning").await else {
        return;
    };
    let gw = add_host(&h, "gw-1", None).await;
    let web1 = add_host(&h, "web-1", Some(zone)).await;
    let web2 = add_host(&h, "web-2", Some(zone)).await;
    let relay = register(
        &h,
        zone,
        "edge-1",
        "relay.edge:7832",
        &key().public_key().key_data().clone(),
    )
    .await;
    sqlx::query("update relays set host_id = $2 where id = $1")
        .bind(relay)
        .bind(gw)
        .execute(&h.db.owner)
        .await
        .expect("relay host");

    let warn = |targets: Vec<Uuid>| {
        let db = h.app.db.clone();
        let ws = h.workspace;
        async move {
            relays::run_warnings(&db, ws, &targets)
                .await
                .expect("warnings")
        }
    };
    assert_eq!(
        warn(vec![gw, web1]).await,
        [relays::relay_dependency_warning(
            "edge-1",
            "gw-1",
            &["web-1".into()]
        )]
    );
    assert!(
        warn(vec![web1, web2]).await.is_empty(),
        "the relay host is not in the run"
    );
    assert!(
        warn(vec![gw]).await.is_empty(),
        "no host behind it is in the run"
    );
    // A second relay for the zone: no longer the only path.
    register(
        &h,
        zone,
        "edge-2",
        "relay2.edge:7832",
        &key().public_key().key_data().clone(),
    )
    .await;
    assert!(warn(vec![gw, web1, web2]).await.is_empty());
    h.finish().await;
}
