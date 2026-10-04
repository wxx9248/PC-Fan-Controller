//! What the controller says about itself, checked against what this daemon
//! commanded.

use crate::domain::stall::{Stall, StallMonitor};

/// A parsed `STATUS PWM=<n> RPM=<n> FAILSAFE=<0|1> UPTIME=<n>` reply, plus
/// `FAILSAFE_TIMEOUT=<s>` from firmware new enough to report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerStatus {
    pub pwm_percent: u8,
    pub rpm: u32,
    pub failsafe: bool,
    pub uptime_seconds: u64,
    /// The firmware's own failsafe timeout. `None` from firmware built before
    /// STATUS carried it.
    pub failsafe_timeout_s: Option<u64>,
}

/// Something the controller reported that disagrees with what we asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discrepancy {
    /// The controller is applying a duty other than the one last commanded.
    DutyMismatch { commanded: u8, reported: u8 },
    /// The controller took control back after the last command: its failsafe
    /// expired, or it restarted. Either way it holds the fan at 100%.
    FailsafeWhileControlled { commanded: u8, uptime_seconds: u64 },
    /// The fan is turning slower than the duty it runs at should ever produce.
    Stalled { duty: u8, rpm: u32, stall: Stall },
}

/// Cross-check a `STATUS` against the last duty this daemon commanded.
///
/// `previous` is that duty, or `None` before anything has been commanded on this
/// connection, when the controller may still legitimately be in its boot or
/// failsafe state and only the fan itself is judged. All three findings come
/// out of a `STATUS` the loop already pays a round-trip for, so checking them is
/// free. Returns findings rather than logging them, so the decisions are
/// testable without a log harness.
pub fn check_controller(
    previous: Option<u8>,
    status: &ControllerStatus,
    stall: Option<&mut StallMonitor>,
) -> Vec<Discrepancy> {
    let mut found = Vec::new();
    if let Some(commanded) = previous {
        // The controller's own report of the duty it applied. A firmware that
        // acknowledges `PWM` with `OK` and then applies something else goes
        // unnoticed forever: a regression in the inverting-stage compensation,
        // say, or a clamp added in a minor version the major-only handshake
        // deliberately accepts.
        if status.pwm_percent != commanded {
            found.push(Discrepancy::DutyMismatch {
                commanded,
                reported: status.pwm_percent,
            });
        }
        // The next `PWM` clears the failsafe flag, so a failsafe that fired
        // between two cycles is visible only to a `STATUS` taken before it.
        if status.failsafe {
            found.push(Discrepancy::FailsafeWhileControlled {
                commanded,
                uptime_seconds: status.uptime_seconds,
            });
        }
    }
    // Close the loop on airflow: without this the tachometer is measured by the
    // firmware and read by nobody, so a dead fan looks identical to a healthy one.
    // Judged against the duty in the same report, which is what the fan was
    // running at while the tachometer counted.
    if let Some(monitor) = stall {
        if let Some(stall) = monitor.observe(status.pwm_percent, status.rpm) {
            found.push(Discrepancy::Stalled {
                duty: status.pwm_percent,
                rpm: status.rpm,
                stall,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(pwm: u8, rpm: u32, failsafe: bool) -> ControllerStatus {
        ControllerStatus {
            pwm_percent: pwm,
            rpm,
            failsafe,
            uptime_seconds: 42,
            failsafe_timeout_s: None,
        }
    }

    /// The deployed fan's expected response; see `domain::stall`'s tests for its
    /// origin.
    fn duty_scaled(required_cycles: u32) -> StallMonitor {
        let table = vec![[0, 840], [31, 840], [32, 900], [50, 1185], [100, 1973]];
        StallMonitor::new(400, required_cycles, Some(table), 70)
    }

    #[test]
    fn a_healthy_controller_reports_nothing() {
        let mut stall = StallMonitor::new(400, 3, None, 70);
        assert!(check_controller(Some(31), &status(31, 840, false), Some(&mut stall)).is_empty());
    }

    #[test]
    fn a_duty_the_controller_did_not_apply_is_caught() {
        // The firmware answers `OK` unconditionally, so this echo is the only
        // evidence that the duty actually landed.
        let found = check_controller(Some(31), &status(100, 1973, false), None);
        assert_eq!(
            found,
            vec![Discrepancy::DutyMismatch {
                commanded: 31,
                reported: 100
            }]
        );
    }

    #[test]
    fn a_failsafe_that_fired_since_the_last_command_is_caught() {
        let found = check_controller(Some(31), &status(31, 840, true), None);
        assert_eq!(
            found,
            vec![Discrepancy::FailsafeWhileControlled {
                commanded: 31,
                uptime_seconds: 42
            }]
        );
    }

    #[test]
    fn several_discrepancies_are_all_reported() {
        // A controller in failsafe is at 100%, so the duty mismatch and the
        // failsafe flag arrive together; neither must mask the other.
        let found = check_controller(Some(31), &status(100, 1973, true), None);
        assert_eq!(found.len(), 2, "got {found:?}");
        assert!(found.contains(&Discrepancy::DutyMismatch {
            commanded: 31,
            reported: 100
        }));
        assert!(found.contains(&Discrepancy::FailsafeWhileControlled {
            commanded: 31,
            uptime_seconds: 42
        }));
    }

    #[test]
    fn a_stall_is_reported_only_after_the_required_cycles() {
        let mut stall = StallMonitor::new(400, 3, None, 70);
        for _ in 0..2 {
            assert!(check_controller(Some(50), &status(50, 0, false), Some(&mut stall)).is_empty());
        }
        let found = check_controller(Some(50), &status(50, 0, false), Some(&mut stall));
        assert!(
            matches!(found.as_slice(), [Discrepancy::Stalled { rpm: 0, .. }]),
            "got {found:?}"
        );
    }

    #[test]
    fn the_stall_check_is_skipped_when_it_is_not_configured() {
        // min_expected_rpm unset must not mean "every cycle is a stall".
        for _ in 0..10 {
            assert!(check_controller(Some(50), &status(50, 0, false), None).is_empty());
        }
    }

    #[test]
    fn a_reading_is_judged_at_the_duty_it_was_taken_at() {
        // Read just after raising the duty, 840 RPM measured at 31% would be
        // judged against the 1381 RPM that 100% needs, and a healthy fan would
        // look stalled. Read before the change, the report pairs 840 RPM with the
        // 31% it was measured at.
        let mut once = duty_scaled(1);
        assert!(
            check_controller(None, &status(31, 840, false), Some(&mut once)).is_empty(),
            "840 RPM is healthy at 31%, whatever is commanded next"
        );
        // A fan that really is stuck at its floor is still caught.
        let found = check_controller(None, &status(100, 840, false), Some(&mut once));
        assert!(
            matches!(
                found.as_slice(),
                [Discrepancy::Stalled {
                    duty: 100,
                    rpm: 840,
                    ..
                }]
            ),
            "got {found:?}"
        );
    }

    #[test]
    fn before_the_first_command_only_the_fan_is_judged() {
        // On a fresh connection the controller may still hold its boot duty or a
        // failsafe from before, and nothing this daemon said has been ignored.
        let mut stall = duty_scaled(1);
        assert!(check_controller(None, &status(100, 1980, true), Some(&mut stall)).is_empty());
    }
}
