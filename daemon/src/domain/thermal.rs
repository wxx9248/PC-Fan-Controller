//! Drive temperatures as the control loop sees them: readings, the drive's
//! own thresholds, and the reduction of many drives to one number.

use std::collections::HashMap;

use anyhow::{Result, bail};

use crate::constants::{FALLBACK_FLOOR_CELSIUS, PLAUSIBLE_CEILING_CELSIUS};

/// One disk's temperature in °C, or why it could not be read.
pub struct DiskReading {
    pub name: String,
    pub celsius: Result<f64>,
    /// What the drive says about its own limits. Best effort: a drive that
    /// publishes none leaves these `None` and the fallbacks apply.
    pub limits: Limits,
}

/// The thresholds a `drivetemp` hwmon publishes alongside `temp1_input`.
///
/// Read rather than hardcoded because they are per-drive and authoritative: this
/// array's 20 TB drive reports -40 / 60 / 70 °C, and another drive will say
/// something different.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Limits {
    /// `temp1_lcrit`: below this, a reading is not a cold drive, it is a broken
    /// sensor.
    pub lcrit: Option<f64>,
    /// `temp1_max`: the top of the drive's rated operating range.
    pub max: Option<f64>,
    /// `temp1_crit`: the manufacturer's critical threshold.
    pub crit: Option<f64>,
}

/// How a reading sits against the drive's own thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalBand {
    /// Within the drive's rated range, or it publishes no thresholds.
    Normal,
    /// At or above `temp1_max`, so outside the rated operating range.
    OverMax,
    /// At or above `temp1_crit`, the manufacturer's critical threshold.
    OverCrit,
}

impl Limits {
    /// Classify a reading. Thresholds the drive does not publish never fire.
    pub fn band(&self, celsius: f64) -> ThermalBand {
        if self.crit.is_some_and(|crit| celsius >= crit) {
            ThermalBand::OverCrit
        } else if self.max.is_some_and(|max| celsius >= max) {
            ThermalBand::OverMax
        } else {
            ThermalBand::Normal
        }
    }
}

/// Reject a reading that no working sensor would produce.
///
/// The asymmetry is what makes this worth doing. An implausibly *low* reading
/// forwarded to the curve clamps to the first point, which is the fan's
/// minimum, so a sensor glitch would quietly command *less* cooling, while an
/// implausibly high one only commands full cooling. The kernel's `drivetemp`
/// sign-extends the SCT temperature byte and filters only the single 0x80
/// sentinel, so bytes 0x81..0xFF surface as valid readings of -127..-1 °C.
pub fn check_plausible(celsius: f64, limits: &Limits) -> Result<f64> {
    let floor = limits.lcrit.unwrap_or(FALLBACK_FLOOR_CELSIUS);
    if celsius < floor {
        bail!(
            "temperature {celsius} °C is below the drive's own lower limit of \
             {floor} °C; treating it as a failed sensor rather than a cold drive"
        );
    }
    if celsius > PLAUSIBLE_CEILING_CELSIUS {
        bail!(
            "temperature {celsius} °C exceeds {PLAUSIBLE_CEILING_CELSIUS} °C, \
             which no working drive sensor reports"
        );
    }
    Ok(celsius)
}

/// Aggregate readings into a control decision.
pub enum TempSummary {
    /// Hottest drive that could be read.
    Hottest(f64),
    /// No drive could be read at all, so the caller must fail safe.
    Unknown,
}

/// Reduce per-disk readings to the value that drives the curve.
///
/// A disk that cannot be read makes the result `Unknown`, even when others read
/// fine. The curve is meant to follow the hottest monitored drive, and an
/// unreadable drive could be that drive, so a reading that merely happens to be
/// available is not a safe substitute. The cost is that one flaky disk commands
/// full cooling; the alternative is quietly cooling for the drives that answered.
/// The failures themselves are the caller's to report.
pub fn summarize(readings: &[DiskReading]) -> TempSummary {
    let mut hottest: Option<f64> = None;
    let mut unreadable = false;

    for disk in readings {
        match &disk.celsius {
            Ok(celsius) => {
                hottest = Some(hottest.map_or(*celsius, |h: f64| h.max(*celsius)));
            }
            Err(_) => unreadable = true,
        }
    }

    match hottest {
        Some(celsius) if !unreadable => TempSummary::Hottest(celsius),
        _ => TempSummary::Unknown,
    }
}

/// Reports a drive crossing the thresholds it publishes about itself.
///
/// The curve tops out at 50 °C, so from there to drive failure the fan is already
/// saturated and the system has nothing left to say. Without this, the hottest
/// temperature is logged only at `debug` while the default filter is `info`, it is
/// absent from `/api/status`, and `temp1_max`/`temp1_crit` are read by nothing at
/// all, so a drive held at 69 °C for a week produces a journal identical to one
/// held at 35 °C. The fan can do no more at that point, but somebody should be
/// told.
#[derive(Default)]
pub struct ThermalWatch {
    last: HashMap<String, ThermalBand>,
}

impl ThermalWatch {
    /// Feed one disk's band. Returns the band when it is worth saying something:
    /// on a meaningful change, and on *every* cycle while critical, because an
    /// emergency should not fall silent just because it has been going on a while.
    ///
    /// A drive that is simply healthy the first time it is seen says nothing.
    /// Treating that initial `Normal` as a transition makes the reporting code
    /// phrase it as a recovery, so a perfectly cold drive announces at startup
    /// that it is "back within its rated range", having never left it.
    pub fn observe(&mut self, disk: &str, band: ThermalBand) -> Option<ThermalBand> {
        let previous = self.last.insert(disk.to_string(), band);
        if band == ThermalBand::OverCrit {
            return Some(band);
        }
        match previous {
            // Nothing to report about a drive that starts out healthy.
            None if band == ThermalBand::Normal => None,
            Some(before) if before == band => None,
            _ => Some(band),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn ok(name: &str, celsius: f64) -> DiskReading {
        DiskReading {
            name: name.into(),
            celsius: Ok(celsius),
            limits: Limits::default(),
        }
    }

    fn err(name: &str) -> DiskReading {
        DiskReading {
            name: name.into(),
            celsius: Err(anyhow!("unreadable")),
            limits: Limits::default(),
        }
    }

    /// What the deployed 20 TB drive publishes, verified on the live array.
    fn real_limits() -> Limits {
        Limits {
            lcrit: Some(-40.0),
            max: Some(60.0),
            crit: Some(70.0),
        }
    }

    #[test]
    fn an_implausibly_low_reading_is_rejected_rather_than_cooling_less() {
        // Forwarded to the curve, -50 C would clamp to the first point and
        // command the fan's *minimum* while the drive may be cooking. The kernel
        // can produce such a value from a sign-extended SCT byte.
        for bogus in [-50.0, -127.0, -41.0] {
            assert!(
                check_plausible(bogus, &real_limits()).is_err(),
                "{bogus} C is below the drive's -40 C lower limit and must be rejected"
            );
        }
    }

    #[test]
    fn the_fallback_floor_applies_when_the_drive_publishes_no_lcrit() {
        let none = Limits::default();
        assert!(check_plausible(-41.0, &none).is_err());
        assert!(
            check_plausible(-40.0, &none).is_ok(),
            "the floor itself is valid"
        );
    }

    #[test]
    fn plausible_but_cold_readings_are_still_accepted() {
        // A genuinely cold drive must not be mistaken for a broken sensor: at
        // these temperatures the first curve point really is the right answer.
        for cold in [-40.0, -5.0, 0.0, 12.0] {
            assert!(
                check_plausible(cold, &real_limits()).is_ok(),
                "{cold} C is plausible"
            );
        }
    }

    #[test]
    fn an_absurd_high_reading_is_rejected_but_a_merely_critical_one_is_not() {
        // Above crit is an emergency to act on, not an implausibility to discard.
        // Rejecting it would throw away the one reading that matters most.
        let l = real_limits();
        assert!(
            check_plausible(75.0, &l).is_ok(),
            "above crit must pass through"
        );
        assert!(check_plausible(149.0, &l).is_ok());
        assert!(
            check_plausible(1000.0, &l).is_err(),
            "no sensor reports 1000 C"
        );
        assert!(check_plausible(1e21, &l).is_err());
    }

    #[test]
    fn bands_follow_the_drives_own_thresholds() {
        let l = real_limits(); // -40 / 60 / 70
        assert_eq!(l.band(35.0), ThermalBand::Normal);
        assert_eq!(l.band(59.9), ThermalBand::Normal);
        assert_eq!(l.band(60.0), ThermalBand::OverMax, "at max is already over");
        assert_eq!(l.band(69.9), ThermalBand::OverMax);
        assert_eq!(
            l.band(70.0),
            ThermalBand::OverCrit,
            "at crit is already critical"
        );
        assert_eq!(l.band(85.0), ThermalBand::OverCrit);
    }

    #[test]
    fn a_drive_publishing_no_thresholds_never_alarms() {
        let none = Limits::default();
        for c in [35.0, 60.0, 70.0, 149.0] {
            assert_eq!(none.band(c), ThermalBand::Normal, "{c} C");
        }
    }

    #[test]
    fn a_partial_threshold_set_only_fires_what_it_publishes() {
        let crit_only = Limits {
            lcrit: None,
            max: None,
            crit: Some(70.0),
        };
        assert_eq!(
            crit_only.band(65.0),
            ThermalBand::Normal,
            "no max to exceed"
        );
        assert_eq!(crit_only.band(70.0), ThermalBand::OverCrit);
    }

    #[test]
    fn hottest_readable_drive_wins() {
        let readings = vec![ok("cool", 30.0), ok("hot", 44.5), ok("mid", 38.0)];
        match summarize(&readings) {
            TempSummary::Hottest(c) => assert_eq!(c, 44.5),
            TempSummary::Unknown => panic!("expected a temperature"),
        }
    }

    #[test]
    fn no_readable_drive_fails_safe() {
        let readings = vec![err("a"), err("b")];
        assert!(
            matches!(summarize(&readings), TempSummary::Unknown),
            "a total read failure must fail safe"
        );
    }

    #[test]
    fn an_empty_reading_set_fails_safe() {
        assert!(matches!(summarize(&[]), TempSummary::Unknown));
    }

    #[test]
    fn an_unreadable_drive_is_not_masked_by_a_readable_one() {
        // The unreadable drive could be the hottest, so a reading that merely
        // happens to be available must not stand in for it.
        let readings = vec![ok("readable", 33.0), err("unreadable")];
        assert!(
            matches!(summarize(&readings), TempSummary::Unknown),
            "an unreadable disk must fail safe even when another reads fine"
        );
    }

    #[test]
    fn order_does_not_matter_when_one_disk_is_unreadable() {
        for readings in [
            vec![err("broken"), ok("fine", 30.0)],
            vec![ok("fine", 30.0), err("broken")],
        ] {
            assert!(matches!(summarize(&readings), TempSummary::Unknown));
        }
    }

    #[test]
    fn thermal_watch_reports_transitions_and_keeps_shouting_while_critical() {
        let mut w = ThermalWatch::default();
        assert_eq!(
            w.observe("sdc", ThermalBand::Normal),
            None,
            "a drive that starts out healthy has nothing to announce"
        );
        assert_eq!(
            w.observe("sdc", ThermalBand::Normal),
            None,
            "steady state is quiet"
        );
        assert_eq!(
            w.observe("sdc", ThermalBand::OverMax),
            Some(ThermalBand::OverMax)
        );
        assert_eq!(
            w.observe("sdc", ThermalBand::OverMax),
            None,
            "still quiet once said"
        );
        assert_eq!(
            w.observe("sdc", ThermalBand::OverCrit),
            Some(ThermalBand::OverCrit)
        );
        for _ in 0..5 {
            assert_eq!(
                w.observe("sdc", ThermalBand::OverCrit),
                Some(ThermalBand::OverCrit),
                "a critical drive must not go quiet"
            );
        }
        assert_eq!(
            w.observe("sdc", ThermalBand::Normal),
            Some(ThermalBand::Normal),
            "recovery is worth reporting too"
        );
    }

    #[test]
    fn a_drive_that_starts_out_hot_is_reported_immediately() {
        // The suppression above must only apply to a healthy first sighting: a
        // daemon starting up against an already-hot drive must still say so.
        let mut w = ThermalWatch::default();
        assert_eq!(
            w.observe("hot", ThermalBand::OverMax),
            Some(ThermalBand::OverMax)
        );
        let mut w = ThermalWatch::default();
        assert_eq!(
            w.observe("cooking", ThermalBand::OverCrit),
            Some(ThermalBand::OverCrit)
        );
    }

    #[test]
    fn thermal_watch_tracks_disks_independently() {
        let mut w = ThermalWatch::default();
        w.observe("a", ThermalBand::Normal);
        w.observe("b", ThermalBand::Normal);
        assert_eq!(
            w.observe("a", ThermalBand::OverMax),
            Some(ThermalBand::OverMax)
        );
        assert_eq!(
            w.observe("b", ThermalBand::Normal),
            None,
            "b did not change"
        );
    }
}
