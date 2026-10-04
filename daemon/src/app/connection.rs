use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::Result;

use super::report;
use crate::config::Config;
use crate::constants::{
    RECONNECT_BACKOFF_FACTOR, RECONNECT_BACKOFF_INITIAL, RECONNECT_BACKOFF_MAX,
};
use crate::domain::failsafe::check_failsafe_timeout;
use crate::infra::serial::Controller;

/// Opens the controller, backing off between failed attempts.
pub struct Connector {
    backoff: Duration,
}

impl Default for Connector {
    fn default() -> Self {
        Self {
            backoff: RECONNECT_BACKOFF_INITIAL,
        }
    }
}

impl Connector {
    pub fn connect(&mut self, device: &Path) -> Option<Controller> {
        match Controller::open(device) {
            Ok(mut controller) => match controller.handshake() {
                Ok(version) => {
                    log::info!("connected to controller ({version})");
                    // Reset only once the link is fully usable. Resetting on a bare
                    // open success lets a peer that opens but never answers re-arm
                    // the backoff every attempt, defeating the cap entirely.
                    self.backoff = RECONNECT_BACKOFF_INITIAL;
                    Some(controller)
                }
                Err(e) => {
                    log::warn!(
                        "connected but handshake failed: {e:#}; retrying in {:?} \
                         (controller failsafe covers the gap)",
                        self.backoff
                    );
                    self.back_off();
                    None
                }
            },
            Err(e) => {
                log::warn!(
                    "cannot open {} ({e:#}); retrying in {:?} (controller failsafe covers the gap)",
                    device.display(),
                    self.backoff
                );
                self.back_off();
                None
            }
        }
    }

    /// Sleep the current backoff, then widen it up to the cap.
    ///
    /// Every failed connection attempt must pass through here. A path that returns
    /// without sleeping turns any persistent, fast-failing peer into a busy loop
    /// that floods the journal and never lets the poll interval throttle it.
    fn back_off(&mut self) {
        thread::sleep(self.backoff);
        // Capped exponential backoff: steady cadence during long outages.
        self.backoff = (self.backoff * RECONNECT_BACKOFF_FACTOR).min(RECONNECT_BACKOFF_MAX);
    }
}

/// Checks that need the controller once per connection.
pub fn verify_controller(config: &Config, controller: &mut Controller) -> Result<()> {
    match controller.status_fields() {
        Ok(status) => {
            let check = check_failsafe_timeout(
                config.poll_interval_s,
                config.controller_failsafe_timeout_s,
                status.failsafe_timeout_s,
            )?;
            report::failsafe_timeout(check);
            Ok(())
        }
        Err(e) => {
            log::warn!("STATUS read failed: {e:#}; cannot cross-check the failsafe timeout");
            Ok(())
        }
    }
}
