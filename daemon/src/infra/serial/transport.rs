//! One command and its reply over a port, held to a single deadline.

use std::io;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serialport::SerialPort;

use crate::constants::{
    CARRIAGE_RETURN, EXCHANGE_TIMEOUT, LINE_TERMINATOR, MAX_REPLY_BYTES, MIN_PORT_TIMEOUT,
};

/// A port whose individual reads and writes can be held to a deadline.
///
/// Exists so the whole-exchange budget is enforceable rather than advisory: the
/// implementation for a real port narrows its timeout to the time remaining
/// before each operation, so no single read or write can outlive the deadline.
/// That also stops a write from inheriting whatever timeout the previous read
/// happened to leave on the port. Keeping it a trait preserves the hostile-peer
/// tests, which need ports that are not serial ports.
pub trait BoundedPort {
    /// Read one byte, blocking at most `budget`.
    fn read_byte(&mut self, into: &mut [u8; 1], budget: Duration) -> io::Result<usize>;
    /// Write some of `data`, blocking at most `budget`, and return how much.
    fn write_some(&mut self, data: &[u8], budget: Duration) -> io::Result<usize>;
}

/// Floored at [`MIN_PORT_TIMEOUT`].
fn port_timeout(budget: Duration) -> Duration {
    budget.max(MIN_PORT_TIMEOUT)
}

impl BoundedPort for dyn SerialPort {
    fn read_byte(&mut self, into: &mut [u8; 1], budget: Duration) -> io::Result<usize> {
        self.set_timeout(port_timeout(budget))
            .map_err(io::Error::other)?;
        self.read(into)
    }

    fn write_some(&mut self, data: &[u8], budget: Duration) -> io::Result<usize> {
        self.set_timeout(port_timeout(budget))
            .map_err(io::Error::other)?;
        self.write(data)
    }
}

/// Send `cmd` and read its reply, all within `deadline`.
pub fn exchange<P: BoundedPort + ?Sized>(
    port: &mut P,
    cmd: &str,
    deadline: Instant,
) -> Result<String> {
    send_line(port, cmd, deadline)?;
    read_reply(port, cmd, deadline)
}

/// Write `cmd` and its terminator, each attempt bounded by what is left of the
/// exchange budget.
fn send_line<P: BoundedPort + ?Sized>(port: &mut P, cmd: &str, deadline: Instant) -> Result<()> {
    let mut line = cmd.as_bytes().to_vec();
    line.push(LINE_TERMINATOR);
    let mut sent = 0;
    while sent < line.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out sending {cmd} after {EXCHANGE_TIMEOUT:?}");
        }
        match port.write_some(&line[sent..], remaining) {
            Ok(0) => bail!("sending {cmd}: the port accepted no data"),
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => bail!("sending {cmd}: {e}"),
        }
    }
    Ok(())
}

/// Read one `\n`-terminated reply, bounded both in time and in bytes.
///
/// Generic over the port so the termination guarantees can be tested against
/// hostile peers without a serial port.
fn read_reply<P: BoundedPort + ?Sized>(
    port: &mut P,
    cmd: &str,
    deadline: Instant,
) -> Result<String> {
    let mut line = Vec::with_capacity(MAX_REPLY_BYTES);
    let mut byte = [0u8; 1];
    // Count every byte the peer sends, not just the ones we keep. If skipped
    // bytes (CR) advanced neither the terminator nor the length cap, a peer
    // streaming them would spin this loop forever at full CPU.
    let mut consumed = 0usize;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timed out reading reply to {cmd} after {EXCHANGE_TIMEOUT:?}");
        }
        match port.read_byte(&mut byte, remaining) {
            Ok(1) => {
                consumed += 1;
                match byte[0] {
                    LINE_TERMINATOR => break,
                    CARRIAGE_RETURN => {}
                    b => line.push(b),
                }
            }
            // A zero-length read makes no progress; the deadline bounds it.
            Ok(_) => {}
            Err(e) => bail!("reading reply to {cmd}: {e}"),
        }
        if consumed > MAX_REPLY_BYTES {
            bail!("reply to {cmd} exceeds {MAX_REPLY_BYTES} bytes");
        }
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    /// Yields the given bytes, then blocks forever by reporting zero-length
    /// reads (what a silent-but-open peer looks like).
    struct Scripted {
        data: Vec<u8>,
        pos: usize,
    }

    impl BoundedPort for Scripted {
        fn read_byte(&mut self, into: &mut [u8; 1], _budget: Duration) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            into[0] = self.data[self.pos];
            self.pos += 1;
            Ok(1)
        }
        fn write_some(&mut self, data: &[u8], _budget: Duration) -> io::Result<usize> {
            Ok(data.len())
        }
    }

    /// Streams a single byte forever, which is the shape that can hang a reader.
    struct Endless(u8);

    impl BoundedPort for Endless {
        fn read_byte(&mut self, into: &mut [u8; 1], _budget: Duration) -> io::Result<usize> {
            into[0] = self.0;
            Ok(1)
        }
        fn write_some(&mut self, data: &[u8], _budget: Duration) -> io::Result<usize> {
            Ok(data.len())
        }
    }

    /// Burns most of whatever budget it is handed, then yields one byte and never
    /// a terminator. This is the shape that overruns the advertised deadline: with
    /// a fixed per-read timeout, two such reads take 1.8x the whole-exchange
    /// budget.
    struct Slow;

    impl BoundedPort for Slow {
        fn read_byte(&mut self, into: &mut [u8; 1], budget: Duration) -> io::Result<usize> {
            std::thread::sleep(budget.mul_f64(0.9));
            into[0] = b'x';
            Ok(1)
        }
        fn write_some(&mut self, data: &[u8], _budget: Duration) -> io::Result<usize> {
            Ok(data.len())
        }
    }

    /// The write-side twin of `Slow`: a transmit buffer that drains one byte per
    /// most-of-the-budget. With a fixed write timeout, this alone could hold a
    /// command for a full port timeout per byte.
    struct SlowWriter;

    impl BoundedPort for SlowWriter {
        fn read_byte(&mut self, into: &mut [u8; 1], _budget: Duration) -> io::Result<usize> {
            into[0] = b'\n';
            Ok(1)
        }
        fn write_some(&mut self, _data: &[u8], budget: Duration) -> io::Result<usize> {
            std::thread::sleep(budget.mul_f64(0.9));
            Ok(1)
        }
    }

    /// Takes a fixed time to accept the command, then answers `OK` at once.
    struct DelayedWrite(Duration);

    impl BoundedPort for DelayedWrite {
        fn read_byte(&mut self, into: &mut [u8; 1], _budget: Duration) -> io::Result<usize> {
            into[0] = b'\n';
            Ok(1)
        }
        fn write_some(&mut self, data: &[u8], _budget: Duration) -> io::Result<usize> {
            std::thread::sleep(self.0);
            Ok(data.len())
        }
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn scripted(s: &str) -> Scripted {
        Scripted {
            data: s.as_bytes().to_vec(),
            pos: 0,
        }
    }

    #[test]
    fn reads_a_plain_reply() {
        let mut p = scripted("OK\n");
        assert_eq!(read_reply(&mut p, "PWM 31", deadline()).unwrap(), "OK");
    }

    #[test]
    fn carriage_returns_are_still_stripped() {
        let mut p = scripted("VER 1.0.0\r\n");
        assert_eq!(read_reply(&mut p, "VER", deadline()).unwrap(), "VER 1.0.0");
    }

    #[test]
    fn an_endless_carriage_return_stream_terminates() {
        // Without counting skipped bytes this loops forever at 100% CPU, because
        // CR advances neither the terminator nor the byte cap.
        let mut p = Endless(b'\r');
        let e = read_reply(&mut p, "PWM 31", deadline()).expect_err("must not hang on a CR stream");
        assert!(
            format!("{e:#}").contains("exceeds"),
            "expected the byte cap to trip, got: {e:#}"
        );
    }

    #[test]
    fn an_endless_data_stream_terminates() {
        let mut p = Endless(b'x');
        assert!(read_reply(&mut p, "STATUS", deadline()).is_err());
    }

    #[test]
    fn a_silent_peer_hits_the_deadline_rather_than_spinning() {
        let mut p = scripted(""); // only zero-length reads
        let e = read_reply(&mut p, "VER", Instant::now() + Duration::from_millis(50))
            .expect_err("must time out");
        assert!(format!("{e:#}").contains("timed out"), "got: {e:#}");
    }

    #[test]
    fn the_whole_exchange_deadline_is_a_real_bound() {
        // Each read is handed only the time still remaining, so a reader that
        // blocks for its whole budget cannot push the exchange past the deadline.
        // Checking the deadline only between reads would let it reach ~2x.
        let budget = Duration::from_millis(400);
        let started = Instant::now();
        let e = read_reply(&mut Slow, "STATUS", started + budget)
            .expect_err("must stop at the deadline");
        let elapsed = started.elapsed();
        assert!(
            format!("{e:#}").contains("timed out"),
            "should be the deadline, got: {e:#}"
        );
        assert!(
            elapsed < budget.mul_f64(1.4),
            "overran the {budget:?} budget: took {elapsed:?}"
        );
    }

    #[test]
    fn a_slow_write_is_held_to_the_same_deadline() {
        let budget = Duration::from_millis(400);
        let started = Instant::now();
        let e = exchange(&mut SlowWriter, "PWM 31", started + budget)
            .expect_err("must stop at the deadline");
        let elapsed = started.elapsed();
        assert!(format!("{e:#}").contains("timed out sending"), "got: {e:#}");
        assert!(
            elapsed < budget.mul_f64(1.4),
            "overran the {budget:?} budget: took {elapsed:?}"
        );
    }

    #[test]
    fn the_write_and_the_reply_share_one_budget() {
        // A write that takes most of the budget leaves the reply only the rest.
        // A reply given a fresh budget once the write returned could stretch the
        // exchange to twice what EXCHANGE_TIMEOUT_S promises.
        let budget = Duration::from_millis(300);
        let started = Instant::now();
        let reply = exchange(
            &mut DelayedWrite(budget.mul_f64(0.5)),
            "STATUS",
            started + budget,
        );
        assert_eq!(
            reply.unwrap(),
            "",
            "a reply arriving inside the budget succeeds"
        );
        let late = Instant::now();
        let e = exchange(
            &mut DelayedWrite(budget.mul_f64(1.2)),
            "STATUS",
            late + budget,
        )
        .expect_err("nothing is left for the reply once the write overran");
        assert!(format!("{e:#}").contains("timed out"), "got: {e:#}");
    }

    #[test]
    fn a_reply_just_under_the_cap_is_accepted() {
        let body = "A".repeat(MAX_REPLY_BYTES - 1);
        let mut p = scripted(&format!("{body}\n"));
        assert_eq!(read_reply(&mut p, "STATUS", deadline()).unwrap(), body);
    }
}
