//! How each finding is worded in the log. Nothing here decides anything.

use super::session::Cycle;
use crate::config::Config;
use crate::constants::MAX_DUTY_PERCENT;
use crate::domain::controller::Discrepancy;
use crate::domain::failsafe::TimeoutCheck;
use crate::domain::thermal::{DiskReading, Limits, TempSummary, ThermalBand};

/// Say something about a drive that has crossed one of its own thresholds.
pub fn band(disk: &str, celsius: f64, limits: &Limits, band: ThermalBand) {
    match band {
        ThermalBand::OverCrit => log::error!(
            "{disk} is at {celsius:.1} °C, at or above its critical threshold of \
             {:.0} °C. The fan is already at its limit; reduce load or improve \
             airflow",
            limits.crit.unwrap_or(f64::NAN)
        ),
        ThermalBand::OverMax => log::warn!(
            "{disk} is at {celsius:.1} °C, above its rated maximum of {:.0} °C",
            limits.max.unwrap_or(f64::NAN)
        ),
        ThermalBand::Normal => {
            log::info!("{disk} is at {celsius:.1} °C, back within its rated range");
        }
    }
}

/// Each drive that could not be read, and why.
pub fn unreadable(readings: &[DiskReading]) {
    for disk in readings {
        if let Err(e) = &disk.celsius {
            log::error!("{}: {e:#}", disk.name);
        }
    }
}

/// The duty this cycle settled on, and what it was based on.
pub fn decision(summary: &TempSummary, filtered: Option<f64>, duty: u8) {
    match summary {
        TempSummary::Hottest(celsius) => {
            if let Some(filtered) = filtered {
                log::debug!("hottest {celsius:.1} °C (smoothed {filtered:.2} °C) -> {duty}%");
            }
        }
        TempSummary::Unknown => {
            log::error!("no drive temperature readable; commanding {MAX_DUTY_PERCENT}%");
        }
    }
}

/// A new duty. Carries the temperature, so the journal records *why* the fan is
/// where it is. Logging the duty alone at the default level makes a drive at
/// 69 °C indistinguishable from one at 35 °C.
pub fn duty_changed(cycle: &Cycle) {
    match cycle.hottest {
        Some(celsius) => log::info!(
            "fan duty set to {}% (hottest drive {celsius:.1} °C)",
            cycle.duty
        ),
        None => log::info!(
            "fan duty set to {}% (no drive temperature readable)",
            cycle.duty
        ),
    }
}

pub fn discrepancy(found: &Discrepancy) {
    match *found {
        Discrepancy::DutyMismatch {
            commanded,
            reported,
        } => log::error!(
            "commanded {commanded}% but the controller reports {reported}% applied, so \
             the fan is not running at the speed this daemon believes"
        ),
        // Warn rather than error: the fan was at full speed, which is safe. It
        // still says the cadence or the controller needs looking at.
        Discrepancy::FailsafeWhileControlled {
            commanded,
            uptime_seconds,
        } => log::warn!(
            "controller reports FAILSAFE although {commanded}% was commanded: it either \
             went its whole failsafe timeout without a PWM command (a slow cycle, or \
             controller_failsafe_timeout_s set higher than the firmware's) or restarted \
             (uptime {uptime_seconds} s), and holds the fan at 100% until the next command"
        ),
        Discrepancy::Stalled { duty, rpm, stall } => log::error!(
            "FAN STALL: the controller is applying {duty}% but the tachometer reads {rpm} \
             RPM for {} consecutive cycles (expected at least {} RPM). Check the fan, its \
             power lead, the PWM drive transistor and the tachometer wiring; drives may be \
             overheating",
            stall.cycles,
            stall.threshold_rpm
        ),
    }
}

/// The failsafe-timeout cross-check, when there is something to say.
pub fn failsafe_timeout(check: TimeoutCheck) {
    match check {
        TimeoutCheck::Unreported => log::debug!(
            "controller does not report its failsafe timeout (older firmware); \
             controller_failsafe_timeout_s is taken on trust"
        ),
        TimeoutCheck::Matches => {}
        TimeoutCheck::DiffersButFits {
            configured_s,
            reported_s,
        } => log::warn!(
            "controller_failsafe_timeout_s is {configured_s} s but the controller reports \
             {reported_s} s; the poll interval still fits, but correct the configuration"
        ),
    }
}

/// What the configured stall detection can and cannot see.
pub fn stall_coverage(config: &Config) {
    if config.min_expected_rpm.is_none() {
        log::warn!("min_expected_rpm is unset: a stopped fan will not be detected");
    } else if config.expected_rpm.is_none() {
        // An absolute floor only catches a fan that has essentially stopped. It
        // cannot catch one spinning at its uncontrolled floor speed while a high
        // duty is commanded, because that speed is above any sane floor.
        log::warn!(
            "expected_rpm is unset: only a fully stopped fan will be detected, not \
             one stuck at its minimum speed while a high duty is commanded"
        );
    }
}
