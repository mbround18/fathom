// Library entrypoint for tests and external consumers
#[cfg(feature = "egui-ui")]
pub mod app;
pub mod backend;
pub mod bridge;
pub mod config;
pub mod protocol;
pub mod sections;
pub mod startup;
