//! The Wayland backend against a real compositor: headless sway, started per test. Every test
//! passes with "skipped" unless `OUTLAY_TEST_SWAY` names the sway binary:
//!
//! ```sh
//! OUTLAY_TEST_SWAY=$(command -v sway) cargo test --test wayland_live
//! ```

mod common;

use common::sway::Sway;
use outlay::backend::{Backend, On, Plan, PlanForm, Planned, PrimaryRule, ScalingChange};
use outlay::model::geometry::{Point, Size};
use outlay::model::{Mode, Reflection, Rotation, Scaling, Snapshot};
use outlay::wayland::WlrBackend;
use outlay::wayland::capture::mode;

fn connect(sway: &Sway) -> WlrBackend {
    WlrBackend::connect(&sway.socket)
        .unwrap_or_else(|e| panic!("could not connect: {e:?}\n{}", sway.log()))
}

fn head<'a>(snap: &'a Snapshot, name: &str) -> &'a outlay::model::Output {
    &snap.outputs[snap.find(name).unwrap_or_else(|| panic!("no head {name}"))]
}

fn on(name: &str, mode: Mode, pos: (i32, i32), rotation: Rotation, scale: f64) -> Planned {
    Planned {
        name: name.to_owned(),
        on: Some(On {
            mode,
            pos: Point::new(pos.0, pos.1),
            rotation,
            reflection: Reflection::Normal,
            scaling: Scaling::Logical(scale),
            scaling_change: ScalingChange::Set,
            primary: false,
        }),
    }
}

fn plan(outputs: Vec<Planned>) -> Plan {
    Plan {
        outputs,
        primary: PrimaryRule::Clear,
        form: PlanForm::Apply,
    }
}

#[test]
fn the_query_sees_both_headless_outputs() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    let snap = backend.query().unwrap();
    let names: Vec<&str> = snap.outputs.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["HEADLESS-1", "HEADLESS-2"]);
    let one = head(&snap, "HEADLESS-1");
    let a = one.active.as_ref().unwrap();
    assert_eq!(a.size, Size::new(1280, 720));
    let current = one.current_mode().unwrap();
    assert!(current.custom, "headless outputs have only custom modes");
    assert_eq!(current.refresh, 0.0);
    assert_eq!(one.description.as_deref(), Some("Headless output 2"));
}

#[test]
fn an_apply_moves_rotates_and_scales() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    let snap = backend.query().unwrap();
    let current = head(&snap, "HEADLESS-1").current_mode().unwrap().clone();
    let p = plan(vec![
        on("HEADLESS-1", current.clone(), (0, 0), Rotation::Left, 1.25),
        on("HEADLESS-2", current, (576, 100), Rotation::Normal, 1.5),
    ]);
    let outcome = backend.apply(&p).unwrap();
    assert!(outcome.success, "{outcome:?}\n{}", sway.log());
    let after = backend.query().unwrap();
    let one = head(&after, "HEADLESS-1").active.clone().unwrap();
    assert_eq!(
        (one.rotation, one.scaling.clone()),
        (Rotation::Left, Scaling::Logical(1.25))
    );
    assert_eq!(one.size, Size::new(576, 1024), "720x1280 / 1.25");
    let two = head(&after, "HEADLESS-2").active.clone().unwrap();
    assert_eq!(two.pos, Point::new(576, 100));
    assert_eq!(two.size, Size::new(853, 480), "1280x720 / 1.5, truncated");
    assert_eq!(sway.rect("HEADLESS-1"), (0, 0, 576, 1024));
    assert_eq!(sway.rect("HEADLESS-2"), (576, 100, 853, 480));
    // sway calls the protocol's 90 "270".
    let outputs = sway.outputs();
    let transform = |name: &str| {
        outputs.iter().find(|o| o["name"] == name).unwrap()["transform"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(transform("HEADLESS-1"), "270");

    // A custom mode, at scales whose logical size has a fraction above one half.
    for (scale, size) in [(1.5, (910, 512)), (1.75, (780, 438)), (1.25, (1092, 614))] {
        let p = plan(vec![on(
            "HEADLESS-2",
            mode(1366, 768, 60_000, false, true),
            (0, 0),
            Rotation::Normal,
            scale,
        )]);
        let outcome = backend.apply(&p).unwrap();
        assert!(outcome.success, "{outcome:?}\n{}", sway.log());
        let after = backend.query().unwrap();
        let two = head(&after, "HEADLESS-2").active.clone().unwrap();
        assert_eq!((two.size.w, two.size.h), size, "outlay at {scale}");
        let (_, _, w, h) = sway.rect("HEADLESS-2");
        assert_eq!(
            (w as i32, h as i32),
            size,
            "sway at {scale}: truncated, not rounded"
        );
    }
}

fn off(name: &str) -> Planned {
    Planned {
        name: name.to_owned(),
        on: None,
    }
}

#[test]
fn a_head_goes_off_and_on_again() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    let before = backend.query().unwrap();
    let outcome = backend.apply(&plan(vec![off("HEADLESS-2")])).unwrap();
    assert!(outcome.success, "{outcome:?}");
    let snap = backend.requery().unwrap();
    let two = head(&snap, "HEADLESS-2");
    assert!(two.active.is_none(), "off");
    assert!(two.is_relevant(), "a head stays while it is connected");
    let back = Plan::restore(&before);
    assert!(backend.apply(&back).unwrap().success);
    assert_eq!(backend.requery().unwrap(), before);
}

#[test]
fn the_watch_sees_heads_come_and_go() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    sway.swaymsg(&["create_output"]);
    let snap = backend.requery().unwrap();
    let names: Vec<&str> = snap.outputs.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["HEADLESS-1", "HEADLESS-2", "HEADLESS-3"]);
    sway.swaymsg(&["output", "HEADLESS-3", "unplug"]);
    let snap = backend.requery().unwrap();
    assert_eq!(snap.outputs.len(), 2);
    assert!(snap.find("HEADLESS-3").is_none());
}

#[test]
fn a_stale_serial_is_cancelled_and_tried_once_more() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    let snap = backend.query().unwrap();
    // A change outlay has not read yet: its serial is stale now.
    sway.swaymsg(&["output", "HEADLESS-2", "pos", "2000", "0"]);
    let current = head(&snap, "HEADLESS-1").current_mode().unwrap().clone();
    let p = plan(vec![on(
        "HEADLESS-1",
        current,
        (0, 720),
        Rotation::Normal,
        1.0,
    )]);
    let done = backend.configure_without_roundtrip(&p, false).unwrap();
    assert_eq!(done.cancelled, 1, "{done:?}");
    assert_eq!(done.answer, Ok(()));
    let after = backend.query().unwrap();
    let pos = |name| head(&after, name).active.as_ref().unwrap().pos;
    assert_eq!(pos("HEADLESS-1"), Point::new(0, 720));
    assert_eq!(
        pos("HEADLESS-2"),
        Point::new(2000, 0),
        "the other change stays"
    );
}

#[test]
fn a_test_does_not_change_anything() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    let before = backend.query().unwrap();
    let current = head(&before, "HEADLESS-1").current_mode().unwrap().clone();
    let p = plan(vec![on(
        "HEADLESS-1",
        current,
        (0, 900),
        Rotation::Inverted,
        2.0,
    )]);
    assert_eq!(
        backend.test(&p).unwrap(),
        outlay::backend::Verdict::Accepted
    );
    assert_eq!(backend.query().unwrap(), before);
}

#[test]
fn revert_sh_restores_the_layout_from_before() {
    let Some(sway) = Sway::start(2) else { return };
    let before = connect(&sway).query().unwrap();
    let script = sway.runtime.join("revert.sh");
    let program = std::path::Path::new(env!("CARGO_BIN_EXE_outlay"));
    std::fs::write(&script, outlay::wayland::revert_script(program, &before)).unwrap();

    sway.swaymsg(&[
        "output",
        "HEADLESS-1",
        "scale",
        "2",
        "transform",
        "90",
        "pos",
        "0",
        "300",
    ]);
    sway.swaymsg(&["output", "HEADLESS-2", "disable"]);
    // No other client stays connected: up to wlroots 0.20.2, enabling a head in a custom mode
    // aborts the compositor when another client still holds that head's old virtual mode
    // (fixed by wlroots 40640950; see MISTAKES.md).
    let backend = connect(&sway);
    assert_ne!(backend.query().unwrap(), before);
    drop(backend);

    let out = sway.command("sh").arg(&script).output().unwrap();
    assert!(
        out.status.success(),
        "{}{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        sway.log()
    );
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(connect(&sway).query().unwrap(), before);
}

#[test]
fn wlr_randr_and_outlay_dump_read_the_same() {
    let Some(sway) = Sway::start(2) else { return };
    sway.swaymsg(&[
        "output",
        "HEADLESS-2",
        "scale",
        "1.5",
        "transform",
        "flipped-90",
    ]);
    let wlr = match sway.command("wlr-randr").arg("--json").output() {
        Ok(out) => out,
        Err(_) => {
            println!("skipped: wlr-randr is not installed");
            return;
        }
    };
    if !wlr.status.success() {
        // wlr-randr before 0.4 has no --json.
        println!(
            "skipped: wlr-randr --json: {}",
            String::from_utf8_lossy(&wlr.stderr).trim()
        );
        return;
    }
    let dump = sway
        .command(env!("CARGO_BIN_EXE_outlay"))
        .env("XDG_CONFIG_HOME", "/nonexistent/outlay-test-config")
        .arg("dump")
        .output()
        .unwrap();
    assert!(
        dump.status.success(),
        "{}",
        String::from_utf8_lossy(&dump.stderr)
    );
    let parse = |bytes: &[u8]| {
        outlay::wayland::capture::parse(std::str::from_utf8(bytes).unwrap()).unwrap()
    };
    let (theirs, ours) = (parse(&wlr.stdout), parse(&dump.stdout));
    assert_eq!(ours, theirs);
    let two = head(&ours, "HEADLESS-2").active.clone().unwrap();
    // sway's "flipped-90" is the protocol's flipped-270.
    assert_eq!(
        (two.rotation, two.reflection),
        (Rotation::Right, Reflection::X)
    );
}

#[test]
fn the_cli_finds_the_compositor() {
    let Some(sway) = Sway::start(2) else { return };
    let run = |args: &[&str]| {
        let out = sway
            .command(env!("CARGO_BIN_EXE_outlay"))
            .env("XDG_CONFIG_HOME", "/nonexistent/outlay-test-config")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (ok, stdout, stderr) = run(&["show"]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with("outlay · 2 on · 0 off · layout 2560x720\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("1  HEADLESS-1  1280x720 @ 0.00"),
        "{stdout}"
    );
    let (ok, stdout, stderr) = run(&["list"]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with("1  HEADLESS-1  connected\n    1280x720  0.00*\n"),
        "{stdout}"
    );
}

#[test]
fn the_editor_applies_waits_for_the_answer_and_reverts() {
    use outlay::tui::app::{App, Options, UiMode};
    use outlay::tui::session::{Input, Session, Settings};
    use ratatui::crossterm::event::Event;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    struct NoInput;
    impl Input for NoInput {
        fn drain(&mut self) {}
    }

    let Some(sway) = Sway::start(2) else { return };
    // sway 1.9 puts HEADLESS-1 first, 1.12 HEADLESS-2: pin them.
    sway.swaymsg(&["output", "HEADLESS-2", "pos", "0", "0"]);
    sway.swaymsg(&["output", "HEADLESS-1", "pos", "1280", "0"]);
    let backend = connect(&sway);
    let before = backend.query().unwrap();
    let revert = sway.runtime.join("state").join("revert.sh");
    let settings = Settings {
        revert_file: Some(revert.clone()),
        restore_program: Some(env!("CARGO_BIN_EXE_outlay").into()),
        ..Settings::default()
    };
    let app = App::new(before.clone(), Options::default());
    let mut session = Session::new(app, &backend, settings, Arc::new(AtomicBool::new(false)));
    let press = |s: &mut Session, script: &str| {
        for key in common::keys(script) {
            s.handle_event(&Event::Key(key), s.app.now);
        }
        s.perform(&mut NoInput);
    };
    // HEADLESS-2 is left of HEADLESS-1. To 150 %, then rotated: it shrinks, and HEADLESS-1
    // follows it.
    press(&mut session, "2>>o");
    press(&mut session, "a");
    let UiMode::ConfirmApply(preview) = &session.app.mode else {
        panic!("{:?}", session.app.mode)
    };
    assert!(preview.errors.is_empty(), "{:?}", preview.errors);
    assert_eq!(
        preview.command,
        "wlr-randr --output HEADLESS-1 --on --custom-mode 1280x720 --pos 480,0 --transform normal \
         --scale 1 --output HEADLESS-2 --on --custom-mode 1280x720 --pos 0,0 --transform 270 \
         --scale 1.5"
    );
    press(&mut session, "<Enter>");
    let UiMode::Countdown(c) = session.app.mode else {
        panic!("no countdown: {:?}", session.app.mode)
    };
    let script = std::fs::read_to_string(&revert).unwrap();
    assert!(
        script.contains(" restore - <<'OUTLAY-CAPTURE'\n["),
        "{script}"
    );
    assert_eq!(sway.rect("HEADLESS-2"), (0, 0, 480, 853));
    assert_eq!(sway.rect("HEADLESS-1"), (480, 0, 1280, 720));

    session.tick(c.deadline + Duration::from_millis(1));
    session.perform(&mut NoInput);
    assert_eq!(session.app.mode, UiMode::Normal);
    assert_eq!(backend.query().unwrap(), before, "reverted");
    assert_eq!(sway.rect("HEADLESS-2"), (0, 0, 1280, 720));
}

/// kanshi against the headless sway, with its own config and log in sway's runtime directory,
/// so it never reads `~/.config/kanshi`. It runs only when `OUTLAY_TEST_KANSHI` names the kanshi
/// binary.
struct Kanshi {
    child: std::process::Child,
    kanshictl: std::path::PathBuf,
    log: std::path::PathBuf,
}

impl Kanshi {
    fn start(sway: &Sway, config: &std::path::Path) -> Option<Self> {
        let Some(kanshi) = std::env::var_os("OUTLAY_TEST_KANSHI") else {
            println!("skipped: set OUTLAY_TEST_KANSHI=/path/to/kanshi to run kanshi too");
            return None;
        };
        let kanshi = std::path::PathBuf::from(kanshi);
        let log = sway.runtime.join("kanshi.log");
        let file = std::fs::File::create(&log).unwrap();
        let child = sway
            .command(&kanshi)
            .arg("-c")
            .arg(config)
            .stdout(file.try_clone().unwrap())
            .stderr(file)
            .spawn()
            .unwrap();
        Some(Self {
            child,
            kanshictl: kanshi.with_file_name("kanshictl"),
            log,
        })
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// Waits up to 5 s until kanshi has applied a profile `n` times in all.
    fn applied(&self, sway: &Sway, n: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.log().matches("' applied").count() < n {
            assert!(
                std::time::Instant::now() < deadline,
                "kanshi did not apply a profile {n} times:\n{}\n{}",
                self.log(),
                sway.log()
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    fn reload(&self, sway: &Sway) {
        let out = sway
            .command(&self.kanshictl)
            .arg("reload")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "kanshictl reload: {}\n{}",
            String::from_utf8_lossy(&out.stderr),
            self.log()
        );
    }
}

impl Drop for Kanshi {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Does kanshi undo an apply during outlay's countdown? It keeps its current profile while that
/// still matches the connected heads (`match_and_apply` in kanshi's `main.c`), so it lets
/// outlay's change stand; it applies its profile again after a hotplug and on `kanshictl reload`.
/// A profile outlay saves is one kanshi applies as it is, transform names included.
#[test]
fn kanshi_lets_an_apply_stand_until_a_hotplug_or_a_reload() {
    let Some(sway) = Sway::start(2) else { return };
    sway.swaymsg(&["output", "HEADLESS-2", "pos", "0", "0"]);
    sway.swaymsg(&["output", "HEADLESS-1", "pos", "1280", "0"]);
    let config = sway.runtime.join("kanshi-config");
    std::fs::write(
        &config,
        "profile desk {\n\toutput HEADLESS-1 enable position 1280,0\n\toutput HEADLESS-2 enable position 0,0\n}\n",
    )
    .unwrap();
    let Some(kanshi) = Kanshi::start(&sway, &config) else {
        return;
    };
    kanshi.applied(&sway, 1);
    let backend = connect(&sway);
    let pos = |name| {
        let snap = backend.requery().unwrap();
        head(&snap, name).active.as_ref().unwrap().pos
    };
    assert_eq!(pos("HEADLESS-1"), Point::new(1280, 0));

    // outlay moves HEADLESS-1 below HEADLESS-2; kanshi lets it be.
    let current = head(&backend.query().unwrap(), "HEADLESS-1")
        .current_mode()
        .unwrap()
        .clone();
    let below = plan(vec![on(
        "HEADLESS-1",
        current.clone(),
        (0, 720),
        Rotation::Normal,
        1.0,
    )]);
    assert!(backend.apply(&below).unwrap().success, "{}", sway.log());
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert_eq!(pos("HEADLESS-1"), Point::new(0, 720), "{}", kanshi.log());
    assert_eq!(
        kanshi.log().matches("' applied").count(),
        1,
        "{}",
        kanshi.log()
    );

    // A reload applies the profile again.
    kanshi.reload(&sway);
    kanshi.applied(&sway, 2);
    assert_eq!(pos("HEADLESS-1"), Point::new(1280, 0));

    // So does a hotplug: no profile matches three heads, and the profile matches again once
    // the third is gone.
    assert!(backend.apply(&below).unwrap().success);
    sway.swaymsg(&["create_output"]);
    sway.swaymsg(&["output", "HEADLESS-3", "unplug"]);
    kanshi.applied(&sway, 3);
    assert_eq!(pos("HEADLESS-1"), Point::new(1280, 0), "{}", kanshi.log());

    // outlay saves the live layout, turned 90 (xrandr's left) at 150 %, into the profile;
    // kanshi applies exactly that.
    let turned = plan(vec![on(
        "HEADLESS-1",
        current,
        (0, 720),
        Rotation::Left,
        1.5,
    )]);
    assert!(backend.apply(&turned).unwrap().success);
    let live = backend.query().unwrap();
    let out = sway
        .command(env!("CARGO_BIN_EXE_outlay"))
        .env("XDG_CONFIG_HOME", "/nonexistent/outlay-test-config")
        .arg("--kanshi-config")
        .arg(&config)
        .args(["save", "desk", "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let saved = std::fs::read_to_string(&config).unwrap();
    assert!(
        saved.contains("\toutput HEADLESS-1 enable mode --custom 1280x720 position 0,720 scale 1.5 transform 90\n"),
        "{saved}"
    );
    // Back to kanshi's old layout first, so the reload has something to do.
    sway.swaymsg(&[
        "output",
        "HEADLESS-1",
        "pos",
        "1280",
        "0",
        "scale",
        "1",
        "transform",
        "normal",
    ]);
    kanshi.reload(&sway);
    kanshi.applied(&sway, 4);
    assert_eq!(backend.query().unwrap(), live, "{}", kanshi.log());
}

/// Up to wlroots 0.20.2, a client that bound while a custom-mode head was off aborts the
/// compositor when it turns that head on again while another client still holds the head's old
/// virtual mode. `revert.sh` runs `outlay restore`, such a client, so the panic hook hangs up
/// on the compositor first.
#[test]
fn hanging_up_first_lets_revert_sh_turn_a_custom_mode_head_back_on() {
    let Some(sway) = Sway::start(2) else { return };
    let backend = connect(&sway);
    let before = backend.query().unwrap();
    let script = sway.runtime.join("revert.sh");
    let program = std::path::Path::new(env!("CARGO_BIN_EXE_outlay"));
    std::fs::write(&script, outlay::wayland::revert_script(program, &before)).unwrap();
    // The editor turns HEADLESS-2 off and still holds its virtual mode.
    assert!(
        backend
            .apply(&plan(vec![off("HEADLESS-2")]))
            .unwrap()
            .success
    );
    // Without this, sway 1.12 (wlroots 0.20.2) dies: head_send_state: Assertion `found' failed.
    backend.hang_up();
    let out = sway.command("sh").arg(&script).output().unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stderr),
        sway.log()
    );
    assert_eq!(connect(&sway).query().unwrap(), before, "{}", sway.log());
}
