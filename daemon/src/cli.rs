//! The command line: what was asked for, and the usage text.

use std::path::PathBuf;

use crate::constants::{DEFAULT_CONFIG_PATH, PROGRAM_NAME};

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub config_path: PathBuf,
    pub once: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Options),
    Help,
    Version,
}

pub fn usage() -> String {
    format!(
        "\
usage: {PROGRAM_NAME} [--config PATH] [--once]
       {PROGRAM_NAME} --help | --version

  --config PATH  read the configuration from PATH (default {DEFAULT_CONFIG_PATH})
  --once         run one measure/control cycle verbosely, then exit
  -h, --help     print this help and exit
  -V, --version  print the version and exit
"
    )
}

/// Parse the arguments after the program name, or say what is wrong with them.
///
/// Arguments are taken in order, and the first `--help`, `--version` or error
/// settles the outcome: `--help --bogus` prints the help.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut options = Options {
        config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
        once: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                options.config_path = args
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--config requires a path".to_string())?;
            }
            "--once" => options.once = true,
            "--help" | "-h" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Command::Run(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn no_arguments_runs_the_service_with_the_default_config() {
        assert_eq!(
            parse_strs(&[]),
            Ok(Command::Run(Options {
                config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
                once: false,
            }))
        );
    }

    #[test]
    fn config_and_once_are_both_taken() {
        assert_eq!(
            parse_strs(&["--once", "--config", "/tmp/x.toml"]),
            Ok(Command::Run(Options {
                config_path: PathBuf::from("/tmp/x.toml"),
                once: true,
            }))
        );
    }

    #[test]
    fn the_first_decisive_argument_wins() {
        assert_eq!(parse_strs(&["--help", "--bogus"]), Ok(Command::Help));
        assert_eq!(parse_strs(&["-V"]), Ok(Command::Version));
        assert_eq!(
            parse_strs(&["--bogus", "--help"]),
            Err("unknown argument: --bogus".to_string())
        );
    }

    #[test]
    fn config_without_a_path_is_an_error() {
        assert_eq!(
            parse_strs(&["--config"]),
            Err("--config requires a path".to_string())
        );
    }
}
