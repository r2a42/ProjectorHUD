//! ProjectorHUD: monitor and control networked projectors.
//!
//! The library half holds everything that is not UI: protocol drivers,
//! per-projector worker threads, discovery, scheduling, the simulator and
//! config persistence.

pub mod config;
pub mod discovery;
pub mod model;
pub mod net;
pub mod panasonic;
pub mod pjlink;
pub mod schedule;
pub mod sim;
pub mod worker;
