//! Screenlayout profiles: the user's own scripts (copied into `tests/fixtures/screenlayout/`)
//! loaded onto captured and synthetic xrandr states, the remap of outputs that are not
//! connected, and the round trip layout → script → layout.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use common::{ix, keys, link, rect, stuck};
use outlay::backend::{Backend, FixtureBackend};
use outlay::model::Snapshot;
use outlay::model::geometry::{Dir, Rect};
use outlay::model::layout::Layout;
use outlay::model::links::{Align, Side};
use outlay::tui::app::{App, Options, UiMode};
use outlay::tui::session::{Input, Session, Settings};
use outlay::xrandr::command;
use outlay::xrandr::script::{Profile, Remap};
use ratatui::crossterm::event::Event;

fn capture(name: &str) -> Snapshot {
    let path = format!(
        "{}/tests/fixtures/xrandr/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    FixtureBackend::from_file(path.as_ref())
        .unwrap()
        .query()
        .unwrap()
}

fn profile(name: &str) -> Profile {
    let path = format!(
        "{}/tests/fixtures/screenlayout/{name}.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    Profile::parse(&std::fs::read_to_string(path).unwrap())
}

/// The profile loaded with its default remap.
fn load(snap: &Snapshot, name: &str) -> (Layout, Vec<String>) {
    let p = profile(name);
    let remap = p.default_remap(snap);
    p.layout(snap, &remap)
}

fn remap(snap: &Snapshot, pairs: &[(&str, Option<&str>)]) -> Remap {
    pairs
        .iter()
        .map(|&(from, to)| (from.to_owned(), to.map(|n| snap.find(n).unwrap())))
        .collect()
}

fn mode_of(layout: &Layout, name: &str) -> String {
    let i = ix(layout, name);
    layout.outputs[i].mode.as_ref().unwrap().summary()
}

#[test]
fn home_setup_maps_edp2_to_the_laptop_panel() {
    let snap = FixtureBackend::demo().query().unwrap();
    let p = profile("home-setup");
    assert_eq!(p.unmatched(&snap), ["eDP-2"]);
    assert_eq!(
        p.default_remap(&snap),
        remap(&snap, &[("eDP-2", Some("eDP-1"))]),
        "a free output with the same connector"
    );
    let (layout, notes) = load(&snap, "home-setup");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(0, 853, 1920, 1080));
    assert_eq!(rect(&layout, "DP-1-2"), Rect::new(1920, 0, 2560, 1440));
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(1920, 1440, 1920, 1080));
    assert_eq!(mode_of(&layout, "eDP-1"), "1920x1080@165.01");
    assert_eq!(mode_of(&layout, "DP-1-2"), "2560x1440@143.91");
    assert_eq!(
        mode_of(&layout, "HDMI-1-0"),
        "1920x1080@59.94",
        "no --rate: the first mode with that name"
    );
    assert_eq!(layout.primary(), Some(ix(&layout, "DP-1-2")));
    assert!(
        !layout.is_enabled(ix(&layout, "DP-1-3")),
        "not mentioned: left as it is"
    );
    assert_eq!(
        link(&layout, "eDP-1"),
        stuck("DP-1-2", Side::Below, Align::Start, 0)
    );
    assert_eq!(
        link(&layout, "HDMI-1-0"),
        stuck("DP-1-2", Side::LeftOf, Align::End, 493)
    );
}

#[test]
fn home_setup_save_on_the_demo() {
    let snap = FixtureBackend::demo().query().unwrap();
    let (layout, notes) = load(&snap, "home-setup_save");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(mode_of(&layout, "HDMI-1-0"), "1920x1080@60.00");
    assert_eq!(rect(&layout, "DP-1-2"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(2560, 0, 1920, 1080));
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(0, 1440, 1920, 1080));
}

#[test]
fn a_rate_that_does_not_exist_picks_the_nearest_and_a_missing_output_is_skipped() {
    let snap = capture("laptop-edp-hdmi");
    let p = profile("home-setup_save");
    assert_eq!(p.unmatched(&snap), ["eDP-2", "DP-1-2"]);
    assert_eq!(
        p.default_remap(&snap),
        remap(&snap, &[("eDP-2", Some("eDP-1")), ("DP-1-2", None)])
    );
    let (layout, notes) = load(&snap, "home-setup_save");
    assert_eq!(notes, ["DP-1-2 is not connected; skipped."]);
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(0, 1440, 1920, 1080));
    // The profile asks for 60.00 Hz; the TV offers 59.94, 50.00 and slower at 1920x1080.
    assert_eq!(mode_of(&layout, "HDMI-1-0"), "1920x1080@59.94");
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(2560, 0, 1920, 1080));
    assert_eq!(
        layout.primary(),
        Some(ix(&layout, "eDP-1")),
        "the profile's primary was skipped, so the live one stays"
    );
}

#[test]
fn tv_home_on_the_live_capture() {
    let snap = capture("laptop-edp-hdmi");
    let p = profile("tv-home");
    assert!(p.unmatched(&snap).is_empty());
    let (layout, notes) = load(&snap, "tv-home");
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(mode_of(&layout, "HDMI-1-0"), "2560x1440@59.95");
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(0, 1440, 1920, 1080));
    assert_eq!(layout.primary(), Some(ix(&layout, "eDP-1")));
    assert_eq!(
        link(&layout, "HDMI-1-0"),
        stuck("eDP-1", Side::Above, Align::Start, 0),
        "the primary is the root"
    );
    let changes: Vec<String> = layout.diff(&snap).iter().map(|d| d.to_string()).collect();
    assert_eq!(changes.len(), 2, "{changes:?}");
}

#[test]
fn a_repeated_off_and_a_profile_that_leaves_an_output_alone() {
    let snap = FixtureBackend::demo().query().unwrap();
    let (layout, notes) = load(&snap, "monitors-only-dynamic");
    assert!(notes.is_empty(), "{notes:?}");
    assert!(!layout.is_enabled(ix(&layout, "eDP-1")));
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(rect(&layout, "DP-1-2"), Rect::new(1920, 0, 1920, 1080));
    assert_eq!(mode_of(&layout, "DP-1-2"), "1920x1080@119.88");
    assert_eq!(layout.primary(), Some(ix(&layout, "DP-1-2")));
}

#[test]
fn work_setup_on_a_dock_with_a_chosen_remap() {
    let snap = capture("dock-mst-3");
    let p = profile("work-setup-2");
    assert_eq!(p.unmatched(&snap), ["DP-4"]);
    assert_eq!(
        p.default_remap(&snap),
        remap(&snap, &[("DP-4", Some("DP-2.3"))]),
        "the first free DP output"
    );
    let (layout, notes) = p.layout(&snap, &remap(&snap, &[("DP-4", Some("eDP-1"))]));
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(rect(&layout, "DP-2.1"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect(&layout, "DP-2.2"), Rect::new(2560, 0, 2560, 1440));
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(5120, 360, 1920, 1080));
    assert!(!layout.is_enabled(ix(&layout, "DP-2.3")));
    assert_eq!(layout.primary(), Some(ix(&layout, "DP-2.2")));
    assert!(
        layout.diff(&snap).is_empty(),
        "the dock capture is this very layout"
    );
}

#[test]
fn identical_positions_become_a_mirror() {
    let snap = FixtureBackend::demo().query().unwrap();
    let p = profile("mirror-work-setup");
    assert_eq!(p.unmatched(&snap), ["DVI-I-3-2", "DVI-I-2-1"]);
    assert_eq!(
        p.default_remap(&snap),
        remap(&snap, &[("DVI-I-3-2", None), ("DVI-I-2-1", None)]),
        "no free DVI output"
    );
    let chosen = remap(
        &snap,
        &[("DVI-I-3-2", Some("DP-1-3")), ("DVI-I-2-1", Some("DP-1-2"))],
    );
    let (layout, notes) = p.layout(&snap, &chosen);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(rect(&layout, "DP-1-3"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect(&layout, "DP-1-2"), Rect::new(2560, 0, 2560, 1440));
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(2560, 0, 2560, 1440));
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(5120, 360, 1920, 1080));
    assert_eq!(
        link(&layout, "HDMI-1-0"),
        stuck("DP-1-2", Side::Same, Align::Start, 0),
        "the primary is the one mirrored"
    );
    assert!(layout.overlapping_pairs().is_empty());

    // Skipping both: the mirror has nothing to mirror, and says so.
    let (layout, notes) = load(&snap, "mirror-work-setup");
    assert_eq!(
        notes,
        [
            "DVI-I-3-2 is not connected; skipped.",
            "DVI-I-2-1 is not connected; skipped."
        ]
    );
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(0, 0, 2560, 1440));
}

#[test]
fn a_rotated_profile_starting_at_x_16_is_normalised() {
    let snap = FixtureBackend::demo().query().unwrap();
    let p = profile("home-setup-rotr");
    let chosen = remap(
        &snap,
        &[
            ("DP-4", Some("eDP-1")),
            ("DP-2", Some("DP-1-2")),
            ("HDMI-0", Some("HDMI-1-0")),
        ],
    );
    let (layout, notes) = p.layout(&snap, &chosen);
    assert!(notes.is_empty(), "{notes:?}");
    // xrandr moves the top-left corner to 0,0 too.
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(0, 0, 1920, 1080));
    assert_eq!(rect(&layout, "DP-1-2"), Rect::new(1920, 0, 1440, 2560));
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(1440, 2560, 1920, 1080));
    assert_eq!(
        link(&layout, "eDP-1"),
        stuck("DP-1-2", Side::Below, Align::End, 0)
    );
}

#[test]
fn relative_options_become_links() {
    let snap = FixtureBackend::demo().query().unwrap();
    let p = Profile::parse(
        "xrandr --output DP-1-2 --primary --mode 2560x1440 --pos 0x0 \
         --output eDP-1 --mode 1920x1080 --below DP-1-2 \
         --output HDMI-1-0 --auto --left-of eDP-1 --output DP-1-3 --off\n",
    );
    let (layout, notes) = p.layout(&snap, &Vec::new());
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        link(&layout, "eDP-1"),
        stuck("DP-1-2", Side::Below, Align::Start, 0)
    );
    assert_eq!(
        link(&layout, "HDMI-1-0"),
        stuck("eDP-1", Side::LeftOf, Align::Start, 0)
    );
    assert_eq!(rect(&layout, "HDMI-1-0"), Rect::new(0, 1440, 3840, 2160));
    assert_eq!(rect(&layout, "eDP-1"), Rect::new(3840, 1440, 1920, 1080));
    assert_eq!(rect(&layout, "DP-1-2"), Rect::new(3840, 0, 2560, 1440));
}

/// Layouts that went through the editor: they must survive a save and a load unchanged.
fn edited_layouts() -> Vec<(String, Snapshot, Layout)> {
    let mut out = Vec::new();
    for name in [
        "demo",
        "scaled",
        "mirror",
        "rotated-left",
        "dock-mst-3",
        "laptop-edp-hdmi",
    ] {
        let snap = if name == "demo" {
            FixtureBackend::demo().query().unwrap()
        } else {
            capture(name)
        };
        out.push((
            format!("{name} as it is"),
            snap.clone(),
            Layout::inferred(&snap),
        ));
    }
    let snap = FixtureBackend::demo().query().unwrap();
    let mut layout = Layout::inferred(&snap);
    let dp13 = ix(&layout, "DP-1-3");
    let edp = ix(&layout, "eDP-1");
    layout.toggle(&snap, dp13).unwrap();
    layout.rotate(dp13, true).unwrap();
    layout.nudge(edp, Dir::Right, 50).unwrap();
    layout.set_primary(edp).unwrap();
    layout
        .stick(
            &snap,
            ix(&layout, "HDMI-1-0"),
            edp,
            Side::Below,
            Align::Center,
        )
        .unwrap();
    out.push(("demo, edited".to_owned(), snap, layout));
    out
}

#[test]
fn a_saved_layout_loads_back_the_same() {
    for (what, snap, mut layout) in edited_layouts() {
        // Links are not stored in scripts; loading infers them from the positions.
        layout.infer_links();
        let text = command::script(&layout);
        let p = Profile::parse(&text);
        assert!(p.warnings.is_empty(), "{what}: {:?}", p.warnings);
        assert!(p.unmatched(&snap).is_empty(), "{what}");
        let (loaded, notes) = p.layout(&snap, &Vec::new());
        assert!(notes.is_empty(), "{what}: {notes:?}");
        layout.restore = loaded.restore.clone();
        assert_eq!(loaded, layout, "{what}\n{text}");
    }
}

// --- The editor: `w`, `e`, the picker and the remap dialog, through the session ------------------

/// A fresh, empty directory for one test; never `~/.screenlayout`.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("outlay-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_fixture(dir: &Path, name: &str) {
    let from = format!(
        "{}/tests/fixtures/screenlayout/{name}.sh",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::copy(from, dir.join(format!("{name}.sh"))).unwrap();
}

struct NoInput;

impl Input for NoInput {
    fn drain(&mut self) {}
}

fn session<'a>(backend: &'a FixtureBackend, dir: Option<&Path>) -> Session<'a> {
    let app = App::new(backend.query().unwrap(), Options::default());
    let settings = Settings {
        layouts_dir: dir.map(Path::to_owned),
        ..Settings::default()
    };
    Session::new(app, backend, settings, Arc::new(AtomicBool::new(false)))
}

fn press(s: &mut Session, script: &str) {
    for key in keys(script) {
        s.handle_event(&Event::Key(key), Instant::now());
        s.perform(&mut NoInput);
    }
}

fn status(s: &Session) -> String {
    s.app
        .status
        .as_ref()
        .map(|st| st.text.clone())
        .unwrap_or_default()
}

#[test]
fn w_saves_and_asks_before_overwriting() {
    let dir = scratch("save");
    let backend = FixtureBackend::demo();
    let mut s = session(&backend, Some(&dir));
    press(&mut s, "w");
    assert_eq!(s.app.mode, UiMode::SavePrompt(String::new()));
    press(&mut s, "desk<Enter>");
    let path = dir.join("desk.sh");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.starts_with("#!/bin/sh\n# generated by outlay "),
        "{saved}"
    );
    assert!(saved.contains("--output DP-1-2 --primary --mode 2560x1440 --rate 143.91"));
    assert!(status(&s).starts_with("Saved "), "{}", status(&s));
    assert!(status(&s).ends_with("desk.sh."), "{}", status(&s));
    assert_eq!(s.app.profile.as_deref(), Some("desk"));
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert!(backend.applied().is_empty(), "saving never runs xrandr");

    // A change: the prompt starts with the profile's name and the save asks first.
    press(&mut s, "3<A-l>w");
    assert_eq!(s.app.mode, UiMode::SavePrompt("desk".to_owned()));
    press(&mut s, "<Enter>");
    let UiMode::Overwrite(plan) = &s.app.mode else {
        panic!("no question: {:?}", s.app.mode)
    };
    let changed: Vec<&String> = plan.diff.iter().filter(|l| !l.starts_with("  ")).collect();
    assert_eq!(changed.len(), 2, "{changed:?}");
    assert!(changed[0].starts_with("- ") && changed[0].contains("--pos 2240x1440"));
    assert!(changed[1].starts_with("+ ") && changed[1].contains("--pos 2250x1440"));
    press(&mut s, "n");
    assert_eq!(status(&s), "Not saved.");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);

    press(&mut s, ":w<Enter>y");
    let now = std::fs::read_to_string(&path).unwrap();
    assert!(now.contains("--pos 2250x1440"), "{now}");
    press(&mut s, ":w desk<Enter>");
    assert!(
        status(&s).ends_with("desk.sh is up to date."),
        "{}",
        status(&s)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn saving_over_a_hand_written_script_keeps_its_other_lines() {
    let dir = scratch("keep-lines");
    let path = dir.join("mine.sh");
    let mine = "#!/bin/sh\n# my desk\nxrandr --output eDP-1 --auto\nfeh --bg-fill ~/wall.png\n";
    std::fs::write(&path, mine).unwrap();
    let backend = FixtureBackend::demo();
    let mut s = session(&backend, Some(&dir));
    press(&mut s, ":w mine<Enter>y");
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "#!/bin/sh");
    assert!(lines[1].starts_with("# generated by outlay "));
    assert_eq!(lines[2], "# my desk");
    assert!(lines[3].starts_with("xrandr --output HDMI-1-0 "));
    assert_eq!(lines.last(), Some(&"feh --bg-fill ~/wall.png"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn e_opens_the_picker_and_a_profile_becomes_pending() {
    let dir = scratch("open");
    copy_fixture(&dir, "tv-home");
    copy_fixture(&dir, "home-setup");
    let backend = FixtureBackend::demo();
    let mut s = session(&backend, Some(&dir));
    press(&mut s, "e");
    let UiMode::Profiles(picker) = &s.app.mode else {
        panic!("no picker: {:?} {}", s.app.mode, status(&s))
    };
    let names: Vec<&str> = picker.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["home-setup", "tv-home"]);
    assert_eq!(picker.items[0].unmatched, ["eDP-2"]);
    assert!(picker.items[1].unmatched.is_empty());

    press(&mut s, "j<Enter>");
    assert_eq!(s.app.mode, UiMode::Normal);
    assert_eq!(s.app.profile.as_deref(), Some("tv-home"));
    assert_eq!(rect(&s.app.layout, "HDMI-1-0"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect(&s.app.layout, "eDP-1"), Rect::new(0, 1440, 1920, 1080));
    assert!(!s.app.layout.is_enabled(ix(&s.app.layout, "DP-1-2")));
    assert!(
        status(&s).starts_with("Opened tv-home: 3 pending changes."),
        "{}",
        status(&s)
    );
    assert!(
        backend.applied().is_empty(),
        "opening only stages the changes"
    );

    press(&mut s, "u");
    assert!(
        s.app.pending().is_empty(),
        "one undo step for the whole profile"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_output_is_remapped_in_a_dialog() {
    let dir = scratch("remap");
    copy_fixture(&dir, "home-setup");
    let backend = FixtureBackend::demo();
    let mut s = session(&backend, Some(&dir));
    press(&mut s, ":e home-setup<Enter>");
    let UiMode::Remap(dialog) = &s.app.mode else {
        panic!("no remap: {:?} {}", s.app.mode, status(&s))
    };
    let edp = ix(&s.app.layout, "eDP-1");
    let dp13 = ix(&s.app.layout, "DP-1-3");
    assert_eq!(dialog.rows, [("eDP-2".to_owned(), Some(edp))]);
    assert_eq!(dialog.free, [edp, dp13]);
    press(&mut s, "l");
    let UiMode::Remap(dialog) = &s.app.mode else {
        panic!()
    };
    assert_eq!(dialog.rows[0].1, Some(dp13));
    press(&mut s, "l");
    let UiMode::Remap(dialog) = &s.app.mode else {
        panic!()
    };
    assert_eq!(dialog.rows[0].1, None, "then skip");
    press(&mut s, "h<Enter>");
    assert_eq!(s.app.mode, UiMode::Normal);
    assert_eq!(
        rect(&s.app.layout, "DP-1-3"),
        Rect::new(1920, 1440, 1920, 1080)
    );
    assert_eq!(mode_of(&s.app.layout, "DP-1-3"), "1920x1080@60.00");
    assert!(
        status(&s).contains("DP-1-3 has no 165.01 Hz at 1920x1080; using 60.00 Hz."),
        "a 165 Hz request on a 60 Hz monitor: {}",
        status(&s)
    );

    // Esc leaves everything as it was.
    press(&mut s, "u:e home-setup<Enter><Esc>");
    assert_eq!(s.app.mode, UiMode::Normal);
    assert!(s.app.pending().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn profile_errors_are_reported() {
    let backend = FixtureBackend::demo();
    let mut s = session(&backend, None);
    press(&mut s, "e");
    assert_eq!(status(&s), "No layouts directory is set.");
    press(&mut s, "wx<Enter>");
    assert_eq!(status(&s), "No layouts directory is set.");

    let dir = scratch("errors");
    let mut s = session(&backend, Some(&dir));
    press(&mut s, "e");
    assert!(
        status(&s).starts_with("There are no profiles in "),
        "{}",
        status(&s)
    );
    press(&mut s, ":e nothing<Enter>");
    assert!(status(&s).starts_with("Could not read "), "{}", status(&s));
    press(&mut s, "w<Enter>");
    assert_eq!(status(&s), "Type a name for the profile.");
    assert!(matches!(s.app.mode, UiMode::SavePrompt(_)));
    std::fs::remove_dir_all(dir).unwrap();
}
