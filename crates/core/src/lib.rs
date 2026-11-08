//! Shared, behaviour-neutral schema for the GUI and the sync engine.
//!
//! Every type here mirrors a file format the GUI and the Rust engine share.
//! Keeping one definition in one crate is what stops the two binaries from
//! drifting apart.

pub mod conf;
pub mod conflict;
pub mod delete_approval;
pub mod health;
pub mod local_trash;
pub mod snapshot;
pub mod state;
pub mod state_snapshot;
pub mod time;
