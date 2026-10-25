//! The network gate (spec 052), wired to the server: records where each person signs in and
//! makes requests from, decides whether that network is trusted, refuses what an untrusted
//! network may not do, and audits it all.
//!
//! The rule itself is [`depths_core::network::trust`]; the restricted set is
//! [`depths_core::role::restricted_ok`]. This module is the glue: the registry
//! ([`crate::networks`]), the per-session side table ([`crate::session_meta`]), the offline
//! geo-IP lookup ([`crate::geoip`]) and the audit log.
//!
//! Every caller checks the role first (`permits`) and only then asks [`Trust::gate`], so a
//! person the role refuses still sees `not_permitted`, never `untrusted_network`.

use std::net::IpAddr;
use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use depths_core::network::{Recorded, TrustDecision, TrustReason};
use depths_core::role::{CommandKind, restricted_ok};
use depths_protocol::{Actor, AuditAction, AuditEntry, AuditNetwork, AuditOutcome};
use tokio::sync::broadcast;
use web_auth::session::SessionStore;

use crate::audit::Audit;
use crate::client_ip::ProxyWatch;
use crate::config::Config;
use crate::geoip::GeoIp;
use crate::networks::{NetworkError, Networks};
use crate::session_meta::SessionTable;

/// The gate and everything it remembers.
pub struct Trust {
    pub networks: Networks,
    pub sessions: Arc<SessionTable>,
    pub geo: GeoIp,
    /// FR-021: the proxy-misconfiguration heuristic.
    pub proxy: ProxyWatch,
    audit: Arc<Audit>,
    /// Session handles whose shells must close now (FR-012): every socket listens and closes
    /// its own shells when the handle is its session.
    shells: broadcast::Sender<String>,
}

impl Trust {
    /// Opens `<data_dir>/networks.json` and the geo-IP file, and starts dropping side-table rows
    /// of ended sessions (when a runtime is running).
    pub fn open(cfg: &Config, audit: Arc<Audit>, store: &SessionStore) -> Result<Self, String> {
        let path = cfg.data_dir.join("networks.json");
        let networks =
            Networks::open(path, cfg.network.policy.clone(), Utc::now()).map_err(|e| e.0)?;
        let sessions = Arc::new(SessionTable::new());
        if tokio::runtime::Handle::try_current().is_ok() {
            sessions.spawn_reaper(store);
        }
        let proxy = ProxyWatch::new(cfg.network.policy.gate && cfg.trusted_proxies.is_empty());
        Ok(Self {
            networks,
            sessions,
            geo: GeoIp::open(cfg.network.geoip_db.as_deref()),
            proxy,
            audit,
            shells: broadcast::channel(64).0,
        })
    }

    fn write(&self, e: &AuditEntry) {
        if let Err(err) = self.audit.record(e) {
            tracing::error!(error = %err, "audit write failed");
        }
    }

    /// Records one use of `ip` by `user` and audits a first or new network.
    fn record(&self, user: &str, actor: &Actor, ip: IpAddr, now: DateTime<Utc>) {
        match self.networks.record(user, ip, now) {
            Ok((Recorded::Seen, _)) => {}
            Ok((how, key)) => {
                let action = if how == Recorded::First {
                    AuditAction::NetworkFirst
                } else {
                    AuditAction::NetworkSeen
                };
                let mut e = crate::audit::entry(actor.clone(), action, ip, AuditOutcome::Succeeded);
                e.network = Some(AuditNetwork {
                    network: Some(key.to_string()),
                    ..AuditNetwork::default()
                });
                self.write(&e);
            }
            Err(NetworkError::Storage(err)) => {
                tracing::error!(error = %err, "network registry write failed");
            }
            Err(_) => {}
        }
    }

    /// At sign-in: the session's address and browser, and the network use. A person's very first
    /// network is recorded here, so it is trusted from their first request.
    pub fn signed_in(
        &self,
        user: &str,
        actor: &Actor,
        handle: &str,
        ip: IpAddr,
        user_agent: Option<&str>,
    ) {
        let now = Utc::now();
        self.proxy.saw(ip);
        self.sessions.insert(handle, ip, user_agent, now);
        self.record(user, actor, ip, now);
    }

    /// An authenticated request: notes the session's address, records the network at most once
    /// an hour (or when the address changes), and returns the decision. An untrusted request
    /// ends the session's shells (FR-012).
    pub fn observe(&self, user: &str, actor: &Actor, handle: &str, ip: IpAddr) -> TrustDecision {
        let now = Utc::now();
        self.proxy.saw(ip);
        if self.sessions.seen(handle, ip, now) {
            self.record(user, actor, ip, now);
        }
        let d = self.networks.trust(user, ip, now);
        if !d.trusted {
            self.end_shells(handle);
        }
        d
    }

    /// The decision, without recording anything.
    pub fn decide(&self, user: &str, ip: IpAddr) -> TrustDecision {
        self.networks.trust(user, ip, Utc::now())
    }

    /// Call after the role check. `Ok` when `kind` is in the restricted set or the network is
    /// trusted; otherwise the refusal is audited (`network_refused`), the session's shells end,
    /// and the decision comes back for the reply.
    pub fn gate(
        &self,
        user: &str,
        actor: &Actor,
        handle: &str,
        ip: IpAddr,
        kind: CommandKind,
        cmd: &str,
    ) -> Result<(), TrustDecision> {
        if restricted_ok(kind) {
            return Ok(());
        }
        let d = self.decide(user, ip);
        if d.trusted {
            return Ok(());
        }
        let mut e = crate::audit::entry(
            actor.clone(),
            AuditAction::NetworkRefused,
            ip,
            AuditOutcome::Denied {
                reason: format!(
                    "{cmd} refused: network {} is not trusted ({})",
                    d.network,
                    reason_text(d.reason)
                ),
            },
        );
        e.network = Some(AuditNetwork {
            network: Some(d.network.to_string()),
            cmd: Some(cmd.chars().take(64).collect()),
            ..AuditNetwork::default()
        });
        self.write(&e);
        self.end_shells(handle);
        Err(d)
    }

    /// Asks every socket of `handle` to close its shells.
    pub fn end_shells(&self, handle: &str) {
        let _ = self.shells.send(handle.to_owned());
    }

    pub fn subscribe_shells(&self) -> broadcast::Receiver<String> {
        self.shells.subscribe()
    }

    /// Writes a network or session audit entry built by a route.
    pub fn audit(&self, e: &AuditEntry) {
        self.write(e);
    }
}

/// Plain words for a reason, for audit text and logs.
pub fn reason_text(r: TrustReason) -> &'static str {
    match r {
        TrustReason::Always => "always trusted",
        TrustReason::First => "first network",
        TrustReason::Whitelist => "whitelisted",
        TrustReason::Matured => "home, matured",
        TrustReason::GateOff => "gate off",
        TrustReason::Unknown => "not answered yet",
        TrustReason::Public => "public",
        TrustReason::Maturing => "home, not matured yet",
    }
}

pub fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The wire form of a reason.
pub fn wire_reason(r: TrustReason) -> depths_protocol::TrustReason {
    use depths_protocol::TrustReason as W;
    match r {
        TrustReason::Always => W::Always,
        TrustReason::First => W::First,
        TrustReason::Whitelist => W::Whitelist,
        TrustReason::Matured => W::Matured,
        TrustReason::GateOff => W::GateOff,
        TrustReason::Unknown => W::Unknown,
        TrustReason::Public => W::Public,
        TrustReason::Maturing => W::Maturing,
    }
}

/// The wire form of a decision.
pub fn wire(d: &TrustDecision) -> depths_protocol::TrustDecision {
    depths_protocol::TrustDecision {
        trusted: d.trusted,
        reason: wire_reason(d.reason),
        matures_at: d.matures_at.map(rfc3339),
        network: d.network.to_string(),
    }
}

/// The HTTP 403 body for a refused request.
pub fn refusal(d: &TrustDecision) -> depths_protocol::UntrustedNetworkRefusal {
    depths_protocol::UntrustedNetworkRefusal {
        error: "untrusted_network".into(),
        network: d.network.to_string(),
        reason: wire_reason(d.reason),
        matures_at: d.matures_at.map(rfc3339),
    }
}
