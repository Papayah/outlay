use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use outlay::cli::{Cli, Command};
use outlay::config::Config;
use outlay::show;
use outlay::tui;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // After a SIGHUP there is no terminal left to report to; eprintln! would panic.
            let _ = writeln!(io::stderr(), "outlay: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let config = Config::load()?;
    let backend = cli.backend()?;
    let Some(command) = cli.command else {
        let (options, settings) = cli.tui_options(&config)?;
        return tui::run(backend.as_ref(), options, settings);
    };
    let snapshot = backend.query()?;
    let text = match command {
        Command::Show => show::show(
            &snapshot,
            &show::DiagramOptions::for_stdout(config.cell_aspect),
        ),
        Command::List => show::list(&snapshot),
    };
    print_stdout(&text)
}

/// Writes to stdout, treating a closed pipe (`outlay list | head`) as success.
fn print_stdout(text: &str) -> Result<()> {
    match io::stdout().lock().write_all(text.as_bytes()) {
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}
