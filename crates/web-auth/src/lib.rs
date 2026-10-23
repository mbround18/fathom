//! `web-auth`: Browser sign-in for the suite's servers: OIDC and GitHub, sessions, CSRF (spec 050).
//!
//! No dependency on `fathom` or `kube`: Fathom's hosted server and Depths both build on it.
pub mod csrf;
pub mod oidc;
pub mod session;
