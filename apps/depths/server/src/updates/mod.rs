//! Image updates (spec 062): the server side.
//!
//! [`Updates`] is the service `AppState` holds. With `DEPTHS_UPDATES=off` it does not exist
//! (research R10): [`Updates::open`] returns `None` without opening the store or constructing a
//! registry client, no scheduler is spawned, the update commands are refused as
//! `FeatureDisabled`, and `StackDetail.updates` stays `None`.
//!
//! Registries are reached only through depths-core's [`Registry`] port; the adapter over the
//! shared `oci-registry` crate is built by the caller and handed in.

pub mod change;
pub mod creds;
pub mod inputs;
pub mod registry;
pub mod run;
pub mod scheduler;
pub mod store;
pub mod view;

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use depths_core::updates::registry::Registry;
use depths_core::updates::window::Clock;
use depths_protocol::UpdatesAvailability;

use crate::config::{Config, UpdatesConfig};
use store::Store;

/// The wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// The image-updates service: settings, the store, and the registry port.
pub struct Updates {
    config: UpdatesConfig,
    store: Store,
    registry: Arc<dyn Registry>,
    clock: Arc<dyn Clock>,
    /// Wakes the scheduler: Check now, or a settings change.
    wake: tokio::sync::Notify,
    /// Check now: when it last ran and what is waiting.
    check_now: Mutex<scheduler::CheckNow>,
    /// Fires after every change to what `ListUpdates` shows.
    changed: tokio::sync::broadcast::Sender<()>,
    /// Settings changes waiting for their `Confirm`.
    pending: Mutex<change::Pending>,
}

impl Updates {
    /// `None` when the feature is off; `registry` is then never called. Otherwise opens
    /// `updates.json` and `update-state.json` in the data directory.
    pub fn open(
        cfg: &Config,
        clock: Arc<dyn Clock>,
        registry: impl FnOnce() -> Arc<dyn Registry>,
    ) -> Result<Option<Arc<Self>>, String> {
        if !cfg.updates.enabled {
            return Ok(None);
        }
        let store = Store::open(&cfg.data_dir, clock.clone()).map_err(|e| {
            format!(
                "cannot open the update state in {}: {e}",
                cfg.data_dir.display()
            )
        })?;
        Ok(Some(Arc::new(Self {
            config: cfg.updates.clone(),
            store,
            registry: registry(),
            clock,
            wake: tokio::sync::Notify::new(),
            check_now: Mutex::new(scheduler::CheckNow::default()),
            changed: tokio::sync::broadcast::channel(16).0,
            pending: Mutex::new(change::Pending::default()),
        })))
    }

    pub fn config(&self) -> &UpdatesConfig {
        &self.config
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn registry(&self) -> &Arc<dyn Registry> {
        &self.registry
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub fn availability(&self) -> UpdatesAvailability {
        self.store.availability()
    }

    /// Whether the scheduled checks are on (`checks_enabled`, and the file readable).
    pub fn checks_enabled(&self) -> bool {
        self.availability() == UpdatesAvailability::On
    }
}

/// The start-up line for `cfg`, with the checks setting from the service when it exists.
pub fn startup_line(cfg: &UpdatesConfig, updates: Option<&Updates>) -> String {
    let zone = crate::config::host_zone_name(
        std::env::var("TZ").ok(),
        std::path::Path::new("/etc/localtime"),
    );
    cfg.startup_line(updates.is_some_and(Updates::checks_enabled), &zone)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use depths_core::updates::reference::ImageRef;
    use depths_core::updates::registry::{Platform, RegistryError, RegistryFuture, Resolution};

    pub(crate) struct NoNetwork;

    impl Registry for NoNetwork {
        fn head_digest<'a>(&'a self, _: &'a ImageRef) -> RegistryFuture<'a, String> {
            Box::pin(async { Err(RegistryError::Unreachable("test".into())) })
        }
        fn resolve<'a>(
            &'a self,
            _: &'a ImageRef,
            _: &'a str,
            _: &'a Platform,
        ) -> RegistryFuture<'a, Resolution> {
            Box::pin(async { Err(RegistryError::Unreachable("test".into())) })
        }
    }

    pub(super) fn config(dir: &std::path::Path, updates: &str) -> Config {
        let stacks = dir.join("stacks");
        std::fs::create_dir_all(&stacks).unwrap();
        let env: std::collections::HashMap<&str, String> = [
            ("DEPTHS_PUBLIC_URL", "https://depths.example".to_owned()),
            ("OAUTH_PROVIDER", "discord".to_owned()),
            ("OAUTH_CLIENT_ID", "1".to_owned()),
            ("OAUTH_CLIENT_SECRET", "s".to_owned()),
            ("DEPTHS_STACKS_DIR", stacks.display().to_string()),
            ("DEPTHS_DATA_DIR", dir.join("data").display().to_string()),
            ("DEPTHS_UPDATES", updates.to_owned()),
        ]
        .into_iter()
        .collect();
        Config::resolve(crate::config::Flags::default(), &move |k| {
            env.get(k).cloned()
        })
        .unwrap()
    }

    #[test]
    fn off_constructs_nothing_and_touches_no_file() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config(d.path(), "off");
        std::fs::create_dir_all(&cfg.data_dir).unwrap();
        let got = Updates::open(&cfg, Arc::new(SystemClock), || {
            panic!("no registry client is constructed when updates are off")
        })
        .unwrap();
        assert!(got.is_none());
        assert_eq!(std::fs::read_dir(&cfg.data_dir).unwrap().count(), 0);
        assert_eq!(startup_line(&cfg.updates, None), "updates: feature=off");
    }

    #[test]
    fn on_opens_the_store_with_checks_off_by_default() {
        let d = tempfile::tempdir().unwrap();
        let cfg = config(d.path(), "on");
        std::fs::create_dir_all(&cfg.data_dir).unwrap();
        let u = Updates::open(&cfg, Arc::new(SystemClock), || Arc::new(NoNetwork))
            .unwrap()
            .expect("on");
        assert_eq!(u.availability(), UpdatesAvailability::Off);
        assert!(!u.checks_enabled());
        assert!(startup_line(&cfg.updates, Some(&u)).contains("feature=on auto=on checks=off"));
    }
}
