//! Carries out the effects the editor asks for: apply → verify → countdown → keep or revert,
//! refresh, copy. Everything that talks to the display server, the file system or the terminal's
//! clipboard happens here, behind the [`Backend`] and [`Input`] traits, so tests run the whole flow with
//! a fake backend, a scripted clock and an injected signal flag.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ratatui::crossterm::event::Event;

use crate::backend::{Backend, Plan, Verdict};
use crate::files::{line_diff, write_atomic};
use crate::model::layout::{restore_mismatches, unplugged};
use crate::model::validate::Severity;
use crate::model::{Kind, Snapshot};
use crate::profiles::ProfileStore;
use crate::wayland;
use crate::xrandr::command;

use super::app::{App, ApplyRequest, Effect, Message, ProfileItem, RevertReason, SavePlan, UiMode};

/// Where input comes from, as far as the session cares: after an apply returns, keys pressed
/// while the screens were dark are thrown away.
pub trait Input {
    fn drain(&mut self);
}

/// How the session applies layouts.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Seconds to keep an applied layout before it reverts; 0 keeps it at once.
    pub revert_seconds: u64,
    /// Where to write `revert.sh` before applying. Only set for a live backend.
    pub revert_file: Option<PathBuf>,
    /// Shell commands run after every change outlay makes to the screens: once an apply checks
    /// out (before the countdown), and after every revert. Never set for a simulated backend.
    pub hooks: Vec<String>,
    /// How long each hook may run before it is stopped.
    pub hook_timeout: Duration,
    /// Where screenlayout scripts live (X11). Unset, `w` and `e` only report that.
    pub layouts_dir: Option<PathBuf>,
    /// kanshi's config file (Wayland). Unset, `w` and `e` only report that.
    pub kanshi_config: Option<PathBuf>,
    /// The program a Wayland `revert.sh` runs (`PROGRAM restore -`): outlay itself. Unset, it
    /// is `outlay` on `PATH`.
    pub restore_program: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            revert_seconds: 15,
            revert_file: None,
            hooks: Vec::new(),
            hook_timeout: Duration::from_secs(10),
            layouts_dir: None,
            kanshi_config: None,
            restore_program: None,
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

/// A revert that did not bring back the state from before the apply.
struct RevertFailure {
    /// What went wrong, ending with how to restore by hand when there is a `revert.sh`.
    lines: Vec<String>,
    /// The state read back after the revert, if the read worked.
    snap: Option<Snapshot>,
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
    /// The `revert.sh` the panic hook runs, if it is armed.
    panic_revert: Option<PathBuf>,
    /// How many reports had opened when the screen was last drawn.
    drawn_reports: u64,
}

impl<'a> Session<'a> {
    pub fn new(
        app: App,
        backend: &'a dyn Backend,
        settings: Settings,
        signal: Arc<AtomicBool>,
    ) -> Self {
        let app_reports = app.reports();
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
            panic_revert: None,
            drawn_reports: app_reports,
        }
    }

    /// The editor drew the screen: the report open now, if any, has been shown.
    pub fn drawn(&mut self) {
        self.drawn_reports = self.app.reports();
    }

    /// The report open now if the screen has not shown it: one made by the last effects or on
    /// the way out, such as a revert on Ctrl-C or a signal that failed. The editor prints it
    /// once the alternate screen has taken the popup away.
    pub fn unseen_report(&self) -> Option<&Message> {
        match &self.app.mode {
            UiMode::Message(message) if self.app.reports() != self.drawn_reports => Some(message),
            _ => None,
        }
    }

    /// The `revert.sh` the panic hook runs now, if it is armed.
    pub fn panic_revert(&self) -> Option<&Path> {
        self.panic_revert.as_deref()
    }

    /// Arms (`Some`) or disarms the panic hook's revert, and remembers which.
    fn arm_panic_revert(&mut self, script: Option<PathBuf>) {
        self.panic_revert.clone_from(&script);
        super::arm_panic_revert(script);
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
    /// is on screen while the apply runs.
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
                Effect::Refresh { probe: true } => match self.backend.query() {
                    Ok(snap) => self.app.refreshed(snap, true),
                    Err(err) => self
                        .app
                        .say(Severity::Error, format!("Refresh failed: {err:#}")),
                },
                // The watch: a re-query that does not probe, so it never wakes a sleeping GPU.
                // A failed read is tried again on the next round.
                Effect::Refresh { probe: false } => {
                    if let Ok(snap) = self.backend.requery() {
                        self.app.refreshed(snap, false);
                    }
                }
                Effect::Test(plan) => {
                    let verdict = self
                        .backend
                        .test(&plan)
                        .unwrap_or_else(|err| Verdict::Rejected(format!("{err:#}")));
                    self.app.tested(verdict);
                }
                Effect::Apply(request) => self.apply(request, input),
                Effect::Revert(reason) => self.revert(reason),
                Effect::Keep => self.keep(),
                Effect::Copy(text) => {
                    self.outbox.extend(osc52(&text).into_bytes());
                    self.app.say(
                        Severity::Info,
                        "Copied the command to the clipboard (OSC 52).",
                    );
                }
                Effect::ListProfiles => self.list_profiles(),
                Effect::OpenProfile(name) => self.open_profile(&name),
                Effect::SaveProfile(name) => self.save_profile(&name),
                Effect::WriteProfile(plan) => self.write_profile(&plan),
            }
        }
    }

    /// Where profiles live for the display server the editor shows; says why when there is no
    /// such place.
    fn store(&mut self) -> Option<ProfileStore> {
        let kind = self.app.snap.caps.kind;
        let settings = &self.settings;
        match ProfileStore::for_kind(
            kind,
            settings.layouts_dir.as_deref(),
            settings.kanshi_config.as_deref(),
        ) {
            Ok(store) => Some(store),
            Err(why) => {
                self.app.say(Severity::Warning, why);
                None
            }
        }
    }

    fn list_profiles(&mut self) {
        let Some(store) = self.store() else {
            return;
        };
        let stored = match store.list() {
            Ok(stored) => stored,
            Err(err) => {
                let text = format!("Could not read {}: {err}.", tilde(store.location()));
                self.app.say(Severity::Error, text);
                return;
            }
        };
        let items = stored
            .into_iter()
            .map(|s| ProfileItem::new(&self.app.snap, s))
            .collect();
        self.app.open_profiles(items, &tilde(store.location()));
    }

    fn open_profile(&mut self, name: &str) {
        let Some(store) = self.store() else {
            return;
        };
        match store.read(name) {
            Ok(stored) => {
                let item = ProfileItem::new(&self.app.snap, stored);
                self.app.open_profile(item);
            }
            Err(err) => {
                let text = err.sentence(&tilde(err.path()));
                self.app.say(Severity::Error, text);
            }
        }
    }

    /// Saves the pending layout. An existing file keeps its other lines; when the result
    /// differs from it, the editor asks first and shows the diff.
    fn save_profile(&mut self, name: &str) {
        let Some(store) = self.store() else {
            return;
        };
        let save = match store.save(name, &self.app.layout, &self.app.snap) {
            Ok(save) => save,
            Err(err) => {
                let text = err.sentence(&tilde(err.path()));
                self.app.say(Severity::Error, text);
                return;
            }
        };
        match save.old {
            Some(old) if old == save.text => {
                let text = format!("{} is up to date.", tilde(&save.path));
                self.app.say(Severity::Info, text);
            }
            Some(old) => self.app.confirm_overwrite(SavePlan {
                diff: line_diff(&old, &save.text),
                name: save.name,
                path: save.path,
                text: save.text,
            }),
            None => self.write_profile(&SavePlan {
                name: save.name,
                path: save.path,
                text: save.text,
                diff: Vec::new(),
            }),
        }
    }

    fn write_profile(&mut self, plan: &SavePlan) {
        let Some(store) = self.store() else {
            return;
        };
        match store.write(&plan.path, &plan.text) {
            Ok(()) => self
                .app
                .saved(plan.name.clone(), &tilde(&plan.path), store.after_save()),
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
        // The compositor may know better than validation; then nothing is touched.
        if let Ok(Verdict::Rejected(why)) = self.backend.test(&request.plan) {
            self.app
                .report("Apply failed", vec![why, "Nothing changed.".to_owned()]);
            return;
        }
        let restore = Plan::restore(&before);
        if let Some(path) = &self.settings.revert_file {
            let text = match before.caps.kind {
                Kind::X11 => command::revert_script(&restore),
                Kind::Wayland => {
                    let program = self.settings.restore_program.clone();
                    wayland::revert_script(
                        &program.unwrap_or_else(|| PathBuf::from("outlay")),
                        &before,
                    )
                }
            };
            if let Err(err) = write_atomic(path, &text, 0o755) {
                self.app.report(
                    "Apply failed",
                    vec![format!(
                        "Could not write {}: {err}. Nothing was applied.",
                        path.display()
                    )],
                );
                return;
            }
            self.arm_panic_revert(Some(path.clone()));
        }

        let outcome = self.backend.apply(&request.plan);
        // Keys pressed while the screens were dark must not answer the countdown.
        input.drain();
        let after = self.backend.requery();

        let mut problems = Vec::new();
        let x11 = before.caps.kind == Kind::X11;
        match &outcome {
            Ok(o) if o.success => {}
            Ok(_) if x11 => problems.push("xrandr reported an error.".to_owned()),
            Ok(_) => {}
            Err(err) if x11 => problems.push(format!("Could not run xrandr: {err:#}")),
            Err(err) => problems.push(format!("{err:#}")),
        }
        if let Ok(o) = &outcome {
            // xrandr's messages get its name; the compositor's are sentences of outlay's own.
            let prefix = if x11 { "xrandr: " } else { "" };
            problems.extend(
                o.stderr
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| format!("{prefix}{}", l.trim())),
            );
        }
        let failed_run = !matches!(&outcome, Ok(o) if o.success);
        let after = match after {
            Ok(snap) => snap,
            Err(err) => {
                problems.push(format!("Could not read the state back: {err:#}"));
                // Assume the worst: something changed.
                let applied = Applied {
                    before: before.clone(),
                    after: before,
                };
                self.revert_after_failure(applied, &mut problems);
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
                let applied = Applied {
                    before,
                    after: after.clone(),
                };
                self.revert_after_failure(applied, &mut problems);
            } else {
                self.arm_panic_revert(None);
                problems.push("Nothing changed.".to_owned());
            }
            self.app.report("Apply failed", problems);
            return;
        }

        self.applied = Some(Applied { before, after });
        // Redraw the wallpaper first, so the layout is judged as it will look.
        let failures = self.redraw();
        // The countdown and its input block start once the hooks are done.
        input.drain();
        let now = Instant::now();
        if self.settings.revert_seconds == 0 {
            self.keep();
        } else {
            self.app.countdown(now, self.settings.revert_seconds);
            if !self.backend.is_live() {
                self.app.say(
                    Severity::Info,
                    "Simulated: nothing was sent to the displays.",
                );
            }
        }
        self.warn(&failures);
    }

    /// The automatic revert after a failed apply that changed the screens; what happened goes
    /// into the report.
    fn revert_after_failure(&mut self, applied: Applied, problems: &mut Vec<String>) {
        let reverted = self.revert_quietly(applied);
        let failures = self.redraw();
        match reverted {
            Ok((snap, notes)) => {
                self.app.adopt(snap);
                problems.push("Reverted to the previous layout.".to_owned());
                problems.extend(notes);
            }
            Err(failure) => {
                if let Some(snap) = failure.snap {
                    self.app.adopt(snap);
                }
                problems.extend(failure.lines);
            }
        }
        problems.extend(failures);
    }

    /// Runs the `post_apply` hooks after the screens changed. Returns one message per failure.
    fn redraw(&mut self) -> Vec<String> {
        run_hooks(&self.settings.hooks, self.settings.hook_timeout)
    }

    /// Adds notes and hook failures to the status line, after what it already says.
    fn warn(&mut self, failures: &[String]) {
        if failures.is_empty() {
            return;
        }
        let mut text = failures.join(" ");
        if let Some(status) = self.app.status.take() {
            text = format!("{} {text}", status.text);
        }
        self.app.say(Severity::Warning, text);
    }

    /// Restores the state from before the apply, reads it back and checks it against that
    /// state: the display server's word that it worked is not enough. Returns the state read
    /// back, with a note for each display unplugged meanwhile. On failure the lines say what is
    /// wrong and how to restore by hand, and the panic hook still runs `revert.sh`.
    fn revert_quietly(
        &mut self,
        applied: Applied,
    ) -> Result<(Snapshot, Vec<String>), RevertFailure> {
        let outcome = self.backend.apply(&Plan::restore(&applied.before));
        let after = self.backend.requery();
        let (mut lines, snap) = match (outcome, after) {
            (Ok(o), Ok(snap)) if o.success => {
                let mismatches = restore_mismatches(&applied.before, &snap);
                if mismatches.is_empty() {
                    self.arm_panic_revert(None);
                    let notes = unplugged(&applied.before, &snap)
                        .iter()
                        .map(|name| format!("{name} was unplugged, so it is not back."))
                        .collect();
                    return Ok((snap, notes));
                }
                let mut lines = vec!["The revert did not restore everything:".to_owned()];
                lines.extend(mismatches.iter().map(|m| format!("  {m}")));
                (lines, Some(snap))
            }
            (Ok(o), Err(err)) if o.success => (
                vec![format!(
                    "The revert ran, but the state could not be read back: {err:#}."
                )],
                None,
            ),
            (outcome, after) => {
                let why = match outcome {
                    Ok(o) => o.stderr.trim().to_owned(),
                    Err(err) => format!("{err:#}"),
                };
                (vec![format!("The revert failed: {why}.")], after.ok())
            }
        };
        if let Some(path) = &self.settings.revert_file {
            lines.push(format!("Run {} to restore it.", path.display()));
        }
        Err(RevertFailure { lines, snap })
    }

    fn revert(&mut self, reason: RevertReason) {
        let Some(applied) = self.applied.take() else {
            return;
        };
        self.kept = false;
        let reverted = self.revert_quietly(applied);
        // Even a failed revert may have changed the screens.
        let failures = self.redraw();
        match reverted {
            Ok((snap, mut notes)) => {
                self.app
                    .reverted(snap, reason, self.settings.revert_seconds);
                notes.extend(failures);
                self.warn(&notes);
            }
            Err(failure) => {
                // The editor shows what is on the screens now; the edits stay pending.
                if let Some(snap) = failure.snap {
                    self.app.adopt(snap);
                }
                let mut lines = failure.lines;
                lines.extend(failures);
                self.app.report("Revert failed", lines);
            }
        }
    }

    fn keep(&mut self) {
        let Some(applied) = self.applied.take() else {
            return;
        };
        self.arm_panic_revert(None);
        self.kept = true;
        // Nothing changes on the screens, so the hooks do not run again.
        self.app.kept(applied.after);
    }
}

/// The OSC 52 sequence that puts `text` on the clipboard.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// How long to wait for a finished hook's stderr to close. A hook that starts a background
/// program (`nitrogen --restore &`) exits at once, but the program may keep the pipe open.
const STDERR_GRACE: Duration = Duration::from_millis(200);

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
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut chunk = [0; 4096];
            // Read to the end even when nobody listens any more, so a background program that
            // still writes does not get SIGPIPE while outlay runs.
            while let Ok(n @ 1..) = stderr.read(&mut chunk) {
                let _ = tx.send(chunk[..n].to_vec());
            }
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
        let text = collect_stderr(&rx);
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

/// What a finished hook wrote to stderr: everything up to the end of the pipe, or what arrived
/// within [`STDERR_GRACE`] when a background program still holds it. The reader is left behind.
fn collect_stderr(rx: &mpsc::Receiver<Vec<u8>>) -> String {
    let deadline = Instant::now() + STDERR_GRACE;
    let mut bytes = Vec::new();
    while let Ok(chunk) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        bytes.extend(chunk);
    }
    String::from_utf8_lossy(&bytes).into_owned()
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

    #[test]
    fn hooks_do_not_wait_for_what_they_leave_running() {
        let started = Instant::now();
        let failures = run_hooks(&["sleep 5 >&2 &".to_owned()], Duration::from_secs(10));
        assert!(failures.is_empty(), "{failures:?}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );

        // What a failing hook wrote before it exited still makes the report.
        let started = Instant::now();
        let hook = "sleep 5 >&2 & echo broken >&2; exit 1".to_owned();
        let failures = run_hooks(std::slice::from_ref(&hook), Duration::from_secs(10));
        assert_eq!(failures, [format!("post_apply `{hook}` failed: broken.")]);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );

        // A hook stopped on timeout does not wait for its children either.
        let started = Instant::now();
        let failures = run_hooks(&["sleep 5; true".to_owned()], Duration::from_millis(300));
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }
}
