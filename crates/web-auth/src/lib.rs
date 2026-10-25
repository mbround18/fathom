//! `web-auth`: Browser sign-in for the suite's servers: OIDC, GitHub and Discord, sessions, CSRF (spec 050).
//!
//! No dependency on `fathom` or `kube`: Fathom's hosted server and Depths both build on it.
pub mod cookie;
pub mod csrf;
pub mod discord;
pub mod extract;
pub mod github;
pub mod login;
pub mod oauth;
pub mod oidc;
pub mod provider;
pub mod session;
