//! Library surface for unit and integration tests.
//!
//! The CLI binary (`main.rs`) is a thin wrapper around these modules.

pub mod app;
pub mod cli;
pub mod clip;
pub mod engine;
#[cfg(feature = "gui")]
pub mod gui;
pub mod ports;
pub mod scan;
pub mod ssh;
