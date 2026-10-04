//! Every fixed value the daemon depends on, in one place.
//!
//! Grouped by who owns the value: the program itself, the controller's wire
//! protocol (which has to agree with PROTOCOL.md and firmware/main/constants.h),
//! the kernel and systemd interfaces, and the daemon's own limits and defaults.
//! Test fixtures stay in their tests, where the literal is the point.

use std::time::Duration;

// ---- Program ----

pub const PROGRAM_NAME: &str = env!("CARGO_PKG_NAME");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Read when `--config` is not given. The shipped unit always passes
/// `--config`, so this only matters for a manual run. It is CONFIG_FILE in
/// every installation target under config/targets/.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/fan-controllerd.toml";

/// Log filter when `RUST_LOG` is unset.
pub const DEFAULT_LOG_LEVEL: &str = "info";
/// `--once` is documented as verbose, so it shows the per-disk readings.
pub const ONCE_LOG_LEVEL: &str = "debug";

/// Exit status for a command-line usage error, as fan-controllerd(8) documents.
pub const EXIT_USAGE: u8 = 2;

// ---- Reconnecting ----

pub const RECONNECT_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
/// Capped well under a minute for two reasons: the firmware holds the fan at
/// 100% for the whole gap, and the systemd watchdog timeout has to exceed the
/// longest possible loop interval, to which this contributes without being the
/// dominant term. See the arithmetic in fan-controllerd.service.
pub const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(15);
/// Each failed attempt multiplies the backoff by this, up to the cap.
pub const RECONNECT_BACKOFF_FACTOR: u32 = 2;

// ---- Controller wire protocol (PROTOCOL.md) ----

/// Budget for one whole command/reply exchange, writing the command included,
/// and a real bound on it.
///
/// The per-read timeout alone bounds nothing: a peer that dribbles one byte just
/// inside it, or streams bytes we skip, keeps the loop alive indefinitely. So
/// `infra::serial::transport::exchange` enforces a deadline for the entire
/// exchange.
///
/// A deadline checked only *between* operations is still not a bound: a read or
/// write starting a moment before the deadline can block a further full timeout,
/// turning a 3 s budget into a 6 s one. Every read and write is therefore given
/// only the time still remaining (see `infra::serial::transport::BoundedPort`),
/// so the number below really is the whole-exchange worst case. The poll-interval
/// rule in `domain::failsafe` depends on that being true.
pub const EXCHANGE_TIMEOUT_S: u64 = 3;
pub const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(EXCHANGE_TIMEOUT_S);

/// The shortest timeout handed to the port. serialport treats a zero timeout as
/// non-blocking, and one last attempt at the very end of the budget is still
/// worth making.
pub const MIN_PORT_TIMEOUT: Duration = Duration::from_millis(1);

/// Largest reply we will buffer. PROTOCOL.md caps lines far below this.
pub const MAX_REPLY_BYTES: usize = 256;

/// Nominal only: the ESP32-C3 USB Serial/JTAG is a native USB CDC device and
/// ignores it.
pub const BAUD: u32 = 115_200;

/// Protocol major version this daemon speaks, against `PROTOCOL_VERSION` in the
/// firmware. A different major means the framing or the meaning of commands has
/// changed, so refusing to drive it is safer than guessing.
pub const REQUIRED_PROTOCOL_MAJOR: u64 = 1;

/// Ends every command and every reply. A carriage return before it is ignored.
pub const LINE_TERMINATOR: u8 = b'\n';
pub const CARRIAGE_RETURN: u8 = b'\r';
/// Between a command or reply word and its argument: `PWM 31`, `VER 1.0.0`.
pub const WORD_SEPARATOR: char = ' ';
/// Between a `STATUS` field's name and its value: `RPM=840`.
pub const FIELD_SEPARATOR: char = '=';
/// Between the parts of a protocol version, major first.
pub const VERSION_SEPARATOR: char = '.';

pub const CMD_PWM: &str = "PWM";
pub const CMD_STATUS: &str = "STATUS";
/// Also the first word of the reply, which is `VER <version>`.
pub const CMD_VER: &str = "VER";
pub const REPLY_OK: &str = "OK";

/// The fields of a `STATUS` reply, which starts with [`CMD_STATUS`].
pub const STATUS_PWM: &str = "PWM";
pub const STATUS_RPM: &str = "RPM";
pub const STATUS_FAILSAFE: &str = "FAILSAFE";
pub const STATUS_UPTIME: &str = "UPTIME";
pub const STATUS_FAILSAFE_TIMEOUT: &str = "FAILSAFE_TIMEOUT";
/// `FAILSAFE=0`; any other value means the failsafe is active.
pub const FAILSAFE_INACTIVE: &str = "0";

// ---- Kernel: drives and their temperature sensors ----

/// Where bare disk names in the configuration are looked up: immutable
/// identifiers built from model+serial or WWN, independent of sdX numbering and
/// SATA port.
pub const DISK_BY_ID_DIR: &str = "/dev/disk/by-id";
pub const SYS_BLOCK_DIR: &str = "/sys/block";
/// Under `/sys/block/<dev>/`, where `drivetemp` registers its hwmon.
pub const BLOCK_HWMON_SUBDIR: &str = "device/hwmon";
pub const HWMON_NAME_FILE: &str = "name";
/// Millidegrees C, like the limit files below.
pub const HWMON_TEMP_INPUT: &str = "temp1_input";
pub const HWMON_TEMP_LCRIT: &str = "temp1_lcrit";
pub const HWMON_TEMP_MAX: &str = "temp1_max";
pub const HWMON_TEMP_CRIT: &str = "temp1_crit";
pub const MILLIDEGREES_PER_DEGREE: f64 = 1000.0;

/// The only temperature source the daemon has.
pub const TEMP_SOURCE_MODULE: &str = "drivetemp";
/// Where a module is listed to be loaded at boot.
pub const MODULES_LOAD_DIR: &str = "/etc/modules-load.d";
/// hwmon names whose `temp1_input` is read. `nvme` is accepted, but no NVMe
/// drive is ever found this way: the nvme driver registers its hwmon directly
/// under the controller (`device/hwmonN`), with no `hwmon/` directory in
/// between. NVMe drives are therefore unsupported, and a listed one reads as
/// unreadable (100% fan).
pub const ACCEPTED_HWMON_NAMES: &[&str] = &[TEMP_SOURCE_MODULE, "nvme"];

/// Plausibility floor for a drive that publishes no `temp1_lcrit`.
///
/// Matches both what this array's drives report and the curve validator's lower
/// bound. The floor is the safety-critical end: a bogus *low* reading clamps to
/// the first curve point and so commands minimum cooling, while a bogus *high*
/// one clamps to the last and commands full cooling. Only one of those
/// directions can cook a drive.
pub const FALLBACK_FLOOR_CELSIUS: f64 = -40.0;

/// Ceiling above which a reading is a broken sensor rather than a hot drive.
///
/// Deliberately far above any `temp1_crit`: a reading above the critical
/// threshold is an emergency to act on, not an implausibility to discard, so this
/// must not be derived from it. It exists to keep a corrupt sensor from being
/// reported as a temperature.
pub const PLAUSIBLE_CEILING_CELSIUS: f64 = 150.0;

// ---- systemd and journald ----

/// `<device>:<inode>` of the stream systemd connected to the journal.
pub const ENV_JOURNAL_STREAM: &str = "JOURNAL_STREAM";
pub const JOURNAL_STREAM_SEPARATOR: char = ':';
pub const ENV_NOTIFY_SOCKET: &str = "NOTIFY_SOCKET";
/// The watchdog timeout systemd armed, in microseconds; zero means none.
pub const ENV_WATCHDOG_USEC: &str = "WATCHDOG_USEC";

pub const NOTIFY_READY: &str = "READY=1";
pub const NOTIFY_WATCHDOG: &str = "WATCHDOG=1";
/// Marks a `$NOTIFY_SOCKET` in the abstract namespace rather than a path.
pub const ABSTRACT_SOCKET_PREFIX: char = '@';

/// syslog(3) priorities, which journald reads from a leading `<N>`.
pub const SYSLOG_ERR: u8 = 3;
pub const SYSLOG_WARNING: u8 = 4;
pub const SYSLOG_INFO: u8 = 6;
pub const SYSLOG_DEBUG: u8 = 7;

// ---- Fan duty ----

pub const MIN_DUTY_PERCENT: u8 = 0;
/// Full speed: what the daemon commands whenever it cannot tell how hot the
/// drives are, the same posture the firmware failsafe takes.
pub const MAX_DUTY_PERCENT: u8 = 100;
/// Converts a percentage to a fraction.
pub const PERCENT_PER_WHOLE: f64 = 100.0;

// ---- Configuration limits ----

/// Plausible bounds for a fan-curve knee. The upper bound is well past any
/// temperature a drive survives, so nothing useful is excluded. Because duty
/// shares the 0-100 range with plausible temperatures, the bound is also what
/// catches a curve written with its columns swapped.
pub const MIN_CURVE_CELSIUS: i64 = -40;
pub const MAX_CURVE_CELSIUS: i64 = 90;

/// A deadband permits steady-state error up to `deadband - 1` points, which at
/// this fan's ~16 RPM per point is ~145 RPM at the ceiling. Wider than that
/// stops being hysteresis and starts being a stuck fan.
pub const MAX_DEADBAND_PERCENT: u8 = 10;

pub const MIN_STALL_TOLERANCE_PERCENT: u8 = 1;
pub const MAX_STALL_TOLERANCE_PERCENT: u8 = 100;

/// The gap between two `PWM` commands may be at most this fraction (1/N) of the
/// controller's failsafe timeout.
pub const FAILSAFE_HEADROOM_DIVISOR: u64 = 2;

// ---- Configuration defaults ----

pub const DEFAULT_POLL_INTERVAL_S: u64 = 10;
/// CONFIG_FAN_FAILSAFE_TIMEOUT_S's default in firmware/main/Kconfig.projbuild.
pub const DEFAULT_FAILSAFE_TIMEOUT_S: u64 = 30;
pub const DEFAULT_SMOOTHING_ALPHA: f64 = 0.3;
pub const DEFAULT_DEADBAND_PERCENT: u8 = 2;
pub const DEFAULT_STALL_CYCLES: u32 = 3;
pub const DEFAULT_STALL_TOLERANCE_PERCENT: u8 = 70;

/// `--once` has a single cycle to judge by, so one bad reading reports.
pub const ONCE_STALL_CYCLES: u32 = 1;
