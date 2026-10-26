//! Shared, behaviour-neutral schema for the GUI and the sync engine.
//!
//! Every type here mirrors a file format the Python engine already writes.
//! Keeping one definition in one crate is what stops the GUI and a future Rust
//! engine from drifting apart; the Python engine remains the writer until the
//! port is complete.

pub mod conf;
pub mod health;
pub mod snapshot;
pub mod state;
pub mod time;
