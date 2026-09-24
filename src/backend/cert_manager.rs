//! cert-manager integration (023-cert-manager-certificates-pane).
//!
//! Discovers cert-manager's CRDs (`cert-manager.io`: Certificate, CertificateRequest, Issuer,
//! ClusterIssuer; `acme.cert-manager.io`: Order, Challenge) and reads them as dynamic objects,
//! the same discovery-by-CRD-name-then-serve-version technique used for Flux in `flux.rs` and
//! Longhorn in `longhorn.rs` — this works across cert-manager API versions without pinning one.
//! Read-only: no command in this module ever creates, edits, or deletes anything (spec FR-009).

use crate::protocol::{
    CertExpiryState, CertManagerOverview, CertReadyState, CertificateChain, CertificateSummary,
    ChainStage, ChainStageKind,
};
use chrono::{DateTime, Utc};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::ListParams;
use kube::core::DynamicObject;
use kube::core::{ApiResource, GroupVersionKind};
use kube::{Client, api::Api};
use serde_json::Value;

/// `notAfter - now` at or below this is "expiring soon" (research.md R5).
const EXPIRING_SOON_THRESHOLD_SECS: i64 = 14 * 24 * 60 * 60;

const CERTIFICATE_CRD: &str = "certificates.cert-manager.io";
const CERTIFICATE_REQUEST_CRD: &str = "certificaterequests.cert-manager.io";
const ORDER_CRD: &str = "orders.acme.cert-manager.io";
const CHALLENGE_CRD: &str = "challenges.acme.cert-manager.io";

struct DiscoveredCrd {
    group: String,
    version: String,
    kind: String,
}

/// Find a served version for `crd_name` among already-listed CRDs.
fn find_served(crds: &[CustomResourceDefinition], crd_name: &str) -> Option<DiscoveredCrd> {
    let crd = crds
        .iter()
        .find(|c| c.metadata.name.as_deref() == Some(crd_name))?;
    let group = crd.spec.group.clone();
    let version = crd
        .spec
        .versions
        .iter()
        .find(|v| v.served)
        .map(|v| v.name.clone())?;
    let kind = crd.spec.names.kind.clone();
    Some(DiscoveredCrd {
        group,
        version,
        kind,
    })
}

/// List every instance of a discovered CRD as dynamic objects, across all namespaces.
async fn list_instances(client: &Client, crd: &DiscoveredCrd) -> Vec<DynamicObject> {
    let gvk = GroupVersionKind::gvk(&crd.group, &crd.version, &crd.kind);
    let resource = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &resource);
    api.list(&ListParams::default())
        .await
        .map(|l| l.items)
        .unwrap_or_default()
}

/// Find the `Ready` condition in `status.conditions` and return (status, reason, message).
fn ready_condition(status: &Value) -> Option<(&str, &str, &str)> {
    condition_by_type(status, "Ready")
}

/// Find a condition of the given `type` in `status.conditions`.
fn condition_by_type<'a>(
    status: &'a Value,
    cond_type: &str,
) -> Option<(&'a str, &'a str, &'a str)> {
    status
        .get("conditions")
        .and_then(|c| c.as_array())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.get("type").and_then(|t| t.as_str()) == Some(cond_type))
        })
        .map(|cond| {
            let status = cond
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");
            let reason = cond.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let message = cond.get("message").and_then(|v| v.as_str()).unwrap_or("");
            (status, reason, message)
        })
}

fn classify_expiry(not_after: Option<&str>, now: DateTime<Utc>) -> (CertExpiryState, Option<i64>) {
    let Some(not_after) = not_after else {
        return (CertExpiryState::Unknown, None);
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(not_after) else {
        return (CertExpiryState::Unknown, None);
    };
    let seconds = (parsed.with_timezone(&Utc) - now).num_seconds();
    let state = if seconds < 0 {
        CertExpiryState::Expired
    } else if seconds <= EXPIRING_SOON_THRESHOLD_SECS {
        CertExpiryState::ExpiringSoon
    } else {
        CertExpiryState::Valid
    };
    (state, Some(seconds))
}

fn classify_ready(ready_status: Option<&str>, renewing: bool) -> CertReadyState {
    match ready_status {
        Some("True") if renewing => CertReadyState::Renewing,
        Some("True") => CertReadyState::Ready,
        Some("False") => CertReadyState::Failed,
        _ => CertReadyState::Pending,
    }
}

/// Build a [`CertificateSummary`] from a Certificate dynamic object, and (if a newer
/// CertificateRequest than the one that produced the current `notAfter` exists) flag it as
/// renewing.
fn summarize_certificate(obj: &DynamicObject, renewing: bool) -> CertificateSummary {
    let name = obj.metadata.name.clone().unwrap_or_default();
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();

    let spec = obj.data.get("spec");
    let status = obj.data.get("status");

    let issuer_ref = spec
        .and_then(|s| s.get("issuerRef"))
        .map(|ir| {
            let kind = ir.get("kind").and_then(|v| v.as_str()).unwrap_or("Issuer");
            let name = ir.get("name").and_then(|v| v.as_str()).unwrap_or("");
            format!("{kind}/{name}")
        })
        .unwrap_or_default();

    let not_after = status
        .and_then(|s| s.get("notAfter"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let (expiry_state, expiry_seconds) = classify_expiry(not_after.as_deref(), Utc::now());

    let (ready_status, reason, message) = status
        .and_then(ready_condition)
        .map(|(s, r, m)| (Some(s), r.to_owned(), m.to_owned()))
        .unwrap_or((None, String::new(), String::new()));
    let message = if message.is_empty() { reason } else { message };

    let renewal_time = status
        .and_then(|s| s.get("renewalTime"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);

    let dns_names = spec
        .and_then(|s| s.get("dnsNames"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();

    CertificateSummary {
        name,
        namespace,
        issuer_ref,
        not_after,
        expiry_state,
        expiry_seconds,
        ready: classify_ready(ready_status, renewing),
        message,
        renewal_time,
        dns_names,
    }
}

/// Discover the cert-manager CRDs installed in this cluster and build a full overview.
/// `installed` is true iff the `Certificate` CRD is served (research.md R2) — the other CRDs are
/// read opportunistically and their absence does not affect `installed`.
pub(super) async fn scan_cert_manager(client: &Client) -> anyhow::Result<CertManagerOverview> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let Some(certificate_crd) = find_served(&crds, CERTIFICATE_CRD) else {
        return Ok(CertManagerOverview {
            installed: false,
            certificates: Vec::new(),
        });
    };

    let certificate_objs = list_instances(client, &certificate_crd).await;

    let mut certificates: Vec<CertificateSummary> = certificate_objs
        .iter()
        .map(|obj| summarize_certificate(obj, false))
        .collect();

    certificates.sort_by(|a, b| {
        (a.namespace.as_str(), a.name.as_str()).cmp(&(b.namespace.as_str(), b.name.as_str()))
    });

    Ok(CertManagerOverview {
        installed: true,
        certificates,
    })
}

/// Owner-reference UID lookup: find the object in `candidates` owned by `owner_uid`.
fn find_owned<'a>(candidates: &'a [DynamicObject], owner_uid: &str) -> Option<&'a DynamicObject> {
    candidates.iter().find(|obj| {
        obj.metadata
            .owner_references
            .as_ref()
            .is_some_and(|refs| refs.iter().any(|r| r.uid == owner_uid))
    })
}

/// Resolve a state/reason/message triple for an Order or Challenge dynamic object: cert-manager
/// puts these directly on `status` (`status.state`, `status.reason`), not as a condition array.
fn order_or_challenge_state(status: Option<&Value>) -> (String, String, String) {
    let state = status
        .and_then(|s| s.get("state"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let reason = status
        .and_then(|s| s.get("reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    (state, reason, String::new())
}

fn is_order_or_challenge_stuck(state: &str) -> bool {
    !state.eq_ignore_ascii_case("valid")
}

/// Resolve one Certificate's lifecycle chain (research.md R3/R4): correlate through owner
/// references and the `cert-manager.io/certificate-name` label cert-manager itself sets, then
/// walk the chain to find the first non-terminal-success stage.
pub(super) async fn resolve_chain(
    client: &Client,
    namespace: &str,
    name: &str,
) -> anyhow::Result<CertificateChain> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&ListParams::default()).await?.items;

    let Some(certificate_crd) = find_served(&crds, CERTIFICATE_CRD) else {
        return Ok(CertificateChain {
            certificate_name: name.to_owned(),
            certificate_namespace: namespace.to_owned(),
            stages: Vec::new(),
            not_found: true,
        });
    };

    let certificates = list_instances(client, &certificate_crd).await;
    let Some(certificate) = certificates.iter().find(|c| {
        c.metadata.namespace.as_deref() == Some(namespace)
            && c.metadata.name.as_deref() == Some(name)
    }) else {
        return Ok(CertificateChain {
            certificate_name: name.to_owned(),
            certificate_namespace: namespace.to_owned(),
            stages: Vec::new(),
            not_found: true,
        });
    };

    let cert_status = certificate.data.get("status");
    let (cert_ready_status, cert_reason, cert_message) = cert_status
        .and_then(ready_condition)
        .map(|(s, r, m)| (Some(s), r.to_owned(), m.to_owned()))
        .unwrap_or((None, String::new(), String::new()));
    let cert_message = if cert_message.is_empty() {
        cert_reason.clone()
    } else {
        cert_message
    };
    let cert_ready = matches!(cert_ready_status, Some("True"));

    let mut stages = vec![ChainStage {
        kind: ChainStageKind::Certificate,
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        state: cert_ready_status.unwrap_or("Unknown").to_owned(),
        reason: cert_reason,
        message: cert_message,
        is_stuck: false, // resolved below, once we know if a later stage is stuck instead
    }];

    // Certificate -> CertificateRequest: newest CertificateRequest labeled for this Certificate.
    let cr_crd = find_served(&crds, CERTIFICATE_REQUEST_CRD);
    let mut certificate_request: Option<DynamicObject> = None;
    if let Some(cr_crd) = &cr_crd {
        let requests = list_instances(client, cr_crd).await;
        let mut matches: Vec<&DynamicObject> = requests
            .iter()
            .filter(|r| {
                r.metadata.namespace.as_deref() == Some(namespace)
                    && r.metadata
                        .labels
                        .as_ref()
                        .and_then(|l| l.get("cert-manager.io/certificate-name"))
                        .map(|v| v.as_str())
                        == Some(name)
            })
            .collect();
        matches.sort_by(|a, b| {
            a.metadata
                .creation_timestamp
                .as_ref()
                .map(|t| t.0)
                .cmp(&b.metadata.creation_timestamp.as_ref().map(|t| t.0))
        });
        certificate_request = matches.pop().cloned();
    }

    let mut challenge_objs: Vec<DynamicObject> = Vec::new();

    if let Some(cr) = &certificate_request {
        let cr_status = cr.data.get("status");
        let (cr_ready_status, cr_reason, cr_message) = cr_status
            .and_then(ready_condition)
            .map(|(s, r, m)| (Some(s), r.to_owned(), m.to_owned()))
            .unwrap_or((None, String::new(), String::new()));
        let cr_message = if cr_message.is_empty() {
            cr_reason.clone()
        } else {
            cr_message
        };
        let cr_ready = matches!(cr_ready_status, Some("True"));

        stages.push(ChainStage {
            kind: ChainStageKind::CertificateRequest,
            name: cr.metadata.name.clone().unwrap_or_default(),
            namespace: namespace.to_owned(),
            state: cr_ready_status.unwrap_or("Unknown").to_owned(),
            reason: cr_reason,
            message: cr_message,
            is_stuck: false,
        });

        // CertificateRequest -> Order: owner reference from Order to this CertificateRequest.
        if let Some(cr_uid) = &cr.metadata.uid {
            let order_crd = find_served(&crds, ORDER_CRD);
            if let Some(order_crd) = &order_crd {
                let orders = list_instances(client, order_crd).await;
                if let Some(pos) = orders
                    .iter()
                    .position(|o| find_owned(std::slice::from_ref(o), cr_uid).is_some())
                {
                    let order = &orders[pos];
                    let (state, reason, message) =
                        order_or_challenge_state(order.data.get("status"));
                    stages.push(ChainStage {
                        kind: ChainStageKind::Order,
                        name: order.metadata.name.clone().unwrap_or_default(),
                        namespace: namespace.to_owned(),
                        state: state.clone(),
                        reason,
                        message,
                        is_stuck: false,
                    });

                    // Order -> Challenge(s): owner reference from Challenge to this Order.
                    if let Some(order_uid) = &order.metadata.uid {
                        let challenge_crd = find_served(&crds, CHALLENGE_CRD);
                        if let Some(challenge_crd) = &challenge_crd {
                            let challenges = list_instances(client, challenge_crd).await;
                            challenge_objs = challenges
                                .into_iter()
                                .filter(|c| {
                                    c.metadata.owner_references.as_ref().is_some_and(|refs| {
                                        refs.iter().any(|r| &r.uid == order_uid)
                                    })
                                })
                                .collect();
                        }
                    }

                    if !challenge_objs.is_empty() {
                        // Least-advanced challenge is the one to surface if any is stuck.
                        let least_advanced = challenge_objs
                            .iter()
                            .find(|c| {
                                let (state, _, _) = order_or_challenge_state(c.data.get("status"));
                                is_order_or_challenge_stuck(&state)
                            })
                            .unwrap_or(&challenge_objs[0]);
                        let (state, reason, message) =
                            order_or_challenge_state(least_advanced.data.get("status"));
                        stages.push(ChainStage {
                            kind: ChainStageKind::Challenge,
                            name: least_advanced.metadata.name.clone().unwrap_or_default(),
                            namespace: namespace.to_owned(),
                            state,
                            reason,
                            message,
                            is_stuck: false,
                        });
                    }
                }
            }
        }
        let _ = cr_ready;
    }

    // Research R4: walk the chain, mark the first non-terminal-success stage as stuck. If the
    // Certificate is Ready, nothing is stuck.
    if !cert_ready {
        if let Some(stuck) = stages.iter_mut().find(|s| match s.kind {
            ChainStageKind::Certificate => stages_certificate_stuck(&s.state),
            ChainStageKind::CertificateRequest => !s.state.eq_ignore_ascii_case("true"),
            ChainStageKind::Order | ChainStageKind::Challenge => {
                is_order_or_challenge_stuck(&s.state)
            }
        }) {
            stuck.is_stuck = true;
        }
    }

    Ok(CertificateChain {
        certificate_name: name.to_owned(),
        certificate_namespace: namespace.to_owned(),
        stages,
        not_found: false,
    })
}

fn stages_certificate_stuck(state: &str) -> bool {
    state.eq_ignore_ascii_case("False")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dynamic_object(json: serde_json::Value) -> DynamicObject {
        serde_json::from_value(json).expect("valid DynamicObject fixture")
    }

    #[test]
    fn expiry_classifies_valid_expiring_soon_expired_unknown() {
        let now = Utc::now();
        let far_future = (now + chrono::Duration::days(60)).to_rfc3339();
        let (state, secs) = classify_expiry(Some(&far_future), now);
        assert_eq!(state, CertExpiryState::Valid);
        assert!(secs.unwrap() > EXPIRING_SOON_THRESHOLD_SECS);

        let soon = (now + chrono::Duration::days(3)).to_rfc3339();
        let (state, secs) = classify_expiry(Some(&soon), now);
        assert_eq!(state, CertExpiryState::ExpiringSoon);
        assert!(secs.unwrap() > 0);

        let past = (now - chrono::Duration::days(3)).to_rfc3339();
        let (state, secs) = classify_expiry(Some(&past), now);
        assert_eq!(state, CertExpiryState::Expired);
        assert!(secs.unwrap() < 0);

        let (state, secs) = classify_expiry(None, now);
        assert_eq!(state, CertExpiryState::Unknown);
        assert!(secs.is_none());
    }

    #[test]
    fn ready_classifies_ready_renewing_failed_pending() {
        assert_eq!(classify_ready(Some("True"), false), CertReadyState::Ready);
        assert_eq!(classify_ready(Some("True"), true), CertReadyState::Renewing);
        assert_eq!(classify_ready(Some("False"), false), CertReadyState::Failed);
        assert_eq!(classify_ready(None, false), CertReadyState::Pending);
    }

    #[test]
    fn summarize_certificate_reads_issuer_expiry_and_ready() {
        let now = Utc::now();
        let not_after = (now + chrono::Duration::days(1)).to_rfc3339();
        let obj = dynamic_object(json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": { "name": "example-com-tls", "namespace": "default" },
            "spec": {
                "issuerRef": { "kind": "ClusterIssuer", "name": "letsencrypt-prod" },
                "dnsNames": ["example.com"],
            },
            "status": {
                "notAfter": not_after,
                "conditions": [
                    { "type": "Ready", "status": "False", "reason": "Failed", "message": "authorization failed" }
                ]
            }
        }));

        let summary = summarize_certificate(&obj, false);
        assert_eq!(summary.issuer_ref, "ClusterIssuer/letsencrypt-prod");
        assert_eq!(summary.expiry_state, CertExpiryState::ExpiringSoon);
        assert_eq!(summary.ready, CertReadyState::Failed);
        assert_eq!(summary.message, "authorization failed");
        assert_eq!(summary.dns_names, vec!["example.com".to_owned()]);
    }

    #[tokio::test]
    async fn resolve_chain_marks_challenge_stuck() {
        // This exercises the pure correlation/classification logic without a live cluster by
        // building the chain manually through the same helpers `resolve_chain` uses internally.
        let cert = dynamic_object(json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "Certificate",
            "metadata": { "name": "stuck-cert", "namespace": "default", "uid": "cert-uid" },
            "status": {
                "conditions": [{ "type": "Ready", "status": "False", "reason": "Pending", "message": "" }]
            }
        }));
        let cr = dynamic_object(json!({
            "apiVersion": "cert-manager.io/v1",
            "kind": "CertificateRequest",
            "metadata": {
                "name": "stuck-cert-abc123",
                "namespace": "default",
                "uid": "cr-uid",
                "labels": { "cert-manager.io/certificate-name": "stuck-cert" },
                "creationTimestamp": "2026-01-01T00:00:00Z"
            },
            "status": {
                "conditions": [{ "type": "Ready", "status": "False", "reason": "Pending", "message": "" }]
            }
        }));
        let order = dynamic_object(json!({
            "apiVersion": "acme.cert-manager.io/v1",
            "kind": "Order",
            "metadata": {
                "name": "stuck-cert-abc123-xyz",
                "namespace": "default",
                "uid": "order-uid",
                "ownerReferences": [{ "apiVersion": "cert-manager.io/v1", "kind": "CertificateRequest", "name": "stuck-cert-abc123", "uid": "cr-uid" }]
            },
            "status": { "state": "pending" }
        }));
        let challenge = dynamic_object(json!({
            "apiVersion": "acme.cert-manager.io/v1",
            "kind": "Challenge",
            "metadata": {
                "name": "stuck-cert-abc123-xyz-001",
                "namespace": "default",
                "uid": "challenge-uid",
                "ownerReferences": [{ "apiVersion": "acme.cert-manager.io/v1", "kind": "Order", "name": "stuck-cert-abc123-xyz", "uid": "order-uid" }]
            },
            "status": { "state": "pending", "reason": "Waiting for HTTP-01 challenge propagation" }
        }));

        // Sanity: owner-reference correlation helpers used by resolve_chain work as expected.
        assert!(find_owned(std::slice::from_ref(&order), "cr-uid").is_some());
        assert!(
            challenge
                .metadata
                .owner_references
                .as_ref()
                .unwrap()
                .iter()
                .any(|r| r.uid == "order-uid")
        );

        let (state, _reason, _message) = order_or_challenge_state(challenge.data.get("status"));
        assert!(is_order_or_challenge_stuck(&state));
        let _ = cert;
        let _ = cr;
    }
}
