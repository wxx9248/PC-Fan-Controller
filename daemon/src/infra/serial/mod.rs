//! The controller, reached over its USB serial port.

mod protocol;
mod transport;

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serialport::{ClearBuffer, SerialPort};

use crate::constants::{BAUD, CMD_STATUS, CMD_VER, EXCHANGE_TIMEOUT};
use crate::domain::controller::ControllerStatus;

pub struct Controller {
    port: Box<dyn SerialPort>,
}

impl Controller {
    pub fn open(device: &Path) -> Result<Self> {
        let device_str = device
            .to_str()
            .context("serial device path is not valid UTF-8")?;
        let mut port = serialport::new(device_str, BAUD)
            .timeout(EXCHANGE_TIMEOUT)
            .open()
            .with_context(|| format!("opening {device_str}"))?;
        // Plain DTR assertion; the ESP32-C3 reset-to-bootloader sequence
        // requires a specific DTR/RTS dance that normal opens never perform.
        port.write_data_terminal_ready(true).ok();
        port.clear(ClearBuffer::All).ok();
        Ok(Self { port })
    }

    fn command(&mut self, cmd: &str) -> Result<String> {
        // Started before the write, so a write blocked on a full transmit buffer
        // spends the same budget as the reply instead of adding its own.
        let deadline = Instant::now() + EXCHANGE_TIMEOUT;
        self.port.clear(ClearBuffer::Input).ok();
        transport::exchange(&mut *self.port, cmd, deadline)
    }

    pub fn set_pwm(&mut self, percent: u8) -> Result<()> {
        let reply = self.command(&protocol::pwm_command(percent))?;
        protocol::check_pwm_reply(percent, &reply)
    }

    /// Exchange `VER` and check the firmware speaks a protocol we understand.
    ///
    /// The check has to look at the reply. Merely logging whatever comes back
    /// proves nothing: that "succeeds" against a device answering `ERR unknown
    /// command`, and an incompatible firmware then surfaces much later as a
    /// generic rejected-PWM reconnect loop instead of a version complaint.
    pub fn handshake(&mut self) -> Result<String> {
        protocol::check_version(&self.command(CMD_VER)?)
    }

    /// `STATUS` parsed into fields.
    pub fn status_fields(&mut self) -> Result<ControllerStatus> {
        let line = self.command(CMD_STATUS)?;
        protocol::parse_status(&line)
    }
}
