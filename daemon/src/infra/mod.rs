//! Adapters to everything outside the process: the controller's serial port,
//! the kernel's drive sensors, systemd and journald.

pub mod hwmon;
pub mod logging;
pub mod serial;
pub mod watchdog;
