use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;

use crate::constants::{
    ENV_JOURNAL_STREAM, JOURNAL_STREAM_SEPARATOR, SYSLOG_DEBUG, SYSLOG_ERR, SYSLOG_INFO,
    SYSLOG_WARNING,
};

/// Set up `log` output: env_logger's usual format on a terminal, journald's own
/// on systemd.
///
/// journald reads a line's priority from a leading `<N>` (sd-daemon(3)) and
/// files everything else at the unit's default, which is info. env_logger's
/// default format has no such prefix, so a FAN STALL error would be filed as
/// info and never show in `journalctl -p err`. The stall and over-temperature
/// checks report rather than act, so that report is the whole of their output.
/// The journal also stamps each line itself, which makes env_logger's timestamp
/// a duplicate.
pub fn init(default_level: &str) {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level));
    if stderr_is_journal() {
        builder.format(|buf, record| {
            writeln!(
                buf,
                "<{}>{}",
                syslog_priority(record.level()),
                record.args()
            )
        });
    }
    builder.init();
}

/// The syslog priority journald should file a record under.
fn syslog_priority(level: log::Level) -> u8 {
    match level {
        log::Level::Error => SYSLOG_ERR,
        log::Level::Warn => SYSLOG_WARNING,
        log::Level::Info => SYSLOG_INFO,
        log::Level::Debug | log::Level::Trace => SYSLOG_DEBUG,
    }
}

/// Whether stderr is the stream systemd connected to the journal.
///
/// `$JOURNAL_STREAM` alone is not enough: it is inherited, so a process whose
/// stderr was redirected elsewhere would still see it. systemd.exec(5) says to
/// compare it with stderr's own device and inode.
fn stderr_is_journal() -> bool {
    std::env::var(ENV_JOURNAL_STREAM).is_ok_and(|value| names_stderr(&value))
}

/// Whether a `$JOURNAL_STREAM` value names this process's stderr.
fn names_stderr(value: &str) -> bool {
    parse_stream(value).is_some_and(|stream| stderr_identity() == Some(stream))
}

/// Parse `$JOURNAL_STREAM`, which is `<device>:<inode>` in decimal.
fn parse_stream(value: &str) -> Option<(u64, u64)> {
    let (dev, ino) = value.split_once(JOURNAL_STREAM_SEPARATOR)?;
    Some((dev.parse().ok()?, ino.parse().ok()?))
}

fn stderr_identity() -> Option<(u64, u64)> {
    // A duplicate, so stderr itself stays open when the File is dropped.
    let fd = std::io::stderr().as_fd().try_clone_to_owned().ok()?;
    let metadata = std::fs::File::from(fd).metadata().ok()?;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_maps_to_its_syslog_priority() {
        assert_eq!(syslog_priority(log::Level::Error), 3);
        assert_eq!(syslog_priority(log::Level::Warn), 4);
        assert_eq!(syslog_priority(log::Level::Info), 6);
        assert_eq!(syslog_priority(log::Level::Debug), 7);
        assert_eq!(syslog_priority(log::Level::Trace), 7);
    }

    #[test]
    fn journal_stream_values_parse() {
        assert_eq!(parse_stream("8:12345"), Some((8, 12345)));
        for bad in ["", "8", "8:", ":1", "a:b", "8:1:2"] {
            assert_eq!(parse_stream(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_a_stream_naming_stderr_itself_counts() {
        let (dev, ino) = stderr_identity().expect("stderr has an identity");
        assert!(names_stderr(&format!("{dev}:{ino}")));
        assert!(
            !names_stderr(&format!("{dev}:{}", ino.wrapping_add(1))),
            "an inherited value for some other stream must not match"
        );
        assert!(!names_stderr("not-a-stream"));
    }
}
