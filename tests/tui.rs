//! The editor without a terminal: key sequences through `App::handle_key`, and screens rendered
//! into ratatui's `TestBackend` and compared with insta snapshots.

mod common;

use common::{ix, keys, link, rect, stuck, unplugged};
use outlay::backend::{Backend, FixtureBackend};
use outlay::model::Snapshot;
use outlay::model::geometry::Rect;
use outlay::model::links::{Align, Side};
use outlay::model::validate::Severity;
use outlay::tui::app::{App, Effect, Options, StickStep, UiMode, WATCH_INTERVAL};
use outlay::tui::theme::Theme;
use outlay::tui::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use std::time::Duration;

fn demo() -> App {
    App::new(FixtureBackend::demo().query().unwrap(), Options::default())
}

fn press(app: &mut App, script: &str) -> Vec<Effect> {
    keys(script)
        .into_iter()
        .flat_map(|k| app.handle_key(k))
        .collect()
}

fn screen(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
}

/// The runs of cells that carry `modifier`, one string per row.
fn marked(buf: &Buffer, modifier: Modifier) -> Vec<String> {
    let area = buf.area;
    (area.top()..area.bottom())
        .filter_map(|y| {
            let text: String = (area.left()..area.right())
                .map(|x| &buf[(x, y)])
                .filter(|c| c.modifier.contains(modifier))
                .map(|c| c.symbol())
                .collect();
            (!text.is_empty()).then_some(text)
        })
        .collect()
}

/// How many cells hold a double box-drawing line.
fn double_lines(buf: &Buffer) -> usize {
    buf.content()
        .iter()
        .filter(|c| ["═", "║", "╔", "╗", "╚", "╝"].contains(&c.symbol()))
        .count()
}

fn status(app: &App) -> String {
    app.status
        .as_ref()
        .map(|s| s.text.clone())
        .unwrap_or_default()
}

#[test]
fn stick_to_the_left_of_display_two() {
    let mut app = demo();
    press(&mut app, "3");
    assert_eq!(app.layout.names[app.focus], "eDP-1");

    press(&mut app, "s");
    let UiMode::Stick(flow) = &app.mode else {
        panic!("not sticking: {:?}", app.mode)
    };
    assert_eq!(flow.step, StickStep::Target);
    assert_eq!(
        app.layout.names[flow.target], "DP-1-2",
        "the current parent is the default target"
    );

    press(&mut app, "2h");
    let UiMode::Stick(flow) = &app.mode else {
        panic!("not sticking")
    };
    assert_eq!(
        (flow.step, flow.side, flow.align),
        (StickStep::Side, Side::LeftOf, Align::Start)
    );
    let ghost_names: Vec<&str> = flow
        .ghosts
        .iter()
        .map(|&(i, _)| app.layout.names[i].as_str())
        .collect();
    assert_eq!(
        ghost_names,
        ["HDMI-1-0", "eDP-1"],
        "HDMI-1-0 moves out beyond eDP-1"
    );
    assert!(app.pending().is_empty(), "the ghost is only a preview");

    press(&mut app, "<Enter>");
    assert_eq!(app.mode, UiMode::Normal);
    assert_eq!(
        link(&app.layout, "eDP-1"),
        stuck("DP-1-2", Side::LeftOf, Align::Start, 0)
    );
    assert_eq!(
        link(&app.layout, "HDMI-1-0"),
        stuck("eDP-1", Side::LeftOf, Align::End, 0)
    );
    assert_eq!(rect(&app.layout, "HDMI-1-0"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(rect(&app.layout, "eDP-1"), Rect::new(1920, 0, 1920, 1080));
    assert_eq!(rect(&app.layout, "DP-1-2"), Rect::new(3840, 0, 2560, 1440));
    assert_eq!(status(&app), "Stuck 3 eDP-1 left-of 2 DP-1-2, top.");
    assert!(app.history.can_undo());

    press(&mut app, "u");
    assert!(
        app.pending().is_empty(),
        "one undo step for the whole stick"
    );
}

#[test]
fn stick_flow_cycles_alignment_goes_back_and_cancels() {
    let mut app = demo();
    press(&mut app, "3s2");
    press(&mut app, "l<Tab>");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!((flow.side, flow.align), (Side::RightOf, Align::Center));
    press(&mut app, "<Tab><Tab>");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(flow.align, Align::Start, "start → center → end → start");
    press(&mut app, "<S-Tab>");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(flow.align, Align::End);

    press(&mut app, "<BS>");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(flow.step, StickStep::Target);
    assert!(flow.ghosts.is_empty());

    press(&mut app, "<Esc>");
    assert_eq!(app.mode, UiMode::Normal);
    assert!(app.pending().is_empty());
    assert!(!app.history.can_undo());
}

#[test]
fn stick_target_can_be_picked_spatially() {
    let mut app = demo();
    press(&mut app, "3s");
    press(&mut app, "<Tab>");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(app.layout.names[flow.target], "HDMI-1-0");
    press(&mut app, "l");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(app.layout.names[flow.target], "DP-1-2");
    press(&mut app, "<Enter>");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(flow.step, StickStep::Side);
    assert_eq!(
        (flow.side, flow.align),
        (Side::Below, Align::Center),
        "starts from the current link"
    );
}

#[test]
fn with_two_displays_the_target_step_is_skipped() {
    let (snap, _) = common::load(&[
        common::on("A", 1920, 1080, 0, 0).primary(),
        common::on("B", 1920, 1080, 1920, 0),
    ]);
    let mut app = App::new(snap, Options::default());
    press(&mut app, "2s");
    let UiMode::Stick(flow) = &app.mode else {
        panic!()
    };
    assert_eq!(flow.step, StickStep::Side);
    assert_eq!(app.layout.names[flow.target], "A");
    press(&mut app, "j<Enter>");
    assert_eq!(rect(&app.layout, "B"), Rect::new(0, 1080, 1920, 1080));
    assert_eq!(
        link(&app.layout, "B"),
        stuck("A", Side::Below, Align::Center, 0)
    );
    // Backspace has no first step to return to, so it closes the flow.
    press(&mut app, "s<BS>");
    assert_eq!(app.mode, UiMode::Normal);
}

#[test]
fn focus_moves_spatially_by_number_and_by_tab() {
    let mut app = demo();
    let name = |app: &App| app.layout.names[app.focus].clone();
    assert_eq!(name(&app), "DP-1-2", "starts on the primary");
    press(&mut app, "h");
    assert_eq!(name(&app), "HDMI-1-0");
    press(&mut app, "l<Down>");
    assert_eq!(name(&app), "eDP-1");
    press(&mut app, "<Tab>");
    assert_eq!(name(&app), "DP-1-3", "Tab reaches the display that is off");
    press(&mut app, "<Tab>");
    assert_eq!(name(&app), "HDMI-1-0");
    press(&mut app, "<S-Tab><S-Tab>");
    assert_eq!(name(&app), "eDP-1");
    press(&mut app, "4");
    assert_eq!(name(&app), "DP-1-3");
    press(&mut app, "9");
    assert_eq!(status(&app), "There is no display 9.");
    assert_eq!(name(&app), "DP-1-3");
}

#[test]
fn snap_move_swaps_and_reports_when_stuck() {
    let mut app = demo();
    press(&mut app, "1L");
    assert_eq!(status(&app), "Swapped 1 HDMI-1-0 and 2 DP-1-2.");
    assert_eq!(rect(&app.layout, "DP-1-2").x, 0);
    assert_eq!(rect(&app.layout, "HDMI-1-0").x, 2560);

    press(&mut app, "3");
    let before = app.layout.clone();
    press(&mut app, "J");
    assert_eq!(
        status(&app),
        "No snap spot further down. Alt-j nudges freely."
    );
    assert_eq!(app.layout, before, "a failed snap changes nothing");
}

#[test]
fn nudge_undo_redo() {
    let mut app = demo();
    press(&mut app, "3<A-l><A-l>");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2260);
    assert_eq!(app.history.undo_len(), 1, "one run of nudges, one step");
    press(&mut app, "<A-j>");
    assert_eq!(
        app.history.undo_len(),
        2,
        "a new direction starts a new run"
    );
    press(&mut app, "u");
    assert_eq!(
        rect(&app.layout, "eDP-1"),
        Rect::new(2260, 1440, 1920, 1080)
    );
    press(&mut app, "u");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2240);
    press(&mut app, "<C-r>");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2260);
    press(&mut app, "<C-r><C-r>");
    assert_eq!(status(&app), "Nothing to redo.");
    press(&mut app, "uuu");
    assert_eq!(status(&app), "Nothing to undo.");
    assert!(app.pending().is_empty());
}

/// Presses `key` `times` times, `every` apart, starting one gap after the app's clock. Returns
/// how far eDP-1 moved right with each press.
fn hold(app: &mut App, key: &str, times: usize, every: Duration) -> Vec<i32> {
    let mut moves = Vec::new();
    for _ in 0..times {
        let before = rect(&app.layout, "eDP-1").x;
        let now = app.now + every;
        app.tick(now);
        press(app, key);
        moves.push(rect(&app.layout, "eDP-1").x - before);
    }
    moves
}

#[test]
fn a_held_nudge_speeds_up() {
    let mut app = demo();
    press(&mut app, "3");
    let moves = hold(&mut app, "<A-l>", 40, Duration::from_millis(40));
    // Held for 0.04 s at the first press: ×1 until 0.4 s, ×2 until 0.8 s, ×5 until 1.2 s, then ×10.
    let expected: Vec<i32> = (1..=40)
        .map(|k| match k * 40 {
            ms if ms - 40 < 400 => 10,
            ms if ms - 40 < 800 => 20,
            ms if ms - 40 < 1200 => 50,
            _ => 100,
        })
        .collect();
    assert_eq!(moves, expected);
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("step 10px ×10"), "{text}");
    assert_eq!(app.history.undo_len(), 1, "a hold is one undo step");

    // One u undoes the whole run.
    press(&mut app, "u");
    assert!(app.pending().is_empty());

    // Once the key is let go, the indicator drops the multiplier.
    app.tick(app.now + Duration::from_millis(300));
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("step 10px "), "{text}");
    assert!(!text.contains('×'), "{text}");
}

#[test]
fn slow_taps_never_speed_up() {
    let mut app = demo();
    press(&mut app, "3");
    let moves = hold(&mut app, "<A-l>", 10, Duration::from_millis(200));
    assert_eq!(moves, [10; 10]);
    assert_eq!(
        app.history.undo_len(),
        1,
        "taps in one direction are one step too"
    );
}

#[test]
fn a_new_direction_starts_again_at_one() {
    let mut app = demo();
    press(&mut app, "3");
    let moves = hold(&mut app, "<A-l>", 30, Duration::from_millis(40));
    assert_eq!(moves.last(), Some(&50));
    let back = hold(&mut app, "<A-h>", 3, Duration::from_millis(40));
    assert_eq!(back, [-10, -10, -10]);
    assert_eq!(app.history.undo_len(), 2);
}

#[test]
fn another_key_ends_the_hold_and_the_run() {
    let mut app = demo();
    press(&mut app, "3");
    let moves = hold(&mut app, "<A-l>", 15, Duration::from_millis(40));
    assert_eq!(moves.last(), Some(&20));
    // A focus key in between: the next nudge starts at ×1 and is a new undo step.
    let after = hold(&mut app, "3<A-l>", 1, Duration::from_millis(40));
    assert_eq!(after, [10]);
    assert_eq!(app.history.undo_len(), 2);
    press(&mut app, "u");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2240 + 10 * 10 + 20 * 5);

    // An unbound key ends the hold but not the run.
    let mut app = demo();
    press(&mut app, "3");
    hold(&mut app, "<A-l>", 15, Duration::from_millis(40));
    let after = hold(&mut app, "v<A-l>", 1, Duration::from_millis(40));
    assert_eq!(after, [10]);
    assert_eq!(app.history.undo_len(), 1);
}

#[test]
fn the_scale_picker_and_the_scale_steps() {
    let mut app = demo();
    press(&mut app, "3x");
    let UiMode::Picker(picker) = &app.mode else {
        panic!("no picker")
    };
    assert_eq!(picker.title, "3 eDP-1 scale");
    let labels: Vec<&str> = picker.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "   ×0.5 → 960x540 desktop",
            "  ×0.75 → 1440x810 desktop",
            "•    ×1 → 1920x1080 desktop",
            "  ×1.25 → 2400x1350 desktop",
            "   ×1.5 → 2880x1620 desktop",
            "  ×1.75 → 3360x1890 desktop",
            "     ×2 → 3840x2160 desktop",
            "   ×2.5 → 4800x2700 desktop",
            "     ×3 → 5760x3240 desktop",
        ]
    );
    assert_eq!(picker.selected, 2, "the current scale");
    press(&mut app, "j<Enter>");
    assert_eq!(app.mode, UiMode::Normal);
    let edp = ix(&app.layout, "eDP-1");
    assert_eq!(app.layout.outputs[edp].scaling.factor(), Some(1.25));
    assert_eq!(
        rect(&app.layout, "eDP-1"),
        Rect::new(2000, 1440, 2400, 1350)
    );
    assert_eq!(app.history.undo_len(), 1);
    let pending: Vec<String> = app.pending().iter().map(ToString::to_string).collect();
    assert_eq!(
        pending,
        ["eDP-1  pos 2240,1440 → 2000,1440, scale ×1 → ×1.25"]
    );
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("│ scale ×1 → ×1.25"), "{text}");
    assert!(text.contains("1920x1080@165.01 ×1.25"), "{text}");

    // > > < steps along the list: 1.25 → 1.5 → 1.75 → 1.5, one undo step each.
    press(&mut app, ">>");
    assert_eq!(app.layout.outputs[edp].scaling.factor(), Some(1.75));
    press(&mut app, "<lt>");
    assert_eq!(app.layout.outputs[edp].scaling.factor(), Some(1.5));
    assert_eq!(app.history.undo_len(), 4);
    press(&mut app, "u");
    assert_eq!(app.layout.outputs[edp].scaling.factor(), Some(1.75));

    // A scale off the list joins it, marked as the current one.
    press(&mut app, ":scale 1.1<Enter>x");
    let UiMode::Picker(picker) = &app.mode else {
        panic!("no picker")
    };
    assert_eq!(picker.items.len(), 10);
    assert_eq!(
        picker.items[picker.selected].label,
        "•  ×1.1 → 2112x1188 desktop"
    );
    press(&mut app, "<Esc>:scale 150%<Enter>");
    assert_eq!(
        status(&app),
        "On X11 a scale is a factor, such as :scale 1.5; percentages are for Wayland."
    );
    press(&mut app, ":scale 1<Enter>");
    assert!(app.pending().is_empty(), "{:?}", app.pending());

    // A size that is not whole gets ~.
    let snap = common::desk(&[common::on("A", 1366, 768, 0, 0)]);
    let mut app = App::new(snap, Options::default());
    press(&mut app, "x");
    let UiMode::Picker(picker) = &app.mode else {
        panic!("no picker")
    };
    assert_eq!(picker.items[3].label, "  ×1.25 → ~1708x960 desktop");
    assert_eq!(picker.items[1].label, "  ×0.75 → ~1025x576 desktop");
}

#[test]
fn mode_and_rate_pickers() {
    let mut app = demo();
    press(&mut app, "m");
    let UiMode::Picker(picker) = &app.mode else {
        panic!("no picker")
    };
    assert_eq!(picker.title, "2 DP-1-2 resolution");
    assert!(
        picker.items[picker.selected].label.contains("2560x1440"),
        "{:?}",
        picker.items[picker.selected]
    );
    press(&mut app, "j<Enter>");
    assert_eq!(app.mode, UiMode::Normal);
    let dp = ix(&app.layout, "DP-1-2");
    let mode = app.layout.outputs[dp].mode.clone().unwrap();
    assert_eq!((mode.width, mode.height), (1920, 1080));
    assert_eq!(
        rect(&app.layout, "eDP-1"),
        Rect::new(1920, 1080, 1920, 1080)
    );

    press(&mut app, "r");
    let UiMode::Picker(picker) = &app.mode else {
        panic!("no picker")
    };
    let labels: Vec<&str> = picker.items.iter().map(|i| i.label.trim()).collect();
    assert_eq!(labels.len(), 4, "{labels:?}");
    press(&mut app, "<Esc>");
    assert_eq!(app.mode, UiMode::Normal);

    press(&mut app, "}");
    assert_eq!(status(&app), "DP-1-2 is at its highest rate.");
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("│ mode  2560x1440@143.91       │"), "{text}");
    assert!(text.contains("│    →  1920x1080@119.88       │"), "{text}");
    press(&mut app, "{");
    let mode = app.layout.outputs[dp].mode.clone().unwrap();
    assert!(mode.refresh < 119.0, "{mode:?}");
    press(&mut app, "]");
    let mode = app.layout.outputs[dp].mode.clone().unwrap();
    assert_eq!(
        (mode.width, mode.height),
        (2560, 1440),
        "the next larger resolution"
    );
    // Back at 2560x1440, now at 59.95: only the rate differs from the live mode.
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("│ mode  2560x1440              │"), "{text}");
    assert!(text.contains("│ rate  143.91 → 59.95         │"), "{text}");
}

#[test]
fn space_rotation_and_primary() {
    let mut app = demo();
    press(&mut app, "4 ");
    let dp13 = ix(&app.layout, "DP-1-3");
    assert!(app.layout.is_enabled(dp13));
    assert_eq!(app.pending().len(), 1);
    press(&mut app, "o");
    assert_eq!(app.layout.size(dp13).w, 2160, "rotated right, portrait");
    press(&mut app, "p");
    assert!(app.layout.outputs[dp13].primary);
    press(&mut app, "1 2 3 4 ");
    assert_eq!(status(&app), "The last enabled display stays on.");
    assert_eq!(app.layout.enabled(), [dp13]);
}

#[test]
fn command_line() {
    let mut app = demo();
    press(&mut app, "3:pos 0 1440<Enter>");
    assert_eq!(app.mode, UiMode::Normal);
    assert_eq!(rect(&app.layout, "eDP-1"), Rect::new(0, 1440, 1920, 1080));
    assert_eq!(
        link(&app.layout, "eDP-1"),
        stuck("HDMI-1-0", Side::Below, Align::Center, 0)
    );
    press(&mut app, ":move 100 0<Enter>");
    assert_eq!(rect(&app.layout, "eDP-1").x, 100);
    press(&mut app, ":mode 1280x720<Enter>");
    assert_eq!(rect(&app.layout, "eDP-1").w, 1280);
    press(&mut app, ":stick hdmi below 3 center<Enter>");
    assert_eq!(
        link(&app.layout, "HDMI-1-0"),
        stuck("eDP-1", Side::Below, Align::Center, 0)
    );
    press(&mut app, ":off DP-1-2<Enter>");
    assert!(!app.layout.is_enabled(ix(&app.layout, "DP-1-2")));
    press(&mut app, ":on 2<Enter>");
    assert!(app.layout.is_enabled(ix(&app.layout, "DP-1-2")));
    press(&mut app, ":stick DP 1 right-of<Enter>");
    assert!(
        status(&app).starts_with("usage: :stick"),
        "{}",
        status(&app)
    );
    press(&mut app, ":stick DP right-of 1<Enter>");
    assert_eq!(status(&app), "\"DP\" could be DP-1-2 or DP-1-3.");
    press(&mut app, ":frob<Enter>");
    assert!(status(&app).starts_with("unknown command"));
    press(&mut app, ":xy<BS><BS><BS>");
    assert_eq!(
        app.mode,
        UiMode::Normal,
        "Backspace on an empty line cancels"
    );
    press(&mut app, ":q<Esc>");
    assert_eq!(app.mode, UiMode::Normal);
}

#[test]
fn quitting_asks_only_when_changes_are_pending() {
    let mut app = demo();
    assert_eq!(press(&mut app, "q"), [Effect::Quit]);

    let mut app = demo();
    press(&mut app, "3<A-l>");
    assert!(press(&mut app, "q").is_empty());
    assert!(matches!(app.mode, UiMode::Confirm(_)));
    assert!(press(&mut app, "<Esc>").is_empty());
    assert_eq!(app.mode, UiMode::Normal);
    assert!(press(&mut app, "<C-c>").is_empty());
    assert_eq!(press(&mut app, "<Enter>"), [Effect::Quit]);

    let mut app = demo();
    press(&mut app, "3<A-l>");
    assert_eq!(press(&mut app, ":q!<Enter>"), [Effect::Quit]);
}

#[test]
fn escape_never_quits() {
    let mut app = demo();
    assert!(press(&mut app, "<Esc><Esc>").is_empty());
    press(&mut app, "?");
    assert!(press(&mut app, "<Esc>").is_empty());
    assert_eq!(app.mode, UiMode::Normal);
}

/// The demo as read live: the fixture backend's state.
fn live() -> Snapshot {
    FixtureBackend::demo().query().unwrap()
}

/// The demo before the LG monitor was plugged into DP-1-3.
fn without_dp13() -> App {
    App::new(unplugged(&live(), "DP-1-3"), Options::default())
}

fn watching() -> App {
    let options = Options {
        watch: Some(WATCH_INTERVAL),
        ..Options::default()
    };
    App::new(live(), options)
}

/// The text of a rendered screen, one string per row.
fn rows(app: &mut App) -> Vec<String> {
    let terminal = screen(app, 100, 30);
    let buf = terminal.backend().buffer();
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// The rows of the off tray, below its ` off ` title.
fn off_tray(app: &mut App) -> Vec<String> {
    let rows = rows(app);
    let Some(top) = rows.iter().position(|r| r.contains("├ off ")) else {
        return Vec::new();
    };
    rows[top + 1..rows.len() - 3]
        .iter()
        .map(|r| r.rsplit('│').nth(1).unwrap_or_default().trim().to_owned())
        .filter(|r| !r.is_empty())
        .collect()
}

#[test]
fn the_watch_asks_every_two_seconds_in_normal_mode_only() {
    let watch = [Effect::Refresh { probe: false }];
    let mut app = watching();
    let start = app.now;
    let at = |ms: u64| start + Duration::from_millis(ms);
    assert!(app.tick(at(1999)).is_empty());
    assert_eq!(app.tick(at(2000)), watch);
    assert!(app.tick(at(2250)).is_empty(), "not again at once");
    assert_eq!(app.tick(at(4000)), watch);

    // Not with a picker open; as soon as it closes.
    press(&mut app, "m");
    assert!(app.tick(at(9000)).is_empty());
    press(&mut app, "<Esc>");
    assert_eq!(app.tick(at(9250)), watch);

    // Not during the countdown, however long it runs.
    app.countdown(at(10_000), 15);
    for ms in (10_250..25_000).step_by(250) {
        assert!(app.tick(at(ms)).is_empty(), "at {ms} ms");
    }

    // A key opened a popup after the watch asked: the reading is dropped, and the next tick
    // in normal mode asks again.
    let mut app = watching();
    let start = app.now;
    assert_eq!(app.tick(start + WATCH_INTERVAL), watch);
    press(&mut app, "m");
    app.refreshed(unplugged(&live(), "DP-1-3"), false);
    assert!(matches!(app.mode, UiMode::Picker(_)));
    assert_eq!(app.snap, live(), "nothing merged under the picker");
    press(&mut app, "<Esc>");
    let next = start + WATCH_INTERVAL + Duration::from_millis(250);
    assert_eq!(app.tick(next), watch);

    // Off unless asked for.
    let mut app = demo();
    assert!(app.tick(app.now + Duration::from_secs(60)).is_empty());
}

#[test]
fn r_refreshes_at_once_and_keeps_the_edits() {
    let mut app = demo();
    assert_eq!(press(&mut app, "R"), [Effect::Refresh { probe: true }]);
    press(&mut app, "3<A-l>");
    assert_eq!(
        press(&mut app, "R"),
        [Effect::Refresh { probe: true }],
        "no question with edits pending"
    );
    assert_eq!(app.mode, UiMode::Normal);
    app.refreshed(live(), true);
    assert_eq!(app.pending().len(), 1, "the edit is still pending");
    assert!(app.history.can_undo());
    assert_eq!(status(&app), "No display changes.");
}

#[test]
fn an_identical_reading_changes_nothing_and_says_nothing() {
    let mut app = demo();
    press(&mut app, "3<A-l>1");
    let (layout, focus) = (app.layout.clone(), app.focus);
    app.status = None;
    app.refreshed(live(), false);
    assert_eq!(app.layout, layout);
    assert_eq!(app.focus, focus);
    assert_eq!(app.history.undo_len(), 1);
    assert_eq!(app.status, None, "the watch stays silent");
}

#[test]
fn a_display_plugged_in_shows_up_off_with_focus() {
    let mut app = without_dp13();
    assert_eq!(app.snap.numbered().len(), 3);
    assert!(
        off_tray(&mut app).is_empty(),
        "every connected display is on"
    );
    press(&mut app, "3<A-l>");
    let edited = app.layout.clone();

    app.refreshed(live(), false);
    let dp13 = ix(&app.layout, "DP-1-3");
    assert_eq!(app.layout.numbers[dp13], Some(4));
    assert_eq!(app.focus, dp13);
    assert!(!app.layout.is_enabled(dp13));
    assert_eq!(status(&app), "DP-1-3 connected: Space turns it on.");
    assert_eq!(off_tray(&mut app), ["4 DP-1-3    LG HDR 4K"]);

    // The edit, the links and the numbers of the others survive.
    let changes: Vec<String> = app.pending().iter().map(ToString::to_string).collect();
    assert_eq!(changes, ["eDP-1  pos 2240,1440 → 2250,1440"]);
    for name in ["HDMI-1-0", "DP-1-2", "eDP-1"] {
        assert_eq!(rect(&app.layout, name), rect(&edited, name), "{name}");
        assert_eq!(link(&app.layout, name), link(&edited, name), "{name}");
        let (i, e) = (ix(&app.layout, name), ix(&edited, name));
        assert_eq!(app.layout.numbers[i], edited.numbers[e], "{name}");
    }
    press(&mut app, "4");
    assert_eq!(app.focus, dp13, "4 reaches it");
    press(&mut app, ":off 3<Enter>");
    assert!(
        !app.layout.is_enabled(ix(&app.layout, "eDP-1")),
        "so does :off 3"
    );
    press(&mut app, "u");

    // The undo survives too.
    press(&mut app, "u");
    assert!(app.pending().is_empty());
    assert_eq!(status(&app), "");

    press(&mut app, " ");
    assert!(app.layout.is_enabled(dp13), "Space turns it on");
    assert!(off_tray(&mut app).is_empty());
}

#[test]
fn a_display_unplugged_while_on_is_turned_off_and_u_brings_it_back() {
    let mut app = demo();
    press(&mut app, "3<A-l>");
    app.refreshed(unplugged(&live(), "HDMI-1-0"), false);

    let hdmi = ix(&app.layout, "HDMI-1-0");
    assert!(!app.layout.is_enabled(hdmi));
    let text = app.status.clone().unwrap();
    assert_eq!(
        text.text,
        "HDMI-1-0 was unplugged while on; it is turned off in the pending layout."
    );
    assert_eq!(text.severity, Severity::Warning);
    assert_eq!(
        app.layout.numbers[hdmi],
        Some(1),
        "still active, still numbered"
    );
    let changes: Vec<String> = app.pending().iter().map(ToString::to_string).collect();
    assert_eq!(
        changes,
        [
            "HDMI-1-0  on → off",
            "DP-1-2  pos 1920,0 → 0,0",
            "eDP-1  pos 2240,1440 → 330,1440",
        ]
    );

    press(&mut app, "u");
    assert!(app.layout.is_enabled(hdmi), "u brings it back");
    let changes: Vec<String> = app.pending().iter().map(ToString::to_string).collect();
    assert_eq!(
        changes,
        ["eDP-1  pos 2240,1440 → 2250,1440"],
        "the edit stays"
    );
    press(&mut app, "u");
    assert!(app.pending().is_empty());

    // An unplugged display that was already off is only reported.
    let mut app = demo();
    app.refreshed(unplugged(&live(), "DP-1-3"), false);
    assert_eq!(status(&app), "DP-1-3 disconnected.");
    assert!(app.pending().is_empty());
    assert!(!app.history.can_undo());
    assert_eq!(app.layout.numbers[ix(&app.layout, "DP-1-3")], None);
}

#[test]
fn a_display_plugged_back_in_returns_to_its_place() {
    let mut app = demo();
    press(&mut app, "1");
    app.refreshed(unplugged(&live(), "HDMI-1-0"), false);
    app.refreshed(live(), false);
    let hdmi = ix(&app.layout, "HDMI-1-0");
    assert_eq!(app.focus, hdmi);
    assert_eq!(status(&app), "HDMI-1-0 connected: Space turns it on.");
    press(&mut app, " ");
    assert!(app.pending().is_empty(), "back where it was");
}

#[test]
fn a_layout_changed_outside_outlay() {
    let mut moved = live();
    let i = moved.find("eDP-1").unwrap();
    moved.outputs[i].active.as_mut().unwrap().pos.x = 2000;

    // Nothing was pending: the editor follows the live layout.
    let mut app = demo();
    app.refreshed(moved.clone(), false);
    assert!(app.pending().is_empty());
    assert_eq!(rect(&app.layout, "eDP-1").x, 2000);
    assert_eq!(status(&app), "The live state changed outside outlay.");

    // With edits pending, the pending layout stays as it was.
    let mut app = demo();
    press(&mut app, "1:move 0 -10<Enter>");
    let edited = app.layout.clone();
    app.refreshed(moved, false);
    assert_eq!(app.layout, edited);
    assert_eq!(
        status(&app),
        "The live state changed outside outlay. Your edits are still pending."
    );
    assert_eq!(app.pending().len(), 2);
}

#[test]
fn stale_outputs_are_announced() {
    let backend = FixtureBackend::from_file(
        format!(
            "{}/tests/fixtures/xrandr/active-disconnected.txt",
            env!("CARGO_MANIFEST_DIR")
        )
        .as_ref(),
    )
    .unwrap();
    let app = App::new(backend.query().unwrap(), Options::default());
    let status = app.status.clone().unwrap();
    assert_eq!(status.severity, Severity::Warning);
    assert!(
        status.text.contains("disconnected but still active"),
        "{}",
        status.text
    );
    let changes: Vec<String> = app.pending().iter().map(|d| d.to_string()).collect();
    assert_eq!(changes, ["eDP-1  pos 2560,0 → 0,0", "DP-1  on → off"]);
}

#[test]
fn the_focused_title_is_a_reversed_chip() {
    let mut app = demo();
    let term = screen(&mut app, 100, 30);
    assert_eq!(
        marked(term.backend().buffer(), Modifier::REVERSED),
        [" 2 DP-1-2 ★ "],
        "only the focused title is reversed"
    );

    press(&mut app, "3");
    let term = screen(&mut app, 100, 30);
    assert_eq!(
        marked(term.backend().buffer(), Modifier::REVERSED),
        [" 3 eDP-1 "]
    );

    // In the stick flow the ghost's label is not a chip.
    press(&mut app, "s2h");
    let term = screen(&mut app, 100, 30);
    assert_eq!(
        marked(term.backend().buffer(), Modifier::REVERSED),
        [" 3 eDP-1 "]
    );

    // Without colour, reverse video alone marks the focus.
    let options = Options {
        theme: Theme::plain(),
        ..Options::default()
    };
    let mut app = App::new(FixtureBackend::demo().query().unwrap(), options);
    let term = screen(&mut app, 100, 30);
    let buf = term.backend().buffer();
    assert_eq!(marked(buf, Modifier::REVERSED), [" 2 DP-1-2 ★ "]);
    assert!(
        buf.content()
            .iter()
            .all(|c| c.fg == ratatui::style::Color::Reset)
    );
}

#[test]
fn a_chip_that_does_not_fit_fills_the_box() {
    let (snap, _) = common::load(&[
        common::on("A", 3840, 2160, 0, 0).primary(),
        common::on("LONG-OUTPUT-NAME", 1280, 1024, 3840, 0),
    ]);
    let mut app = App::new(snap, Options::default());
    press(&mut app, "2");
    let term = screen(&mut app, 60, 16);
    let chip = marked(term.backend().buffer(), Modifier::REVERSED);
    assert_eq!(chip.len(), 1, "{chip:?}");
    assert!(chip[0].ends_with('…'), "{chip:?}");
    assert!(!chip[0].starts_with(' '), "{chip:?}");
    let text = term.backend().to_string();
    assert!(text.contains(&format!("┃{}┃", chip[0])), "{text}");
}

#[test]
fn a_tiny_focused_display_shows_its_number_reversed() {
    let (snap, _) = common::load(&[
        common::on("A", 15000, 2000, 0, 0).primary(),
        common::on("B", 100, 100, 15000, 0),
    ]);
    let mut app = App::new(snap, Options::default());
    press(&mut app, "2");
    let term = screen(&mut app, 60, 16);
    assert_eq!(marked(term.backend().buffer(), Modifier::REVERSED), ["2"]);
}

#[test]
fn no_double_borders_by_default() {
    let mut app = demo();
    press(&mut app, "3");
    let term = screen(&mut app, 100, 30);
    assert_eq!(
        double_lines(term.backend().buffer()),
        0,
        "the parent is not marked"
    );

    // The stick target is thick, with a bold, underlined title.
    press(&mut app, "s2h");
    let term = screen(&mut app, 100, 30);
    let buf = term.backend().buffer();
    assert_eq!(double_lines(buf), 0, "{}", term.backend());
    assert_eq!(marked(buf, Modifier::UNDERLINED), ["2 DP-1-2 ★"]);
    assert!(term.backend().to_string().contains('┓'));
}

#[test]
fn double_borders_bring_the_old_look_back() {
    let options = Options {
        double_borders: true,
        ..Options::default()
    };
    let mut app = App::new(FixtureBackend::demo().query().unwrap(), options);
    press(&mut app, "3");
    let term = screen(&mut app, 100, 30);
    assert!(
        double_lines(term.backend().buffer()) > 0,
        "the parent is double"
    );

    press(&mut app, "s2h");
    let term = screen(&mut app, 100, 30);
    let buf = term.backend().buffer();
    assert!(double_lines(buf) > 0, "the target is double");
    assert!(marked(buf, Modifier::UNDERLINED).is_empty());
}

#[test]
fn snapshot_demo_overview() {
    let mut app = demo();
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_stick_ghost() {
    let mut app = demo();
    press(&mut app, "3s2h");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_mode_picker() {
    let mut app = demo();
    press(&mut app, "m");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_scale_picker() {
    let mut app = demo();
    press(&mut app, "3x");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_small_terminal() {
    let mut app = demo();
    insta::assert_snapshot!(screen(&mut app, 60, 20).backend());
}

#[test]
fn snapshot_overlap() {
    let mut app = demo();
    press(&mut app, "3:move 0 -500<Enter>");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_help() {
    let mut app = demo();
    press(&mut app, "?");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_plugged_in() {
    let mut app = without_dp13();
    press(&mut app, "3<A-l>");
    app.refreshed(live(), false);
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_too_small() {
    let mut app = demo();
    insta::assert_snapshot!(screen(&mut app, 50, 12).backend());
}

#[test]
fn snapshot_apply_confirmation() {
    let mut app = demo();
    press(&mut app, "3<A-l>4 a");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_countdown() {
    let mut app = demo();
    press(&mut app, "3<A-l>");
    let start = app.now;
    app.countdown(start, 15);
    app.tick(start + std::time::Duration::from_millis(4500));
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_apply_failed() {
    let mut app = demo();
    app.report(
        "Apply failed",
        vec![
            "xrandr: warning: output DP-1-3 not found; ignoring".to_owned(),
            "The layout did not come out as asked:".to_owned(),
            "  DP-1-3 is off; it should be on.".to_owned(),
            "Reverted to the previous layout.".to_owned(),
        ],
    );
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

fn profile_item(app: &App, name: &str) -> outlay::tui::app::ProfileItem {
    let path = format!(
        "{}/tests/fixtures/screenlayout/{name}.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap();
    let stored = outlay::profiles::Stored {
        name: name.to_owned(),
        path: format!("/layouts/{name}.sh").into(),
        profile: outlay::xrandr::script::Profile::parse(&text),
    };
    outlay::tui::app::ProfileItem::new(&app.snap, stored)
}

#[test]
fn snapshot_profile_picker() {
    let mut app = demo();
    let items = ["home-setup", "monitors-only-dynamic", "tv-home"]
        .map(|n| profile_item(&app, n))
        .to_vec();
    app.open_profiles(items, "/layouts");
    press(&mut app, "jj");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_remap() {
    let mut app = demo();
    let item = profile_item(&app, "mirror-work-setup");
    app.open_profile(item);
    press(&mut app, "l");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_overwrite() {
    let mut app = demo();
    let old = "#!/bin/sh\nxrandr --output HDMI-1-0 --off \\\n       --output eDP-1 --auto\n";
    let new = "#!/bin/sh\n# generated by outlay 0.1.0\nxrandr --output HDMI-1-0 --auto \\\n       --output eDP-1 --auto\n";
    app.confirm_overwrite(outlay::tui::app::SavePlan {
        name: "desk".to_owned(),
        path: "/layouts/desk.sh".into(),
        text: new.to_owned(),
        diff: outlay::files::line_diff(old, new),
    });
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

fn animated() -> App {
    let options = Options {
        animations: true,
        ..Options::default()
    };
    App::new(FixtureBackend::demo().query().unwrap(), options)
}

#[test]
fn a_swap_glides_into_place_with_an_ease_out() {
    let mut app = animated();
    let start = app.now;
    press(&mut app, "1L");
    assert_eq!(
        rect(&app.layout, "HDMI-1-0").x,
        2560,
        "the edit is done at once"
    );
    let drawn_x = |app: &App| rect(&app.drawn_layout(), "HDMI-1-0").x;
    assert!(app.animating());
    assert_eq!(drawn_x(&app), 0, "drawn where it was");

    app.tick(start + Duration::from_millis(40));
    let early = drawn_x(&app);
    app.tick(start + Duration::from_millis(80));
    let late = drawn_x(&app);
    assert!(0 < early && early < late && late < 2560, "{early} {late}");
    assert!(
        early > 2560 / 3,
        "ease-out: most of the way comes first: {early}"
    );

    app.tick(start + Duration::from_millis(120));
    assert!(!app.animating());
    assert_eq!(drawn_x(&app), 2560);
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("1 HDMI-1-0"), "{text}");
}

#[test]
fn nudges_never_glide_and_cut_a_glide_short() {
    let mut app = animated();
    press(&mut app, "3<A-l>");
    assert!(!app.animating());
    press(&mut app, "L");
    assert!(app.animating(), "a snap-move glides");
    press(&mut app, "<A-h>");
    assert!(!app.animating(), "a nudge ends the glide at once");
    assert_eq!(*app.drawn_layout(), app.layout);

    // Undo glides back; with animations off nothing glides.
    press(&mut app, "u");
    assert!(app.animating());
    let mut still = demo();
    press(&mut still, "1L");
    assert!(!still.animating());
}

#[test]
fn plus_and_minus_change_the_step() {
    let mut app = demo();
    press(&mut app, "+");
    assert_eq!(
        (app.step, status(&app)),
        (50, "Nudge step 50 px.".to_owned())
    );
    press(&mut app, "++");
    assert_eq!(app.step, 100);
    assert_eq!(status(&app), "100 px is the largest nudge step.");
    press(&mut app, "----");
    assert_eq!(app.step, 1);
    press(&mut app, "-");
    assert_eq!(status(&app), "1 px is the smallest nudge step.");
    press(&mut app, "3<A-l>");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2241);
    let text = screen(&mut app, 100, 30).backend().to_string();
    assert!(text.contains("step 1px "), "{text}");
}

#[test]
fn tab_completes_on_the_command_line() {
    let mut app = demo();
    press(&mut app, ":sti<Tab>3 be<Tab>dp<Tab>");
    assert_eq!(app.mode, UiMode::Command("stick 3 below DP-1-".to_owned()));
    assert_eq!(status(&app), "DP-1-2  DP-1-3");
    press(&mut app, "2<Enter>");
    assert_eq!(
        link(&app.layout, "eDP-1"),
        stuck("DP-1-2", Side::Below, Align::Center, 0)
    );
}

// --- Wayland ----------------------------------------------------------------------------------

fn wayland_demo() -> App {
    App::new(
        FixtureBackend::demo_wayland().query().unwrap(),
        Options::default(),
    )
}

fn text(term: &Terminal<TestBackend>) -> String {
    let buf = term.backend().buffer();
    let area = buf.area;
    (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

#[test]
fn wayland_numbers_the_built_in_panel_first() {
    let app = wayland_demo();
    let numbered: Vec<String> = app
        .snap
        .numbered()
        .iter()
        .map(|&i| app.layout.label(i))
        .collect();
    assert_eq!(numbered, ["1 eDP-1", "2 DP-3", "3 DP-4", "4 HDMI-A-1"]);
    assert_eq!(app.layout.names[app.focus], "eDP-1", "no primary to focus");
    assert!(app.issues.is_empty(), "{:?}", app.issues);
}

#[test]
fn wayland_has_no_primary_and_no_mirror() {
    let mut app = wayland_demo();
    press(&mut app, "p");
    assert_eq!(status(&app), "Wayland compositors have no primary display.");
    assert!(!app.history.can_undo(), "no undo step");
    assert_eq!(app.layout.primary(), None);

    press(&mut app, ":primary 2<Enter>");
    assert_eq!(status(&app), "Wayland compositors have no primary display.");
    press(&mut app, ":stick 4 same-as 2<Enter>");
    assert_eq!(status(&app), "This compositor cannot mirror displays.");
    assert!(!app.history.can_undo());

    press(&mut app, "s2");
    let UiMode::Stick(flow) = &app.mode else {
        panic!("not sticking: {:?}", app.mode)
    };
    assert_eq!(flow.step, StickStep::Side);
    press(&mut app, "=");
    let UiMode::Stick(flow) = &app.mode else {
        panic!("the flow goes on: {:?}", app.mode)
    };
    assert_ne!(flow.side, Side::Same);
    assert_eq!(
        app.stick_summary(flow),
        "Stick 1 eDP-1 below 2 DP-3, centre (This compositor cannot mirror displays)"
    );
    let shown = text(&screen(&mut app, 120, 30));
    let hints = shown.lines().last().unwrap();
    assert!(
        hints.contains("side") && !hints.contains("mirror"),
        "{hints}"
    );
    press(&mut app, "<Esc>");
    assert!(!app.history.can_undo());

    press(&mut app, "?");
    let help = text(&screen(&mut app, 100, 200));
    assert!(!help.contains("Make primary"), "{help}");
    assert!(!help.contains("same-as"), "{help}");
    assert!(help.contains(":reflect normal|x "), "{help}");
    assert!(help.contains("Pick a scale"), "{help}");
}

#[test]
fn wayland_refuses_profiles_until_kanshi() {
    let mut app = wayland_demo();
    for keys in ["w", "e", ":w home<Enter>", ":e<Enter>"] {
        let effects = press(&mut app, keys);
        assert!(effects.is_empty(), "{keys}: {effects:?}");
        assert_eq!(app.mode, UiMode::Normal, "{keys}");
        assert_eq!(
            status(&app),
            "Profiles on Wayland are kanshi profiles, which arrive in the next version of outlay.",
            "{keys}"
        );
    }
}

#[test]
fn wayland_turns_a_new_head_on_at_100_percent() {
    let mut app = wayland_demo();
    press(&mut app, "3 ");
    let dp4 = ix(&app.layout, "DP-4");
    assert!(app.layout.is_enabled(dp4));
    assert_eq!(status(&app), "3 DP-4 starts at 100%: x picks a scale.");
    assert_eq!(rect(&app.layout, "DP-4"), Rect::new(4480, 0, 3840, 2160));
    // Back off and on again: it has been on now, so it says nothing more.
    press(&mut app, "  ");
    assert_eq!(status(&app), "");
}

#[test]
fn wayland_reflect_y_is_reflect_x_upside_down() {
    let mut app = wayland_demo();
    press(&mut app, ":reflect y<Enter>");
    let st = &app.layout.outputs[app.focus];
    assert_eq!(
        (st.rotation, st.reflection),
        (
            outlay::model::Rotation::Inverted,
            outlay::model::Reflection::X
        )
    );
    assert_eq!(
        status(&app),
        "Wayland reflects only in x: reflect y is the same picture as rotate inverted, reflect x."
    );
    let (line, candidates) = outlay::tui::cmdline::complete("reflect ", &app.layout);
    assert_eq!((line.as_str(), candidates.len()), ("reflect ", 2));
}

struct NoInput;

impl outlay::tui::session::Input for NoInput {
    fn drain(&mut self) {}
}

#[test]
fn the_wayland_demo_sticks_scales_applies_and_reverts() {
    use outlay::tui::session::{Session, Settings};
    use ratatui::crossterm::event::Event;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let backend = FixtureBackend::demo_wayland();
    let original = backend.query().unwrap();
    let app = App::new(original.clone(), Options::default());
    let settings = Settings::default();
    let mut session = Session::new(app, &backend, settings, Arc::new(AtomicBool::new(false)));
    let press = |s: &mut Session, script: &str| {
        for key in keys(script) {
            s.handle_event(&Event::Key(key), s.app.now);
        }
        s.perform(&mut NoInput);
    };

    // eDP-1 from 200 % down to 150 %: 1920x1200 logical, still centred below DP-3.
    press(&mut session, "1<lt><lt>");
    assert_eq!(
        rect(&session.app.layout, "eDP-1"),
        Rect::new(2240, 1440, 1920, 1200)
    );
    assert_eq!(
        link(&session.app.layout, "eDP-1"),
        stuck("DP-3", Side::Below, Align::Center, 0)
    );
    // The TV goes left of the laptop, top-aligned.
    press(&mut session, "4s1h<Enter>");
    assert_eq!(
        link(&session.app.layout, "HDMI-A-1"),
        stuck("eDP-1", Side::LeftOf, Align::Start, 0)
    );
    assert_eq!(
        rect(&session.app.layout, "HDMI-A-1"),
        Rect::new(0, 1440, 1920, 1080),
        "everything shifts right by 320 to start at 0"
    );

    press(&mut session, "a");
    let UiMode::ConfirmApply(preview) = &session.app.mode else {
        panic!("no confirmation: {:?}", session.app.mode)
    };
    assert_eq!(
        preview.command,
        "wlr-randr --output eDP-1 --on --mode 2880x1800@60.001Hz --pos 1920,1440 \
         --transform normal --scale 1.5 \
         --output DP-3 --on --mode 2560x1440@143.912Hz --pos 1600,0 --transform normal --scale 1 \
         --output DP-4 --off \
         --output HDMI-A-1 --on --mode 1920x1080@59.940Hz --pos 0,1440 --transform normal \
         --scale 1"
    );
    assert!(preview.errors.is_empty(), "{:?}", preview.errors);
    let changes = preview.changes.join("\n");
    assert!(
        changes.contains("eDP-1  pos 2480,1440 → 1920,1440, scale 200% → 150%"),
        "{changes}"
    );

    press(&mut session, "<Enter>");
    let UiMode::Countdown(c) = session.app.mode else {
        panic!("no countdown: {:?}", session.app.mode)
    };
    assert_eq!(
        status_of(&session.app),
        "Simulated: nothing was sent to the displays."
    );
    let applied = backend.query().unwrap();
    let edp = &applied.outputs[applied.find("eDP-1").unwrap()];
    let a = edp.active.as_ref().unwrap();
    assert_eq!(
        (a.pos.x, a.pos.y, a.size.w, a.size.h),
        (1920, 1440, 1920, 1200)
    );
    assert_eq!(a.scaling, outlay::model::Scaling::Logical(1.5));

    session.tick(c.deadline + Duration::from_millis(1));
    session.perform(&mut NoInput);
    assert_eq!(session.app.mode, UiMode::Normal);
    assert_eq!(backend.query().unwrap(), original, "reverted");
    assert!(status_of(&session.app).starts_with("No answer in 15 s: reverted"));
    assert_eq!(
        session.app.pending().len(),
        3,
        "the edits are still pending"
    );
    let plans = backend.applied();
    assert_eq!(plans.len(), 2, "the apply and the revert");
    assert_eq!(plans[1].form, outlay::backend::PlanForm::Restore);
}

fn status_of(app: &App) -> String {
    status(app)
}

#[test]
fn snapshot_wayland_overview() {
    let mut app = wayland_demo();
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}

#[test]
fn snapshot_wayland_apply_confirmation() {
    let mut app = wayland_demo();
    press(&mut app, "1<lt>a");
    insta::assert_snapshot!(screen(&mut app, 100, 30).backend());
}
