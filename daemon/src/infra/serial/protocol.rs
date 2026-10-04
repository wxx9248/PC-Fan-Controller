//! The controller's line protocol (PROTOCOL.md): building commands and reading
//! replies. Text in, text out, so it is tested without a port.

use anyhow::{Context, Result, bail};

use crate::constants::{
    CMD_PWM, CMD_STATUS, CMD_VER, FAILSAFE_INACTIVE, FIELD_SEPARATOR, REPLY_OK,
    REQUIRED_PROTOCOL_MAJOR, STATUS_FAILSAFE, STATUS_FAILSAFE_TIMEOUT, STATUS_PWM, STATUS_RPM,
    STATUS_UPTIME, VERSION_SEPARATOR, WORD_SEPARATOR,
};
use crate::domain::controller::ControllerStatus;

/// The command that sets the fan duty.
pub fn pwm_command(percent: u8) -> String {
    format!("{CMD_PWM}{WORD_SEPARATOR}{percent}")
}

/// Accept the reply to a `PWM` command, which must be `OK`.
pub fn check_pwm_reply(percent: u8, reply: &str) -> Result<()> {
    if reply != REPLY_OK {
        bail!("controller rejected {CMD_PWM} {percent}: {reply}");
    }
    Ok(())
}

/// Parse a `STATUS` reply into its fields.
pub fn parse_status(line: &str) -> Result<ControllerStatus> {
    let mut fields = line.split_whitespace();
    if fields.next() != Some(CMD_STATUS) {
        bail!("not a {CMD_STATUS} reply: {line:?}");
    }
    let mut pwm = None;
    let mut rpm = None;
    let mut failsafe = None;
    let mut uptime = None;
    let mut failsafe_timeout = None;
    for field in fields {
        let (key, value) = field
            .split_once(FIELD_SEPARATOR)
            .with_context(|| format!("malformed {CMD_STATUS} field {field:?}"))?;
        match key {
            STATUS_PWM => pwm = Some(parse_field(key, value)?),
            STATUS_RPM => rpm = Some(parse_field(key, value)?),
            STATUS_FAILSAFE => failsafe = Some(value != FAILSAFE_INACTIVE),
            STATUS_UPTIME => uptime = Some(parse_field(key, value)?),
            STATUS_FAILSAFE_TIMEOUT => failsafe_timeout = Some(parse_field(key, value)?),
            // Tolerate fields a newer firmware may add.
            _ => {}
        }
    }
    Ok(ControllerStatus {
        pwm_percent: require_field(pwm, STATUS_PWM)?,
        rpm: require_field(rpm, STATUS_RPM)?,
        failsafe: require_field(failsafe, STATUS_FAILSAFE)?,
        uptime_seconds: require_field(uptime, STATUS_UPTIME)?,
        failsafe_timeout_s: failsafe_timeout,
    })
}

fn parse_field<T>(key: &str, value: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value
        .parse()
        .with_context(|| format!("{CMD_STATUS} {key}={value:?}"))
}

fn require_field<T>(field: Option<T>, key: &str) -> Result<T> {
    field.with_context(|| format!("{CMD_STATUS} missing {key}"))
}

/// Validate a `VER` reply and return the version it reported.
pub fn check_version(reply: &str) -> Result<String> {
    let version = reply
        .strip_prefix(CMD_VER)
        .and_then(|rest| rest.strip_prefix(WORD_SEPARATOR))
        .with_context(|| format!("not a {CMD_VER} reply: {reply:?}"))?
        .trim();
    let major: u64 = version
        .split(VERSION_SEPARATOR)
        .next()
        .unwrap_or_default()
        .parse()
        .with_context(|| format!("unparseable protocol version {version:?}"))?;
    if major != REQUIRED_PROTOCOL_MAJOR {
        bail!(
            "firmware speaks protocol v{version}, this daemon requires \
             v{REQUIRED_PROTOCOL_MAJOR}.x"
        );
    }
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matching_version_is_accepted() {
        assert_eq!(check_version("VER 1.0.0").unwrap(), "1.0.0");
        assert_eq!(check_version("VER 1.4.2").unwrap(), "1.4.2");
    }

    #[test]
    fn an_incompatible_major_is_rejected() {
        let e = check_version("VER 2.0.0").expect_err("major 2 must be rejected");
        assert!(format!("{e:#}").contains("requires"), "got: {e:#}");
    }

    #[test]
    fn a_non_version_reply_is_rejected() {
        // Without the prefix and version checks, every one of these counts as a
        // successful handshake.
        for bad in ["OK", "ERR unknown command", "PONG", "", "VERSION 1.0.0"] {
            assert!(check_version(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn a_malformed_version_is_rejected() {
        for bad in ["VER ", "VER x.y.z", "VER .1.0", "VER one"] {
            assert!(check_version(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn status_parses() {
        let s = parse_status("STATUS PWM=31 RPM=840 FAILSAFE=0 UPTIME=98045").unwrap();
        assert_eq!(
            s,
            ControllerStatus {
                pwm_percent: 31,
                rpm: 840,
                failsafe: false,
                uptime_seconds: 98045,
                failsafe_timeout_s: None,
            }
        );
    }

    #[test]
    fn status_parses_failsafe_set() {
        let s = parse_status("STATUS PWM=100 RPM=1980 FAILSAFE=1 UPTIME=5").unwrap();
        assert!(s.failsafe);
        assert_eq!(s.pwm_percent, 100);
    }

    #[test]
    fn status_tolerates_unknown_future_fields() {
        let s = parse_status("STATUS PWM=31 RPM=840 FAILSAFE=0 UPTIME=1 VOLTS=12").unwrap();
        assert_eq!(s.rpm, 840);
    }

    #[test]
    fn status_rejects_other_replies() {
        for bad in ["OK", "ERR unknown command", "VER 1.0.0", "STATUS PWM=31"] {
            assert!(parse_status(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn status_reports_the_failsafe_timeout_when_the_firmware_sends_it() {
        let s =
            parse_status("STATUS PWM=31 RPM=840 FAILSAFE=0 UPTIME=7 FAILSAFE_TIMEOUT=30").unwrap();
        assert_eq!(s.failsafe_timeout_s, Some(30));
        let old = parse_status("STATUS PWM=31 RPM=840 FAILSAFE=0 UPTIME=7").unwrap();
        assert_eq!(old.failsafe_timeout_s, None, "older firmware omits it");
        assert!(
            parse_status("STATUS PWM=31 RPM=840 FAILSAFE=0 UPTIME=7 FAILSAFE_TIMEOUT=x").is_err(),
            "a malformed value is an error, not a missing field"
        );
    }
}
