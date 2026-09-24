//! fathom-daemon: optional, read-only in-cluster daemon (spec 036).
//!
//! Every data path is filtered per calling user (`authz`); `identity` accepts only verified
//! front-proxy identity; the `SeriesStore` trait keeps metrics storage swappable.

pub mod authz;
pub mod config;
pub mod derive;
pub mod eventlog;
pub mod http;
pub mod hub;
pub mod identity;
pub mod metrics_ring;
pub mod series_store;
pub mod tls;
pub mod topology;
pub mod watchers;
