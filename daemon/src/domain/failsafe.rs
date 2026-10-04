//! The rule that keeps the controller's failsafe from firing while this daemon
//! is in control.

use anyhow::{Result, anyhow, bail};

use crate::constants::{EXCHANGE_TIMEOUT_S, FAILSAFE_HEADROOM_DIVISOR};

/// The rule that keeps the controller's failsafe from tripping in normal
/// operation: the gap between two `PWM` commands must be at most half the
/// failsafe timeout.
///
/// That gap is the poll interval plus the `STATUS` exchange between the two
/// commands, which the serial exchange budget bounds. Budgeting only the sleep
/// would call a configuration safe when the worst case is several seconds
/// longer.
///
/// Used both on the configured timeout at startup and on the one the controller
/// reports about itself, so the two checks cannot disagree about the rule.
pub fn check_poll_interval(
    poll_interval_s: u64,
    failsafe_timeout_s: u64,
    timeout_source: &str,
) -> Result<()> {
    // saturating_add rather than `+`: release builds have overflow-checks off, so
    // an absurd interval would otherwise wrap straight through this check.
    // Comparison is by division for the same reason.
    let worst_case_s = poll_interval_s.saturating_add(EXCHANGE_TIMEOUT_S);
    if worst_case_s > failsafe_timeout_s / FAILSAFE_HEADROOM_DIVISOR {
        bail!(
            "poll_interval_s ({poll_interval_s}) plus the {EXCHANGE_TIMEOUT_S} s serial \
             exchange budget is {worst_case_s} s, which must be at most 1/\
             {FAILSAFE_HEADROOM_DIVISOR} of {timeout_source} ({failsafe_timeout_s} s)"
        );
    }
    Ok(())
}

/// How the controller's own failsafe timeout compares with the configured one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutCheck {
    /// Firmware built before STATUS carried it, so the configured value is
    /// taken on trust.
    Unreported,
    Matches,
    /// Different, but the poll interval still fits the real value.
    DiffersButFits {
        configured_s: u64,
        reported_s: u64,
    },
}

/// Compare the failsafe timeout the controller reports about itself with the
/// configured one that the poll interval was validated against.
///
/// Firmware built before STATUS carried it does not report it (`None`), and
/// is accepted unchecked. When the two disagree, the poll interval is judged
/// against the real value: a configured timeout higher than the firmware's
/// otherwise passes validation and then trips the failsafe between cycles. A
/// poll interval that does not fit is as fatal as an invalid configuration, and
/// the controller's failsafe holds the fan at 100% while it stays that way.
pub fn check_failsafe_timeout(
    poll_interval_s: u64,
    configured_s: u64,
    reported_s: Option<u64>,
) -> Result<TimeoutCheck> {
    let Some(actual) = reported_s else {
        return Ok(TimeoutCheck::Unreported);
    };
    if actual == configured_s {
        return Ok(TimeoutCheck::Matches);
    }
    check_poll_interval(
        poll_interval_s,
        actual,
        "the controller's own failsafe timeout",
    )
    .map_err(|e| {
        anyhow!(
            "{e:#}; controller_failsafe_timeout_s says {configured_s} s, so set it to \
                 {actual} and choose a poll interval that fits"
        )
    })?;
    Ok(TimeoutCheck::DiffersButFits {
        configured_s,
        reported_s: actual,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_check_uses_the_same_rule() {
        // The controller-reported timeout is judged by the same function as the
        // configured one, so both agree on where the boundary is.
        assert!(check_poll_interval(12, 30, "t").is_ok());
        assert!(check_poll_interval(13, 30, "t").is_err());
        assert!(
            check_poll_interval(10, 20, "t").is_err(),
            "13 s against a 10 s half"
        );
    }

    #[test]
    fn a_matching_or_unreported_failsafe_timeout_is_accepted() {
        assert!(check_failsafe_timeout(10, 30, Some(30)).is_ok());
        assert!(
            check_failsafe_timeout(10, 30, None).is_ok(),
            "firmware that predates the field"
        );
    }

    #[test]
    fn a_different_failsafe_timeout_is_fatal_only_when_the_poll_interval_no_longer_fits() {
        // Firmware stricter than configured: 10 s + 3 s no longer fits in half of 20 s.
        let e = check_failsafe_timeout(10, 30, Some(20)).expect_err("13 s against a 10 s half");
        assert!(
            format!("{e:#}").contains("controller_failsafe_timeout_s"),
            "got: {e:#}"
        );
        // Firmware more lenient than configured: still safe, so only a warning.
        assert!(check_failsafe_timeout(10, 30, Some(60)).is_ok());
    }
}
