//! `~/.config/proton-sync.conf` - the engine's settings file.
//!
//! The implementation moved to `proton-core` so the GUI and the Rust engine
//! read exactly the same defaults and write the same nine keys.

pub use proton_core::conf::*;
