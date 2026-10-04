use std::thread;
use std::time::Duration;

use anyhow::Result;

use super::connection::{Connector, verify_controller};
use super::report;
use super::session::ControlSession;
use crate::config::Config;
use crate::infra::serial::Controller;
use crate::infra::watchdog::Watchdog;

/// The long-running service: reconnect whenever the link drops, and run one
/// control cycle per poll interval for as long as it holds.
pub fn run(config: &Config) -> Result<()> {
    let watchdog = Watchdog::from_env();
    if watchdog.can_notify() {
        watchdog.ready();
    }
    // Reported from $WATCHDOG_USEC rather than from "a notify socket exists":
    // the socket is set for any notify-capable unit, so its presence says
    // nothing about whether a watchdog is armed.
    match watchdog.interval() {
        Some(timeout) => log::info!("systemd watchdog active ({timeout:?} timeout)"),
        None if watchdog.can_notify() => {
            log::info!("systemd notifications available, but no watchdog is configured");
        }
        None => {}
    }

    let mut controller: Option<Controller> = None;
    let mut connector = Connector::default();
    let mut session = ControlSession::new(config, config.stall_cycles);
    report::stall_coverage(config);

    loop {
        // Pinged at the top of every iteration, so it covers the reconnect-retry
        // paths too, not just successful control cycles.
        watchdog.ping();

        if controller.is_none() {
            controller = connector.connect(&config.device);
            match controller.as_mut() {
                None => continue,
                Some(active) => verify_controller(config, active)?,
            }
        }

        let mut cycle_failed = false;
        if let Some(active) = controller.as_mut() {
            match session.cycle(config, active) {
                Ok(cycle) => {
                    if cycle.changed {
                        report::duty_changed(&cycle);
                    }
                }
                Err(e) => {
                    log::warn!("control cycle failed: {e:#}; reconnecting");
                    cycle_failed = true;
                }
            }
        }
        if cycle_failed {
            controller = None;
            session.reset();
        }

        // Reached on every path that did not already sleep inside connect(), so
        // the loop always yields: a cycle that fails instantly cannot spin.
        thread::sleep(Duration::from_secs(config.poll_interval_s));
    }
}
