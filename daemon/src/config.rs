use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::constants::{
    DEFAULT_DEADBAND_PERCENT, DEFAULT_FAILSAFE_TIMEOUT_S, DEFAULT_POLL_INTERVAL_S,
    DEFAULT_SMOOTHING_ALPHA, DEFAULT_STALL_CYCLES, DEFAULT_STALL_TOLERANCE_PERCENT,
    MAX_CURVE_CELSIUS, MAX_DEADBAND_PERCENT, MAX_DUTY_PERCENT, MAX_STALL_TOLERANCE_PERCENT,
    MIN_CURVE_CELSIUS, MIN_DUTY_PERCENT, MIN_STALL_TOLERANCE_PERCENT,
};
use crate::domain::failsafe::check_poll_interval;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Serial device path. Use the immutable /dev/serial/by-id/ path so the
    /// controller is found regardless of USB port or other ttyACM devices.
    pub device: PathBuf,

    #[serde(default = "default_poll_interval_s")]
    pub poll_interval_s: u64,

    /// Must match CONFIG_FAN_FAILSAFE_TIMEOUT_S in the firmware; used only to
    /// validate that the poll interval keeps the failsafe from triggering.
    #[serde(default = "default_failsafe_timeout_s")]
    pub controller_failsafe_timeout_s: u64,

    /// Disks to monitor: names under /dev/disk/by-id/ (immutable, based on
    /// model+serial or WWN), or absolute paths.
    pub disks: Vec<String>,

    /// Fan curve as [temperature_celsius, pwm_percent] points, strictly
    /// ascending in temperature. Linear interpolation in between; clamped
    /// to the first/last point outside the range.
    pub curve: Vec<[i64; 2]>,

    /// The highest duty that still produces the fan's minimum speed, which is
    /// the top of its dead zone. Purely diagnostic: curve points at or below it
    /// are physically indistinguishable, so they are reported at startup. It
    /// never rescales the curve: what you write in `curve` is what gets
    /// commanded.
    #[serde(default)]
    pub fan_floor_duty_percent: Option<u8>,

    /// Weight given to each new temperature sample in the exponential moving
    /// average (0 < alpha <= 1; 1.0 disables smoothing).
    ///
    /// `drivetemp` reports whole degrees, so a drive sitting on a curve
    /// boundary dithers by 1 C every poll. Smoothing the input rejects that at
    /// its source. The thermal time constant here is hours, so the lag this
    /// adds is irrelevant.
    #[serde(default = "default_smoothing_alpha")]
    pub smoothing_alpha: f64,

    /// Suppress commanded duty changes smaller than this many percentage
    /// points. Saturation at 0% and 100% is always honoured regardless.
    #[serde(default = "default_deadband_percent")]
    pub deadband_percent: u8,

    /// Below this tachometer reading the fan is treated as not turning, and a
    /// stall is reported after `stall_cycles` consecutive cycles. Set it well
    /// under the fan's measured minimum speed so normal operation can never
    /// trip it. `None` disables the check.
    #[serde(default)]
    pub min_expected_rpm: Option<u32>,

    /// Consecutive low-RPM cycles required before reporting a stall, so one
    /// dropped tachometer sample cannot raise a false alarm.
    #[serde(default = "default_stall_cycles")]
    pub stall_cycles: u32,

    /// The fan's measured response as `[pwm_percent, rpm]` points, ascending in
    /// duty. Interpolated linearly, clamped outside the range.
    ///
    /// Without this, stall detection has only `min_expected_rpm`, a single
    /// absolute floor that cannot catch the most important fault. A fan whose PWM
    /// line is pinned low free-runs at its minimum speed, and that speed is
    /// *above* any sane absolute floor, so the fan reads healthy at every
    /// commanded duty while airflow is a fraction of what it should be. With this
    /// table the threshold scales with what was actually commanded.
    ///
    /// Measure it the same way as the curve: sweep duty, record RPM.
    #[serde(default)]
    pub expected_rpm: Option<Vec<[i64; 2]>>,

    /// Fraction of the expected RPM the fan must reach, as a percentage. Covers
    /// fan-to-fan spread, bearing wear and tachometer jitter; only meaningful
    /// alongside `expected_rpm`.
    #[serde(default = "default_stall_tolerance_percent")]
    pub stall_tolerance_percent: u8,
}

fn default_poll_interval_s() -> u64 {
    DEFAULT_POLL_INTERVAL_S
}

fn default_failsafe_timeout_s() -> u64 {
    DEFAULT_FAILSAFE_TIMEOUT_S
}

fn default_smoothing_alpha() -> f64 {
    DEFAULT_SMOOTHING_ALPHA
}

fn default_deadband_percent() -> u8 {
    DEFAULT_DEADBAND_PERCENT
}

fn default_stall_cycles() -> u32 {
    DEFAULT_STALL_CYCLES
}

fn default_stall_tolerance_percent() -> u8 {
    DEFAULT_STALL_TOLERANCE_PERCENT
}

/// Each pair of neighbouring points in a table, for the ordering checks.
fn segments(points: &[[i64; 2]]) -> impl Iterator<Item = [[i64; 2]; 2]> + '_ {
    points.windows(2).map(|pair| [pair[0], pair[1]])
}

/// Valid duties, in the type the TOML point arrays deserialize to.
fn duty_range() -> std::ops::RangeInclusive<i64> {
    i64::from(MIN_DUTY_PERCENT)..=i64::from(MAX_DUTY_PERCENT)
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let config: Config =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Curve shape: ranges, ordering, and the transposition trap.
    fn validate_curve(&self) -> Result<()> {
        if self.curve.is_empty() {
            bail!("config: 'curve' must contain at least one [temp, pwm] point");
        }
        for &[celsius, duty] in &self.curve {
            if !duty_range().contains(&duty) {
                bail!(
                    "config: curve PWM value {duty} out of range \
                     {MIN_DUTY_PERCENT}-{MAX_DUTY_PERCENT}"
                );
            }
            if !(MIN_CURVE_CELSIUS..=MAX_CURVE_CELSIUS).contains(&celsius) {
                bail!(
                    "config: curve temperature {celsius} °C is outside the plausible \
                     range {MIN_CURVE_CELSIUS}..={MAX_CURVE_CELSIUS}. Are the columns \
                     swapped? points are [temperature_celsius, pwm_percent]"
                );
            }
        }
        for [[celsius0, _], [celsius1, _]] in segments(&self.curve) {
            if celsius1 <= celsius0 {
                bail!("config: curve temperatures must be strictly ascending");
            }
        }
        // Duty must never fall as temperature rises. Without this check the two
        // columns are interchangeable to the validator, so a transposed
        // [pwm, temp] curve passes and the fan *slows down* as drives heat up.
        // Unlike an ineffective curve this is actively unsafe, and it is a typo
        // rather than a choice, so refuse to start rather than warn.
        for [[celsius0, duty0], [celsius1, duty1]] in segments(&self.curve) {
            if duty1 < duty0 {
                bail!(
                    "config: curve duty must not decrease as temperature rises \
                     ({duty0}% at {celsius0} °C then {duty1}% at {celsius1} °C). Are \
                     the columns swapped? points are [temperature_celsius, pwm_percent]"
                );
            }
        }
        // Warn rather than bail: capping the top of the curve may be a
        // deliberate noise trade-off, but it is worth saying out loud.
        let [top_celsius, top_duty] = self.curve[self.curve.len() - 1];
        if top_duty < i64::from(MAX_DUTY_PERCENT) {
            log::warn!(
                "curve tops out at {top_duty}% duty ({top_celsius} °C): maximum cooling \
                 is never commanded however hot the drives get"
            );
        }
        Ok(())
    }

    /// The optional measured fan response the stall monitor scales against.
    fn validate_expected_rpm(&self) -> Result<()> {
        if let Some(expected) = &self.expected_rpm {
            if expected.is_empty() {
                bail!("config: 'expected_rpm' must contain at least one [pwm, rpm] point");
            }
            for &[duty, rpm] in expected {
                if !duty_range().contains(&duty) {
                    bail!(
                        "config: expected_rpm duty {duty} is out of range \
                         {MIN_DUTY_PERCENT}-{MAX_DUTY_PERCENT}. Points are \
                         [pwm_percent, rpm], not [rpm, pwm_percent]"
                    );
                }
                if rpm < 0 {
                    bail!("config: expected_rpm value {rpm} is negative");
                }
            }
            for [[duty0, rpm0], [duty1, rpm1]] in segments(expected) {
                if duty1 <= duty0 {
                    bail!("config: expected_rpm duties must be strictly ascending");
                }
                if rpm1 < rpm0 {
                    bail!(
                        "config: expected_rpm must not fall as duty rises ({rpm0} RPM at \
                         {duty0}% then {rpm1} RPM at {duty1}%)"
                    );
                }
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.disks.is_empty() {
            bail!("config: 'disks' must list at least one disk");
        }
        self.validate_curve()?;
        if self.poll_interval_s == 0 {
            bail!("config: poll_interval_s must be > 0");
        }
        if !(self.smoothing_alpha > 0.0 && self.smoothing_alpha <= 1.0) {
            bail!(
                "config: smoothing_alpha must be in (0.0, 1.0], got {}",
                self.smoothing_alpha
            );
        }
        if self.stall_cycles == 0 {
            bail!("config: stall_cycles must be > 0");
        }
        // Zero would compare every reading against nothing, which without an
        // expected_rpm table switches stall detection off while the startup
        // warning about it being off stays silent.
        if self.min_expected_rpm == Some(0) {
            bail!(
                "config: min_expected_rpm must be at least 1; leave it out to disable \
                 stall detection"
            );
        }
        if let Some(floor) = self.fan_floor_duty_percent {
            if floor > MAX_DUTY_PERCENT {
                bail!(
                    "config: fan_floor_duty_percent {floor} is out of range \
                     {MIN_DUTY_PERCENT}-{MAX_DUTY_PERCENT}"
                );
            }
        }
        if self.deadband_percent > MAX_DEADBAND_PERCENT {
            bail!(
                "config: deadband_percent {} is implausibly wide (max {MAX_DEADBAND_PERCENT})",
                self.deadband_percent
            );
        }
        // A curve point inside the fan's dead zone commands a duty the fan
        // cannot distinguish from its minimum, so that segment silently has no
        // effect. Warn rather than bail: a merely ineffective curve still cools
        // the drives, whereas refusing to start leaves the firmware failsafe
        // holding the fan at 100% indefinitely, which is much worse.
        if let Some(floor) = self.fan_floor_duty_percent {
            let dead: Vec<i64> = self
                .curve
                .iter()
                .map(|&[_, duty]| duty)
                .filter(|&duty| duty <= i64::from(floor))
                .collect();
            if dead.len() == self.curve.len() {
                log::warn!(
                    "every curve point ({:?}%) is at or below the fan's dead-zone \
                     ceiling of {}%, so the fan runs at its minimum speed no \
                     matter how hot the drives get",
                    dead,
                    floor
                );
            } else if dead.len() > 1 {
                // One point inside the dead zone is the intended idle anchor;
                // two or more means those segments are flat and wasted.
                log::info!(
                    "curve points {:?}% are all inside the fan's dead zone (ceiling \
                     {}%) and produce the same minimum speed; only the highest of \
                     them has any effect",
                    dead,
                    floor
                );
            }
        }
        self.validate_expected_rpm()?;
        if !(MIN_STALL_TOLERANCE_PERCENT..=MAX_STALL_TOLERANCE_PERCENT)
            .contains(&self.stall_tolerance_percent)
        {
            bail!(
                "config: stall_tolerance_percent must be in \
                 {MIN_STALL_TOLERANCE_PERCENT}-{MAX_STALL_TOLERANCE_PERCENT}, got {}",
                self.stall_tolerance_percent
            );
        }
        check_poll_interval(
            self.poll_interval_s,
            self.controller_failsafe_timeout_s,
            "controller_failsafe_timeout_s",
        )
        .map_err(|e| anyhow!("config: {e:#}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::EXCHANGE_TIMEOUT_S;
    use crate::test_support::{render_all_targets, rendered_target_configs};

    /// A valid configuration to vary one field at a time. Shaped like the
    /// shipped one, but it is a fixture: tests about what ships read
    /// the rendered targets instead.
    fn base() -> Config {
        Config {
            device: PathBuf::from("/dev/null"),
            poll_interval_s: 10,
            controller_failsafe_timeout_s: 30,
            disks: vec!["ata-X".into()],
            curve: vec![[38, 31], [41, 45], [44, 60], [47, 80], [50, 100]],
            fan_floor_duty_percent: Some(31),
            smoothing_alpha: 0.3,
            deadband_percent: 2,
            min_expected_rpm: Some(400),
            stall_cycles: 3,
            expected_rpm: Some(vec![[0, 840], [31, 840], [32, 900], [100, 1973]]),
            stall_tolerance_percent: 70,
        }
    }

    #[test]
    fn the_test_fixture_is_valid() {
        base()
            .validate()
            .expect("the fixture every other test varies must validate");
    }

    #[test]
    fn every_installation_target_renders_a_valid_config() {
        // Each target ships a real config file, so each one has to parse and pass
        // the same validator the daemon runs at startup. Adding a key to the
        // struct and forgetting the template fails here, and so does a target
        // whose overrides are out of range.
        for (target, text) in render_all_targets() {
            assert!(
                !text.contains('@'),
                "target {target} left a placeholder in the rendered config"
            );
        }
        for (target, config) in rendered_target_configs() {
            config
                .validate()
                .unwrap_or_else(|e| panic!("target {target} produced an invalid config: {e:#}"));
            assert!(
                config.expected_rpm.is_some(),
                "target {target} should set expected_rpm, or the stall check cannot \
                 see a fan stuck at its minimum"
            );
        }
    }

    #[test]
    fn a_transposed_curve_is_rejected() {
        // [pwm, temp] instead of [temp, pwm]: temperatures still ascend and every
        // duty is in range, so without the plausibility bound this passes
        // validation and then slows the fan down as the drives heat up.
        let mut c = base();
        c.curve = vec![[31, 38], [45, 41], [60, 44], [80, 47], [100, 50]];
        let e = c
            .validate()
            .expect_err("a transposed curve must be rejected");
        assert!(
            format!("{e:#}").contains("columns"),
            "the error should point at the likely cause, got: {e:#}"
        );
    }

    #[test]
    fn an_implausible_curve_temperature_is_rejected() {
        for bad_temp in [91, 200, -41] {
            let mut c = base();
            c.curve = vec![[bad_temp, 100]];
            assert!(c.validate().is_err(), "{bad_temp} °C should be rejected");
        }
    }

    #[test]
    fn plausible_extremes_are_still_allowed() {
        let mut c = base();
        c.curve = vec![[-40, 31], [90, 100]];
        c.validate().expect("the bounds themselves must be valid");
    }

    #[test]
    fn a_curve_whose_duty_dips_is_rejected() {
        let mut c = base();
        c.curve = vec![[38, 31], [44, 100], [50, 80]];
        assert!(
            c.validate().is_err(),
            "duty must not fall as temperature rises"
        );
    }

    #[test]
    fn a_flat_curve_segment_is_allowed() {
        // Equal duty across a segment is legitimate (a plateau), only a dip is not.
        let mut c = base();
        c.curve = vec![[38, 31], [44, 31], [50, 100]];
        c.validate().expect("a plateau is valid");
    }

    #[test]
    fn the_poll_interval_rule_budgets_for_the_in_cycle_serial_work() {
        // Comparing the poll interval alone against half the failsafe timeout is
        // not enough: the gap between two PWM commands also contains a STATUS
        // exchange bounded by the serial timeout, so such a rule calls a
        // configuration safe when it really sits several seconds closer to the
        // failsafe than advertised.
        //
        // With a 3 s exchange budget and a 30 s failsafe the limit is 12 s, not
        // 15 s, and these cases pin it.
        let budget = EXCHANGE_TIMEOUT_S;
        assert_eq!(budget, 3, "the cases below assume a 3 s exchange budget");
        for (poll, failsafe, ok) in [
            (12, 30, true),
            (13, 30, false),
            (12, 31, true),
            (13, 31, false),
            (1, 30, true),
        ] {
            let mut c = base();
            c.poll_interval_s = poll;
            c.controller_failsafe_timeout_s = failsafe;
            assert_eq!(
                c.validate().is_ok(),
                ok,
                "poll={poll} failsafe={failsafe} should be ok={ok}"
            );
        }
    }

    #[test]
    fn the_shipped_poll_interval_fits_with_margin() {
        // The stricter rule must not lock out what ships: 10 s poll plus a 3 s
        // exchange is 13 s against a 15 s limit.
        for (target, c) in rendered_target_configs() {
            check_poll_interval(
                c.poll_interval_s,
                c.controller_failsafe_timeout_s,
                "the timeout",
            )
            .unwrap_or_else(|e| panic!("target {target}: {e:#}"));
        }
    }

    #[test]
    fn a_transposed_expected_rpm_table_is_rejected() {
        // [rpm, duty] instead of [duty, rpm]: the RPM values land in the duty
        // column, where anything above 100 is impossible.
        let mut c = base();
        c.expected_rpm = Some(vec![[840, 0], [900, 32], [1973, 100]]);
        let e = c
            .validate()
            .expect_err("a transposed table must be rejected");
        assert!(
            format!("{e:#}").contains("[pwm_percent, rpm]"),
            "the error should name the expected order, got: {e:#}"
        );
    }

    #[test]
    fn expected_rpm_must_be_ascending_in_duty_and_not_fall_in_rpm() {
        let mut c = base();
        c.expected_rpm = Some(vec![[32, 900], [31, 840]]);
        assert!(c.validate().is_err(), "duties must strictly ascend");

        let mut c = base();
        c.expected_rpm = Some(vec![[0, 900], [100, 840]]);
        assert!(c.validate().is_err(), "RPM must not fall as duty rises");

        let mut c = base();
        c.expected_rpm = Some(vec![[0, 840], [50, 840], [100, 1973]]);
        c.validate()
            .expect("a flat segment is legitimate below the knee");
    }

    #[test]
    fn an_empty_or_negative_expected_rpm_table_is_rejected() {
        let mut c = base();
        c.expected_rpm = Some(vec![]);
        assert!(c.validate().is_err());
        let mut c = base();
        c.expected_rpm = Some(vec![[0, -1]]);
        assert!(c.validate().is_err());
    }

    #[test]
    fn an_absent_expected_rpm_table_is_allowed() {
        // Opt-in: the absolute floor still works on its own, and the daemon warns
        // at startup that it cannot see a fan stuck at its minimum.
        let mut c = base();
        c.expected_rpm = None;
        c.validate().expect("expected_rpm is optional");
    }

    #[test]
    fn stall_tolerance_range_is_enforced() {
        for (tolerance, ok) in [(0, false), (1, true), (70, true), (100, true), (101, false)] {
            let mut c = base();
            c.stall_tolerance_percent = tolerance;
            assert_eq!(c.validate().is_ok(), ok, "tolerance={tolerance}");
        }
    }

    #[test]
    fn an_overflowing_poll_interval_is_rejected() {
        // Only reachable by constructing a Config directly, since TOML integers
        // stop at i64::MAX, but release builds do not trap overflow: without the
        // saturating add, u64::MAX plus the 3 s exchange budget wraps to 2 and
        // passes the check.
        let mut c = base();
        c.poll_interval_s = u64::MAX;
        assert!(
            c.validate().is_err(),
            "a wrapping poll interval must not slip through the failsafe check"
        );
    }

    #[test]
    fn a_zero_min_expected_rpm_is_rejected_rather_than_silently_disabling() {
        let mut c = base();
        c.min_expected_rpm = Some(0);
        c.expected_rpm = None;
        let e = c.validate().expect_err("zero must be refused");
        assert!(format!("{e:#}").contains("leave it out"), "got: {e:#}");
    }

    #[test]
    fn an_out_of_range_fan_floor_is_rejected() {
        let mut c = base();
        c.fan_floor_duty_percent = Some(101);
        assert!(c.validate().is_err());
        c.fan_floor_duty_percent = Some(100);
        c.validate().expect("100 is in range");
    }

    #[test]
    fn deadband_ceiling_is_enforced() {
        let mut c = base();
        c.deadband_percent = 25;
        assert!(c.validate().is_err(), "25 points is too wide");
        c.deadband_percent = 10;
        c.validate().expect("10 is the ceiling and must be allowed");
    }

    #[test]
    fn smoothing_alpha_range_is_enforced() {
        for (alpha, ok) in [(0.0, false), (0.3, true), (1.0, true), (1.5, false)] {
            let mut c = base();
            c.smoothing_alpha = alpha;
            assert_eq!(c.validate().is_ok(), ok, "alpha={alpha}");
        }
    }

    #[test]
    fn empty_disks_and_curve_are_rejected() {
        let mut c = base();
        c.disks.clear();
        assert!(c.validate().is_err());
        let mut c = base();
        c.curve.clear();
        assert!(c.validate().is_err());
    }

    #[test]
    fn out_of_range_duty_is_rejected() {
        let mut c = base();
        c.curve = vec![[38, 31], [50, 150]];
        assert!(c.validate().is_err());
    }

    #[test]
    fn stall_cycles_must_be_positive() {
        let mut c = base();
        c.stall_cycles = 0;
        assert!(c.validate().is_err());
    }
}
