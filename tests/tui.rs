//! The editor without a terminal: key sequences through `App::handle_key`, and screens rendered
//! into ratatui's `TestBackend` and compared with insta snapshots.

mod common;

use common::{ix, keys, link, rect, stuck};
use outlay::model::geometry::Rect;
use outlay::model::links::{Align, Side};
use outlay::model::validate::Severity;
use outlay::tui::app::{App, Effect, Options, StickStep, UiMode};
use outlay::tui::ui;
use outlay::xrandr::{Backend, FixtureBackend};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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
    assert_eq!(app.history.undo_len(), 2);
    press(&mut app, "u");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2250);
    press(&mut app, "<C-r>");
    assert_eq!(rect(&app.layout, "eDP-1").x, 2260);
    press(&mut app, "<C-r>");
    assert_eq!(status(&app), "Nothing to redo.");
    press(&mut app, "uuu");
    assert_eq!(status(&app), "Nothing to undo.");
    assert!(app.pending().is_empty());
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

#[test]
fn reload_starts_over_from_the_live_state() {
    let mut app = demo();
    assert_eq!(press(&mut app, "R"), [Effect::Query]);
    press(&mut app, "3<A-l>");
    assert!(press(&mut app, "R").is_empty());
    assert_eq!(press(&mut app, "y"), [Effect::Query]);
    app.reloaded(FixtureBackend::demo().query().unwrap());
    assert!(app.pending().is_empty());
    assert!(!app.history.can_undo());
    assert_eq!(app.layout.names[app.focus], "eDP-1", "focus stays on eDP-1");
    assert_eq!(status(&app), "Reloaded the live state.");
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
fn snapshot_too_small() {
    let mut app = demo();
    insta::assert_snapshot!(screen(&mut app, 50, 12).backend());
}
