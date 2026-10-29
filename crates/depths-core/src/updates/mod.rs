//! Image updates (spec 062): the pure decisions, with no I/O.
//!
//! References, Watchtower labels, the effective policy per service, maintenance windows and the
//! `Registry` port the check logic reads through. Time comes in as `now` and through the
//! [`window::LocalZone`] and [`window::Clock`] traits; the registry only through
//! [`registry::Registry`]. The server owns the state files, credentials, the scheduler and the
//! runner.

pub mod reference;
