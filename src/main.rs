use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

use outlay::cli::{Cli, Command};
use outlay::show;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("outlay: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let snapshot = cli.backend()?.query()?;
    // Until the interactive editor exists, a bare `outlay` prints the table.
    let text = match cli.command.unwrap_or(Command::Show) {
        Command::Show => show::table(&snapshot),
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
