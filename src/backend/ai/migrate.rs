//! One-time migration of the 013 plaintext agent credential (`credentials.json -> agent`)
//! into the OS credential store (038 FR-008 / Principle VI: never lose a credential).
//!
//! Order is fixed: write to the keyring, read it back and compare, persist the non-secret
//! settings, and only then atomically remove the legacy `agent` field. Every failure path
//! leaves the legacy file exactly as it was. Other credentials in the file are preserved.

use std::path::Path;

use secrecy::{ExposeSecret, SecretString};

use super::store::CredentialStore;
use crate::backend::persistence;
use crate::protocol::v1::{CredentialMigrationOutcome, Provider};

fn failed(message: impl Into<String>) -> CredentialMigrationOutcome {
    CredentialMigrationOutcome::Failed {
        message: message.into(),
    }
}

/// Atomic rewrite of the legacy file (tmp + rename), keeping 0600 permissions on Unix.
fn write_legacy_atomic(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let tmp = path.with_extension("json.migrate-tmp");
    std::fs::write(
        &tmp,
        serde_json::to_string_pretty(value).unwrap_or_default(),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

/// Remove the `agent` field, then re-read to confirm it is gone and the rest is intact.
fn remove_agent_field(
    path: &Path,
    mut root: serde_json::Value,
) -> Result<(), CredentialMigrationOutcome> {
    if let Some(obj) = root.as_object_mut() {
        obj.remove("agent");
    }
    write_legacy_atomic(path, &root).map_err(|e| {
        failed(format!(
            "could not rewrite the legacy credentials file: {e}"
        ))
    })?;
    let back = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    match back {
        Some(v) if v.get("agent").is_none() => Ok(()),
        _ => Err(failed(
            "legacy credentials file did not verify after rewrite",
        )),
    }
}

/// Run the migration. `legacy_path` is `~/.fathom/credentials.json`, `config_path` is
/// `~/.fathom/config.json` (where the non-secret selection goes). Blocking.
#[allow(clippy::too_many_lines)] // one linear, ordered sequence; splitting would hide the safety order
pub(crate) fn migrate(
    store: &CredentialStore,
    legacy_path: &Path,
    config_path: &Path,
) -> CredentialMigrationOutcome {
    if !legacy_path.exists() {
        return CredentialMigrationOutcome::NothingToMigrate;
    }
    let raw = match std::fs::read_to_string(legacy_path) {
        Ok(raw) => raw,
        Err(e) => return failed(format!("could not read the legacy credentials file: {e}")),
    };
    let root: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return failed(format!(
                "the legacy credentials file is not valid JSON and was left untouched: {e}"
            ));
        }
    };
    let Some(agent) = root.get("agent").cloned() else {
        return CredentialMigrationOutcome::NothingToMigrate;
    };
    let kind = agent.get("type").and_then(|t| t.as_str()).unwrap_or("");

    let mut settings = persistence::load_ai_settings_at(config_path);
    match kind {
        "anthropic" => {
            let key = agent
                .get("key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .trim()
                .to_owned();
            if key.is_empty() {
                return match remove_agent_field(legacy_path, root) {
                    Ok(()) => CredentialMigrationOutcome::NothingToMigrate,
                    Err(o) => o,
                };
            }
            match store.get(Provider::Claude) {
                Ok(Some(existing)) if existing.expose_secret() == key => {}
                Ok(Some(_)) => {
                    return failed(
                        "the credential store already holds a different Claude key; the legacy \
                         key was kept in credentials.json so nothing is lost. Resolve it by \
                         signing in again or removing one of them.",
                    );
                }
                Ok(None) => {
                    if let Err(e) = store.save_verified(Provider::Claude, &SecretString::from(key))
                    {
                        return failed(format!(
                            "could not store the key in the OS credential store ({e}); the legacy \
                             key was kept in credentials.json"
                        ));
                    }
                }
                Err(e) => {
                    return failed(format!(
                        "could not read the OS credential store ({e}); the legacy key was kept"
                    ));
                }
            }
            if settings.active_provider.is_none() {
                settings.active_provider = Some(Provider::Claude);
                if let Err(e) = persistence::save_ai_settings_at(config_path, &settings) {
                    return failed(format!(
                        "key stored, but the settings could not be saved ({e}); the legacy key \
                         was kept"
                    ));
                }
            }
        }
        "ollama" => {
            if let Some(endpoint) = agent.get("endpoint").and_then(|v| v.as_str())
                && !endpoint.trim().is_empty()
            {
                settings.ollama_endpoint = endpoint.to_owned();
            }
            settings.ollama_model = agent
                .get("model")
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .map(str::to_owned);
            if settings.active_provider.is_none() {
                settings.active_provider = Some(Provider::Ollama);
            }
            if let Err(e) = persistence::save_ai_settings_at(config_path, &settings) {
                return failed(format!(
                    "could not save the Ollama selection ({e}); the legacy entry was kept"
                ));
            }
        }
        // `Disabled` maps to "no active provider" (the default): nothing to carry over.
        "disabled" | "unconfigured" => {}
        other => {
            return failed(format!(
                "unrecognized legacy agent entry type {other:?}; left untouched"
            ));
        }
    }

    let had_secret_or_selection = matches!(kind, "anthropic" | "ollama");
    match remove_agent_field(legacy_path, root) {
        Ok(()) if had_secret_or_selection => CredentialMigrationOutcome::Migrated,
        Ok(()) => CredentialMigrationOutcome::NothingToMigrate,
        Err(o) => o,
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::testing::*;
    use super::*;

    struct Env {
        fx: Fixture,
        dir: tempfile::TempDir,
    }

    impl Env {
        fn new() -> Self {
            Self {
                fx: Fixture::new(),
                dir: tempfile::tempdir().unwrap(),
            }
        }
        fn legacy(&self) -> std::path::PathBuf {
            self.dir.path().join("credentials.json")
        }
        fn config(&self) -> std::path::PathBuf {
            self.dir.path().join("config.json")
        }
        fn write_legacy(&self, v: serde_json::Value) {
            std::fs::write(self.legacy(), serde_json::to_string_pretty(&v).unwrap()).unwrap();
        }
        fn run(&self) -> CredentialMigrationOutcome {
            migrate(&self.fx.store, &self.legacy(), &self.config())
        }
        fn legacy_json(&self) -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(self.legacy()).unwrap()).unwrap()
        }
    }

    #[test]
    fn missing_file_is_nothing_to_migrate() {
        let env = Env::new();
        assert_eq!(env.run(), CredentialMigrationOutcome::NothingToMigrate);
    }

    #[test]
    fn migrates_key_verifies_then_removes_only_the_agent_field() {
        let env = Env::new();
        env.write_legacy(serde_json::json!({
            "agent": {"type":"anthropic","key":"sk-ant-legacy"},
            "other_cluster": {"token":"keep-me"}
        }));
        assert_eq!(env.run(), CredentialMigrationOutcome::Migrated);
        assert_eq!(
            env.fx.keyring.raw("ai-assist/claude").as_deref(),
            Some("sk-ant-legacy")
        );
        let after = env.legacy_json();
        assert!(after.get("agent").is_none());
        assert_eq!(after["other_cluster"]["token"], "keep-me");
        assert!(
            !std::fs::read_to_string(env.legacy())
                .unwrap()
                .contains("sk-ant-legacy")
        );
        let settings = persistence::load_ai_settings_at(&env.config());
        assert_eq!(settings.active_provider, Some(Provider::Claude));
    }

    #[test]
    fn is_idempotent() {
        let env = Env::new();
        env.write_legacy(serde_json::json!({"agent":{"type":"anthropic","key":"k"}}));
        assert_eq!(env.run(), CredentialMigrationOutcome::Migrated);
        assert_eq!(env.run(), CredentialMigrationOutcome::NothingToMigrate);
        assert_eq!(env.fx.keyring.raw("ai-assist/claude").as_deref(), Some("k"));
    }

    #[test]
    fn locked_keyring_keeps_legacy_file_intact() {
        let env = Env::new();
        let original = serde_json::json!({"agent":{"type":"anthropic","key":"sk-precious"}});
        env.write_legacy(original.clone());
        env.fx.keyring.set_mode(Mode::Locked);
        assert!(matches!(
            env.run(),
            CredentialMigrationOutcome::Failed { .. }
        ));
        assert_eq!(env.legacy_json(), original);
    }

    #[test]
    fn unavailable_keyring_keeps_legacy_file_intact() {
        let env = Env::new();
        let original = serde_json::json!({"agent":{"type":"anthropic","key":"sk-precious"}});
        env.write_legacy(original.clone());
        env.fx.keyring.set_mode(Mode::Unavailable);
        assert!(matches!(
            env.run(),
            CredentialMigrationOutcome::Failed { .. }
        ));
        assert_eq!(env.legacy_json(), original);
    }

    #[test]
    fn lossy_write_is_caught_by_verification_and_keeps_legacy() {
        let env = Env::new();
        let original = serde_json::json!({"agent":{"type":"anthropic","key":"sk-precious"}});
        env.write_legacy(original.clone());
        env.fx.keyring.set_mode(Mode::LossyWrites);
        assert!(matches!(
            env.run(),
            CredentialMigrationOutcome::Failed { .. }
        ));
        assert_eq!(env.legacy_json(), original);
    }

    #[test]
    fn conflicting_existing_key_keeps_both() {
        let env = Env::new();
        let original = serde_json::json!({"agent":{"type":"anthropic","key":"legacy"}});
        env.write_legacy(original.clone());
        env.fx
            .store
            .save(Provider::Claude, &SecretString::from("newer".to_owned()))
            .unwrap();
        assert!(matches!(
            env.run(),
            CredentialMigrationOutcome::Failed { .. }
        ));
        assert_eq!(env.legacy_json(), original);
        assert_eq!(
            env.fx.keyring.raw("ai-assist/claude").as_deref(),
            Some("newer")
        );
    }

    #[test]
    fn corrupt_legacy_file_is_left_untouched() {
        let env = Env::new();
        std::fs::write(env.legacy(), "{ nope").unwrap();
        assert!(matches!(
            env.run(),
            CredentialMigrationOutcome::Failed { .. }
        ));
        assert_eq!(std::fs::read_to_string(env.legacy()).unwrap(), "{ nope");
    }

    #[test]
    fn ollama_entry_moves_to_settings() {
        let env = Env::new();
        env.write_legacy(serde_json::json!({
            "agent": {"type":"ollama","endpoint":"http://127.0.0.1:9999","model":"llama3.2"}
        }));
        assert_eq!(env.run(), CredentialMigrationOutcome::Migrated);
        let s = persistence::load_ai_settings_at(&env.config());
        assert_eq!(s.active_provider, Some(Provider::Ollama));
        assert_eq!(s.ollama_endpoint, "http://127.0.0.1:9999");
        assert_eq!(s.ollama_model.as_deref(), Some("llama3.2"));
        assert!(env.legacy_json().get("agent").is_none());
    }

    #[test]
    fn disabled_maps_to_no_active_provider() {
        let env = Env::new();
        env.write_legacy(serde_json::json!({"agent":{"type":"disabled"},"x":1}));
        assert_eq!(env.run(), CredentialMigrationOutcome::NothingToMigrate);
        assert_eq!(
            persistence::load_ai_settings_at(&env.config()).active_provider,
            None
        );
        assert_eq!(env.legacy_json()["x"], 1);
    }

    #[test]
    fn file_without_agent_field_is_untouched() {
        let env = Env::new();
        let original = serde_json::json!({"cluster":{"t":"x"}});
        env.write_legacy(original.clone());
        assert_eq!(env.run(), CredentialMigrationOutcome::NothingToMigrate);
        assert_eq!(env.legacy_json(), original);
    }
}
