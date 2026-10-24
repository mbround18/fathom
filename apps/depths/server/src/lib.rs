//! `depths-server`: the Depths server (spec 050) as a library, so the binary stays a thin
//! execution layer and the routes are testable in-process.

pub mod audit;
pub mod client_ip;
pub mod config;
pub mod diff;
pub mod headers;
pub mod http;
pub mod inventory;
pub mod limits;
pub mod ops;
pub mod refresh;
pub mod serve;
pub mod users;
pub mod watch;
pub mod ws;
