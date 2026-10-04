use anyhow::Result;

use super::report;
use crate::config::Config;
use crate::domain::controller::check_controller;
use crate::domain::damper::Damper;
use crate::domain::stall::StallMonitor;
use crate::domain::thermal::{self, TempSummary, ThermalWatch};
use crate::infra::hwmon;
use crate::infra::serial::Controller;

/// What one cycle decided, for the caller to report.
pub struct Cycle {
    pub duty: u8,
    pub hottest: Option<f64>,
    /// Whether `duty` differs from what the previous cycle commanded.
    pub changed: bool,
}

/// What the control loop remembers between cycles: the damping filter, the run
/// of low tachometer readings, each drive's last thermal band, and the duty last
/// commanded on this connection.
pub struct ControlSession {
    damper: Damper,
    stall: Option<StallMonitor>,
    thermal: ThermalWatch,
    last_duty: Option<u8>,
}

impl ControlSession {
    /// `stall_cycles` is how many consecutive low readings make a stall.
    pub fn new(config: &Config, stall_cycles: u32) -> Self {
        Self {
            damper: Damper::new(config.smoothing_alpha, config.deadband_percent),
            stall: stall_monitor(config, stall_cycles),
            thermal: ThermalWatch::default(),
            last_duty: None,
        }
    }

    /// Drop the history that an outage of unknown length invalidates, for use
    /// when the link to the controller has dropped: the EWMA must not blend a
    /// pre-outage temperature into the first reading after reconnecting, and "N
    /// consecutive cycles" must describe cycles that really were consecutive
    /// rather than a count spanning the gap.
    pub fn reset(&mut self) {
        self.last_duty = None;
        self.damper.reset();
        if let Some(monitor) = self.stall.as_mut() {
            monitor.reset();
        }
    }

    /// One pass: read the drives, decide a duty, check the controller's report
    /// of the previous one, then command the new one.
    pub fn cycle(&mut self, config: &Config, controller: &mut Controller) -> Result<Cycle> {
        let readings = hwmon::read_all(&config.disks);
        for disk in &readings {
            if let Ok(celsius) = &disk.celsius {
                log::debug!("{}: {celsius:.1} °C", disk.name);
                if let Some(band) = self.thermal.observe(&disk.name, disk.limits.band(*celsius)) {
                    report::band(&disk.name, *celsius, &disk.limits, band);
                }
            }
        }
        // Failures are reported together, once every reading has been seen.
        report::unreadable(&readings);
        let summary = thermal::summarize(&readings);
        let hottest = match &summary {
            TempSummary::Hottest(celsius) => Some(*celsius),
            TempSummary::Unknown => None,
        };
        let duty = self.damper.decide(&config.curve, &summary);
        report::decision(&summary, self.damper.filtered_celsius(), duty);

        // STATUS comes before the new duty. The tachometer reports its last completed
        // one-second window, so RPM read straight after a change still shows the old
        // speed, and judged against the new duty every large step up looked like a
        // stall. Before the change, the duty and RPM in STATUS describe the same
        // settled state. Comparing against the previous command also catches a
        // failsafe that fired in between, which this cycle's PWM would clear unseen.
        //
        // A STATUS failure only skips the checks. Failing the cycle for it would drop
        // a link whose PWM may well succeed, costing a full reconnect for a
        // diagnostic. A genuinely broken link fails set_pwm, and that is what should
        // trigger the reconnect.
        match controller.status_fields() {
            Ok(status) => {
                for found in check_controller(self.last_duty, &status, self.stall.as_mut()) {
                    report::discrepancy(&found);
                }
            }
            Err(e) => log::warn!("STATUS read failed: {e:#}; skipped this cycle's checks"),
        }
        controller.set_pwm(duty)?;
        let changed = self.last_duty != Some(duty);
        self.last_duty = Some(duty);
        Ok(Cycle {
            duty,
            hottest,
            changed,
        })
    }
}

/// Build the stall monitor from configuration, if it is enabled at all.
fn stall_monitor(config: &Config, required_cycles: u32) -> Option<StallMonitor> {
    config.min_expected_rpm.map(|min_rpm| {
        StallMonitor::new(
            min_rpm,
            required_cycles,
            config.expected_rpm.clone(),
            config.stall_tolerance_percent,
        )
    })
}
