use anyhow::Result;

use super::connection::verify_controller;
use super::report;
use super::session::ControlSession;
use crate::config::Config;
use crate::constants::{MAX_DUTY_PERCENT, ONCE_STALL_CYCLES};
use crate::domain::controller::check_controller;
use crate::infra::serial::Controller;

/// The `--once` diagnostic: one verbose cycle against the real controller.
///
/// Shares `ControlSession` with the service deliberately. A second copy of the
/// cycle drifts: an open-coded stall check with no consecutive-cycle requirement
/// would make the diagnostic you run by hand disagree with the one running in
/// service about what counts as a stall.
pub fn run(config: &Config) -> Result<()> {
    let mut controller = Controller::open(&config.device)?;
    println!("controller: VER {}", controller.handshake()?);
    verify_controller(config, &mut controller)?;
    let mut session = ControlSession::new(config, ONCE_STALL_CYCLES);
    let cycle = session.cycle(config, &mut controller)?;
    match cycle.hottest {
        Some(celsius) => println!("hottest drive: {celsius:.1} °C"),
        None => println!("hottest drive: UNREADABLE (commanding {MAX_DUTY_PERCENT}%)"),
    }
    println!("commanded duty: {}%", cycle.duty);
    let status = controller.status_fields()?;
    println!(
        "status: pwm={}% rpm={} failsafe={} uptime={}s",
        status.pwm_percent, status.rpm, status.failsafe, status.uptime_seconds
    );
    // There is no next cycle to confirm this command, so confirm it now. The
    // fan itself was already judged, before the command changed its speed.
    for found in check_controller(Some(cycle.duty), &status, None) {
        report::discrepancy(&found);
    }
    Ok(())
}
