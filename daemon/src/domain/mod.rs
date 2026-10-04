//! The rules: what the fan should do, and what counts as a fault.
//!
//! Pure logic. Nothing here touches a file, a port or the log: findings are
//! returned, and `app` decides how to report them.

pub mod controller;
pub mod curve;
pub mod damper;
pub mod failsafe;
pub mod stall;
pub mod thermal;
