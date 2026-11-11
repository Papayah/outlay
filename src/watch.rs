//! `outlay watch`: applies a saved layout when displays come or go, as autorandr does, and never
//! leaves the laptop panel dark after the last external display goes (`docs/PLAN-auto.md`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

use crate::backend::{Backend, Plan, command_text};
use crate::cli::Cli;
use crate::config::Config;
use crate::model::layout::Layout;
use crate::model::validate::{Severity, validate};
use crate::model::{Kind, Snapshot};
use crate::profile::{carry_out, say};
use crate::profiles::{ProfileStore, Stored};
use crate::tui::app::{App, Options};
use crate::tui::confine;
use crate::tui::session::{Settings, default_revert_file};
use crate::wayland::built_in;

/// How long to wait between two readings.
pub const INTERVAL: Duration = Duration::from_secs(2);

/// What the service would do for one set of displays.
#[derive(Debug)]
pub struct Decision {
    /// The profile that fits, if one does.
    pub profile: Option<String>,
    /// Other profiles that fit too, passed over.
    pub others: Vec<String>,
    pub layout: Layout,
    /// What the layout changed to fit, and the fail-safe.
    pub notes: Vec<String>,
}

impl Decision {
    /// One line for the log.
    pub fn summary(&self) -> String {
        let mut text = match &self.profile {
            Some(name) => format!("{name} fits."),
            None => "No profile fits.".to_owned(),
        };
        if !self.others.is_empty() {
            text.push_str(&format!(" Also fits: {}.", self.others.join(", ")));
        }
        for note in &self.notes {
            text.push(' ');
            text.push_str(note);
        }
        text
    }
}

/// The layout for `snap`: on X11 the first of `profiles` that fits, else the live layout with
/// unplugged outputs off. A built-in panel that is the only connected display is always on.
pub fn decide(snap: &Snapshot, profiles: &[Stored]) -> Decision {
    // On Wayland kanshi switches profiles; outlay only keeps the panel on.
    let mut fitting = profiles
        .iter()
        .filter(|_| snap.caps.kind == Kind::X11)
        .filter_map(|s| s.profile.fit(snap).map(|remap| (s, remap)));
    let first = fitting.next();
    let others = fitting.map(|(s, _)| s.name.clone()).collect();
    let (profile, (mut layout, mut notes)) = match first {
        Some((s, remap)) => (Some(s.name.clone()), s.profile.layout(snap, &remap)),
        None => (None, Layout::from_snapshot(snap)),
    };
    let panel = snap
        .outputs
        .iter()
        .position(|o| o.is_connected() && built_in(&o.name));
    let external = snap
        .outputs
        .iter()
        .any(|o| o.is_connected() && !built_in(&o.name));
    if let Some(p) = panel.filter(|&p| !external && !layout.is_enabled(p)) {
        (layout, notes) = Layout::from_snapshot(snap);
        if layout.turn_on(snap, p).is_ok() {
            notes.push(format!(
                "Only {} is connected: turning it on.",
                layout.names[p]
            ));
        }
        // Outputs left on after their monitor went: the panel now keeps the screen lit.
        for d in 0..layout.len() {
            if layout.is_enabled(d) && snap.outputs[d].is_stale() && layout.turn_off(d).is_ok() {
                notes.push(format!(
                    "{} is disconnected; turning it off.",
                    layout.names[d]
                ));
            }
        }
    }
    Decision {
        profile,
        others,
        layout,
        notes,
    }
}

/// The connected outputs, by name and monitor. Positions are left out, so a layout changed by
/// hand stays until a display comes or goes.
fn connections(snap: &Snapshot) -> Vec<(String, String)> {
    snap.outputs
        .iter()
        .filter(|o| o.is_connected())
        .map(|o| (o.name.clone(), crate::model::profile::description(o)))
        .collect()
}

/// The service's state between rounds.
pub struct Watch {
    options: Options,
    settings: Settings,
    dry_run: bool,
    signal: Arc<AtomicBool>,
    /// The connections of the last reading.
    seen: Option<Vec<(String, String)>>,
    /// The connections the service last acted on.
    acted: Option<Vec<(String, String)>>,
}

impl Watch {
    /// `settings.layouts_dir` holds the profiles on X11. With `dry_run` an action prints the
    /// command and applies nothing.
    pub fn new(
        options: Options,
        settings: Settings,
        dry_run: bool,
        signal: Arc<AtomicBool>,
    ) -> Self {
        Self {
            options,
            settings,
            dry_run,
            signal,
            seen: None,
            acted: None,
        }
    }

    /// One round: reads the displays, and acts once two readings in a row show a set of
    /// connections it has not acted on yet. The first set counts as new.
    pub fn step(&mut self, backend: &dyn Backend) -> Result<()> {
        let set = connections(&backend.requery()?);
        let settled = self.seen.as_ref() == Some(&set);
        self.seen = Some(set.clone());
        if !settled || self.acted.as_ref() == Some(&set) {
            return Ok(());
        }
        let snap = backend.query()?;
        // A failed apply is not tried again until the displays change.
        self.acted = Some(set);
        self.act(backend, snap)
    }

    /// Decides for `snap` and applies the result, kept at once.
    pub fn act(&self, backend: &dyn Backend, snap: Snapshot) -> Result<()> {
        let profiles = match (snap.caps.kind, &self.settings.layouts_dir) {
            (Kind::X11, Some(dir)) => ProfileStore::Screenlayout(dir.clone()).list()?,
            _ => Vec::new(),
        };
        let decision = decide(&snap, &profiles);
        say(decision.summary());
        if decision.layout.diff(&snap).is_empty() {
            say("Nothing to change.");
            return Ok(());
        }
        let errors: Vec<String> = validate(&decision.layout, &snap)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| i.message)
            .collect();
        if !errors.is_empty() {
            bail!("the layout cannot be applied: {}", errors.join(" "));
        }
        let plan = Plan::pending(&decision.layout, &snap);
        if self.dry_run {
            println!("{}", command_text(snap.caps.kind, &plan));
            return Ok(());
        }
        let (options, settings) = confine(backend, self.options.clone(), self.settings.clone());
        let mut app = App::new(snap, options);
        app.layout = decision.layout;
        carry_out(app, plan, backend, settings, Arc::clone(&self.signal))
    }
}

/// `outlay watch`: with `once`, decides for the displays as they are, applies, and exits.
pub fn watch(cli: &Cli, config: &Config, once: bool) -> Result<()> {
    let (mut options, mut settings) = cli.tui_options(config)?;
    options.watch = None;
    settings.revert_seconds = 0;
    // Its own file, so it never overwrites the editor's while the editor waits for an answer.
    settings.revert_file = default_revert_file().map(|p| p.with_file_name("watch-revert.sh"));
    let signal = Arc::new(AtomicBool::new(false));
    for sig in [SIGHUP, SIGTERM, SIGINT] {
        signal_hook::flag::register(sig, Arc::clone(&signal))?;
    }
    let mut watch = Watch::new(options, settings, cli.dry_run, Arc::clone(&signal));
    if once {
        let backend = cli.backend()?;
        return watch.act(backend.as_ref(), backend.query()?);
    }
    let mut last_error = String::new();
    while !signal.load(Ordering::SeqCst) {
        // A fresh backend each round: it survives a compositor restart, and on Wayland a
        // long-lived second client can make wlroots abort (MISTAKES.md).
        match cli.backend().and_then(|b| watch.step(b.as_ref())) {
            Ok(()) => last_error.clear(),
            Err(err) => {
                let text = format!("{err:#}");
                if text != last_error {
                    say(&text);
                }
                last_error = text;
            }
        }
        // shortcut: polls every 2 s, replace with RandR events (x11rb) per ROADMAP
        std::thread::sleep(INTERVAL);
    }
    Ok(())
}

/// The systemd user unit that runs this binary as the service.
pub fn unit() -> Result<String> {
    let exe = std::env::current_exe()?;
    Ok(format!(
        "[Unit]\n\
         Description=outlay: apply saved monitor layouts when displays change\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart=\"{}\" watch\n\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        exe.display()
    ))
}
