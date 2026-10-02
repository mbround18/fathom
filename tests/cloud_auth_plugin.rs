//! 035-cloud-auth-plugins: integration tests against the fake ExecCredential plugin
//! (`tests/support/fake_plugin.rs`). Real EKS/GKE/AKS are not exercised here (manual quickstart).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fathom::backend::cloud_auth::cache::CredentialCache;
use fathom::backend::cloud_auth::{PrepareEnv, plugin_error_of, prepare_config_with};
use fathom::protocol::{CloudProvider, ExecPolicy, PluginErrorKind};
use kube::config::{KubeConfigOptions, Kubeconfig};
use secrecy::ExposeSecret;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-exec-plugin");
const TOKEN: &str = "k8s-aws-v1.RkFLRVRPS0VOMTIzNDU2Nzg5MEFCQ0RFRkdISUpLTE1O";

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    /// A tempdir holding the fake plugin named like a real provider plugin.
    fn new(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_owned()
        });
        // A symlink, not a copy. These tests run concurrently in one process, and a copy opens the
        // destination for writing: any sibling test that forks between that open and its close
        // inherits the writable descriptor, and its `exec` of the same inode fails with ETXTBSY
        // ("Text file busy"). A symlink never opens the target for writing, and `fs::metadata`
        // follows it, so discovery still sees an executable regular file.
        #[cfg(unix)]
        std::os::unix::fs::symlink(FAKE, &bin).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(FAKE, &bin).unwrap();
        Self { dir }
    }

    fn search(&self) -> Vec<PathBuf> {
        vec![self.dir.path().to_path_buf()]
    }

    fn counter(&self) -> PathBuf {
        self.dir.path().join("count")
    }

    fn spawns(&self) -> usize {
        std::fs::read(self.counter()).map_or(0, |b| b.len())
    }
}

async fn config(command: &str, args: &[&str], extra: &str) -> kube::Config {
    let yaml = format!(
        r"
apiVersion: v1
kind: Config
clusters: [{{name: c, cluster: {{server: 'https://127.0.0.1:1'}}}}]
contexts: [{{name: ctx, context: {{cluster: c, user: u}}}}]
current-context: ctx
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: {command:?}
      args: {args:?}
      {extra}
"
    );
    let kc = Kubeconfig::from_yaml(&yaml).unwrap();
    kube::Config::from_custom_kubeconfig(kc, &KubeConfigOptions::default())
        .await
        .unwrap()
}

fn env<'a>(search: &'a [PathBuf], cache: &'a CredentialCache, secs: u64) -> PrepareEnv<'a> {
    PrepareEnv {
        policy: ExecPolicy::AllowKnownOnly,
        search,
        cache,
        timeout: Duration::from_secs(secs),
    }
}

fn plugin_args(f: &Fixture, mode: &str, lifetime: &str) -> Vec<String> {
    vec![
        mode.to_owned(),
        f.counter().display().to_string(),
        lifetime.to_owned(),
    ]
}

async fn prepare(
    f: &Fixture,
    mode: &str,
    lifetime: &str,
    cache: &CredentialCache,
    timeout: u64,
) -> Result<kube::Config, fathom::protocol::PluginError> {
    let args = plugin_args(f, mode, lifetime);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cfg = config("gke-gcloud-auth-plugin", &refs, "").await;
    prepare_config_with("ctx", &mut cfg, &env(&f.search(), cache, timeout)).await?;
    Ok(cfg)
}

#[tokio::test]
async fn token_is_injected_and_exec_removed() {
    let f = Fixture::new("gke-gcloud-auth-plugin");
    let cache = CredentialCache::default();
    let cfg = prepare(&f, "ok", "900", &cache, 10).await.unwrap();
    assert!(cfg.auth_info.exec.is_none());
    assert_eq!(cfg.auth_info.token.unwrap().expose_secret(), TOKEN);
}

#[tokio::test]
async fn n_polls_in_one_lifetime_spawn_once_and_expiry_respawns() {
    let f = Fixture::new("gke-gcloud-auth-plugin");
    let cache = CredentialCache::default();
    for _ in 0..12 {
        prepare(&f, "ok", "900", &cache, 10).await.unwrap();
    }
    assert_eq!(f.spawns(), 1, "SC-002: one spawn per identity per lifetime");

    // A credential shorter than the 60s skew is never reusable: every poll re-invokes.
    let f2 = Fixture::new("gke-gcloud-auth-plugin");
    let cache2 = CredentialCache::default();
    prepare(&f2, "ok", "30", &cache2, 10).await.unwrap();
    prepare(&f2, "ok", "30", &cache2, 10).await.unwrap();
    assert_eq!(f2.spawns(), 2);
}

#[tokio::test]
async fn concurrent_polls_single_flight() {
    let f = Arc::new(Fixture::new("gke-gcloud-auth-plugin"));
    let cache = Arc::new(CredentialCache::default());
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let (f, cache) = (f.clone(), cache.clone());
        tasks.push(tokio::spawn(async move {
            prepare(&f, "ok", "900", &cache, 10).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    assert_eq!(f.spawns(), 1);
}

#[tokio::test]
async fn invalidation_after_401_respawns_once() {
    let f = Fixture::new("gke-gcloud-auth-plugin");
    let cache = CredentialCache::default();
    prepare(&f, "ok", "900", &cache, 10).await.unwrap();
    assert!(cache.invalidate_context("ctx"));
    prepare(&f, "ok", "900", &cache, 10).await.unwrap();
    prepare(&f, "ok", "900", &cache, 10).await.unwrap();
    assert_eq!(f.spawns(), 2);
}

#[tokio::test]
async fn each_failure_class_maps_to_its_typed_error() {
    let cases = [
        ("fail", PluginErrorKind::PluginFailed),
        ("garbage", PluginErrorKind::PluginFailed),
        ("login", PluginErrorKind::CredentialExpired),
        ("expired", PluginErrorKind::CredentialExpired),
    ];
    for (mode, kind) in cases {
        let f = Fixture::new("gke-gcloud-auth-plugin");
        let err = prepare(&f, mode, "900", &CredentialCache::default(), 10)
            .await
            .unwrap_err();
        assert_eq!(err.kind, kind, "{mode}");
        assert_eq!(err.provider, CloudProvider::Gke, "{mode}");
        assert_eq!(err.context, "ctx");
        assert_eq!(err.command, "gke-gcloud-auth-plugin");
    }
}

#[tokio::test]
async fn login_error_carries_provider_relogin_hint_never_executed() {
    let f = Fixture::new("gke-gcloud-auth-plugin");
    let err = prepare(&f, "login", "900", &CredentialCache::default(), 10)
        .await
        .unwrap_err();
    assert!(err.hint.unwrap().contains("gcloud auth login"));
    assert_eq!(
        f.spawns(),
        1,
        "the plugin ran once; no login command was run"
    );
}

#[tokio::test]
async fn hanging_plugin_is_killed_at_the_timeout() {
    let f = Fixture::new("gke-gcloud-auth-plugin");
    let started = std::time::Instant::now();
    let err = prepare(&f, "hang", "900", &CredentialCache::default(), 1)
        .await
        .unwrap_err();
    assert_eq!(err.kind, PluginErrorKind::PluginTimeout);
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[tokio::test]
async fn missing_plugin_on_empty_path_names_plugin_provider_context() {
    let mut cfg = config("gke-gcloud-auth-plugin", &[], "").await;
    let cache = CredentialCache::default();
    let err = prepare_config_with("ctx", &mut cfg, &env(&[], &cache, 5))
        .await
        .unwrap_err();
    assert_eq!(err.kind, PluginErrorKind::PluginMissing);
    assert_eq!(err.provider, CloudProvider::Gke);
    assert_eq!(err.context, "ctx");
    assert!(err.hint.unwrap().contains("gke-gcloud-auth-plugin"));
}

#[tokio::test]
async fn interactive_mode_always_is_unsupported_and_never_spawns() {
    let f = Fixture::new("kubelogin");
    let args = plugin_args(&f, "ok", "900");
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut cfg = config("kubelogin", &refs, "interactiveMode: Always").await;
    let cache = CredentialCache::default();
    let err = prepare_config_with("ctx", &mut cfg, &env(&f.search(), &cache, 5))
        .await
        .unwrap_err();
    assert_eq!(err.kind, PluginErrorKind::PluginInteractiveUnsupported);
    assert_eq!(f.spawns(), 0);
}

#[tokio::test]
async fn unknown_command_is_blocked_unless_allow_all() {
    let f = Fixture::new("totally-custom-plugin");
    let args = plugin_args(&f, "ok", "900");
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let cache = CredentialCache::default();

    let mut cfg = config("totally-custom-plugin", &refs, "").await;
    let err = prepare_config_with("ctx", &mut cfg, &env(&f.search(), &cache, 5))
        .await
        .unwrap_err();
    assert_eq!(err.kind, PluginErrorKind::PluginBlocked);
    assert!(err.message.contains("totally-custom-plugin"));
    assert_eq!(f.spawns(), 0, "a blocked plugin is never run");

    let mut cfg = config("totally-custom-plugin", &refs, "").await;
    let allow_all = PrepareEnv {
        policy: ExecPolicy::AllowAll,
        search: &f.search(),
        cache: &cache,
        timeout: Duration::from_secs(5),
    };
    prepare_config_with("ctx", &mut cfg, &allow_all)
        .await
        .unwrap();
    assert_eq!(f.spawns(), 1);
}

/// SC-003: the fixture token must not appear in any error, event payload, or debug rendering.
#[tokio::test]
async fn token_never_leaks_into_errors_events_or_debug_output() {
    // `fail` mode makes the plugin echo the token on stderr.
    let f = Fixture::new("gke-gcloud-auth-plugin");
    let err = prepare(&f, "fail", "900", &CredentialCache::default(), 10)
        .await
        .unwrap_err();
    let rendered = [
        err.to_string(),
        format!("{err:?}"),
        serde_json::to_string(&err).unwrap(),
        serde_json::to_string(&fathom::protocol::SystemEvent::ContextStatusUpdated {
            context: "ctx".into(),
            status: fathom::protocol::ContextConnectionStatus::Error,
            error: Some(err.to_string()),
            plugin_error: Some(err.clone()),
        })
        .unwrap(),
    ];
    for text in &rendered {
        assert!(!text.contains(TOKEN), "token leaked: {text}");
        assert!(
            !text.contains("RkFLRVRPS0"),
            "token fragment leaked: {text}"
        );
    }

    // Success path: neither Debug of the config nor of the cached credential shows it.
    let cache = CredentialCache::default();
    let cfg = prepare(&f, "ok", "900", &cache, 10).await.unwrap();
    assert!(!format!("{:?}", cfg.auth_info).contains(TOKEN));
    let any: anyhow::Error = err.into();
    assert!(plugin_error_of(&any).is_some());
}

/// T023: the MCP server reuses the backend's shared client and has no exec-plugin surface.
#[test]
fn mcp_has_no_plugin_execution_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backend/mcp");
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for banned in ["cloud_auth", "run_plugin", "ExecSpec", "expose_secret"] {
                    assert!(
                        !text.contains(banned),
                        "{} mentions {banned}",
                        path.display()
                    );
                }
            }
        }
    }
}
