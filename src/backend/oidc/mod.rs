//! OIDC identity backend, spec `033-rauthy-oidc-teams-users`.
//!
//! Modules land per user story (`flow`, `validate`, `session`, ...). `store` is the only place
//! that touches the OS credential store; nothing outside it may persist a token or key.

pub mod store;
