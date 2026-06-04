//! mlt-core — the pure heart of MLT: domain types + ports, with **zero IO**.
//!
//! No network, disk, OS, clock, or randomness lives here. Every side effect enters
//! through a trait in [`ports`]; adapters (crate `mlt-adapters`) implement them, and the
//! Tauri app wires them together. This is what makes the logic testable in milliseconds
//! with fakes and keeps OS-specific code out of the core. See `docs/adr/0006-hexagonal-core.md`.

pub mod alarms;
pub mod domain;
pub mod ports;
pub mod providers;
pub mod sources;

pub use alarms::*;
pub use domain::*;
pub use ports::*;
pub use providers::*;
pub use sources::*;
