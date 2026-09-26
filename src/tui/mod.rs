//! The interactive editor: terminal setup and teardown, and the event loop.

pub mod app;
pub mod canvas;
pub mod cmdline;
pub mod keys;
pub mod panels;
pub mod popups;
pub mod theme;
pub mod ui;

use std::io::stdout;
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, Event, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{supports_keyboard_enhancement, window_size};

use crate::model::validate::Severity;
use crate::xrandr::Backend;
use app::{App, Effect, Options};

/// How long the loop waits for input before redrawing anyway.
const IDLE_POLL: Duration = Duration::from_millis(250);

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

/// Opens the editor on `backend`'s state and runs it until the user quits.
pub fn run(backend: &dyn Backend, options: Options) -> Result<()> {
    let fixed_aspect = options.cell_aspect;
    let snap = backend.query()?;
    let mut app = App::new(snap, options);
    let mut terminal = ratatui::try_init()?;
    // The alternate screen is on; only now push the keyboard flags, and only where supported.
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    install_panic_hook(enhanced);
    app.set_cell_aspect(fixed_aspect.or_else(detect_cell_aspect).unwrap_or(2.0));
    let result = event_loop(&mut terminal, &mut app, backend, fixed_aspect);
    restore(enhanced);
    result
}

fn restore(enhanced: bool) {
    if enhanced {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = ratatui::try_restore();
}

/// Wraps ratatui's panic hook (which restores the terminal) so the keyboard flags are popped
/// first.
fn install_panic_hook(enhanced: bool) {
    let restore_terminal = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if enhanced {
            let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
        }
        restore_terminal(info);
    }));
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    backend: &dyn Backend,
    fixed_aspect: Option<f64>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if !event::poll(IDLE_POLL)? {
            continue;
        }
        let mut effects = Vec::new();
        // Handle everything that is queued before drawing again.
        loop {
            match event::read()? {
                Event::Key(key) => effects.extend(app.handle_key(key)),
                Event::Resize(..) => {
                    if fixed_aspect.is_none()
                        && let Some(aspect) = detect_cell_aspect()
                    {
                        app.set_cell_aspect(aspect);
                    }
                }
                _ => {}
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
        for effect in effects {
            match effect {
                Effect::Quit => return Ok(()),
                Effect::Query => match backend.query() {
                    Ok(snap) => app.reloaded(snap),
                    Err(err) => app.say(Severity::Error, format!("Reload failed: {err:#}")),
                },
            }
        }
    }
}
