//! `keel_server`: the Keel server library (spec 055). `main.rs` is a thin binary over it, and
//! the integration tests in `tests/` drive it directly.

pub mod api;
pub mod audit;
pub mod bindings;
pub mod config;
pub mod credentials;
pub mod db;
pub mod dispatch;
pub mod hooks;
pub mod identity;
pub mod inventory;
pub mod outside;
pub mod passkeys;
pub mod preview;
pub mod queue;
pub mod receivers;
pub mod runner_listener;
pub mod seal;
pub mod serve;
pub mod signin;
pub mod sync;
pub mod telemetry;
pub mod tls;
pub mod ws;

// Governance (US6)
pub mod governance;

// Runs (US2)
pub mod runs;

// Hosts and the SSH CAs (US7)
pub mod hosts;

// Signed mode with reviewed overrides (US8)
pub mod review;
