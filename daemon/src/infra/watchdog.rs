use std::cell::Cell;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::constants::{
    ABSTRACT_SOCKET_PREFIX, ENV_NOTIFY_SOCKET, ENV_WATCHDOG_USEC, NOTIFY_READY, NOTIFY_WATCHDOG,
};

/// systemd watchdog keepalive.
///
/// `Restart=always` only reacts to a process that *exits*. With `WatchdogSec=`
/// in the unit, systemd also kills and restarts one whose pings stop arriving,
/// which covers a daemon that is alive but stuck in its own code.
///
/// It cannot rescue one blocked reading `temp1_input` behind a failing drive.
/// drivetemp gives each command 10 s and up to 5 retries, and the read sleeps
/// uninterruptibly throughout, so systemd's kill takes effect only once the read
/// returns, when the daemon would carry on by itself anyway. The controller's
/// failsafe holds the fan at 100% in the meantime.
///
/// The `sd_notify` protocol is a single datagram to the socket named by
/// `$NOTIFY_SOCKET`, so it is implemented here rather than taking a dependency
/// for it.
pub struct Watchdog {
    socket: Option<UnixDatagram>,
    address: Option<SocketAddr>,
    /// The timeout systemd actually armed, from `$WATCHDOG_USEC`. `None` means
    /// notifications may work but no watchdog is configured.
    interval: Option<Duration>,
    /// So a persistent send failure is reported once rather than every cycle.
    reported_failure: Cell<bool>,
}

/// Resolve `$NOTIFY_SOCKET` into an address we can actually send to.
///
/// Both forms sd_notify(3) defines on Linux are handled: a leading `/` is a
/// filesystem path, a leading `@` is the abstract namespace.
///
/// The abstract form has to be built as a real `SocketAddr`. Translating it into
/// a NUL-prefixed *path* and handing that to `send_to()` looks right, since the
/// NUL prefix is genuinely how the address is spelled at the sockets layer, but it
/// cannot work. `std` rejects any path containing an interior NUL before it ever
/// reaches `sendto(2)`, so every notification fails with `InvalidInput` while the
/// module reports itself healthy.
fn parse_notify_address(name: &str) -> Result<SocketAddr> {
    if let Some(abstract_name) = name.strip_prefix(ABSTRACT_SOCKET_PREFIX) {
        return SocketAddr::from_abstract_name(abstract_name.as_bytes())
            .with_context(|| format!("abstract notify socket {name:?}"));
    }
    if Path::new(name).is_absolute() {
        return SocketAddr::from_pathname(name)
            .with_context(|| format!("notify socket path {name:?}"));
    }
    // `vsock:` and anything else: refusing loudly beats retrying forever
    // against an address we would silently fail to reach.
    bail!(
        "unsupported {ENV_NOTIFY_SOCKET} form {name:?} (expected an absolute path or a \
         leading '{ABSTRACT_SOCKET_PREFIX}')"
    )
}

/// Parse `$WATCHDOG_USEC`. Zero means systemd armed no watchdog.
fn parse_watchdog_usec(raw: &str) -> Option<Duration> {
    let usec: u64 = raw.trim().parse().ok()?;
    (usec > 0).then(|| Duration::from_micros(usec))
}

impl Watchdog {
    fn disabled() -> Self {
        Self {
            socket: None,
            address: None,
            interval: None,
            reported_failure: Cell::new(false),
        }
    }

    /// Connect to `$NOTIFY_SOCKET` if systemd provided one.
    ///
    /// Absent (a manual run, or a unit without the watchdog) the pings become
    /// no-ops, so nothing has to care whether it is running under systemd.
    pub fn from_env() -> Self {
        let Ok(name) = std::env::var(ENV_NOTIFY_SOCKET) else {
            return Self::disabled();
        };
        let address = match parse_notify_address(&name) {
            Ok(address) => address,
            Err(e) => {
                log::error!("{e:#}; watchdog disabled");
                return Self::disabled();
            }
        };
        let socket = match UnixDatagram::unbound() {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("cannot create notify socket ({e}); watchdog disabled");
                return Self::disabled();
            }
        };
        Self {
            socket: Some(socket),
            address: Some(address),
            interval: std::env::var(ENV_WATCHDOG_USEC)
                .ok()
                .as_deref()
                .and_then(parse_watchdog_usec),
            reported_failure: Cell::new(false),
        }
    }

    /// Whether systemd gave us somewhere to send notifications at all.
    pub fn can_notify(&self) -> bool {
        self.socket.is_some()
    }

    /// The watchdog timeout systemd armed, if any.
    ///
    /// Distinct from [`Self::can_notify`] on purpose: `$NOTIFY_SOCKET` is set
    /// for any notify-capable unit, so its presence alone does not mean a
    /// watchdog exists.
    pub fn interval(&self) -> Option<Duration> {
        self.interval
    }

    /// Tell systemd the service finished starting up.
    pub fn ready(&self) {
        self.send(NOTIFY_READY);
    }

    /// Tell systemd the loop is still making progress.
    pub fn ping(&self) {
        self.send(NOTIFY_WATCHDOG);
    }

    fn send(&self, message: &str) {
        let (Some(socket), Some(address)) = (&self.socket, &self.address) else {
            return;
        };
        match socket.send_to_addr(message.as_bytes(), address) {
            Ok(_) => self.reported_failure.set(false),
            // A watchdog that cannot ping is the one condition this module
            // exists to prevent, and systemd will kill the service for it, so the
            // first failure is reported loudly. At debug!, which the unit never
            // enables, the only symptom is a restart loop with no stated cause.
            Err(e) if self.reported_failure.replace(true) => {
                log::debug!("notify {message} still failing: {e}");
            }
            Err(e) => {
                log::error!(
                    "notify {message} failed: {e}; systemd will restart this \
                     service when the watchdog expires"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listener plus a `Watchdog` aimed at it, bypassing the environment so
    /// these tests cannot race each other over a process-global variable.
    fn aimed_at(address: SocketAddr) -> Watchdog {
        Watchdog {
            socket: Some(UnixDatagram::unbound().unwrap()),
            address: Some(address),
            interval: None,
            reported_failure: Cell::new(false),
        }
    }

    fn received(listener: &UnixDatagram) -> Option<String> {
        let mut buf = [0u8; 64];
        listener.set_nonblocking(true).unwrap();
        listener
            .recv_from(&mut buf)
            .ok()
            .map(|(n, _)| String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    fn unique(tag: &str) -> String {
        format!("fan-controllerd-test-{}-{tag}", std::process::id())
    }

    #[test]
    fn an_abstract_namespace_socket_actually_receives_the_ping() {
        // A NUL-prefixed path delivers nothing here, and the send error is easy
        // to miss, so systemd kills the service on the watchdog timeout with no
        // explanation anywhere.
        let name = unique("abstract");
        let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
        let listener = UnixDatagram::bind_addr(&address).unwrap();

        aimed_at(address).ping();

        assert_eq!(received(&listener).as_deref(), Some("WATCHDOG=1"));
    }

    #[test]
    fn a_filesystem_path_socket_still_receives_the_ping() {
        // The form systemd actually uses today (/run/systemd/notify), so this is
        // the regression guard for the deployed configuration.
        let dir = std::env::temp_dir().join(unique("path"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify");
        let listener = UnixDatagram::bind(&path).unwrap();
        let address = SocketAddr::from_pathname(&path).unwrap();

        aimed_at(address).ready();

        assert_eq!(received(&listener).as_deref(), Some("READY=1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_nul_prefixed_path_spelling_cannot_work() {
        // If anyone "simplifies" the address handling back to a NUL-prefixed
        // path, this records why that cannot work.
        let socket = UnixDatagram::unbound().unwrap();
        let e = socket
            .send_to(b"WATCHDOG=1", "\0some-abstract-name")
            .expect_err("std must reject an interior NUL in a path");
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn both_documented_address_forms_parse() {
        let abstract_addr = parse_notify_address("@/org/freedesktop/systemd1/notify").unwrap();
        assert_eq!(
            abstract_addr.as_abstract_name(),
            Some(&b"/org/freedesktop/systemd1/notify"[..])
        );
        let path_addr = parse_notify_address("/run/systemd/notify").unwrap();
        assert_eq!(
            path_addr.as_pathname(),
            Some(std::path::Path::new("/run/systemd/notify"))
        );
    }

    #[test]
    fn unsupported_address_forms_are_refused_rather_than_silently_broken() {
        for bad in ["vsock:2:1234", "relative/path", "", "notify"] {
            assert!(
                parse_notify_address(bad).is_err(),
                "{bad:?} must be refused, not accepted and then silently undeliverable"
            );
        }
    }

    #[test]
    fn an_unusable_address_degrades_instead_of_panicking() {
        // Over-long names and interior NULs must come back as errors.
        assert!(parse_notify_address(&format!("@{}", "x".repeat(200))).is_err());
        assert!(parse_notify_address(&format!("/{}", "x".repeat(200))).is_err());
        assert!(parse_notify_address("/tmp/a\0b").is_err());
    }

    #[test]
    fn a_disabled_watchdog_is_inert() {
        let w = Watchdog::disabled();
        assert!(!w.can_notify());
        assert_eq!(w.interval(), None);
        w.ready();
        w.ping(); // must not panic
    }

    #[test]
    fn watchdog_usec_is_parsed_and_zero_means_no_watchdog() {
        assert_eq!(
            parse_watchdog_usec("60000000"),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            parse_watchdog_usec(" 30000000 "),
            Some(Duration::from_secs(30))
        );
        assert_eq!(parse_watchdog_usec("0"), None, "zero means none");
        for bad in ["", "abc", "-1", "1.5"] {
            assert_eq!(parse_watchdog_usec(bad), None, "{bad:?}");
        }
    }
}
