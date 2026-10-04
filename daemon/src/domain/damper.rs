use crate::constants::{MAX_DUTY_PERCENT, MIN_DUTY_PERCENT};
use crate::domain::curve;
use crate::domain::thermal::TempSummary;

/// Turns a stream of noisy temperature readings into a stable commanded duty.
///
/// Two things are damped, for one reason each:
///
/// * Input smoothing (EWMA). `drivetemp` reports whole degrees, so a drive
///   hovering on a curve boundary alternates by 1 C on consecutive polls. On the
///   deployed curve's steepest segments (6.7 %/C) that is an audible ~105 RPM
///   step every cycle. The noise is in the measurement, so it is rejected at the
///   measurement, rather than with output hysteresis, which would make the
///   commanded duty depend on which direction the temperature arrived from and
///   is far harder to reason about or test.
/// * Output deadband. Suppresses the residual 1-2 point jitter that would
///   otherwise churn the log. Saturation is exempt: reaching 0% or 100% is
///   never blocked, so maximum cooling is always available immediately.
///
/// Neither applies to the failsafe paths. See [`Damper::force`].
pub struct Damper {
    alpha: f64,
    deadband: u8,
    filtered_c: Option<f64>,
    commanded: Option<u8>,
}

impl Damper {
    pub fn new(alpha: f64, deadband_percent: u8) -> Self {
        Self {
            alpha,
            deadband: deadband_percent,
            filtered_c: None,
            commanded: None,
        }
    }

    /// The filtered temperature currently driving the curve, for logging.
    pub fn filtered_celsius(&self) -> Option<f64> {
        self.filtered_c
    }

    /// Decide the duty for this cycle. Unknown temperatures mean full speed, the
    /// same posture the firmware failsafe takes.
    ///
    /// Only the normal path is damped. An unreadable temperature bypasses the
    /// filter entirely: putting lag in front of the failsafe response would
    /// defeat the one behaviour that must never be delayed.
    pub fn decide(&mut self, curve: &[[i64; 2]], summary: &TempSummary) -> u8 {
        match summary {
            TempSummary::Hottest(celsius) => self.duty_for(curve, *celsius),
            TempSummary::Unknown => {
                self.force(MAX_DUTY_PERCENT);
                MAX_DUTY_PERCENT
            }
        }
    }

    /// Feed a real temperature reading and get the duty to command.
    pub fn duty_for(&mut self, curve: &[[i64; 2]], celsius: f64) -> u8 {
        // Seed from the first sample instead of from zero. A zero-seeded average
        // would read cold for the first minute after every restart and command
        // too little cooling, which is a filter-init bug that fails in the
        // unsafe direction.
        let filtered = match self.filtered_c {
            None => celsius,
            Some(previous) => self.alpha * celsius + (1.0 - self.alpha) * previous,
        };
        self.filtered_c = Some(filtered);

        let target = curve::evaluate(curve, filtered);
        let duty = match self.commanded {
            // Saturation always wins over the deadband.
            _ if target == MIN_DUTY_PERCENT || target == MAX_DUTY_PERCENT => target,
            Some(last) if target.abs_diff(last) < self.deadband => last,
            _ => target,
        };
        self.commanded = Some(duty);
        duty
    }

    /// Forget all history, for use when the link to the controller has dropped.
    ///
    /// Both halves matter. The filter must not blend a pre-outage temperature
    /// across a gap of arbitrary length: after a long outage the first real
    /// reading should be taken at face value, not averaged with whatever the
    /// drives were doing before. The deadband must also not stay anchored to a
    /// duty the controller may no longer be applying.
    pub fn reset(&mut self) {
        self.filtered_c = None;
        self.commanded = None;
    }

    /// Record a duty that was chosen without consulting the curve (currently
    /// only the no-temperature-readable failsafe).
    ///
    /// The filter state is dropped so the next real reading reseeds from itself
    /// rather than blending across the gap, and the deadband is anchored to what
    /// was actually commanded.
    pub fn force(&mut self, duty: u8) {
        self.filtered_c = None;
        self.commanded = Some(duty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Matches the deployed curve: floor at 31% up to 38 C, full speed by 50 C.
    const CURVE: &[[i64; 2]] = &[[38, 31], [41, 45], [44, 60], [47, 80], [50, 100]];

    fn damper() -> Damper {
        Damper::new(0.3, 2)
    }

    #[test]
    fn seeds_from_first_sample_not_from_zero() {
        let mut d = damper();
        // A zero-seeded EWMA would see ~13 C here and command the floor.
        assert_eq!(d.duty_for(CURVE, 45.0), curve::evaluate(CURVE, 45.0));
        assert_eq!(d.filtered_celsius(), Some(45.0));
    }

    #[test]
    fn rejects_one_degree_dither() {
        let mut d = damper();
        d.duty_for(CURVE, 42.0);
        let mut seen = Vec::new();
        // Temperature alternates 42/43 forever; duty must settle, not oscillate.
        for i in 0..20 {
            seen.push(d.duty_for(CURVE, if i % 2 == 0 { 43.0 } else { 42.0 }));
        }
        let tail = &seen[10..];
        assert!(
            tail.windows(2).all(|w| w[0] == w[1]),
            "duty still oscillating: {seen:?}"
        );
    }

    #[test]
    fn follows_a_sustained_rise() {
        let mut d = damper();
        d.duty_for(CURVE, 38.0);
        let mut duty = 0;
        for _ in 0..40 {
            duty = d.duty_for(CURVE, 47.0);
        }
        // A deadband necessarily permits steady-state error inside its own
        // width, so the target is approached to within it rather than hit
        // exactly. One duty point is ~16 RPM out of ~1970 here.
        assert!(
            duty.abs_diff(80) < 2,
            "should converge to within the deadband of the 47 C point, got {duty}%"
        );
    }

    #[test]
    fn deadband_holds_small_changes_but_not_large_ones() {
        let mut d = Damper::new(1.0, 5); // no smoothing, wide deadband
        assert_eq!(d.duty_for(CURVE, 41.0), 45);
        // 41.2 C -> 45.9 -> rounds to 46, one point away: held.
        assert_eq!(d.duty_for(CURVE, 41.2), 45);
        // 44 C -> 60, fifteen points away: honoured.
        assert_eq!(d.duty_for(CURVE, 44.0), 60);
    }

    #[test]
    fn full_speed_is_never_withheld_by_the_deadband() {
        let mut d = Damper::new(1.0, 25); // absurdly wide deadband
        d.duty_for(CURVE, 49.8); // 99%
        assert_eq!(
            d.duty_for(CURVE, 60.0),
            100,
            "saturation must bypass deadband"
        );
    }

    #[test]
    fn force_reseeds_the_filter() {
        let mut d = damper();
        for _ in 0..10 {
            d.duty_for(CURVE, 50.0); // drive the average up hot
        }
        d.force(100);
        assert_eq!(d.filtered_celsius(), None);
        // The next real reading must be taken at face value, not blended with
        // the stale hot average.
        assert_eq!(d.duty_for(CURVE, 38.0), 31);
    }
}
