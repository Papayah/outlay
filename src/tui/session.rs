//! Carries out the effects the editor asks for: apply → verify → countdown → keep or revert,
//! reload, copy. Everything that talks to xrandr, the file system or the terminal's clipboard
//! happens here, behind the [`Backend`] and [`Input`] traits, so tests run the whole flow with
//! a fake backend, a scripted clock and an injected signal flag.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ratatui::crossterm::event::Event;

use crate::model::Snapshot;
use crate::model::validate::Severity;
use crate::xrandr::script::{self, line_diff, profile_path, save_text};
use crate::xrandr::{Backend, command};

use super::app::{App, ApplyRequest, Effect, ProfileItem, RevertReason, SavePlan};

/// Where input comes from, as far as the session cares: after xrandr returns, keys pressed
/// while the screens were dark are thrown away.
pub trait Input {
    fn drain(&mut self);
}

/// How the session applies layouts.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Seconds to keep an applied layout before it reverts; 0 keeps it at once.
    pub revert_seconds: u64,
    /// Where to write `revert.sh` before applying. Only set for a backend that touches X.
    pub revert_file: Option<PathBuf>,
    /// Shell commands run after a layout is kept.
    pub hooks: Vec<String>,
    pub hook_timeout: Duration,
    /// Where profiles live. Unset, `w` and `e` only report that.
    pub layouts_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            revert_seconds: 15,
            revert_file: None,
            hooks: Vec::new(),
            hook_timeout: Duration::from_secs(10),
            layouts_dir: None,
        }
    }
}

/// A path as the status line shows it: `~/.screenlayout/home.sh`.
pub fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `$XDG_STATE_HOME/outlay/revert.sh`
pub fn default_revert_file() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("outlay").join("revert.sh"))
}

/// An apply waiting for its answer.
struct Applied {
    /// The live state just before the apply; the revert restores it.
    before: Snapshot,
    /// The state read back after it.
    after: Snapshot,
}

pub struct Session<'a> {
    pub app: App,
    backend: &'a dyn Backend,
    settings: Settings,
    signal: Arc<AtomicBool>,
    applied: Option<Applied>,
    queue: VecDeque<Effect>,
    /// Bytes for the terminal, written between two draws (OSC 52).
    pub outbox: Vec<u8>,
    pub quit: bool,
    /// Whether the last applied layout was kept.
    pub kept: bool,
}

impl<'a> Session<'a> {
    pub fn new(
        app: App,
        backend: &'a dyn Backend,
        settings: Settings,
        signal: Arc<AtomicBool>,
    ) -> Self {
        Self {
            app,
            backend,
            settings,
            signal,
            applied: None,
            queue: VecDeque::new(),
            outbox: Vec::new(),
            quit: false,
            kept: false,
        }
    }

    /// Hands one terminal event to the editor at time `now`.
    pub fn handle_event(&mut self, event: &Event, now: Instant) {
        let effects = self.app.tick(now);
        self.queue.extend(effects);
        if let Event::Key(key) = event {
            let effects = self.app.handle_key(*key);
            self.queue.extend(effects);
        }
    }

    /// Advances the clock without input; the countdown may run out.
    pub fn tick(&mut self, now: Instant) {
        let effects = self.app.tick(now);
        self.queue.extend(effects);
    }

    /// Queues work, as a key would: `outlay apply` asks for the apply and the answer this way.
    pub fn push(&mut self, effect: Effect) {
        self.queue.push_back(effect);
    }

    /// Whether effects are waiting. The loop draws once before performing them, so "applying…"
    /// is on screen while xrandr runs.
    pub fn has_work(&self) -> bool {
        !self.queue.is_empty() || self.signalled()
    }

    pub fn signalled(&self) -> bool {
        self.signal.load(Ordering::SeqCst)
    }

    /// Whether an applied layout is still waiting for its answer.
    pub fn awaiting_answer(&self) -> bool {
        self.applied.is_some()
    }

    /// Carries out the queued effects. A SIGHUP or SIGTERM reverts an unconfirmed apply first,
    /// then quits.
    pub fn perform(&mut self, input: &mut dyn Input) {
        if self.signalled() {
            self.queue.clear();
            if self.applied.is_some() {
                self.revert(RevertReason::Signal);
            }
            self.quit = true;
            return;
        }
        while let Some(effect) = self.queue.pop_front() {
            match effect {
                Effect::Quit => {
                    self.quit = true;
                    self.queue.clear();
                }
                Effect::Query => match self.backend.query() {
                    Ok(snap) => self.app.reloaded(snap),
                    Err(err) => self
                        .app
                        .say(Severity::Error, format!("Reload failed: {err:#}")),
                },
                Effect::Apply(request) => self.apply(request, input),
                Effect::Revert(reason) => self.revert(reason),
                Effect::Keep => self.keep(),
                Effect::Copy(text) => {
                    self.outbox.extend(osc52(&text).into_bytes());
                    self.app.say(
                        Severity::Info,
                        "Copied the xrandr command to the clipboard (OSC 52).",
                    );
                }
                Effect::ListProfiles => self.list_profiles(),
                Effect::OpenProfile(name) => self.open_profile(&name),
                Effect::SaveProfile(name) => self.save_profile(&name),
                Effect::WriteProfile(plan) => self.write_profile(&plan),
            }
        }
    }

    fn layouts_dir(&mut self) -> Option<PathBuf> {
        if self.settings.layouts_dir.is_none() {
            self.app
                .say(Severity::Warning, "No layouts directory is set.");
        }
        self.settings.layouts_dir.clone()
    }

    fn list_profiles(&mut self) {
        let Some(dir) = self.layouts_dir() else {
            return;
        };
        let paths = match script::list_profiles(&dir) {
            Ok(paths) => paths,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                let text = format!("Could not read {}: {err}.", tilde(&dir));
                self.app.say(Severity::Error, text);
                return;
            }
        };
        let items = paths
            .into_iter()
            .filter_map(|path| {
                let text = std::fs::read_to_string(&path).ok()?;
                Some(ProfileItem::new(&self.app.snap, path, &text))
            })
            .collect();
        self.app.open_profiles(items, &tilde(&dir));
    }

    fn open_profile(&mut self, name: &str) {
        let Some(dir) = self.layouts_dir() else {
            return;
        };
        let path = profile_path(&dir, name);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let item = ProfileItem::new(&self.app.snap, path, &text);
                self.app.open_profile(item);
            }
            Err(err) => {
                let text = format!("Could not read {}: {err}.", tilde(&path));
                self.app.say(Severity::Error, text);
            }
        }
    }

    /// Saves the pending layout. An existing file keeps its other lines; when the result
    /// differs from it, the editor asks first and shows the diff.
    fn save_profile(&mut self, name: &str) {
        let Some(dir) = self.layouts_dir() else {
            return;
        };
        let path = profile_path(&dir, name);
        let old = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => {
                let text = format!("Could not read {}: {err}.", tilde(&path));
                self.app.say(Severity::Error, text);
                return;
            }
        };
        let command = command::script_command(&self.app.layout, &self.app.snap);
        let text = save_text(old.as_deref(), &command);
        match old {
            Some(old) if old == text => {
                let text = format!("{} is up to date.", tilde(&path));
                self.app.say(Severity::Info, text);
            }
            Some(old) => self.app.confirm_overwrite(SavePlan {
                diff: line_diff(&old, &text),
                path,
                text,
            }),
            None => self.write_profile(&SavePlan {
                path,
                text,
                diff: Vec::new(),
            }),
        }
    }

    fn write_profile(&mut self, plan: &SavePlan) {
        match script::write_atomic(&plan.path, &plan.text, 0o755) {
            Ok(()) => self
                .app
                .saved(script::profile_name(&plan.path), &tilde(&plan.path)),
            Err(err) => {
                let text = format!("Could not write {}: {err}.", tilde(&plan.path));
                self.app.say(Severity::Error, text);
            }
        }
    }

    /// Reverts an apply that is still waiting for its answer. The loop calls this on the way
    /// out, whatever made it stop, so no unconfirmed layout outlives the editor.
    pub fn revert_if_waiting(&mut self) {
        if self.applied.is_some() {
            self.revert(RevertReason::Signal);
        }
    }

    fn apply(&mut self, request: ApplyRequest, input: &mut dyn Input) {
        let before = match self.backend.requery() {
            Ok(snap) => snap,
            Err(err) => {
                self.app.report(
                    "Apply failed",
                    vec![format!("Could not read the live state: {err:#}")],
                );
                return;
            }
        };
        let revert_args = command::revert_args(&before);
        if let Some(path) = &self.settings.revert_file {
            if let Err(err) =
                script::write_atomic(path, &command::revert_script(&revert_args), 0o755)
            {
                self.app.report(
                    "Apply failed",
                    vec![format!(
                        "Could not write {}: {err}. Nothing was applied.",
                        path.display()
                    )],
                );
                return;
            }
            super::arm_panic_revert(Some(path.clone()));
        }

        let outcome = self.backend.apply(&request.argv);
        // Keys pressed while the screens were dark must not answer the countdown.
        input.drain();
        let now = Instant::now();
        let after = self.backend.requery();

        let mut problems = Vec::new();
        match &outcome {
            Ok(o) if o.success => {}
            Ok(_) => problems.push("xrandr reported an error.".to_owned()),
            Err(err) => problems.push(format!("Could not run xrandr: {err:#}")),
        }
        if let Ok(o) = &outcome {
            problems.extend(
                o.stderr
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| format!("xrandr: {}", l.trim())),
            );
        }
        let failed_run = !matches!(&outcome, Ok(o) if o.success);
        let after = match after {
            Ok(snap) => snap,
            Err(err) => {
                problems.push(format!("Could not read the state back: {err:#}"));
                // Assume the worst: something changed.
                self.applied = Some(Applied {
                    before: before.clone(),
                    after: before,
                });
                self.revert(RevertReason::Declined);
                self.app.report("Apply failed", problems);
                return;
            }
        };
        let mismatches = request.layout.mismatches(&after);
        if failed_run || !mismatches.is_empty() {
            if !mismatches.is_empty() {
                problems.push("The layout did not come out as asked:".to_owned());
                problems.extend(mismatches.iter().map(|m| format!("  {m}")));
            }
            if after != before {
                self.applied = Some(Applied {
                    before,
                    after: after.clone(),
                });
                problems.push(match self.revert_quietly() {
                    Ok(snap) => {
                        if let Some(snap) = snap {
                            self.app.adopt(snap);
                        }
                        "Reverted to the previous layout.".to_owned()
                    }
                    Err(err) => err,
                });
            } else {
                super::arm_panic_revert(None);
                problems.push("Nothing changed.".to_owned());
            }
            self.app.report("Apply failed", problems);
            return;
        }

        self.applied = Some(Applied { before, after });
        if self.settings.revert_seconds == 0 {
            self.keep();
        } else {
            self.app.countdown(now, self.settings.revert_seconds);
            if !self.backend.touches_x() {
                self.app.say(
                    Severity::Info,
                    "Simulated: nothing was sent to the X server.",
                );
            }
        }
    }

    /// Runs the revert command and reads the state back. On failure the message says how to
    /// restore by hand.
    fn revert_quietly(&mut self) -> Result<Option<Snapshot>, String> {
        let Some(applied) = self.applied.take() else {
            return Ok(None);
        };
        let args = command::revert_args(&applied.before);
        let outcome = self.backend.apply(&args);
        super::arm_panic_revert(None);
        let snap = self.backend.requery().ok();
        match outcome {
            Ok(o) if o.success => Ok(snap),
            other => {
                let why = match other {
                    Ok(o) => o.stderr.trim().to_owned(),
                    Err(err) => format!("{err:#}"),
                };
                let hint = match &self.settings.revert_file {
                    Some(path) => format!(" Run {} to restore it.", path.display()),
                    None => String::new(),
                };
                Err(format!("The revert failed: {why}.{hint}"))
            }
        }
    }

    fn revert(&mut self, reason: RevertReason) {
        if self.applied.is_none() {
            return;
        }
        self.kept = false;
        match self.revert_quietly() {
            Ok(snap) => self
                .app
                .reverted(snap, reason, self.settings.revert_seconds),
            Err(err) => self.app.report("Revert failed", vec![err]),
        }
    }

    fn keep(&mut self) {
        let Some(applied) = self.applied.take() else {
            return;
        };
        super::arm_panic_revert(None);
        self.kept = true;
        self.app.kept(applied.after);
        let failures = run_hooks(&self.settings.hooks, self.settings.hook_timeout);
        if !failures.is_empty() {
            self.app.say(Severity::Warning, failures.join(" "));
        }
    }
}

/// The OSC 52 sequence that puts `text` on the clipboard.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// Runs each hook with `sh -c`, with no terminal attached and a time limit. Returns one message
/// per hook that failed.
pub fn run_hooks(hooks: &[String], timeout: Duration) -> Vec<String> {
    let mut failures = Vec::new();
    for hook in hooks {
        let child = Command::new("sh")
            .arg("-c")
            .arg(hook)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(err) => {
                failures.push(format!("post_apply `{hook}` could not start: {err}."));
                continue;
            }
        };
        // Read stderr on the side, so a chatty hook cannot fill the pipe and stall.
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let text = reader.join().unwrap_or_default();
        match status {
            Some(s) if s.success() => {}
            Some(s) => {
                let first = text.lines().next().unwrap_or("").trim();
                let detail = if first.is_empty() {
                    s.to_string()
                } else {
                    first.to_owned()
                };
                failures.push(format!("post_apply `{hook}` failed: {detail}."));
            }
            None => failures.push(format!(
                "post_apply `{hook}` took longer than {} s and was stopped.",
                timeout.as_secs()
            )),
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_encodes_the_text() {
        assert_eq!(osc52("xrandr"), "\x1b]52;c;eHJhbmRy\x07");
    }

    #[test]
    fn hooks_report_failures_and_timeouts() {
        let hooks = vec![
            "true".to_owned(),
            "echo broken >&2; exit 3".to_owned(),
            "sleep 5".to_owned(),
        ];
        let failures = run_hooks(&hooks, Duration::from_millis(300));
        assert_eq!(
            failures,
            [
                "post_apply `echo broken >&2; exit 3` failed: broken.",
                "post_apply `sleep 5` took longer than 0 s and was stopped.",
            ]
        );
    }
}
