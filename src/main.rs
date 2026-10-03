use std::io::{self, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, Parser};

use outlay::cli::{Cli, Command};
use outlay::config::Config;
use outlay::show;
use outlay::{profile, tui};

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
    if let Some(Command::Completions { shell }) = &cli.command {
        let mut out = Vec::new();
        clap_complete::generate(*shell, &mut Cli::command(), "outlay", &mut out);
        return print_stdout(&String::from_utf8_lossy(&out));
    }
    let config = Config::load()?;
    if let Some(Command::Keys) = &cli.command {
        return print_stdout(&config.keymap()?.reference_text());
    }
    let backend = cli.backend()?;
    let Some(command) = cli.command.clone() else {
        let (options, settings) = cli.tui_options(&config)?;
        return tui::run(backend.as_ref(), options, settings);
    };
    let text = match &command {
        Command::Apply { profile } => {
            return profile::apply(&cli, &config, backend.as_ref(), profile);
        }
        Command::Save { profile, force } => {
            return profile::save(&cli, &config, backend.as_ref(), profile, *force);
        }
        Command::Show => show::show(
            &backend.query()?,
            &show::DiagramOptions::for_stdout(config.cell_aspect),
        ),
        Command::List => show::list(&backend.query()?),
        Command::Dump => backend.dump()?,
        Command::Keys | Command::Completions { .. } => unreachable!("handled above"),
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
