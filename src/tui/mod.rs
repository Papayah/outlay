//! The interactive editor: terminal setup and teardown, signals, the panic hook and the event
//! loop. The state lives in [`app`], the side effects in [`session`].

pub mod app;
pub mod canvas;
pub mod cmdline;
pub mod keys;
pub mod panels;
pub mod popups;
pub mod session;
pub mod theme;
pub mod ui;

use std::io::{Write, stdout};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement, window_size,
};
use ratatui::{DefaultTerminal, Terminal};
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};

use crate::xrandr::Backend;
use app::{App, Options};
use session::{Input, Session, Settings};

/// The loop never blocks longer than this, so it notices signals and redraws the countdown.
const IDLE_POLL: Duration = Duration::from_millis(250);
const COUNTDOWN_POLL: Duration = Duration::from_millis(100);
/// About 60 frames a second while displays glide.
const ANIMATION_POLL: Duration = Duration::from_millis(16);

/// The `revert.sh` the panic hook runs while an applied layout waits for its answer.
static PANIC_REVERT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Arms (`Some`) or disarms the panic hook's revert.
pub fn arm_panic_revert(script: Option<PathBuf>) {
    *PANIC_REVERT.lock().unwrap_or_else(|e| e.into_inner()) = script;
}

/// Puts the terminal back: keyboard flags popped, raw mode off, main screen. Every step ignores
/// errors, since after a SIGHUP each write fails with EIO.
fn restore_terminal(pop_keyboard_flags: bool) {
    if pop_keyboard_flags {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
}

/// Wraps the current panic hook so a panic first pops the keyboard flags, runs `revert.sh`
/// during a countdown, and restores the terminal, then reports as usual. It never prints on its
/// own: ratatui's hook does, and `eprintln!` panics when the terminal is gone, which turns the
/// panic into an abort.
pub fn install_panic_hook(pop_keyboard_flags: bool) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let armed = PANIC_REVERT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(script) = armed {
            let _ = Command::new("sh")
                .arg(script)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        restore_terminal(pop_keyboard_flags);
        previous(info);
    }));
}

/// Cell height over cell width from the terminal's pixel size, or `None` when the terminal does
/// not report pixels (tmux reports 0).
pub fn detect_cell_aspect() -> Option<f64> {
    let size = window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    let cell_w = f64::from(size.width) / f64::from(size.columns);
    let cell_h = f64::from(size.height) / f64::from(size.rows);
    Some(cell_h / cell_w)
}

/// Keys queued in the terminal.
struct TerminalInput;

impl Input for TerminalInput {
    fn drain(&mut self) {
        while matches!(event::poll(Duration::ZERO), Ok(true)) {
            if event::read().is_err() {
                break;
            }
        }
    }
}

/// Settings made safe for `backend`. A simulated backend (`--demo`, `--from-file`, `-n`) must
/// never reach the real desktop: no `revert.sh` that would change the real screens, and no
/// `post_apply` hook that would redraw the real wallpaper.
pub fn confine(backend: &dyn Backend, mut settings: Settings) -> Settings {
    if !backend.touches_x() {
        settings.revert_file = None;
        settings.hooks.clear();
    }
    settings
}

/// Opens the editor on `backend`'s state and runs it until the user quits.
pub fn run(backend: &dyn Backend, options: Options, settings: Settings) -> Result<()> {
    let settings = confine(backend, settings);
    let fixed_aspect = options.cell_aspect;
    let app = App::new(backend.query()?, options);
    // The loop polls with a timeout, so it notices these flags; the default action (die on the
    // spot, leaving an unconfirmed layout) is replaced.
    let signal = Arc::new(AtomicBool::new(false));
    for sig in [SIGHUP, SIGTERM, SIGINT] {
        signal_hook::flag::register(sig, Arc::clone(&signal))?;
    }
    let mut session = Session::new(app, backend, settings, signal);

    // Set up by hand rather than with ratatui::init, whose panic hook prints (see above).
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(terminal) => terminal,
        Err(err) => {
            restore_terminal(false);
            return Err(err.into());
        }
    };
    // The alternate screen is on; only now push the keyboard flags, and only where supported.
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    install_panic_hook(enhanced);
    session
        .app
        .set_cell_aspect(fixed_aspect.or_else(detect_cell_aspect).unwrap_or(2.0));

    let result = event_loop(&mut terminal, &mut session, fixed_aspect);
    // Whatever stopped the loop, an unconfirmed layout does not outlive the editor.
    session.revert_if_waiting();
    if terminal.show_cursor().is_err() {
        // The terminal is gone. Dropping it would retry and eprintln!, which panics on EIO.
        std::mem::forget(terminal);
    }
    restore_terminal(enhanced);
    if session.signalled() {
        return Ok(());
    }
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    session: &mut Session,
    fixed_aspect: Option<f64>,
) -> Result<()> {
    let mut input = TerminalInput;
    loop {
        if session.signalled() {
            session.perform(&mut input);
            return Ok(());
        }
        terminal.draw(|frame| ui::draw(frame, &mut session.app))?;
        if !session.outbox.is_empty() {
            // Written between two draws, so the escape sequence never splits a frame.
            let mut out = stdout();
            out.write_all(&session.outbox)?;
            out.flush()?;
            session.outbox.clear();
        }
        if session.has_work() {
            session.perform(&mut input);
            if session.quit {
                return Ok(());
            }
            continue;
        }
        let timeout = if session.app.animating() {
            ANIMATION_POLL
        } else if session.app.counting_down() {
            COUNTDOWN_POLL
        } else {
            IDLE_POLL
        };
        if !event::poll(timeout)? {
            session.tick(Instant::now());
            continue;
        }
        loop {
            let event = event::read()?;
            if matches!(event, Event::Resize(..))
                && fixed_aspect.is_none()
                && let Some(aspect) = detect_cell_aspect()
            {
                session.app.set_cell_aspect(aspect);
            }
            session.handle_event(&event, Instant::now());
            // Stop at the first effect so it is drawn ("applying…") before it runs.
            if session.has_work() || !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xrandr::FixtureBackend;

    #[test]
    fn a_simulated_backend_gets_no_revert_file_and_no_hooks() {
        let settings = Settings {
            revert_file: Some(PathBuf::from("/state/outlay/revert.sh")),
            hooks: vec!["feh --bg-fill ~/wall/*".to_owned()],
            revert_seconds: 7,
            ..Settings::default()
        };
        let backend = FixtureBackend::demo();
        assert!(!backend.touches_x());
        let confined = confine(&backend, settings);
        assert_eq!(confined.revert_file, None);
        assert!(confined.hooks.is_empty());
        assert_eq!(confined.revert_seconds, 7, "the rest stays");

        // The live backend keeps both; nothing here runs xrandr.
        let live = crate::xrandr::XrandrCli::new();
        let kept = confine(&live, confined.clone());
        assert_eq!(kept.hooks, confined.hooks);
        let settings = Settings {
            revert_file: Some(PathBuf::from("/state/outlay/revert.sh")),
            hooks: vec!["true".to_owned()],
            ..Settings::default()
        };
        let kept = confine(&live, settings);
        assert!(kept.revert_file.is_some());
        assert_eq!(kept.hooks, ["true"]);
    }
}
