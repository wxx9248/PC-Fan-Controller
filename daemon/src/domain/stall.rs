use crate::constants::{MIN_DUTY_PERCENT, PERCENT_PER_WHOLE};
use crate::domain::curve;

/// Watches the tachometer for a fan that has stopped turning.
///
/// The firmware failsafe detects a silent *host*, and nothing else detects a dead
/// *fan*. A seized bearing, a loose 12 V lead or a shorted drive transistor all
/// leave every other safeguard satisfied, with temperatures readable, the host
/// talking and the duty commanded correctly, while airflow is zero.
///
/// Deliberately reports rather than escalates. If the fan is dead, commanding
/// more duty achieves nothing; if instead the *tachometer* has failed while the
/// fan still spins, escalating would run it flat out for no reason. Either way
/// the actionable output is a loud, specific alarm.
pub struct StallMonitor {
    min_rpm: u32,
    /// Measured `[duty, rpm]` response, if configured. Lets the threshold scale
    /// with what was actually commanded.
    expected: Option<Vec<[i64; 2]>>,
    tolerance: f64,
    required_cycles: u32,
    consecutive: u32,
}

/// A stall the monitor is prepared to report, and the threshold it tripped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stall {
    pub cycles: u32,
    pub threshold_rpm: u32,
}

impl StallMonitor {
    pub fn new(
        min_rpm: u32,
        required_cycles: u32,
        expected: Option<Vec<[i64; 2]>>,
        tolerance_percent: u8,
    ) -> Self {
        Self {
            min_rpm,
            expected,
            tolerance: f64::from(tolerance_percent) / PERCENT_PER_WHOLE,
            required_cycles,
            consecutive: 0,
        }
    }

    /// The lowest RPM this commanded duty should ever produce.
    ///
    /// A single absolute floor cannot catch the fault that matters most. This fan
    /// turns 840 RPM with its PWM line held low, which is *above* any sane
    /// absolute floor. A shorted drive transistor, or any fault pinning PWM low,
    /// leaves the fan at 840 RPM while the daemon commands 100% and expects
    /// ~1970: every safeguard reads healthy and airflow is 42% of what it should
    /// be. Scaling the threshold with the commanded duty is what catches it.
    ///
    /// Never looser than the absolute floor, so configuring an expected-response
    /// table can only tighten the check, never weaken it.
    fn threshold_for(&self, duty: u8) -> u32 {
        let Some(points) = &self.expected else {
            return self.min_rpm;
        };
        let expected = curve::interpolate(points, f64::from(duty));
        let scaled = (expected * self.tolerance)
            .round()
            .clamp(0.0, f64::from(u32::MAX));
        self.min_rpm.max(scaled as u32)
    }

    /// Feed one observation. Returns `Some(_)` while the fan is considered
    /// stalled, `None` while healthy.
    ///
    /// `duty` must be the duty the fan was running at while `rpm` was measured,
    /// which is what the controller reports alongside it, not one just
    /// commanded: the tachometer reports its last completed window, so a reading
    /// taken moments after a change still shows the old speed.
    ///
    /// A duty of 0 is exempt: a fan told to stop is allowed to read zero. (This
    /// particular fan floors at ~840 RPM even at 0, but relying on that would
    /// make the check wrong for any other fan.)
    pub fn observe(&mut self, duty: u8, rpm: u32) -> Option<Stall> {
        if duty == MIN_DUTY_PERCENT {
            self.consecutive = 0;
            return None;
        }
        let threshold = self.threshold_for(duty);
        if rpm < threshold {
            self.consecutive += 1;
        } else {
            self.consecutive = 0;
        }
        (self.consecutive >= self.required_cycles).then_some(Stall {
            cycles: self.consecutive,
            threshold_rpm: threshold,
        })
    }

    /// Forget the run of bad cycles.
    ///
    /// Called when the link drops, so "3 consecutive cycles" describes three
    /// cycles that really were consecutive rather than a count spanning an
    /// outage of arbitrary length.
    pub fn reset(&mut self) {
        self.consecutive = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An expected-response table in the shape the shipped config uses: the
    /// regression of the 2026-09-29 sweep, which sits slightly below the
    /// measurements. A fixture; the test against what really ships reads the
    /// rendered config.
    fn example_response() -> Vec<[i64; 2]> {
        vec![[0, 840], [31, 840], [32, 900], [50, 1185], [100, 1973]]
    }

    /// The raw 2026-09-29 hardware sweep as `(duty, rpm)`, shared with the
    /// frontend mock's tests.
    fn measured_sweep() -> Vec<(u8, u32)> {
        include_str!("../../tests/fixtures/fan-sweep-2026-09-29.csv")
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .skip(1) // the column header
            .map(|line| {
                let (duty, rpm) = line.split_once(',').expect("rows are duty,rpm");
                (duty.parse().expect("duty"), rpm.parse().expect("rpm"))
            })
            .collect()
    }

    fn absolute_only() -> StallMonitor {
        StallMonitor::new(400, 3, None, 70)
    }

    fn duty_scaled() -> StallMonitor {
        StallMonitor::new(400, 3, Some(example_response()), 70)
    }

    #[test]
    fn stall_needs_consecutive_cycles_before_alarming() {
        let mut m = absolute_only();
        assert_eq!(m.observe(50, 0), None, "one bad cycle is not an alarm");
        assert_eq!(m.observe(50, 0), None);
        assert_eq!(
            m.observe(50, 0).map(|s| s.cycles),
            Some(3),
            "third consecutive cycle alarms"
        );
        assert_eq!(
            m.observe(50, 0).map(|s| s.cycles),
            Some(4),
            "and keeps alarming"
        );
    }

    #[test]
    fn a_healthy_reading_resets_the_stall_counter() {
        let mut m = absolute_only();
        m.observe(50, 0);
        m.observe(50, 0);
        assert_eq!(m.observe(50, 840), None, "fan recovered");
        assert_eq!(m.observe(50, 0), None, "counter restarted from zero");
    }

    #[test]
    fn healthy_fan_never_alarms() {
        let mut m = absolute_only();
        for _ in 0..50 {
            assert_eq!(m.observe(31, 840), None);
        }
    }

    #[test]
    fn zero_duty_is_exempt() {
        let mut m = absolute_only();
        for _ in 0..10 {
            assert_eq!(m.observe(0, 0), None, "a fan told to stop may read zero");
        }
    }

    #[test]
    fn a_fan_stuck_at_its_floor_is_caught_at_high_duty() {
        // The fault the absolute floor cannot see: PWM pinned low, so the fan
        // free-runs at 840 RPM while 100% is commanded and ~1973 expected.
        let mut absolute = absolute_only();
        for _ in 0..10 {
            assert_eq!(
                absolute.observe(100, 840),
                None,
                "an absolute 400 RPM floor cannot see 840 RPM at full duty"
            );
        }
        let mut scaled = duty_scaled();
        scaled.observe(100, 840);
        scaled.observe(100, 840);
        let stall = scaled
            .observe(100, 840)
            .expect("duty-scaled check must catch it");
        assert_eq!(stall.cycles, 3);
        assert_eq!(
            stall.threshold_rpm, 1381,
            "70% of the 1973 RPM expected at 100%"
        );
    }

    #[test]
    fn the_duty_scaled_check_never_loosens_the_absolute_floor() {
        // At the floor duty the expected RPM is 840, and 70% of that is 588 --
        // above min_rpm, so it tightens. With a high min_rpm the floor must win.
        let strict = StallMonitor::new(900, 3, Some(example_response()), 70);
        assert_eq!(strict.threshold_for(31), 900, "the absolute floor must win");
        let loose = StallMonitor::new(100, 3, Some(example_response()), 70);
        assert_eq!(loose.threshold_for(31), 588, "70% of the 840 RPM floor");
    }

    #[test]
    fn a_fan_exactly_on_its_expected_response_never_trips() {
        // Walk the whole table at exactly the RPM it expects; nothing may alarm.
        let mut m = duty_scaled();
        for duty in 1..=100u8 {
            let expected = curve::interpolate(&example_response(), f64::from(duty));
            assert_eq!(
                m.observe(duty, expected.round() as u32),
                None,
                "duty {duty}% at its expected {expected} RPM must be healthy"
            );
        }
    }

    #[test]
    fn the_real_measured_sweep_never_trips_the_shipped_thresholds() {
        // The false-positive guard: the raw hardware measurements,
        // checked against the expected_rpm table each installation target really
        // ships. If a table is ever tightened past the hardware, this fails
        // rather than the fan appearing to stall. One reading at a time, so no
        // consecutive-cycle allowance can hide a single bad point.
        let sweep = measured_sweep();
        assert!(
            sweep.len() >= 10,
            "the sweep fixture looks truncated: {sweep:?}"
        );
        for (target, config) in crate::test_support::rendered_target_configs() {
            let mut m = StallMonitor::new(
                config.min_expected_rpm.unwrap_or(1),
                1,
                config.expected_rpm.clone(),
                config.stall_tolerance_percent,
            );
            for &(duty, rpm) in &sweep {
                assert_eq!(
                    m.observe(duty, rpm),
                    None,
                    "target {target}: measured {rpm} RPM at {duty}% duty must never \
                     alarm (threshold {} RPM)",
                    m.threshold_for(duty)
                );
            }
        }
    }

    #[test]
    fn a_reconnect_clears_the_consecutive_run() {
        let mut m = absolute_only();
        m.observe(50, 0);
        m.observe(50, 0);
        m.reset();
        assert_eq!(
            m.observe(50, 0),
            None,
            "the run must start over after a reset"
        );
        assert_eq!(m.observe(50, 0), None);
        assert!(
            m.observe(50, 0).is_some(),
            "and then alarm on its own three"
        );
    }
}
