//! Rust sync engine.
//!
//! Pure scan/plan/state/execute pieces used by the background service. All
//! Proton traffic still goes through the official CLI; this crate owns the
//! local/remote tree reconciliation and the file operations.

pub mod action;
pub mod cli;
pub mod execute;
pub mod filter;
pub mod firstsync;
pub mod health;
pub mod loop_;
pub mod plan;
pub mod policy;
pub mod push;
pub mod scan;
pub mod state_update;
