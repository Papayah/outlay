//! `outlay apply` and `outlay save`: profiles from the shell. An apply goes through the same
//! [`Session`] as the editor's, so it is verified, reverts unless kept, and runs the hooks.

use std::io::{self, BufRead, IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

use crate::cli::Cli;
use crate::config::Config;
use crate::model::layout::Layout;
use crate::model::validate::{Severity, validate};
use crate::tui::app::{App, ApplyRequest, Effect, ProfileItem, RevertReason, UiMode};
use crate::tui::session::{Input, Session, tilde};
use crate::xrandr::script::{line_diff, profile_path, save_text, write_atomic};
use crate::xrandr::{Backend, command};

/// Writes a line to stderr. After a SIGHUP there is no terminal, and `eprintln!` would panic.
fn say(text: impl AsRef<str>) {
    let _ = writeln!(io::stderr(), "{}", text.as_ref());
}

/// There are no queued keys to throw away: the countdown reads whole lines, and ignores the
/// ones typed in the first second.
struct Lines;

impl Input for Lines {
    fn drain(&mut self) {}
}

/// `outlay apply PROFILE`
pub fn apply(cli: &Cli, config: &Config, backend: &dyn Backend, name: &str) -> Result<()> {
    let path = profile_path(&cli.layouts_dir(config), name);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let snap = backend.query()?;
    let item = ProfileItem::new(&snap, path, &text);
    let remap = item.profile.default_remap(&snap);
    for (from, to) in &remap {
        if let Some(i) = to {
            say(format!(
                "{from} is not connected; using {} instead.",
                snap.outputs[*i].name
            ));
        }
    }
    let (layout, notes) = item.profile.layout(&snap, &remap);
    for note in &notes {
        say(note);
    }
    let errors: Vec<String> = validate(&layout, &snap)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.message)
        .collect();
    if !errors.is_empty() {
        bail!("{} cannot be applied: {}", item.name, errors.join(" "));
    }
    let argv = command::apply_args(&layout, &snap);
    if cli.dry_run {
        println!("{}", command::command_line(&argv));
        return Ok(());
    }
    let changes = layout.diff(&snap);
    if changes.is_empty() {
        say(format!(
            "Nothing to change: the layout is {} already.",
            item.name
        ));
        return Ok(());
    }
    for change in &changes {
        say(format!("  {change}"));
    }

    let (options, mut settings) = cli.tui_options(config)?;
    if !backend.touches_x() {
        settings.revert_file = None;
    }
    let seconds = settings.revert_seconds;
    let mut app = App::new(snap, options);
    app.load_profile(&item, &remap);
    let signal = Arc::new(AtomicBool::new(false));
    for sig in [SIGHUP, SIGTERM, SIGINT] {
        signal_hook::flag::register(sig, Arc::clone(&signal))?;
    }
    let mut session = Session::new(app, backend, settings, signal);
    let layout = session.app.layout.clone();
    session.push(Effect::Apply(ApplyRequest { argv, layout }));
    session.perform(&mut Lines);
    if let UiMode::Message(message) = &session.app.mode {
        for line in &message.lines {
            say(line);
        }
        bail!("{}", message.title.to_lowercase());
    }
    if session.awaiting_answer() {
        countdown(&mut session, seconds);
    }
    if let Some(status) = &session.app.status {
        say(&status.text);
    }
    if session.kept {
        Ok(())
    } else {
        bail!("the layout was not kept")
    }
}

/// "Keep this layout?" on the terminal: `y` and Enter keeps it; anything else, the timeout, or
/// a signal reverts it.
fn countdown(session: &mut Session, seconds: u64) {
    let mut hint = String::new();
    if !io::stdin().is_terminal() {
        hint = " (stdin is not a terminal; --revert-timeout 0 keeps a layout without asking)"
            .to_owned();
    }
    say(format!(
        "Keep this layout? Type y and Enter within {seconds} s; anything else reverts it.{hint}"
    ));
    let blocked_until = match &session.app.mode {
        UiMode::Countdown(c) => c.blocked_until,
        _ => Instant::now(),
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    loop {
        if session.signalled() {
            session.perform(&mut Lines);
            return;
        }
        session.tick(Instant::now());
        if session.has_work() {
            session.perform(&mut Lines);
            return;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            // Typed while the screens were dark: not an answer.
            Ok(Ok(_)) if Instant::now() < blocked_until => {}
            Ok(Ok(line)) => {
                let effect = if line.trim().eq_ignore_ascii_case("y") {
                    Effect::Keep
                } else {
                    Effect::Revert(RevertReason::Declined)
                };
                session.push(effect);
                session.perform(&mut Lines);
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
            // No more input: the timeout decides.
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// `outlay save PROFILE`
pub fn save(
    cli: &Cli,
    config: &Config,
    backend: &dyn Backend,
    name: &str,
    force: bool,
) -> Result<()> {
    let snap = backend.query()?;
    let layout = Layout::inferred(&snap);
    let path = profile_path(&cli.layouts_dir(config), name);
    let old = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("could not read {}", path.display())),
    };
    let text = save_text(old.as_deref(), &command::script_command(&layout, &snap));
    if cli.dry_run {
        print!("{text}");
        return Ok(());
    }
    if let Some(old) = &old {
        if *old == text {
            say(format!("{} is up to date.", tilde(&path)));
            return Ok(());
        }
        for line in line_diff(old, &text) {
            say(line);
        }
        if !force {
            if !io::stdin().is_terminal() {
                bail!(
                    "{} exists and differs; pass --force to overwrite it",
                    path.display()
                );
            }
            let _ = write!(io::stderr(), "Overwrite {}? [y/N] ", tilde(&path));
            let _ = io::stderr().flush();
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            if !answer.trim().eq_ignore_ascii_case("y") {
                bail!("not saved");
            }
        }
    }
    write_atomic(&path, &text, 0o755)
        .with_context(|| format!("could not write {}", path.display()))?;
    say(format!("Saved {}.", tilde(&path)));
    Ok(())
}
