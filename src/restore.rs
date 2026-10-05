//! `outlay restore CAPTURE`: puts back, at once, the layout a capture describes. The Wayland
//! `revert.sh` runs it with the state from before an apply. It reads no config file, so a broken
//! one cannot block a revert, and it has no countdown, writes no revert file and runs no hooks.
//! It reads the state back afterwards, and anything that did not come back is an error, except
//! a display that was unplugged meanwhile.

use std::io::{self, Read, Write};

use anyhow::{Context, Result, bail};

use crate::backend::{Plan, PrimaryRule, parse_capture};
use crate::cli::Cli;
use crate::model::layout::{restore_mismatches, unplugged};

/// Restores the capture in the file `source`, or on stdin for `-`. Problems go to stderr, and
/// any problem is an error.
pub fn restore(cli: &Cli, source: &str) -> Result<()> {
    let text = if source == "-" {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .context("could not read the capture from stdin")?;
        text
    } else {
        std::fs::read_to_string(source).with_context(|| format!("could not read {source}"))?
    };
    let captured = parse_capture(&text).context("could not read the capture")?;
    let backend = cli.backend()?;
    let live = backend.query()?;
    if live.caps.kind != captured.caps.kind {
        bail!(
            "the capture is from {}, but this session is {}",
            captured.caps.kind.name(),
            live.caps.kind.name()
        );
    }
    let mut plan = Plan::restore(&captured);
    // A display unplugged since the capture cannot come back, and xrandr would warn about it.
    plan.outputs.retain(|p| live.find(&p.name).is_some());
    if let PrimaryRule::Output(name) = &plan.primary
        && live.find(name).is_none()
    {
        plan.primary = PrimaryRule::Keep;
    }
    let outcome = backend.apply(&plan)?;
    let problems: Vec<&str> = outcome
        .stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    for problem in &problems {
        // After a SIGHUP there is no terminal; eprintln! would panic.
        let _ = writeln!(io::stderr(), "{problem}");
    }
    if !outcome.success {
        bail!("the restore failed");
    }
    // The display server's word is not enough: read the state back and compare.
    let after = backend
        .requery()
        .context("the restore ran, but the state could not be read back")?;
    for name in unplugged(&captured, &after) {
        let _ = writeln!(io::stderr(), "{name} was unplugged, so it is not back.");
    }
    let mismatches = restore_mismatches(&captured, &after);
    for mismatch in &mismatches {
        let _ = writeln!(io::stderr(), "{mismatch}");
    }
    if !problems.is_empty() || !mismatches.is_empty() {
        bail!("the restore left something out");
    }
    Ok(())
}
