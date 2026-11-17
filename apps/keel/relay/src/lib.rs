//! Keel relay (spec 067): sits in a spoke network Keel cannot reach. Keel dials it over SSH
//! (`link_listen`); runners dial its `runner_listen` port, and each runner TCP connection is
//! handed to Keel as a `forwarded-tcpip` channel. The relay never terminates a runner session:
//! it only copies ciphertext (contracts/relay.md).
//!
//! - [`config`], `/etc/keel-relay/relay.toml`.
//! - [`key`], the relay's Ed25519 host key and the `hub_keys` file.
//! - [`link`], the SSH server Keel dials, and the `keel-relay-v1` control channel.
//! - [`forward`], the runner listener and the byte copy.
//! - [`cache`], the optional runner release cache.
//!
//! [`Relay`] ties them together. Tests start one in-process on ephemeral ports:
//!
//! ```no_run
//! # async fn demo(config: keel_relay::config::Config, host_key: russh::keys::PrivateKey,
//! #     hub_keys: Vec<russh::keys::ssh_key::public::KeyData>) -> std::io::Result<()> {
//! let relay = keel_relay::Relay::bind(&config, host_key, hub_keys).await?;
//! let (link, runners) = (relay.link_addr(), relay.runner_addr());
//! tokio::spawn(relay.run());
//! # Ok(()) }
//! ```

pub mod cache;
pub mod config;
pub mod forward;
pub mod key;
pub mod link;

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::PrivateKey;
use russh::keys::ssh_key::public::KeyData;
use tokio::net::TcpListener;

use crate::config::Config;
use crate::link::Shared;

/// The relay's own limits (contracts/relay.md "Limits"). Keel enforces its own again (T060).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Concurrent runner streams, pending ones included (`max_streams`, default 256).
    pub max_streams: u32,
    /// Runner channels waiting for Keel to accept them (64).
    pub max_pending_accepts: usize,
    /// How long Keel has to accept a runner channel before the TCP side is closed (20 s).
    pub channel_accept_timeout: Duration,
    /// How long a link connection has from TCP accept to an open control channel (10 s).
    pub link_setup_timeout: Duration,
    /// Link connections still in handshake or authentication at once (16).
    pub max_link_setups: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_streams: 256,
            max_pending_accepts: 64,
            channel_accept_timeout: keel_exec::relay::RUNNER_CHANNEL_TIMEOUT,
            link_setup_timeout: keel_exec::relay::HELLO_TIMEOUT,
            max_link_setups: 16,
        }
    }
}

impl Limits {
    /// The defaults, with `max_streams` from the config.
    pub fn from_config(config: &Config) -> Self {
        Self {
            max_streams: config.max_streams,
            ..Self::default()
        }
    }
}

/// A bound relay: both listeners are open, nothing is served until [`Relay::run`].
pub struct Relay {
    link: TcpListener,
    runner: TcpListener,
    host_key: PrivateKey,
    hub_keys: Vec<KeyData>,
    limits: Limits,
    http_listen: Option<SocketAddr>,
}

impl std::fmt::Debug for Relay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Relay")
            .field("link", &self.link.local_addr().ok())
            .field("runner", &self.runner.local_addr().ok())
            .field("hub_keys", &self.hub_keys.len())
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Relay {
    /// Binds `link_listen` and `runner_listen` (port 0 picks a free port; see
    /// [`Relay::link_addr`] and [`Relay::runner_addr`]). Limits come from
    /// [`Limits::from_config`].
    pub async fn bind(
        config: &Config,
        host_key: PrivateKey,
        hub_keys: Vec<KeyData>,
    ) -> std::io::Result<Self> {
        let link = TcpListener::bind(config.link_listen).await?;
        let runner = TcpListener::bind(config.runner_listen).await?;
        Ok(Self {
            link,
            runner,
            host_key,
            hub_keys,
            limits: Limits::from_config(config),
            http_listen: config.http_listen,
        })
    }

    /// Replaces the limits, e.g. a short accept timeout in a test.
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Where Keel dials the link.
    pub fn link_addr(&self) -> SocketAddr {
        self.link
            .local_addr()
            .expect("a bound listener has an address")
    }

    /// Where runners dial.
    pub fn runner_addr(&self) -> SocketAddr {
        self.runner
            .local_addr()
            .expect("a bound listener has an address")
    }

    /// Serves until a listener fails. Dropping the future stops accepting; use
    /// [`Relay::run_until`] to also close the link and its streams.
    pub async fn run(self) -> std::io::Result<()> {
        self.run_until(std::future::pending()).await
    }

    /// Serves until `shutdown` completes, then closes the link and every runner stream.
    pub async fn run_until(self, shutdown: impl Future<Output = ()>) -> std::io::Result<()> {
        if let Some(http) = self.http_listen {
            tracing::warn!(%http, "http_listen is set, but the release cache is not built yet");
        }
        let shared = Arc::new(Shared::new(&self.host_key, &self.limits));
        let config = Arc::new(keel_exec::relay::relay_server_config(self.host_key));
        tracing::info!(
            link = %self.link.local_addr()?,
            runners = %self.runner.local_addr()?,
            max_streams = self.limits.max_streams,
            "keel-relay serving"
        );
        let result = tokio::select! {
            r = link::serve(self.link, shared.clone(), config, Arc::new(self.hub_keys), self.limits.clone()) => r,
            r = forward::serve(self.runner, shared.clone(), self.limits) => r,
            () = shutdown => Ok(()),
        };
        shared.shutdown();
        result
    }
}
