//! Drive temperatures from the kernel's `drivetemp` hwmon.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::constants::{
    ACCEPTED_HWMON_NAMES, BLOCK_HWMON_SUBDIR, DISK_BY_ID_DIR, HWMON_NAME_FILE, HWMON_TEMP_CRIT,
    HWMON_TEMP_INPUT, HWMON_TEMP_LCRIT, HWMON_TEMP_MAX, MILLIDEGREES_PER_DEGREE, MODULES_LOAD_DIR,
    SYS_BLOCK_DIR, TEMP_SOURCE_MODULE,
};
use crate::domain::thermal::{DiskReading, Limits, check_plausible};

/// Resolve a configured disk name to a device path. Bare names are looked up
/// under [`DISK_BY_ID_DIR`].
fn resolve(name: &str) -> PathBuf {
    let path = Path::new(name);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(DISK_BY_ID_DIR).join(name)
    }
}

/// Read one disk's temperature from the kernel `drivetemp` hwmon.
///
/// This is the only source, deliberately. Do not add a `smartctl` fallback
/// behind it: it would spawn a process per disk per cycle with no timeout, need
/// root for ATA passthrough, and, worst of all, its exit status cannot separate
/// "drive is asleep" from "drive could not be read at all". An unreadable drive
/// would then be treated as one that merely needed minimum cooling.
///
/// Nothing this array needs is lost by leaving it out. `drivetemp` covers the
/// native-SATA drives this fan actually cools, those drives never spin down, so
/// standby detection is moot, and a USB-bridged drive, which `drivetemp` cannot
/// bind to, sits outside the airflow anyway. If the module is absent the read
/// fails, and a failed read means 100% fan.
///
/// Polling has a cost on drives that do spin down: the kernel's drivetemp
/// documentation notes that reading the temperature can reset some drives'
/// spin-down timer, so such a drive may never get to sleep.
fn read_disk(name: &str) -> Result<(f64, Limits)> {
    let path = resolve(name);
    let canonical = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;
    read_hwmon(&canonical)
}

/// Read the drive's temperature from its `drivetemp` hwmon:
/// `/sys/block/<dev>/device/hwmon/hwmon*/temp1_input`, in millidegrees C.
///
/// See [`ACCEPTED_HWMON_NAMES`] for why NVMe drives are never found this way.
fn read_hwmon(canonical_dev: &Path) -> Result<(f64, Limits)> {
    let block = canonical_dev
        .file_name()
        .and_then(|n| n.to_str())
        .context("device path has no basename")?;
    let hwmon_root = Path::new(SYS_BLOCK_DIR)
        .join(block)
        .join(BLOCK_HWMON_SUBDIR);

    let entries = std::fs::read_dir(&hwmon_root).with_context(|| {
        format!(
            "opening {}: is the {TEMP_SOURCE_MODULE} module loaded? (`modprobe \
             {TEMP_SOURCE_MODULE}`, and list `{TEMP_SOURCE_MODULE}` in a file under \
             {MODULES_LOAD_DIR}/ to persist it)",
            hwmon_root.display()
        )
    })?;

    for entry in entries.flatten() {
        let dir = entry.path();
        let hwmon_name = std::fs::read_to_string(dir.join(HWMON_NAME_FILE)).unwrap_or_default();
        if !ACCEPTED_HWMON_NAMES.contains(&hwmon_name.trim()) {
            continue;
        }
        let input = dir.join(HWMON_TEMP_INPUT);
        let raw = std::fs::read_to_string(&input)
            .with_context(|| format!("reading {}", input.display()))?;
        let celsius =
            parse_millidegrees(&raw).with_context(|| format!("in {}", input.display()))?;
        let limits = Limits {
            lcrit: optional_limit(&dir, HWMON_TEMP_LCRIT),
            max: optional_limit(&dir, HWMON_TEMP_MAX),
            crit: optional_limit(&dir, HWMON_TEMP_CRIT),
        };
        let celsius =
            check_plausible(celsius, &limits).with_context(|| format!("in {}", input.display()))?;
        return Ok((celsius, limits));
    }

    bail!(
        "no {} hwmon under {}: is the {TEMP_SOURCE_MODULE} module loaded? \
         (`modprobe {TEMP_SOURCE_MODULE}`)",
        ACCEPTED_HWMON_NAMES.join(" or "),
        hwmon_root.display()
    )
}

/// Parse a hwmon `temp*_input` reading (millidegrees C) into °C.
///
/// Rejects non-finite values rather than passing them on. Rust's `f64: FromStr`
/// accepts "nan" and "inf", and a NaN temperature would survive the fan curve's
/// clamp comparisons (every comparison against NaN is false) to reach code that
/// assumes it cannot. Refusing it here means an unparseable sensor fails safe to
/// 100% instead of behaving unpredictably.
fn parse_millidegrees(raw: &str) -> Result<f64> {
    let trimmed = raw.trim();
    let millidegrees: f64 = trimmed
        .parse()
        .with_context(|| format!("parsing temperature {trimmed:?}"))?;
    if !millidegrees.is_finite() {
        bail!("temperature is not a finite number: {trimmed:?}");
    }
    Ok(millidegrees / MILLIDEGREES_PER_DEGREE)
}

/// Read one of the drive's optional limit files, ignoring anything unusable.
///
/// Absent or unparseable limits simply do not constrain anything, so there is no
/// error to report: a missing `temp1_max` means no over-range warning, not a
/// failed temperature read.
fn optional_limit(dir: &Path, file: &str) -> Option<f64> {
    let raw = std::fs::read_to_string(dir.join(file)).ok()?;
    parse_millidegrees(&raw).ok()
}

pub fn read_all(disks: &[String]) -> Vec<DiskReading> {
    disks
        .iter()
        .map(|name| match read_disk(name) {
            Ok((celsius, limits)) => DiskReading {
                name: name.clone(),
                celsius: Ok(celsius),
                limits,
            },
            Err(e) => DiskReading {
                name: name.clone(),
                celsius: Err(e),
                limits: Limits::default(),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_names_resolve_under_by_id() {
        assert_eq!(
            resolve("ata-MODEL_SERIAL"),
            Path::new("/dev/disk/by-id/ata-MODEL_SERIAL")
        );
    }

    #[test]
    fn absolute_paths_are_used_verbatim() {
        assert_eq!(resolve("/dev/sdc"), Path::new("/dev/sdc"));
    }

    #[test]
    fn millidegrees_become_celsius() {
        assert_eq!(parse_millidegrees("34000").unwrap(), 34.0);
        assert_eq!(parse_millidegrees("  27500 \n").unwrap(), 27.5);
        assert_eq!(parse_millidegrees("-5000").unwrap(), -5.0);
    }

    #[test]
    fn non_finite_sensor_values_are_rejected() {
        // "nan"/"inf" parse successfully as f64, and a NaN would slip through the
        // curve's clamp comparisons. It must fail here instead.
        for bad in ["nan", "NaN", "inf", "-inf", "infinity"] {
            assert!(
                parse_millidegrees(bad).is_err(),
                "{bad:?} must be rejected, not forwarded to the curve"
            );
        }
    }

    #[test]
    fn unparseable_sensor_values_are_rejected() {
        for bad in ["", "   ", "abc", "34c", "0x8000"] {
            assert!(parse_millidegrees(bad).is_err(), "{bad:?} must be rejected");
        }
    }
}
