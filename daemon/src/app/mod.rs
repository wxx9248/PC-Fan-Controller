//! The order things happen in, for the service and for `--once`.
//!
//! The rules live in `domain` and the I/O in `infra`; this layer only wires one
//! to the other and reports what the rules found.

mod connection;
mod once;
mod report;
mod service;
mod session;

use anyhow::Result;

use crate::cli::Options;
use crate::config::Config;
use crate::constants::{PROGRAM_NAME, VERSION};

pub fn run(options: &Options) -> Result<()> {
    log::info!("{PROGRAM_NAME} {VERSION}");
    let config = Config::load(&options.config_path)?;
    if options.once {
        once::run(&config)
    } else {
        service::run(&config)
    }
}
