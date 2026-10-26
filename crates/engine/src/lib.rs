//! Rust sync engine.
//!
//! This crate is being built beside the Python engine, not instead of it yet:
//! the pure scan/plan/state pieces land first so they can be diffed against
//! `scripts/proton-sync.py` before anything is allowed to touch files.

pub mod action;
pub mod cli;
pub mod filter;
pub mod plan;
pub mod policy;
pub mod scan;
pub mod state_update;
