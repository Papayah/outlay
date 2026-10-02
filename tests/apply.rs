//! The apply flow through `Session`, with a fake backend: apply → verify → countdown → keep or
//! revert, with a scripted clock, a queue of pending keys, and an injected signal flag.

mod common;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use common::{ix, keys, unplugged};
use outlay::backend::{ApplyOutcome, Backend, FixtureBackend, Plan};
use outlay::model::Rotation;
use outlay::model::Snapshot;
use outlay::tui::app::{App, Options, UiMode, WATCH_INTERVAL};
use outlay::tui::session::{Input, Session, Settings};
use outlay::xrandr::command;
use ratatui::crossterm::event::Event;

/// What the fake does with the next apply.
#[derive(Clone, Debug)]
enum Next {
    /// Applies it like xrandr would.
    Works,
    /// Exits non-zero, optionally after changing the screens anyway.
    Fails { stderr: &'static str, changes: bool },
    /// Succeeds but skips one planned output, as xrandr does for an output it cannot find.
    Skips(&'static str),
    /// Cannot even run.
    Breaks,
}

/// A backend that records every plan, counts full probes and re-queries, and misbehaves on
/// request.
struct Fake {
    inner: FixtureBackend,
    next: Mutex<VecDeque<Next>>,
    calls: Mutex<Vec<Plan>>,
    probes: Mutex<usize>,
    requeries: Mutex<usize>,
}

impl Fake {
    fn new(inner: FixtureBackend) -> Self {
        Self {
            inner,
            next: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
            probes: Mutex::new(0),
            requeries: Mutex::new(0),
        }
    }

    fn demo() -> Self {
        Self::new(FixtureBackend::demo())
    }

    /// (full probes, re-queries) so far.
    fn reads(&self) -> (usize, usize) {
        (
            *self.probes.lock().unwrap(),
            *self.requeries.lock().unwrap(),
        )
    }

    fn then(self, next: Next) -> Self {
        self.next.lock().unwrap().push_back(next);
        self
    }

    fn calls(&self) -> Vec<Plan> {
        self.calls.lock().unwrap().clone()
    }

    /// The plans as the xrandr arguments that carry them out.
    fn calls_argv(&self) -> Vec<Vec<String>> {
        self.calls().iter().map(command::argv).collect()
    }
}

impl Backend for Fake {
    fn query(&self) -> Result<Snapshot> {
        *self.probes.lock().unwrap() += 1;
        self.inner.query()
    }

    fn requery(&self) -> Result<Snapshot> {
        *self.requeries.lock().unwrap() += 1;
        self.inner.requery()
    }

    fn apply(&self, plan: &Plan) -> Result<ApplyOutcome> {
        self.calls.lock().unwrap().push(plan.clone());
        let next = self.next.lock().unwrap().pop_front().unwrap_or(Next::Works);
        match next {
            Next::Works => self.inner.apply(plan),
            Next::Fails { stderr, changes } => {
                if changes {
                    self.inner.apply(plan)?;
                }
                Ok(ApplyOutcome {
                    success: false,
                    stdout: String::new(),
                    stderr: stderr.to_owned(),
                })
            }
            Next::Skips(name) => {
                let mut kept = plan.clone();
                kept.outputs.retain(|p| p.name != name);
                let mut outcome = self.inner.apply(&kept)?;
                outcome.stderr = format!("warning: output {name} not found; ignoring\n");
                Ok(outcome)
            }
            Next::Breaks => Err(anyhow!("xrandr crashed")),
        }
    }

    fn dump(&self) -> Result<String> {
        self.inner.dump()
    }
}

/// Keys that arrived while xrandr ran; the session must throw them away.
#[derive(Default)]
struct Queue {
    events: Vec<Event>,
}

impl Input for Queue {
    fn drain(&mut self) {
        self.events.clear();
    }
}

struct Rig<'a> {
    session: Session<'a>,
    signal: Arc<AtomicBool>,
    input: Queue,
}

impl<'a> Rig<'a> {
    fn new(backend: &'a Fake, settings: Settings) -> Self {
        Self::with(backend, Options::default(), settings)
    }

    /// A rig whose editor watches for displays that are plugged in, as the live editor does.
    fn watching(backend: &'a Fake, settings: Settings) -> Self {
        let options = Options {
            watch: Some(WATCH_INTERVAL),
            ..Options::default()
        };
        Self::with(backend, options, settings)
    }

    fn with(backend: &'a Fake, options: Options, settings: Settings) -> Self {
        let app = App::new(backend.query().unwrap(), options);
        let signal = Arc::new(AtomicBool::new(false));
        Rig {
            session: Session::new(app, backend, settings, Arc::clone(&signal)),
            signal,
            input: Queue::default(),
        }
    }

    /// Presses keys at `now`, then carries out what they asked for.
    fn press_at(&mut self, script: &str, now: Instant) {
        for key in keys(script) {
            self.session.handle_event(&Event::Key(key), now);
        }
        self.session.perform(&mut self.input);
    }

    fn press(&mut self, script: &str) {
        let now = self.session.app.now;
        self.press_at(script, now);
    }

    fn app(&self) -> &App {
        &self.session.app
    }

    /// Lets the clock run to `now` without input, then carries out what came of it.
    fn wait_until(&mut self, now: Instant) {
        self.session.tick(now);
        self.session.perform(&mut self.input);
    }

    fn countdown(&self) -> outlay::tui::app::Countdown {
        match &self.app().mode {
            UiMode::Countdown(c) => *c,
            other => panic!("no countdown: {other:?}"),
        }
    }

    fn status(&self) -> String {
        self.app()
            .status
            .as_ref()
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }

    fn message(&self) -> String {
        match &self.app().mode {
            UiMode::Message(m) => format!("{}: {}", m.title, m.lines.join(" | ")),
            other => panic!("no message: {other:?}"),
        }
    }
}

fn edp_x(backend: &Fake) -> i32 {
    let snap = backend.query().unwrap();
    snap.outputs[snap.find("eDP-1").unwrap()]
        .active
        .as_ref()
        .unwrap()
        .pos
        .x
}

#[test]
fn a_kept_layout_stays() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>a");
    let UiMode::ConfirmApply(plan) = &rig.app().mode else {
        panic!("no confirmation")
    };
    assert_eq!(plan.changes, ["eDP-1  pos 2240,1440 → 2250,1440"]);
    let expected = Plan::pending(&rig.app().layout, &rig.app().snap);
    assert_eq!(plan.plan, expected);
    assert_eq!(
        plan.command,
        command::command_line(&command::argv(&expected))
    );

    rig.press("<Enter>");
    let c = rig.countdown();
    assert_eq!(c.deadline - c.started, Duration::from_secs(15));
    assert_eq!(backend.calls(), std::slice::from_ref(&expected));
    assert_eq!(backend.calls_argv(), [command::argv(&expected)]);
    assert_eq!(edp_x(&backend), 2250);

    rig.press_at("y", c.blocked_until);
    assert_eq!(rig.app().mode, UiMode::Normal);
    assert_eq!(backend.calls().len(), 1, "no revert");
    assert!(rig.app().pending().is_empty());
    assert_eq!(rig.status(), "Kept the new layout.");
    assert_eq!(
        rig.app().layout.rect(ix(&rig.app().layout, "eDP-1")).x,
        2250
    );
    assert!(!rig.session.awaiting_answer());
}

#[test]
fn no_answer_reverts_and_keeps_the_edits_pending() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    let before = backend.query().unwrap();
    rig.press("3<A-l>a<Enter>");
    let c = rig.countdown();

    rig.session.tick(c.deadline - Duration::from_millis(1));
    rig.session.perform(&mut rig.input);
    assert!(matches!(rig.app().mode, UiMode::Countdown(_)));

    rig.session.tick(c.deadline);
    rig.session.perform(&mut rig.input);
    let calls = backend.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1], Plan::restore(&before));
    assert_eq!(
        command::command_line(&backend.calls_argv()[1]),
        "xrandr --output HDMI-1-0 --mode 0x1c3 --pos 0x360 --rotate normal --reflect normal --transform none \
         --output DP-1-2 --primary --mode 0x1cb --pos 1920x0 --rotate normal --reflect normal --transform none \
         --output eDP-1 --mode 0x1cf --pos 2240x1440 --rotate normal --reflect normal --transform none \
         --output DP-1-3 --off"
    );
    assert_eq!(backend.query().unwrap(), before, "the screens are back");
    assert_eq!(rig.app().mode, UiMode::Normal);
    assert_eq!(rig.app().pending().len(), 1, "the edit is still pending");
    assert!(
        rig.status().starts_with("No answer in 15 s"),
        "{}",
        rig.status()
    );
}

#[test]
fn n_escape_and_ctrl_c_revert() {
    for (script, quits) in [("n", false), ("<Esc>", false), ("<C-c>", true)] {
        let backend = Fake::demo();
        let mut rig = Rig::new(&backend, Settings::default());
        rig.press("3<A-l>a<Enter>");
        let c = rig.countdown();
        rig.press_at(script, c.blocked_until);
        assert_eq!(backend.calls().len(), 2, "{script} reverts");
        assert_eq!(edp_x(&backend), 2240);
        assert_eq!(rig.session.quit, quits, "{script}");
        assert!(rig.status().starts_with("Reverted"), "{}", rig.status());
    }
}

#[test]
fn keys_pressed_while_the_screens_were_dark_do_not_answer() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>a");
    // Enter confirms; an Enter and a y are already queued behind it.
    let [enter] = keys("<Enter>")[..] else {
        unreachable!()
    };
    rig.session.handle_event(&Event::Key(enter), rig.app().now);
    rig.input.events = keys("<Enter>y").into_iter().map(Event::Key).collect();
    rig.session.perform(&mut rig.input);
    assert!(rig.input.events.is_empty(), "the queue was drained");
    let c = rig.countdown();

    // Within the first second no key counts, not even y.
    rig.press_at("y", c.blocked_until - Duration::from_millis(1));
    assert!(matches!(rig.app().mode, UiMode::Countdown(_)));
    // Enter never keeps.
    rig.press_at("<Enter>", c.blocked_until);
    assert!(matches!(rig.app().mode, UiMode::Countdown(_)));
    assert_eq!(backend.calls().len(), 1);
    rig.press_at("y", c.blocked_until);
    assert_eq!(rig.status(), "Kept the new layout.");
}

#[test]
fn a_failed_apply_that_changed_the_screens_is_reverted() {
    let backend = Fake::demo().then(Next::Fails {
        stderr: "xrandr: Configure crtc 2 failed\n",
        changes: true,
    });
    let mut rig = Rig::new(&backend, Settings::default());
    let before = backend.query().unwrap();
    rig.press("3<A-l>a<Enter>");
    assert_eq!(backend.calls().len(), 2, "apply, then revert");
    assert_eq!(backend.query().unwrap(), before);
    let message = rig.message();
    assert!(message.starts_with("Apply failed"), "{message}");
    assert!(
        message.contains("xrandr: Configure crtc 2 failed"),
        "{message}"
    );
    assert!(
        message.contains("Reverted to the previous layout."),
        "{message}"
    );
    assert_eq!(rig.app().pending().len(), 1, "the edit is still pending");
    rig.press("<Esc>");
    assert_eq!(rig.app().mode, UiMode::Normal);
}

#[test]
fn a_failed_apply_that_changed_nothing_is_not_reverted() {
    let backend = Fake::demo().then(Next::Fails {
        stderr: "xrandr: cannot find mode\n",
        changes: false,
    });
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>a<Enter>");
    assert_eq!(backend.calls().len(), 1);
    assert!(rig.message().contains("Nothing changed."));

    let backend = Fake::demo().then(Next::Breaks);
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>a<Enter>");
    assert!(
        rig.message()
            .contains("Could not run xrandr: xrandr crashed")
    );
}

#[test]
fn verification_catches_what_xrandr_ignored() {
    // xrandr exits 0 but skips DP-1-3, which was to be turned on; eDP-1 did move.
    let backend = Fake::demo().then(Next::Skips("DP-1-3"));
    let mut rig = Rig::new(&backend, Settings::default());
    let before = backend.query().unwrap();
    rig.press("3<A-l>4 a<Enter>");
    assert_eq!(backend.calls().len(), 2, "apply, then revert");
    assert_eq!(backend.query().unwrap(), before);
    let message = rig.message();
    assert!(
        message.contains("xrandr: warning: output DP-1-3 not found; ignoring"),
        "{message}"
    );
    assert!(
        message.contains("DP-1-3 is off; it should be on."),
        "{message}"
    );
    assert!(
        message.contains("Reverted to the previous layout."),
        "{message}"
    );
}

#[test]
fn a_hangup_during_the_countdown_reverts_then_quits() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>a<Enter>");
    rig.countdown();
    rig.signal.store(true, Ordering::SeqCst);
    assert!(rig.session.has_work());
    rig.session.perform(&mut rig.input);
    assert_eq!(backend.calls().len(), 2);
    assert_eq!(edp_x(&backend), 2240);
    assert!(rig.session.quit);

    // Without an apply waiting, a signal just quits.
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.signal.store(true, Ordering::SeqCst);
    rig.session.perform(&mut rig.input);
    assert!(rig.session.quit);
    assert!(backend.calls().is_empty());
}

#[test]
fn leaving_the_loop_reverts_an_unanswered_apply() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>a<Enter>");
    assert!(rig.session.awaiting_answer());
    rig.session.revert_if_waiting();
    assert_eq!(backend.calls().len(), 2);
    assert_eq!(edp_x(&backend), 2240);
}

#[test]
fn revert_sh_is_written_before_applying() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("outlay-apply-{}", std::process::id()));
    let path = dir.join("outlay").join("revert.sh");
    let backend = Fake::demo();
    let settings = Settings {
        revert_file: Some(path.clone()),
        ..Settings::default()
    };
    let mut rig = Rig::new(&backend, settings);
    let before = backend.query().unwrap();
    rig.press("3<A-l>a<Enter>");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("#!/bin/sh\n"), "{text}");
    assert_eq!(text, command::revert_script(&Plan::restore(&before)));
    assert!(
        text.ends_with(
            "\nxrandr --output HDMI-1-0 --mode 0x1c3 --pos 0x360 --rotate normal --reflect normal --transform none \
             --output DP-1-2 --primary --mode 0x1cb --pos 1920x0 --rotate normal --reflect normal --transform none \
             --output eDP-1 --mode 0x1cf --pos 2240x1440 --rotate normal --reflect normal --transform none \
             --output DP-1-3 --off\n"
        ),
        "{text}"
    );
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    rig.press_at("n", rig.countdown().blocked_until);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_zero_timeout_keeps_at_once() {
    let backend = Fake::demo();
    let settings = Settings {
        revert_seconds: 0,
        ..Settings::default()
    };
    let mut rig = Rig::new(&backend, settings);
    rig.press("3<A-l>a<Enter>");
    assert_eq!(rig.app().mode, UiMode::Normal);
    assert_eq!(rig.status(), "Kept the new layout.");
    assert_eq!(edp_x(&backend), 2250);
}

/// A scratch directory for a hook that counts its runs; removed on drop.
struct Counter {
    dir: PathBuf,
}

const BROKEN_HOOK: &str = "echo no wallpaper >&2; exit 4";
const BROKEN_TEXT: &str = "post_apply `echo no wallpaper >&2; exit 4` failed: no wallpaper.";

impl Counter {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("outlay-hooks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Counter { dir }
    }

    /// A counting hook and one that always fails.
    fn settings(&self) -> Settings {
        Settings {
            hooks: vec![
                format!("echo x >> '{}/count'", self.dir.display()),
                BROKEN_HOOK.to_owned(),
            ],
            ..Settings::default()
        }
    }

    fn runs(&self) -> usize {
        std::fs::read_to_string(self.dir.join("count")).map_or(0, |t| t.lines().count())
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn hooks_run_once_the_apply_checks_out_and_not_again_on_keep() {
    let counter = Counter::new("keep");
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, counter.settings());
    rig.press("3<A-l>a<Enter>");
    rig.countdown();
    assert_eq!(counter.runs(), 1, "before the countdown");
    // The fake is simulated, which the status says first.
    assert_eq!(
        rig.status(),
        format!("Simulated: nothing was sent to the displays. {BROKEN_TEXT}"),
        "shown during the countdown"
    );

    rig.press_at("y", rig.countdown().blocked_until);
    assert_eq!(rig.status(), "Kept the new layout.");
    assert_eq!(counter.runs(), 1, "keeping changes nothing");
}

#[test]
fn hooks_run_again_after_every_revert() {
    let revert = |rig: &mut Rig, how: &str| match how {
        "timeout" => {
            let c = rig.countdown();
            rig.session.tick(c.deadline);
            rig.session.perform(&mut rig.input);
        }
        "signal" => {
            rig.signal.store(true, Ordering::SeqCst);
            rig.session.perform(&mut rig.input);
        }
        "exit" => rig.session.revert_if_waiting(),
        keys => rig.press_at(keys, rig.countdown().blocked_until),
    };
    for how in ["timeout", "n", "<Esc>", "<C-c>", "signal", "exit"] {
        let counter = Counter::new(&how.replace(['<', '>'], ""));
        let backend = Fake::demo();
        let mut rig = Rig::new(&backend, counter.settings());
        rig.press("3<A-l>a<Enter>");
        revert(&mut rig, how);
        assert_eq!(backend.calls().len(), 2, "{how} reverts");
        assert_eq!(
            counter.runs(),
            2,
            "{how}: after the apply and after the revert"
        );
        if how != "exit" {
            let status = rig.status();
            assert!(
                status.contains("Your edits are still pending."),
                "{how}: {status}"
            );
            assert!(status.ends_with(BROKEN_TEXT), "{how}: {status}");
        }
    }
}

#[test]
fn a_zero_timeout_runs_the_hooks_once() {
    let counter = Counter::new("zero");
    let backend = Fake::demo();
    let settings = Settings {
        revert_seconds: 0,
        ..counter.settings()
    };
    let mut rig = Rig::new(&backend, settings);
    rig.press("3<A-l>a<Enter>");
    assert_eq!(rig.app().mode, UiMode::Normal);
    assert_eq!(counter.runs(), 1);
    assert_eq!(rig.status(), format!("Kept the new layout. {BROKEN_TEXT}"));
}

#[test]
fn hooks_run_after_the_revert_of_a_failed_apply_only() {
    let counter = Counter::new("fails");
    let backend = Fake::demo().then(Next::Fails {
        stderr: "xrandr: Configure crtc 2 failed\n",
        changes: true,
    });
    let mut rig = Rig::new(&backend, counter.settings());
    rig.press("3<A-l>a<Enter>");
    // Not after the apply, which did not check out, but after the revert that follows it.
    assert_eq!(counter.runs(), 1, "after the revert");
    let message = rig.message();
    assert!(
        message.contains("Reverted to the previous layout. | post_apply"),
        "{message}"
    );
    assert!(message.ends_with(BROKEN_TEXT), "{message}");

    let counter = Counter::new("fails-unchanged");
    let backend = Fake::demo().then(Next::Fails {
        stderr: "xrandr: cannot find mode\n",
        changes: false,
    });
    let mut rig = Rig::new(&backend, counter.settings());
    rig.press("3<A-l>a<Enter>");
    assert!(rig.message().contains("Nothing changed."));
    assert_eq!(counter.runs(), 0, "nothing changed, nothing to redraw");
}

#[test]
fn the_countdown_starts_once_the_hooks_are_done() {
    let backend = Fake::demo();
    let settings = Settings {
        hooks: vec!["sleep 0.3".to_owned()],
        ..Settings::default()
    };
    let mut rig = Rig::new(&backend, settings);
    let before = Instant::now();
    rig.press("3<A-l>a<Enter>");
    let c = rig.countdown();
    assert!(
        c.started >= before + Duration::from_millis(300),
        "{:?}",
        c.started - before
    );
    assert_eq!(rig.status(), "Simulated: nothing was sent to the displays.");
}

#[test]
fn applying_without_changes_re_applies_the_live_layout() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    let before = backend.query().unwrap();
    rig.press("a");
    let UiMode::ConfirmApply(plan) = &rig.app().mode else {
        panic!()
    };
    assert_eq!(
        plan.changes,
        ["No changes: this re-applies the current layout."]
    );
    rig.press("<Enter>");
    rig.press_at("y", rig.countdown().blocked_until);
    assert_eq!(rig.status(), "Kept the new layout.");
    assert_eq!(backend.query().unwrap(), before);
}

#[test]
fn errors_block_the_apply() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3:pos 20000 0<Enter>a");
    let UiMode::ConfirmApply(plan) = &rig.app().mode else {
        panic!()
    };
    assert_eq!(plan.errors.len(), 1, "{:?}", plan.errors);
    assert!(plan.errors[0].contains("screen maximum"));
    rig.press("<Enter>");
    assert!(matches!(rig.app().mode, UiMode::ConfirmApply(_)));
    assert_eq!(rig.status(), "Fix the errors before applying.");
    assert!(backend.calls().is_empty());
    rig.press("<Esc>");
    assert_eq!(rig.app().mode, UiMode::Normal);
}

#[test]
fn y_copies_the_portable_command() {
    let backend = Fake::demo();
    let mut rig = Rig::new(&backend, Settings::default());
    rig.press("3<A-l>y");
    let expected = command::command_line(&command::portable_argv(&Plan::pending(
        &rig.app().layout,
        &rig.app().snap,
    )));
    let out = String::from_utf8(rig.session.outbox.clone()).unwrap();
    assert_eq!(out, outlay::tui::session::osc52(&expected));
    assert!(
        expected.contains("--mode 1920x1080 --rate 165.01 --pos 2250x1440"),
        "{expected}"
    );
    assert!(backend.calls().is_empty(), "copying applies nothing");
}

#[test]
fn the_watch_re_queries_and_never_applies() {
    let backend = Fake::demo();
    let mut rig = Rig::watching(&backend, Settings::default());
    let start = rig.app().now;
    let (probes, _) = backend.reads();
    let at = |rounds: u32| start + WATCH_INTERVAL * rounds;

    rig.wait_until(start + Duration::from_secs(1));
    assert_eq!(backend.reads(), (probes, 0), "not yet");
    rig.wait_until(at(1));
    assert_eq!(backend.reads(), (probes, 1), "a re-query, not a probe");
    assert_eq!(rig.status(), "", "nothing changed, nothing to say");

    let live = backend.query().unwrap();
    backend.inner.set_state(unplugged(&live, "DP-1-3"));
    rig.wait_until(at(2));
    assert_eq!(rig.status(), "DP-1-3 disconnected.");
    let dp13 = ix(&rig.app().layout, "DP-1-3");
    assert_eq!(rig.app().layout.numbers[dp13], None);

    backend.inner.set_state(live);
    rig.wait_until(at(3));
    assert_eq!(rig.status(), "DP-1-3 connected: Space turns it on.");
    assert_eq!(rig.app().layout.numbers[dp13], Some(4));
    assert_eq!(rig.app().focus, dp13);
    assert_eq!(backend.reads().1, 3);
    assert!(backend.calls().is_empty(), "the watch applies nothing");
    assert!(rig.app().pending().is_empty());

    // R is a full probe.
    let (probes, requeries) = backend.reads();
    rig.press("R");
    assert_eq!(backend.reads(), (probes + 1, requeries));
    assert_eq!(rig.status(), "No display changes.");
}

#[test]
fn the_watch_waits_while_the_countdown_runs() {
    let backend = Fake::demo();
    let mut rig = Rig::watching(&backend, Settings::default());
    rig.press("3<A-l>a<Enter>");
    let c = rig.countdown();
    let reads = backend.reads();
    let mut now = c.started;
    while now + Duration::from_millis(250) < c.deadline {
        now += Duration::from_millis(250);
        rig.wait_until(now);
    }
    assert_eq!(backend.reads(), reads, "no reads during the countdown");
    assert!(matches!(rig.app().mode, UiMode::Countdown(_)));

    rig.press_at("y", now);
    assert_eq!(rig.status(), "Kept the new layout.");
    rig.wait_until(now + Duration::from_millis(250));
    assert_eq!(
        backend.reads().1,
        reads.1 + 1,
        "the watch goes on after the answer"
    );
    assert_eq!(rig.status(), "Kept the new layout.");
}

#[test]
fn a_dock_plugged_in_brings_its_displays_and_keeps_the_edits() {
    let path = format!(
        "{}/tests/fixtures/xrandr/dock-mst-3.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let docked = FixtureBackend::from_file(path.as_ref())
        .unwrap()
        .query()
        .unwrap();
    let mut undocked = docked.clone();
    undocked.outputs.retain(|o| !o.name.starts_with("DP-2."));
    let backend = Fake::new(FixtureBackend::new(undocked));
    let mut rig = Rig::watching(&backend, Settings::default());
    assert_eq!(rig.app().snap.numbered().len(), 1, "the laptop alone");
    rig.press("o");
    let edp = ix(&rig.app().layout, "eDP-1");
    let rotated = rig.app().layout.outputs[edp].rotation;
    assert_ne!(rotated, Rotation::Normal);

    backend.inner.set_state(docked);
    let now = rig.app().now + WATCH_INTERVAL;
    rig.wait_until(now);

    let layout = &rig.app().layout;
    let number = |name: &str| layout.numbers[ix(layout, name)];
    assert_eq!(number("eDP-1"), Some(1), "the laptop keeps its number");
    assert_eq!(number("DP-2.1"), Some(2));
    assert_eq!(number("DP-2.2"), Some(3));
    assert_eq!(number("DP-2.3"), Some(4));
    let edp = ix(layout, "eDP-1");
    assert_eq!(layout.outputs[edp].rotation, rotated, "the edit survives");
    assert!(
        layout.is_enabled(ix(layout, "DP-2.1")),
        "on, as xrandr says"
    );
    assert!(layout.is_enabled(ix(layout, "DP-2.2")));
    assert_eq!(
        rig.app().focus,
        ix(layout, "DP-2.3"),
        "focus goes to the display that is off"
    );
    assert_eq!(
        rig.status(),
        "DP-2.1, DP-2.2 and DP-2.3 connected: Space turns DP-2.3 on."
    );
    assert!(backend.calls().is_empty());

    rig.press("u");
    assert!(
        rig.app().pending().is_empty(),
        "the undo survives: {:?}",
        rig.app().pending()
    );
    rig.press("<C-r>");
    let layout = &rig.app().layout;
    assert_eq!(layout.outputs[ix(layout, "eDP-1")].rotation, rotated);
}
