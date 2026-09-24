//! Read-only Rauthy admin API client and directory cache (033 US3, research R5).
//!
//! Only `GET /users` and `GET /groups` are ever issued, with an API key limited to
//! `Users:read` + `Groups:read` (the lab's `fathom-read` key). The key is the full
//! `<name>$<secret>` string Rauthy prints on creation (secret >= 64 chars in 0.36.2) and lives
//! in the credential store only. Failure never drops cached data: the last good snapshot is
//! returned with `stale = true` and the verbatim error (Constitution VI).

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use serde_json::Value;

use crate::protocol::v1::{
    FathomRoleMap, IdpDirectorySnapshot, IdpGroup, IdpUser, OidcIdentity, OidcIssuerConfig,
};

const PAGE_SIZE: usize = 100;
/// Hard stop so a server that ignores paging can never loop forever (10k users).
const MAX_PAGES: usize = 100;

/// Base URL of the admin API: the explicit `admin_api_base`, else the issuer URL (Rauthy serves
/// the admin REST API under the same `/auth/v1` prefix as its OIDC endpoints).
pub fn admin_base(cfg: &OidcIssuerConfig) -> String {
    cfg.admin_api_base
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or(&cfg.issuer_url)
        .trim_end_matches('/')
        .to_owned()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Rauthy serialises `groups` as a comma-separated string; tolerate a JSON array too.
fn group_names(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|g| g.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_user(v: &Value) -> Option<IdpUser> {
    let sub = v.get("id").or_else(|| v.get("sub"))?.as_str()?.to_owned();
    let email = v.get("email").and_then(Value::as_str).map(str::to_owned);
    let username = v
        .get("username")
        .or_else(|| v.get("preferred_username"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| email.clone())
        .unwrap_or_else(|| sub.clone());
    Some(IdpUser {
        sub,
        username,
        email,
        groups: group_names(v.get("groups")),
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Turn a non-2xx status into the verbatim, actionable failure text.
fn status_error(what: &str, status: reqwest::StatusCode, body: &str) -> String {
    let hint = match status.as_u16() {
        401 => " (the admin API key was rejected; re-enter it)",
        403 => " (the admin API key lacks Users:read / Groups:read)",
        _ => "",
    };
    let body = body.trim();
    if body.is_empty() {
        format!("IdP {what}: HTTP {status}{hint}")
    } else {
        format!("IdP {what}: HTTP {status}{hint}: {body}")
    }
}

async fn get_pages(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    what: &str,
) -> Result<Vec<Value>, String> {
    let mut all: Vec<Value> = Vec::new();
    let mut first_of_prev: Option<Value> = None;
    for page in 0..MAX_PAGES {
        let resp = http
            .get(url)
            .header("Authorization", format!("API-Key {api_key}"))
            .header("Accept", "application/json")
            .query(&[
                ("page_size", PAGE_SIZE.to_string()),
                ("offset", (page * PAGE_SIZE).to_string()),
            ])
            .send()
            .await
            .map_err(|e| format!("IdP unreachable ({what}): {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("IdP unreachable ({what}): {e}"))?;
        if !status.is_success() {
            return Err(status_error(what, status, &body));
        }
        let items: Vec<Value> = match serde_json::from_str::<Value>(&body) {
            Ok(Value::Array(a)) => a,
            Ok(_) | Err(_) => return Err(format!("IdP {what}: unexpected response shape")),
        };
        // A server that ignores `offset` repeats page one: stop instead of duplicating.
        if page > 0 && !items.is_empty() && items.first() == first_of_prev.as_ref() {
            break;
        }
        first_of_prev = items.first().cloned();
        let short = items.len() < PAGE_SIZE;
        all.extend(items);
        if short {
            break;
        }
    }
    Ok(all)
}

/// One full read of users and groups. Group `member_count` is derived from the users' memberships
/// so the two endpoints cannot disagree.
pub async fn fetch(
    http: &reqwest::Client,
    base: &str,
    api_key: &str,
) -> Result<(Vec<IdpUser>, Vec<IdpGroup>), String> {
    let users_raw = get_pages(http, &format!("{base}/users"), api_key, "users").await?;
    let groups_raw = get_pages(http, &format!("{base}/groups"), api_key, "groups").await?;
    let users: Vec<IdpUser> = users_raw.iter().filter_map(parse_user).collect();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for u in &users {
        for g in &u.groups {
            *counts.entry(g.clone()).or_default() += 1;
        }
    }
    let mut groups: Vec<IdpGroup> = groups_raw
        .iter()
        .filter_map(|g| g.get("name").and_then(Value::as_str))
        .map(|name| IdpGroup {
            name: name.to_owned(),
            member_count: counts.get(name).copied().unwrap_or(0),
        })
        .collect();
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((users, groups))
}

/// Last-good directory per issuer. `snapshot()` is what other features (US4 picker, orphan
/// detection) read.
#[derive(Default)]
pub struct DirectoryCache {
    snapshots: Mutex<HashMap<String, IdpDirectorySnapshot>>,
}

impl DirectoryCache {
    pub fn snapshot(&self, issuer_id: &str) -> Option<IdpDirectorySnapshot> {
        self.snapshots.lock().ok()?.get(issuer_id).cloned()
    }

    /// Fold a fetch result in: success replaces the data; failure keeps it, flags it stale and
    /// carries the error. Returns the snapshot to broadcast.
    pub fn apply(
        &self,
        issuer_id: &str,
        result: Result<(Vec<IdpUser>, Vec<IdpGroup>), String>,
    ) -> IdpDirectorySnapshot {
        let mut map = self.snapshots.lock().expect("directory cache");
        let snap = match result {
            Ok((users, groups)) => IdpDirectorySnapshot {
                issuer_id: issuer_id.to_owned(),
                users,
                groups,
                fetched_at: unix_now(),
                stale: false,
                error: None,
            },
            Err(error) => {
                let mut prev = map.get(issuer_id).cloned().unwrap_or(IdpDirectorySnapshot {
                    issuer_id: issuer_id.to_owned(),
                    users: Vec::new(),
                    groups: Vec::new(),
                    fetched_at: 0,
                    stale: true,
                    error: None,
                });
                prev.stale = true;
                prev.error = Some(error);
                prev
            }
        };
        map.insert(issuer_id.to_owned(), snap.clone());
        snap
    }

    pub fn forget(&self, issuer_id: &str) {
        if let Ok(mut m) = self.snapshots.lock() {
            m.remove(issuer_id);
        }
    }
}

// -- Fathom-level roles (R9) ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FathomRole {
    Viewer,
    Editor,
    Admin,
}

/// Highest Fathom role the identity's groups map to, or `None` (no Fathom-level access).
/// Never used to grant cluster access.
pub fn fathom_role(groups: &[String], map: &FathomRoleMap) -> Option<FathomRole> {
    let has = |name: &str| !name.is_empty() && groups.iter().any(|g| g == name);
    if has(&map.admins_group) {
        Some(FathomRole::Admin)
    } else if has(&map.editors_group) {
        Some(FathomRole::Editor)
    } else if has(&map.viewers_group) {
        Some(FathomRole::Viewer)
    } else {
        None
    }
}

/// `Ok` when there is no OIDC identity in play (kubeconfig-only use is unrestricted) or the
/// identity's role is at least `required`; otherwise the refusal reason.
pub fn require_role(
    identity: Option<&OidcIdentity>,
    map: &FathomRoleMap,
    required: FathomRole,
) -> Result<(), String> {
    let Some(identity) = identity else {
        return Ok(());
    };
    match fathom_role(&identity.groups, map) {
        Some(have) if have >= required => Ok(()),
        have => {
            let needed = match required {
                FathomRole::Viewer => &map.viewers_group,
                FathomRole::Editor => &map.editors_group,
                FathomRole::Admin => &map.admins_group,
            };
            Err(format!(
                "{} needs membership of '{needed}' (or a higher Fathom group); signed in as {} with {}",
                match required {
                    FathomRole::Viewer => "viewing team data",
                    FathomRole::Editor => "publishing team data",
                    FathomRole::Admin => "managing team links",
                },
                identity.username,
                match have {
                    Some(FathomRole::Viewer) => "role viewer",
                    Some(FathomRole::Editor) => "role editor",
                    Some(FathomRole::Admin) => "role admin",
                    None => "no Fathom role",
                }
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v1::OidcIdentityState;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const KEY: &str =
        "fathom-read$abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijkl";

    fn user_json(i: usize, groups: &str) -> Value {
        serde_json::json!({"id": format!("u{i}"), "email": format!("u{i}@x.test"), "groups": groups, "enabled": true})
    }

    #[tokio::test]
    async fn pages_users_and_derives_group_counts() {
        let server = MockServer::start().await;
        let page1: Vec<Value> = (0..PAGE_SIZE)
            .map(|i| user_json(i, "fathom-viewers"))
            .collect();
        let page2 = vec![user_json(900, "fathom-editors,fathom-viewers")];
        Mock::given(method("GET"))
            .and(path("/auth/v1/users"))
            .and(header("Authorization", format!("API-Key {KEY}").as_str()))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page1))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/auth/v1/users"))
            .and(query_param("offset", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page2))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/auth/v1/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "g1", "name": "fathom-viewers"},
                {"id": "g2", "name": "fathom-editors"}
            ])))
            .mount(&server)
            .await;
        let base = format!("{}/auth/v1", server.uri());
        let (users, groups) = fetch(&reqwest::Client::new(), &base, KEY).await.unwrap();
        assert_eq!(users.len(), 101);
        assert_eq!(users[0].username, "u0@x.test");
        let count = |n: &str| groups.iter().find(|g| g.name == n).unwrap().member_count;
        assert_eq!(count("fathom-viewers"), 101);
        assert_eq!(count("fathom-editors"), 1);
    }

    #[tokio::test]
    async fn server_ignoring_offset_does_not_duplicate_or_loop() {
        let server = MockServer::start().await;
        let full: Vec<Value> = (0..PAGE_SIZE).map(|i| user_json(i, "")).collect();
        Mock::given(method("GET"))
            .and(path("/users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(full))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/groups"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        let (users, _) = fetch(&reqwest::Client::new(), &server.uri(), KEY)
            .await
            .unwrap();
        assert_eq!(users.len(), PAGE_SIZE);
    }

    #[tokio::test]
    async fn unauthorized_and_forbidden_are_verbatim_and_actionable() {
        for (code, needle) in [(401u16, "rejected"), (403, "lacks Users:read")] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(code).set_body_string("nope"))
                .mount(&server)
                .await;
            let err = fetch(&reqwest::Client::new(), &server.uri(), KEY)
                .await
                .unwrap_err();
            assert!(
                err.contains(&code.to_string()) && err.contains(needle),
                "{err}"
            );
            assert!(err.contains("nope"));
            assert!(!err.contains(KEY), "the key must never appear in errors");
        }
    }

    #[tokio::test]
    async fn unreachable_idp_is_reported() {
        let err = fetch(&reqwest::Client::new(), "http://127.0.0.1:1", KEY)
            .await
            .unwrap_err();
        assert!(err.contains("unreachable"), "{err}");
    }

    #[test]
    fn failure_keeps_last_good_data_marked_stale() {
        let cache = DirectoryCache::default();
        let user = IdpUser {
            sub: "u1".into(),
            username: "alice".into(),
            email: None,
            groups: vec![],
            enabled: true,
        };
        let ok = cache.apply("rauthy", Ok((vec![user], vec![])));
        assert!(!ok.stale && ok.fetched_at > 0);
        let bad = cache.apply("rauthy", Err("IdP unreachable".into()));
        assert!(bad.stale);
        assert_eq!(bad.users.len(), 1);
        assert_eq!(bad.fetched_at, ok.fetched_at);
        assert_eq!(bad.error.as_deref(), Some("IdP unreachable"));
        assert_eq!(cache.snapshot("rauthy"), Some(bad));
        // A first-ever failure yields an empty stale snapshot, never a panic.
        let first = cache.apply("new", Err("401".into()));
        assert!(first.stale && first.users.is_empty() && first.fetched_at == 0);
    }

    #[test]
    fn admin_base_prefers_explicit_then_issuer() {
        let mut cfg = OidcIssuerConfig {
            id: "r".into(),
            issuer_url: "https://a.test/auth/v1/".into(),
            client_id: "c".into(),
            scopes: vec![],
            ca_bundle_path: None,
            username_claim: "preferred_username".into(),
            groups_claim: "groups".into(),
            username_prefix: None,
            groups_prefix: None,
            admin_api_base: None,
        };
        assert_eq!(admin_base(&cfg), "https://a.test/auth/v1");
        cfg.admin_api_base = Some("https://admin.test/x/".into());
        assert_eq!(admin_base(&cfg), "https://admin.test/x");
    }

    fn ident(groups: &[&str]) -> OidcIdentity {
        OidcIdentity {
            issuer_id: "r".into(),
            sub: "s".into(),
            username: "u".into(),
            email: None,
            groups: groups.iter().map(|g| (*g).to_owned()).collect(),
            expires_at: 0,
            state: OidcIdentityState::SignedIn,
        }
    }

    #[test]
    fn role_map_gates_by_highest_role() {
        let map = FathomRoleMap::default();
        let viewer = ident(&[&map.viewers_group]);
        let editor = ident(&[&map.viewers_group, &map.editors_group]);
        let admin = ident(&[&map.admins_group]);
        let none = ident(&[]);
        assert!(require_role(None, &map, FathomRole::Admin).is_ok());
        assert!(require_role(Some(&viewer), &map, FathomRole::Viewer).is_ok());
        assert!(require_role(Some(&viewer), &map, FathomRole::Editor).is_err());
        assert!(require_role(Some(&editor), &map, FathomRole::Editor).is_ok());
        assert!(require_role(Some(&editor), &map, FathomRole::Admin).is_err());
        assert!(require_role(Some(&admin), &map, FathomRole::Editor).is_ok());
        let e = require_role(Some(&none), &map, FathomRole::Viewer).unwrap_err();
        assert!(e.contains("no Fathom role"), "{e}");
    }
}
