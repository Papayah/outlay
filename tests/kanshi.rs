//! kanshi profiles: the synthetic configs in `tests/fixtures/kanshi/` read onto the Wayland
//! demo, the criteria rules, the remap of a display no criteria match, saving that keeps every
//! other byte, the round trip layout → block → layout, and `w`/`e` through the session. Every
//! config is a copy in a temporary directory; `~/.config/kanshi` is never read.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use common::{ix, keys, rect};
use outlay::backend::{Backend, FixtureBackend};
use outlay::model::geometry::Rect;
use outlay::model::layout::Layout;
use outlay::model::profile::{ModeRequest, Profile, Target};
use outlay::model::{Identity, Reflection, Rotation, Scaling, Snapshot};
use outlay::tui::app::{App, Options, UiMode};
use outlay::tui::session::{Input, Session, Settings};
use outlay::wayland::kanshi::{self, Config};
use ratatui::crossterm::event::Event;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kanshi")
}

/// A copy of the kanshi fixtures in a fresh directory.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("outlay-kanshi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("desks.d")).unwrap();
    for name in ["config", "broken", "desks.d/office"] {
        std::fs::copy(fixtures().join(name), dir.join(name)).unwrap();
    }
    dir
}

fn demo() -> Snapshot {
    FixtureBackend::demo_wayland().query().unwrap()
}

fn config() -> Config {
    kanshi::read(&fixtures().join("config")).unwrap()
}

fn profile(name: &str) -> Profile {
    config().find(name).unwrap().profile.clone()
}

/// The profile loaded with its default remap.
fn load(snap: &Snapshot, name: &str) -> (Layout, Vec<String>) {
    let p = profile(name);
    let remap = p.default_remap(snap);
    p.layout(snap, &remap)
}

fn mode_of(layout: &Layout, name: &str) -> String {
    let i = ix(layout, name);
    layout.outputs[i].mode.as_ref().unwrap().summary()
}

#[test]
fn reads_every_profile_with_includes_defaults_and_aliases() {
    let config = config();
    let names: Vec<&str> = config.profiles.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["home", "nomad", "<anonymous profile 1>", "office"]);
    assert!(config.problems.is_empty(), "{:?}", config.problems);
    let files: Vec<PathBuf> = config.files.iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(
        files,
        [fixtures().join("config"), fixtures().join("desks.d/office")]
    );
    assert_eq!(config.find("office").unwrap().file, 1);
    assert!(config.profiles[2].anonymous);

    let home = profile("home");
    let targets: Vec<&Target> = home.entries.iter().map(|e| &e.target).collect();
    assert_eq!(
        targets,
        [
            &Target::Name("eDP-1".to_owned()),
            &Target::Description("Dell Inc. DELL U2719D 7LQ2M43".to_owned()),
            &Target::Description("Philips Consumer Electronics Company PHILIPS FTV *".to_owned()),
            &Target::Description("LG Electronics LG HDR 4K 0x0000B1D2".to_owned()),
        ],
        "the alias $dell stands for its description"
    );
    let edp = &home.entries[0];
    assert_eq!(
        edp.scaling,
        Some(Scaling::Logical(2.0)),
        "the global default"
    );
    assert!(!edp.off && !edp.keep_off);
    let dell = &home.entries[1];
    assert_eq!(
        dell.mode,
        Some(ModeRequest::Size {
            width: 2560,
            height: 1440
        })
    );
    assert_eq!(dell.rate, Some(143.912));
    let tv = &home.entries[2];
    assert_eq!(
        tv.pos.map(|p| (p.x, p.y)),
        Some((2560, 0)),
        "a block output"
    );
    assert_eq!(tv.rate, None);
    assert!(home.entries[3].off);
    assert_eq!(
        home.warnings,
        ["kanshi runs this profile's exec commands; outlay does not."]
    );
    assert_eq!(
        profile("nomad").warnings,
        ["outlay leaves ...output entries to kanshi."]
    );

    let office = profile("office");
    assert_eq!(office.entries[0].rotation, Some(Rotation::Left), "90");
    assert_eq!(office.entries[0].reflection, Some(Reflection::Normal));
    assert_eq!(
        office.entries[1].mode,
        Some(ModeRequest::Custom {
            width: 3000,
            height: 2000
        })
    );
    assert_eq!(office.entries[1].rate, Some(59.5));
}

#[test]
fn problems_are_reported_where_they_are() {
    let config = kanshi::read(&fixtures().join("broken")).unwrap();
    assert_eq!(
        config.problems,
        [
            "broken, line 3: a quoted word is not closed",
            "broken, line 6: unexpected '}'",
            "broken, line 7: unknown directive workspace.",
        ]
    );
    let broken = &config.find("broken").unwrap().profile;
    assert_eq!(
        broken.warnings[3..],
        [
            "broken, line 2: \"fast\" is not a valid mode; ignored.",
            "broken, line 2: \"-1\" is not a valid scale; ignored.",
            "broken, line 2: frobnicate needs a value.",
            "broken, line 4: unknown profile directive alias.",
        ]
    );
}

#[test]
fn home_loads_onto_the_wayland_demo() {
    let snap = demo();
    let (layout, notes) = load(&snap, "home");
    assert_eq!(
        notes,
        ["kanshi runs this profile's exec commands; outlay does not."]
    );
    assert_eq!(rect(&layout, "DP-3"), Rect::new(0, 0, 2560, 1440));
    assert_eq!(rect(&layout, "HDMI-A-1"), Rect::new(2560, 0, 1920, 1080));
    assert_eq!(
        rect(&layout, "eDP-1"),
        Rect::new(560, 1440, 1440, 900),
        "scale 2 from the global default"
    );
    assert_eq!(mode_of(&layout, "DP-3"), "2560x1440@143.91");
    assert_eq!(
        mode_of(&layout, "HDMI-A-1"),
        "1920x1080@60.00",
        "no rate: the highest, as kanshi picks it"
    );
    assert!(!layout.is_enabled(ix(&layout, "DP-4")));
}

#[test]
fn criteria_match_as_kanshi_matches() {
    let snap = demo();
    let index = |name: &str| snap.find(name).unwrap();
    let p = |text: &str| {
        kanshi::parse(Path::new("/k/config"), &format!("profile p {{\n{text}}}\n")).profiles[0]
            .profile
            .clone()
    };
    // `*` goes last, whatever the order: it never takes a display a later entry names.
    let wild = p(" output * enable\n output eDP-1 enable\n");
    assert_eq!(
        wild.matches(&snap),
        [Some(index("DP-3")), Some(index("eDP-1"))]
    );
    // A glob takes the first display, in number order, that no exact entry took.
    let globs = p(
        " output \"* Unknown\" enable\n output \"Dell Inc. DELL U2719D 7LQ2M43\" enable\n output \"*\" enable\n",
    );
    assert_eq!(
        globs.matches(&snap),
        [
            Some(index("eDP-1")),
            Some(index("DP-3")),
            Some(index("DP-4"))
        ],
        "eDP-1 reports no serial: Samsung Display Corp. ATNA40YK07-1 Unknown"
    );
    // A name matches only that name; a description never matches a name.
    let names = p(" output HDMI-A-1 enable\n output \"HDMI-A-1 x\" enable\n output DP-9 enable\n");
    assert_eq!(names.matches(&snap), [Some(index("HDMI-A-1")), None, None]);
    assert_eq!(names.unmatched(&snap), ["HDMI-A-1 x", "DP-9"]);
    // An entry with neither enable nor disable leaves a display that is off as it is.
    let keep = p(" output \"LG Electronics *\" mode 1920x1080 position 0,0\n");
    let (layout, _) = keep.layout(&snap, &Vec::new());
    assert!(!layout.is_enabled(index("DP-4")));
}

#[test]
fn an_unknown_description_is_remapped() {
    let snap = demo();
    let office = profile("office");
    assert_eq!(office.unmatched(&snap), ["Dell Inc. DELL U2720Q *"]);
    assert_eq!(
        office.default_remap(&snap),
        [("Dell Inc. DELL U2720Q *".to_owned(), None)],
        "another model: no default"
    );
    let dp3 = snap.find("DP-3").unwrap();
    let (layout, notes) = office.layout(
        &snap,
        &vec![("Dell Inc. DELL U2720Q *".to_owned(), Some(dp3))],
    );
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(
        rect(&layout, "eDP-1"),
        Rect::new(0, 0, 900, 1440),
        "turned 90 at 200%"
    );
    let mode = layout.outputs[dp3].mode.clone().unwrap();
    assert!(mode.custom, "mode --custom");
    assert_eq!(mode.summary(), "3000x2000@59.50");
    assert_eq!(rect(&layout, "DP-3"), Rect::new(900, 0, 3000, 2000));
    assert!(
        outlay::model::validate::validate(&layout, &snap)
            .iter()
            .all(|i| i.severity != outlay::model::validate::Severity::Error),
        "a compositor takes a custom mode it does not list"
    );

    // The same model with another serial is the default.
    let mut other = snap.clone();
    let id = other.outputs[dp3].identity.as_mut().unwrap();
    *id = Identity {
        serial: Some("OTHER".to_owned()),
        ..id.clone()
    };
    let p = kanshi::parse(
        Path::new("/k/config"),
        "profile p {\n output \"Dell Inc. DELL U2719D 7LQ2M43\" enable\n}\n",
    )
    .profiles[0]
        .profile
        .clone();
    assert_eq!(
        p.default_remap(&other),
        [("Dell Inc. DELL U2719D 7LQ2M43".to_owned(), Some(dp3))]
    );
}

/// What the fixture's `home` block becomes when the demo's live layout is saved over it.
const HOME_SAVED: &str = "\
# generated by outlay VERSION
profile home {
\toutput eDP-1 enable mode 2880x1800@60.001Hz position 2480,1440 scale 2 transform normal
\toutput $dell enable mode 2560x1440@143.912Hz position 1920,0 scale 1 transform normal
\t# the TV, by a glob on its description
\toutput \"Philips Consumer Electronics Company PHILIPS FTV *\" enable mode 1920x1080@59.94Hz position 0,360 scale 1 transform normal
\toutput \"LG Electronics LG HDR 4K 0x0000B1D2\" disable
\texec swaymsg workspace 1, move workspace to eDP-1
}
";

fn with_version(text: &str) -> String {
    text.replace("VERSION", env!("CARGO_PKG_VERSION"))
}

#[test]
fn saving_rewrites_one_block_and_keeps_every_other_byte() {
    let snap = demo();
    let config = config();
    let main = fixtures().join("config");
    let layout = Layout::inferred(&snap);
    let (path, text) = kanshi::save(Some(&config), &main, "home", &layout, &snap);
    assert_eq!(path, main);
    let old = &config.files[0].1;
    let block = config.find("home").unwrap();
    let expected = format!(
        "{}{}{}",
        &old[..block.lines.start],
        with_version(HOME_SAVED),
        &old[block.lines.end..]
    );
    assert_eq!(text, expected);

    // Saving again changes nothing: the header is updated in place.
    let again = kanshi::parse(&main, &text);
    let (_, twice) = kanshi::save(Some(&again), &main, "home", &layout, &snap);
    assert_eq!(twice, text);

    // An included profile is saved in its own file.
    let (path, text) = kanshi::save(Some(&config), &main, "office", &layout, &snap);
    assert_eq!(path, fixtures().join("desks.d/office"));
    assert_eq!(
        text,
        with_version(
            "# Included from ../config.\n# generated by outlay VERSION\nprofile office {\n\
             \toutput eDP-1 enable mode 2880x1800@60.001Hz position 2480,1440 scale 2 transform normal\n\
             \toutput \"Dell Inc. DELL U2719D 7LQ2M43\" enable mode 2560x1440@143.912Hz position 1920,0 scale 1 transform normal\n\
             \toutput \"LG Electronics LG HDR 4K 0x0000B1D2\" disable\n\
             \toutput \"Philips Consumer Electronics Company PHILIPS FTV 0x01010101\" enable mode 1920x1080@59.94Hz position 0,360 scale 1 transform normal\n\
             }\n"
        ),
        "the U2720Q no display matches is gone; the others are added with the block's indent"
    );
}

#[test]
fn a_new_profile_goes_at_the_end_with_new_criteria() {
    let snap = demo();
    let config = config();
    let main = fixtures().join("config");
    let layout = Layout::inferred(&snap);
    let (path, text) = kanshi::save(Some(&config), &main, "desk two", &layout, &snap);
    assert_eq!(path, main);
    let old = &config.files[0].1;
    let block = with_version(
        "\n# generated by outlay VERSION\nprofile \"desk two\" {\n    \
         output eDP-1 enable mode 2880x1800@60.001Hz position 2480,1440 scale 2 transform normal\n    \
         output \"Dell Inc. DELL U2719D 7LQ2M43\" enable mode 2560x1440@143.912Hz position 1920,0 scale 1 transform normal\n    \
         output \"LG Electronics LG HDR 4K 0x0000B1D2\" disable\n    \
         output \"Philips Consumer Electronics Company PHILIPS FTV 0x01010101\" enable mode 1920x1080@59.94Hz position 0,360 scale 1 transform normal\n\
         }\n",
    );
    assert_eq!(text, format!("{old}{block}"));
    let reread = kanshi::parse(&main, &text);
    assert_eq!(reread.profiles.len(), 5);
    assert_eq!(
        reread.find("desk two").unwrap().profile.unmatched(&snap),
        Vec::<String>::new()
    );

    // No config yet: the block alone.
    let (_, fresh) = kanshi::save(None, &main, "desk", &layout, &snap);
    assert!(fresh.starts_with("# generated by outlay "), "{fresh}");
    assert!(fresh.contains("\nprofile desk {\n"), "{fresh}");
}

#[test]
fn heads_without_a_full_identity_are_named() {
    let snap = FixtureBackend::from_file(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/wayland/headless-sway.json")
            .as_ref(),
    )
    .unwrap()
    .query()
    .unwrap();
    let layout = Layout::inferred(&snap);
    let (_, text) = kanshi::save(None, Path::new("/k/config"), "h", &layout, &snap);
    assert!(
        text.contains("    output HEADLESS-1 enable mode --custom 1280x720 position 1280,0 scale 1 transform normal\n"),
        "{text}"
    );
    assert!(text.contains("    output HEADLESS-2 enable mode --custom 1280x720 position 0,0"));
}

#[test]
fn layouts_survive_the_round_trip_through_a_block() {
    for name in [
        "demo",
        "laptop-scaled",
        "rotated",
        "disabled-head",
        "custom-mode",
        "no-serial",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/wayland/{name}.json"));
        let snap = FixtureBackend::from_file(&path).unwrap().query().unwrap();
        let mut layout = Layout::inferred(&snap);
        layout.restore = vec![None; layout.len()];
        let (_, text) = kanshi::save(None, Path::new("/k/config"), "p", &layout, &snap);
        let config = kanshi::parse(Path::new("/k/config"), &text);
        assert!(config.problems.is_empty(), "{name}: {:?}", config.problems);
        let p = &config.find("p").unwrap().profile;
        assert!(p.warnings.is_empty(), "{name}: {:?}", p.warnings);
        assert!(p.unmatched(&snap).is_empty(), "{name}");
        let (mut loaded, notes) = p.layout(&snap, &Vec::new());
        assert!(notes.is_empty(), "{name}: {notes:?}");
        for i in 0..layout.len() {
            if !layout.is_enabled(i) && !loaded.is_enabled(i) {
                loaded.outputs[i] = layout.outputs[i].clone();
            }
        }
        assert_eq!(loaded, layout, "{name}: {text}");
    }
}

// --- Through the session ---------------------------------------------------------------------------

struct NoInput;

impl Input for NoInput {
    fn drain(&mut self) {}
}

fn session<'a>(backend: &'a FixtureBackend, config: Option<&Path>) -> Session<'a> {
    let app = App::new(backend.query().unwrap(), Options::default());
    let settings = Settings {
        kanshi_config: config.map(Path::to_owned),
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
fn e_lists_the_kanshi_profiles_and_opens_one() {
    let dir = scratch("open");
    let backend = FixtureBackend::demo_wayland();
    let mut s = session(&backend, Some(&dir.join("config")));
    press(&mut s, "e");
    let UiMode::Profiles(picker) = &s.app.mode else {
        panic!("no picker: {:?} {}", s.app.mode, status(&s))
    };
    let names: Vec<&str> = picker.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["home", "nomad", "<anonymous profile 1>", "office"]);
    assert_eq!(picker.items[3].unmatched, ["Dell Inc. DELL U2720Q *"]);
    press(&mut s, "<Enter>");
    assert_eq!(s.app.profile.as_deref(), Some("home"));
    assert_eq!(rect(&s.app.layout, "DP-3"), Rect::new(0, 0, 2560, 1440));
    assert!(
        status(&s).starts_with("Opened home: 3 pending changes."),
        "{}",
        status(&s)
    );
    assert!(backend.applied().is_empty());

    // The remap dialog names the description.
    press(&mut s, ":e office<Enter>");
    let UiMode::Remap(dialog) = &s.app.mode else {
        panic!("no remap: {:?} {}", s.app.mode, status(&s))
    };
    assert_eq!(dialog.rows, [("Dell Inc. DELL U2720Q *".to_owned(), None)]);
    press(&mut s, "<Esc>:e nothing<Enter>");
    assert!(
        status(&s).starts_with("There is no profile nothing in "),
        "{}",
        status(&s)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn w_saves_a_kanshi_profile_and_asks_before_rewriting_one() {
    let dir = scratch("save");
    let path = dir.join("config");
    let before = std::fs::read_to_string(&path).unwrap();
    let backend = FixtureBackend::demo_wayland();
    let mut s = session(&backend, Some(&path));
    // A new block changes the file too: the diff first.
    press(&mut s, "wdesk<Enter>");
    let UiMode::Overwrite(plan) = &s.app.mode else {
        panic!("no question: {:?} {}", s.app.mode, status(&s))
    };
    assert!(
        plan.diff.iter().all(|l| !l.starts_with("- ")),
        "{:?}",
        plan.diff
    );
    assert!(
        plan.diff.contains(&"+ profile desk {".to_owned()),
        "{:?}",
        plan.diff
    );
    press(&mut s, "y");
    let now = std::fs::read_to_string(&path).unwrap();
    assert!(now.starts_with(&before), "the rest stays");
    assert!(now.contains("\nprofile desk {\n"), "{now}");
    assert!(
        status(&s).ends_with("config. Run `kanshictl reload` so kanshi uses it."),
        "{}",
        status(&s)
    );
    assert_eq!(s.app.profile.as_deref(), Some("desk"));

    // An existing profile: the diff first.
    press(&mut s, ":w home<Enter>");
    let UiMode::Overwrite(plan) = &s.app.mode else {
        panic!("no question: {:?} {}", s.app.mode, status(&s))
    };
    assert_eq!(plan.name, "home");
    let changed: Vec<&str> = plan
        .diff
        .iter()
        .filter(|l| !l.starts_with("  "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        changed[..3],
        [
            format!("+ # generated by outlay {}", env!("CARGO_PKG_VERSION")).as_str(),
            "- \toutput eDP-1 enable position 560,1440",
            "- \toutput $dell enable mode 2560x1440@143.912Hz position 0,0",
        ],
        "{changed:?}"
    );
    press(&mut s, "y");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains(&with_version(HOME_SAVED)), "{saved}");
    press(&mut s, ":w home<Enter>");
    assert!(
        status(&s).ends_with("config is up to date."),
        "{}",
        status(&s)
    );

    // A new file.
    let fresh = dir.join("new").join("config");
    let mut s = session(&backend, Some(&fresh));
    press(&mut s, "wfirst<Enter>");
    let text = std::fs::read_to_string(&fresh).unwrap();
    assert!(text.starts_with("# generated by outlay "), "{text}");
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn no_config_is_reported() {
    let backend = FixtureBackend::demo_wayland();
    let mut s = session(&backend, None);
    press(&mut s, "e");
    assert_eq!(status(&s), "No kanshi config is set.");
    let dir = scratch("none");
    let mut s = session(&backend, Some(&dir.join("missing")));
    press(&mut s, "e");
    assert!(
        status(&s).starts_with("There are no profiles in "),
        "{}",
        status(&s)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
