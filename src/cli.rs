//! Command-line interface.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::xrandr::{Backend, FixtureBackend, XrandrCli};

#[derive(Debug, Parser)]
#[command(
    name = "outlay",
    version,
    about = "Keyboard-driven xrandr layout editor"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Read state from an `xrandr --verbose` capture; never touches X
    #[arg(
        long,
        global = true,
        value_name = "XRANDR-VERBOSE.TXT",
        conflicts_with = "demo"
    )]
    pub from_file: Option<PathBuf>,

    /// Use the built-in four-output fixture; never touches X
    #[arg(long, global = true)]
    pub demo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Print the output table, then exit
    Show,
    /// List outputs, then resolutions with their rates
    List,
}

impl Cli {
    /// The backend the global flags select: a fixture for `--demo` and `--from-file`, else the
    /// real `xrandr`.
    pub fn backend(&self) -> Result<Box<dyn Backend>> {
        if self.demo {
            return Ok(Box::new(FixtureBackend::demo()));
        }
        if let Some(path) = &self.from_file {
            return Ok(Box::new(FixtureBackend::from_file(path)?));
        }
        Ok(Box::new(XrandrCli::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_flags_work_after_the_subcommand() {
        let cli = Cli::try_parse_from(["outlay", "list", "--demo"]).unwrap();
        assert_eq!(cli.command, Some(Command::List));
        assert!(cli.demo);
        assert!(Cli::try_parse_from(["outlay", "--demo", "--from-file", "x.txt"]).is_err());
    }
}
