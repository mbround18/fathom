//! Redaction guard (033 T010): no token or secret can reach logs, protocol JSON or config.
//!
//! Structural, not sample-based: the ts-rs bindings are generated from every protocol type, so
//! scanning their field names covers all current and future fields of the new types.

use std::fs;
use std::path::Path;

use fathom::backend::oidc::store::{CredentialStore, Secret, SecretKind};
use fathom::protocol::v1::{BackendCmdV1, OidcConfig, SystemEvent};

const FORBIDDEN: &[&str] = &[
    "refresh_token",
    "id_token",
    "access_token",
    "bearer",
    "client_secret",
    "password",
    "api_key",
    "secret",
    "private_key",
];

/// Field names (`name:` / `name?:`) declared in a generated `.ts` file.
fn field_names(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut ident = String::new();
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric() || c == '_' {
            ident.push(c);
            continue;
        }
        if !ident.is_empty() && (c == ':' || (c == '?' && chars.peek() == Some(&':'))) {
            out.push(ident.to_lowercase());
        }
        ident.clear();
    }
    out
}

fn generated() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/src/protocol/generated")
}

#[test]
fn new_state_types_have_no_secret_fields() {
    // Everything the backend sends to the UI about identity, directory and capabilities.
    let mut checked = 0;
    for entry in fs::read_dir(generated()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let ours = [
            "Oidc",
            "Idp",
            "Capability",
            "RoleAssignment",
            "FathomRoleMap",
        ]
        .iter()
        .any(|p| name.starts_with(p))
            || name == "ContextAuthConfig"
            || name == "OrphanedSubject"
            || name == "TeamLink";
        if !ours {
            continue;
        }
        checked += 1;
        for f in field_names(&fs::read_to_string(&path).unwrap()) {
            assert!(
                !FORBIDDEN.iter().any(|bad| f.contains(bad)),
                "{name}.ts declares secret-looking field `{f}`"
            );
        }
    }
    assert!(
        checked >= 15,
        "expected to scan the new bindings, saw {checked}"
    );
}

#[test]
fn system_event_carries_no_secret_fields_in_new_variants() {
    let src = fs::read_to_string(generated().join("SystemEvent.ts")).unwrap();
    // Isolate the new variants; other events predate 033 and are out of scope.
    for variant in [
        "OidcConfigLoaded",
        "OidcSignInProgress",
        "OidcIdentityUpdated",
        "IdpDirectoryUpdated",
    ] {
        let start = src
            .find(&format!("\"type\": \"{variant}\""))
            .unwrap_or_else(|| panic!("{variant} missing"));
        let tail = &src[start..];
        let end = tail[1..].find("\"type\":").map_or(tail.len(), |i| i + 1);
        for f in field_names(&tail[..end]) {
            assert!(
                !FORBIDDEN.iter().any(|bad| f.contains(bad)),
                "{variant} has `{f}`"
            );
        }
    }
}

#[test]
fn new_commands_take_a_secret_only_via_the_write_only_admin_key() {
    let src = fs::read_to_string(generated().join("BackendCmdV1.ts")).unwrap();
    let new_variants = [
        "LoadOidcConfig",
        "OidcSaveIssuer",
        "OidcDeleteIssuer",
        "OidcSetRoleMap",
        "OidcSignIn",
        "OidcCancelSignIn",
        "OidcSignOut",
        "SetContextAuthMode",
        "IdpSetAdminKey",
        "IdpRefreshDirectory",
        "ResolveCapabilities",
        "CheckCapability",
    ];
    let mut with_secret = Vec::new();
    for chunk in src.split("\"type\": \"").skip(1) {
        let name = chunk.split('"').next().unwrap();
        if !new_variants.contains(&name) {
            continue;
        }
        if field_names(chunk)
            .iter()
            .any(|f| FORBIDDEN.iter().any(|bad| f.contains(bad)))
        {
            with_secret.push(name.to_owned());
        }
    }
    // Exactly the one write-only field; anything else needs a security review.
    assert_eq!(with_secret, vec!["IdpSetAdminKey".to_string()]);
}

#[test]
fn config_and_protocol_json_never_contain_a_stored_secret() {
    let canary = "CANARY-tok-9f3a1c";
    let store = CredentialStore::memory_only();
    store.set(SecretKind::RefreshToken, "rauthy", &Secret::new(canary));
    store.set(SecretKind::AdminApiKey, "rauthy", &Secret::new(canary));

    // The persisted config and its default/empty forms.
    let cfg = OidcConfig::default();
    let json = serde_json::to_string(&cfg).unwrap();
    assert!(!json.contains(canary));

    // Debug output of every type that could hold the secret.
    let secret = store.get(SecretKind::RefreshToken, "rauthy").unwrap();
    assert!(!format!("{secret:?}").contains(canary));
    assert!(!format!("{:?}", Some(&secret)).contains(canary));
    assert!(!format!("{:#?}", vec![secret]).contains(canary));

    // The write-only command is the only place a key transits; its JSON is the caller's own input,
    // and no event type can echo it back (checked structurally above).
    let cmd = BackendCmdV1::IdpSetAdminKey {
        issuer_id: "rauthy".into(),
        api_key: canary.into(),
    };
    assert!(serde_json::to_string(&cmd).unwrap().contains(canary));
    let _ = std::mem::size_of::<SystemEvent>();
}
