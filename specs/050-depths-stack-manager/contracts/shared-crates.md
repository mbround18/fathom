# Contract: shared crates and packages

What each extracted unit exposes. "Fathom unchanged" means: every existing Fathom test passes,
`cargo test -p fathom --test export_bindings` produces byte-identical files in
`ui/src/protocol/generated/`, and `scripts/ci.sh` is green, at every commit of the extraction.

## crates/container-runtime

No dependency on `fathom`, `kube`, `tauri`, or any app crate.

```rust
pub trait Runner: Send + Sync {            // moved from fathom::backend::kind
    async fn run(&self, program: &str, args: &[&str], cwd: Option<&Path>) -> Result<CmdOutput, RunError>;
    async fn run_with_stdin(&self, program: &str, args: &[&str], cwd: Option<&Path>, stdin: &[u8])
        -> Result<CmdOutput, RunError>;    // new; default impl returns RunError::Unsupported
}
pub struct SystemRunner; pub struct CmdOutput { .. } pub enum RunError { .. }
pub mod stream { pub trait StreamRunner; pub struct SystemStreamRunner; }
pub trait StreamSink: Send + Sync { fn emit(&self, ev: StreamEvent) -> bool; }   // was log_streams-bound
pub trait ClusterImpact: Send + Sync { .. }   // Fathom implements; Depths passes None
pub struct PreviewContext<'a> { pub app_name: &'a str, pub cluster: Option<&'a dyn ClusterImpact>, .. }
pub mod {probe, inventory, grouping, compose, preview, blast_radius, mutate, stats, logs, session};
pub mod adoption { pub struct AdoptionStore; impl AdoptionStore { pub fn open(path: PathBuf) -> Self } pub fn requirement_for_scope(..) }
pub mod wire;      // the 37 protocol types + KindRuntimeKind, serde + ts-rs, unchanged shape
#[cfg(feature = "test-util")] pub mod test_util;   // ScriptedProcess, MockRunner
```

Fathom keeps `src/backend/container_runtime/{cluster_link.rs, mod.rs}` with: the kind-label
post-pass, `requirement(&BackendCmdV1)`, the `StreamSink` impl over `BackendEventEnvelope`, and
`AdoptionStore::open(fathom_config_dir().join("container_adoption.json"))`.

## crates/web-auth

No dependency on `fathom` or `kube`.

```rust
pub mod session { pub struct SessionStore; pub struct Session; pub struct Identity { provider, issuer, subject, username, email, groups } }
pub mod csrf { pub fn check_mutation(..); pub fn new_token() -> String; .. }       // moved as is
pub mod login { pub struct LoginStore; }                                           // from server/src/http.rs
pub mod cookie { pub struct CookieConfig { pub name: &'static str, pub secure: bool, .. } }
pub mod extract { pub struct SessionUser(pub Arc<Session>); /* axum FromRequestParts */ }
pub mod provider {
    #[async_trait] pub trait IdentityProvider: Send + Sync {
        fn id(&self) -> &'static str;
        async fn authorize(&self, redirect: &Url) -> Result<AuthRedirect>;
        async fn complete(&self, pending: PendingLogin, code: &str, state: &str) -> Result<Completed>;
        async fn refresh(&self, s: &Session) -> Result<Refreshed>;   // re-reads groups
        fn end_session_url(&self, s: &Session) -> Option<Url>;
    }
}
pub mod oidc { pub struct OidcConfig; pub struct ClaimsConfig; pub struct OidcProvider; }  // moved, narrowed
pub mod github { pub struct GithubConfig; pub struct GithubProvider; }               // new (R5)
```

`fathom-server` keeps `refresh.rs` (kube token), `oidc_seed.rs`, and builds `OidcConfig` /
`groups_required` from `HostedConfig` + `AccessModel`. Cookie name stays `__Host-fathom_session`.

## crates/audit-log

```rust
pub struct AuditLog<E> { .. }
pub struct AuditOptions { pub dir: PathBuf, pub retention: Retention }
pub enum Retention { CompactAfter(usize) /* Fathom: 10_000 */, RotateAtBytes(u64) /* Depths */ }
impl<E: Serialize + DeserializeOwned> AuditLog<E> {
    pub fn open(opts: AuditOptions) -> io::Result<Self>;   // 0700 dir, 0600 file
    pub fn append(&self, entry: &E) -> io::Result<()>;    // one O_APPEND write, mutex
    pub fn read(&self, filter: impl Fn(&E) -> bool, cursor: Option<Cursor>, limit: usize) -> io::Result<Page<E>>;
}
```

`fathom-server::audit` becomes a wrapper implementing `fathom::backend::audit_log::AuditSink` over
`AuditLog<AuditLogEntry>` with `CompactAfter(10_000)`; on-disk path and format unchanged.

## packages/theme — `@companion/theme`

Exports: `./tokens.css`, `./effects.css`, `./index.css` (Tailwind entry pieces), `applyTheme`,
`ThemePrefs` (structural `{mode, accent, effects, glitch}`), `color` (contrast math), `loadFonts`,
`BrandMark` (takes `name` and `glyph` props; Fathom passes its own), `CircuitTrace`, `cn`.

## packages/ui — `@companion/ui`

The 22 shadcn primitives, relative imports only, peer deps `react`, `react-dom`, radix, phosphor.
`Toaster` takes `theme` as a prop. Consumers add the package to Tailwind's `@source`.

## packages/terminal — `@companion/terminal`

`TerminalSurface({transport, subscribe, onResize})`, `RingBuffer<T>(cap)`,
`LogView({lines, follow, wrap, onLoadOlder?})` (virtualised), `PaneStates`, `YamlDiffView`.
Fathom's `TerminalView` and `LogScroller` become adapters from its store onto these.
