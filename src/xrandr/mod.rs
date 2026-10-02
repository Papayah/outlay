//! Talking to xrandr: the [`Backend`] trait and its implementations.

pub mod command;
pub mod parse;
pub mod script;

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};

use crate::model::geometry::{Point, Size, effective_size};
use crate::model::{ActiveConfig, ModeId, Reflection, Rotation, Scaling, Snapshot, Transform};
pub use parse::{ParseError, parse_verbose};

/// The built-in fixture behind `--demo`.
pub const DEMO: &str = include_str!("../../tests/fixtures/xrandr/demo.txt");

/// What running `xrandr` with some arguments produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub trait Backend {
    /// A full probe of the outputs (`xrandr --verbose`).
    fn query(&self) -> Result<Snapshot>;

    /// A re-query that does not re-probe (`xrandr --verbose --current`), used after an apply.
    fn requery(&self) -> Result<Snapshot> {
        self.query()
    }

    /// Runs `xrandr` with `argv`, the arguments after the program name.
    fn apply(&self, argv: &[String]) -> Result<ApplyOutcome>;

    /// Whether an apply changes the real screens. Only then does outlay write `revert.sh`, which
    /// the panic hook may run.
    fn touches_x(&self) -> bool {
        false
    }
}

/// The real `xrandr` program.
pub struct XrandrCli {
    program: OsString,
}

impl Default for XrandrCli {
    fn default() -> Self {
        Self {
            program: "xrandr".into(),
        }
    }
}

impl XrandrCli {
    pub fn new() -> Self {
        Self::default()
    }

    fn run(&self, args: &[&str]) -> Result<ApplyOutcome> {
        let program = self.program.to_string_lossy();
        let out = match Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .output()
        {
            Ok(out) => out,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
                bail!(
                    "`{program}` is not installed. {}",
                    install_hint(&os_release)
                );
            }
            Err(err) => return Err(err).with_context(|| format!("could not run `{program}`")),
        };
        Ok(ApplyOutcome {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn read(&self, args: &[&str]) -> Result<Snapshot> {
        let out = self.run(args)?;
        if !out.success {
            bail!("`xrandr {}` failed: {}", args.join(" "), out.stderr.trim());
        }
        parse_verbose(&out.stdout).context("could not read the xrandr output")
    }
}

impl Backend for XrandrCli {
    fn query(&self) -> Result<Snapshot> {
        self.read(&["--verbose"])
    }

    fn requery(&self) -> Result<Snapshot> {
        self.read(&["--verbose", "--current"])
    }

    fn apply(&self, argv: &[String]) -> Result<ApplyOutcome> {
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.run(&args)
    }

    fn touches_x(&self) -> bool {
        true
    }
}

/// `-n`: the live state, read once, with applies simulated in memory. Nothing reaches X; the
/// editor shows the commands it would have run.
pub struct DryRun {
    inner: FixtureBackend,
}

impl DryRun {
    pub fn new(live: &dyn Backend) -> Result<Self> {
        Ok(Self {
            inner: FixtureBackend::new(live.query()?),
        })
    }

    /// Every argv an apply would have run.
    pub fn applied(&self) -> Vec<Vec<String>> {
        self.inner.applied()
    }
}

impl Backend for DryRun {
    fn query(&self) -> Result<Snapshot> {
        self.inner.query()
    }

    fn apply(&self, argv: &[String]) -> Result<ApplyOutcome> {
        self.inner.apply(argv)
    }
}

/// Refuses to drive the live X server where xrandr cannot work: under Wayland it would only
/// reconfigure XWayland, and without `DISPLAY` there is no server to talk to. `env` looks up an
/// environment variable.
pub fn check_session(env: impl Fn(&str) -> Option<String>) -> Result<(), String> {
    let set = |name: &str| env(name).is_some_and(|v| !v.is_empty());
    if set("WAYLAND_DISPLAY") || env("XDG_SESSION_TYPE").as_deref() == Some("wayland") {
        return Err(
            "this is a Wayland session, where xrandr would only reconfigure XWayland. \
                    Use wlr-randr, kanshi, hyprctl or the desktop's display settings instead. \
                    (--demo and --from-file still work.)"
                .to_owned(),
        );
    }
    if !set("DISPLAY") {
        return Err("DISPLAY is not set, so there is no X server to talk to. \
             (--demo and --from-file still work.)"
            .to_owned());
    }
    Ok(())
}

/// How to install xrandr on the distribution `/etc/os-release` describes.
pub fn install_hint(os_release: &str) -> String {
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let has = |id: &str| ids.split_whitespace().any(|w| w == id);
    let command = if has("arch") {
        "sudo pacman -S xorg-xrandr"
    } else if has("debian") || has("ubuntu") {
        "sudo apt install x11-xserver-utils"
    } else if has("fedora") || has("rhel") {
        "sudo dnf install xrandr"
    } else if has("opensuse") || has("suse") {
        "sudo zypper install xrandr"
    } else if has("void") {
        "sudo xbps-install xrandr"
    } else if has("alpine") {
        "sudo apk add xrandr"
    } else {
        return "Install your distribution's xrandr package.".to_owned();
    };
    format!("Install it with: {command}")
}

/// A snapshot held in memory, for `--demo`, `--from-file` and tests. An apply never touches X:
/// it records the arguments and updates the snapshot the way xrandr would, so the whole
/// apply → verify → revert flow can run without real displays.
pub struct FixtureBackend {
    state: Mutex<Snapshot>,
    applied: Mutex<Vec<Vec<String>>>,
}

impl FixtureBackend {
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            state: Mutex::new(snapshot),
            applied: Mutex::new(Vec::new()),
        }
    }

    pub fn demo() -> Self {
        Self::new(parse_verbose(DEMO).expect("the built-in demo fixture parses"))
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let snapshot =
            parse_verbose(&text).with_context(|| format!("could not parse {}", path.display()))?;
        Ok(Self::new(snapshot))
    }

    /// Every argv passed to [`Backend::apply`] so far.
    pub fn applied(&self) -> Vec<Vec<String>> {
        self.applied.lock().expect("fixture lock").clone()
    }

    /// Replaces the state, as plugging a display in or unplugging one does.
    pub fn set_state(&self, snapshot: Snapshot) {
        *self.state.lock().expect("fixture lock") = snapshot;
    }
}

impl Backend for FixtureBackend {
    fn query(&self) -> Result<Snapshot> {
        Ok(self.state.lock().expect("fixture lock").clone())
    }

    fn apply(&self, argv: &[String]) -> Result<ApplyOutcome> {
        self.applied
            .lock()
            .expect("fixture lock")
            .push(argv.to_vec());
        let mut state = self.state.lock().expect("fixture lock");
        Ok(simulate(&mut state, argv))
    }
}

/// Per-output settings collected from one `--output` group.
#[derive(Default)]
struct Request {
    off: bool,
    auto: bool,
    mode: Option<String>,
    rate: Option<f64>,
    pos: Option<Point>,
    rotation: Option<Rotation>,
    reflection: Option<Reflection>,
    transform: Option<Transform>,
    filter: Option<String>,
}

fn failure(stderr: String) -> ApplyOutcome {
    ApplyOutcome {
        success: false,
        stdout: String::new(),
        stderr,
    }
}

/// The requests of one xrandr call, per output in order of first mention.
#[derive(Default)]
struct Call {
    requests: Vec<(usize, Request)>,
    /// `Some(None)` for `--noprimary`.
    primary: Option<Option<usize>>,
    warnings: String,
}

/// Groups xrandr arguments by `--output`. Like xrandr, an unknown output is a warning and its
/// options are skipped; an unknown option or a malformed value is an error.
fn collect(snapshot: &Snapshot, argv: &[String]) -> Result<Call, String> {
    let mut call = Call::default();
    // `Some(None)` while the options belong to an unknown output.
    let mut current: Option<Option<usize>> = None;
    let mut discard = Request::default();
    let mut args = argv.iter();
    while let Some(arg) = args.next() {
        let mut value = |what: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("xrandr: {arg} requires {what}\n"))
        };
        match arg.as_str() {
            "--output" => {
                let name = value("an output name")?;
                current = Some(snapshot.find(&name));
                match snapshot.find(&name) {
                    Some(i) if !call.requests.iter().any(|(k, _)| *k == i) => {
                        call.requests.push((i, Request::default()))
                    }
                    Some(_) => {}
                    None => call
                        .warnings
                        .push_str(&format!("warning: output {name} not found; ignoring\n")),
                }
                continue;
            }
            "--noprimary" => {
                call.primary = Some(None);
                continue;
            }
            _ => {}
        }
        let Some(target) = current else {
            return Err(format!("xrandr: {arg} must follow --output\n"));
        };
        let req = match target {
            Some(i) => {
                &mut call
                    .requests
                    .iter_mut()
                    .find(|(k, _)| *k == i)
                    .expect("request exists")
                    .1
            }
            None => &mut discard,
        };
        let invalid = |what: &str, v: &str| format!("xrandr: invalid {what} {v}\n");
        match arg.as_str() {
            "--off" => req.off = true,
            "--auto" => req.auto = true,
            "--primary" => call.primary = Some(target),
            "--mode" => req.mode = Some(value("a mode")?),
            "--rate" | "--refresh" => {
                let v = value("a rate")?;
                req.rate = Some(v.parse().map_err(|_| invalid("rate", &v))?);
            }
            "--pos" => {
                let v = value("a position")?;
                let parsed = v
                    .split_once('x')
                    .and_then(|(x, y)| Some(Point::new(x.parse().ok()?, y.parse().ok()?)));
                req.pos = Some(parsed.ok_or_else(|| invalid("position", &v))?);
            }
            "--rotate" | "--orientation" => {
                let v = value("a rotation")?;
                req.rotation = Some(Rotation::parse(&v).ok_or_else(|| invalid("rotation", &v))?);
            }
            "--reflect" => {
                let v = value("a reflection")?;
                req.reflection =
                    Some(Reflection::parse(&v).ok_or_else(|| invalid("reflection", &v))?);
            }
            "--transform" => {
                let v = value("a transform")?;
                req.transform =
                    Some(parse_transform_arg(&v).ok_or_else(|| invalid("transform", &v))?);
            }
            "--scale" => {
                let v = value("a scale")?;
                let (sx, sy) = v.split_once('x').unwrap_or((v.as_str(), v.as_str()));
                let parsed = sx
                    .parse()
                    .ok()
                    .zip(sy.parse().ok())
                    .map(|(sx, sy)| Transform::scale(sx, sy));
                req.transform = Some(parsed.ok_or_else(|| invalid("scale", &v))?);
            }
            "--filter" => req.filter = Some(value("a filter")?),
            other => return Err(format!("xrandr: unrecognized option '{other}'\n")),
        }
    }
    Ok(call)
}

/// Applies xrandr arguments to a snapshot the way xrandr would. A failed call changes nothing.
fn simulate(snapshot: &mut Snapshot, argv: &[String]) -> ApplyOutcome {
    let Call {
        requests,
        primary,
        warnings,
    } = match collect(snapshot, argv) {
        Ok(call) => call,
        Err(message) => return failure(message),
    };

    let mut next = snapshot.clone();
    for (i, req) in requests {
        let out = &mut next.outputs[i];
        if req.off {
            out.active = None;
            out.crtc = None;
            continue;
        }
        let mode = match (&req.mode, req.auto) {
            (Some(m), _) => match m
                .strip_prefix("0x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            {
                Some(xid) => out.mode(ModeId::from_xid(xid)),
                None => out.find_mode(m, req.rate),
            },
            (None, true) => out.preferred_mode(),
            (None, false) => out.current_mode().or_else(|| out.preferred_mode()),
        };
        let Some(mode) = mode.cloned() else {
            return failure(format!(
                "xrandr: cannot find mode {} for output {}\n",
                req.mode.unwrap_or_default(),
                out.name
            ));
        };
        let old = out.active.clone();
        let rotation = req
            .rotation
            .or(old.as_ref().map(|a| a.rotation))
            .unwrap_or_default();
        let reflection = req
            .reflection
            .or(old.as_ref().map(|a| a.reflection))
            .unwrap_or_default();
        let mut transform = req
            .transform
            .or(old.as_ref().and_then(|a| a.scaling.transform().cloned()))
            .unwrap_or_default();
        if let Some(filter) = req.filter {
            transform.filter = filter;
        }
        if transform.is_identity() {
            transform = Transform::identity();
        }
        let pos = req.pos.or(old.as_ref().map(|a| a.pos)).unwrap_or_default();
        let scaling = Scaling::X11(transform);
        let size: Size = effective_size(mode.size(), rotation, &scaling);
        out.active = Some(ActiveConfig {
            mode: mode.id,
            pos,
            size,
            rotation,
            reflection,
            scaling,
            panning: old.and_then(|a| a.panning),
        });
    }

    if let Some(p) = primary {
        for (k, out) in next.outputs.iter_mut().enumerate() {
            out.primary = Some(k) == p;
        }
    }

    // Hand out CRTCs: keep the ones in use, give newly enabled outputs the first free one.
    let mut used: Vec<u32> = next
        .outputs
        .iter()
        .filter(|o| o.active.is_some())
        .filter_map(|o| o.crtc)
        .collect();
    for out in next
        .outputs
        .iter_mut()
        .filter(|o| o.active.is_some() && o.crtc.is_none())
    {
        let Some(&free) = out.crtcs.iter().find(|c| !used.contains(c)) else {
            return failure(format!(
                "xrandr: cannot find crtc for output {}\n",
                out.name
            ));
        };
        used.push(free);
        out.crtc = Some(free);
    }

    let (w, h) = next
        .outputs
        .iter()
        .filter_map(|o| o.active.as_ref())
        .fold((0, 0), |(w, h), a| {
            (w.max(a.pos.x + a.size.w), h.max(a.pos.y + a.size.h))
        });
    if let Some(screen) = &mut next.screen {
        if w > screen.max.w || h > screen.max.h {
            return failure(format!(
                "xrandr: screen cannot be larger than {}x{} (desired size {w}x{h})\n",
                screen.max.w, screen.max.h
            ));
        }
        screen.current = Size::new(w.max(screen.min.w), h.max(screen.min.h));
    }
    *snapshot = next;
    ApplyOutcome {
        success: true,
        stdout: String::new(),
        stderr: warnings,
    }
}

/// `none`, or nine comma-separated numbers.
pub(crate) fn parse_transform_arg(value: &str) -> Option<Transform> {
    if value == "none" {
        return Some(Transform::identity());
    }
    let values: Vec<f64> = value
        .split(',')
        .map(|v| v.trim().parse().ok())
        .collect::<Option<_>>()?;
    if values.len() != 9 {
        return None;
    }
    let mut matrix = [[0.0; 3]; 3];
    for (k, v) in values.into_iter().enumerate() {
        matrix[k / 3][k % 3] = v;
    }
    Some(Transform {
        matrix,
        filter: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn live_x_needs_an_x_session() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert!(check_session(env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")])).is_ok());
        let wayland = check_session(env(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(wayland.unwrap_err().contains("wlr-randr"));
        let wayland = check_session(env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "wayland")]));
        assert!(wayland.is_err());
        assert!(
            check_session(env(&[]))
                .unwrap_err()
                .starts_with("DISPLAY is not set")
        );
    }

    #[test]
    fn install_hints_follow_os_release() {
        let hint = |text: &str| install_hint(text);
        assert!(
            hint("NAME=\"EndeavourOS\"\nID=\"endeavouros\"\nID_LIKE=\"arch\"\n")
                .ends_with("pacman -S xorg-xrandr")
        );
        assert!(hint("ID=ubuntu\nID_LIKE=debian\n").ends_with("apt install x11-xserver-utils"));
        assert!(hint("ID=fedora\n").ends_with("dnf install xrandr"));
        assert!(
            hint("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n")
                .ends_with("zypper install xrandr")
        );
        assert!(hint("ID=void\n").ends_with("xbps-install xrandr"));
        assert!(hint("ID=alpine\n").ends_with("apk add xrandr"));
        assert_eq!(hint(""), "Install your distribution's xrandr package.");
    }

    #[test]
    fn dry_run_simulates_without_touching_x() {
        let dry = DryRun::new(&FixtureBackend::demo()).unwrap();
        assert!(!dry.touches_x());
        assert!(XrandrCli::new().touches_x());
        let outcome = dry.apply(&args("--output HDMI-1-0 --off")).unwrap();
        assert!(outcome.success);
        assert!(dry.query().unwrap().outputs[0].active.is_none());
        assert_eq!(dry.applied().len(), 1);
    }

    #[test]
    fn demo_parses() {
        let snap = FixtureBackend::demo().query().unwrap();
        let names: Vec<&str> = snap.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["HDMI-1-0", "DP-1-2", "eDP-1", "DP-1-3"]);
    }

    #[test]
    fn fixture_apply_records_and_updates() {
        let backend = FixtureBackend::demo();
        let demo = backend.query().unwrap();
        let edp_xid = demo.outputs[demo.find("eDP-1").unwrap()]
            .preferred_mode()
            .unwrap()
            .id;
        let argv = args(&format!(
            "--output HDMI-1-0 --off --output DP-1-3 --mode 3840x2160 --rate 60 --pos 4480x0 --rotate left \
             --output eDP-1 --primary --mode 0x{edp_xid:x} --pos 2240x1440"
        ));
        let outcome = backend.apply(&argv).unwrap();
        assert!(outcome.success, "{}", outcome.stderr);
        assert_eq!(backend.applied(), vec![argv]);

        let snap = backend.query().unwrap();
        let out = |name: &str| &snap.outputs[snap.find(name).unwrap()];
        assert!(out("HDMI-1-0").active.is_none());
        let dp13 = out("DP-1-3").active.as_ref().unwrap();
        assert_eq!(
            (dp13.pos, dp13.size, dp13.rotation),
            (Point::new(4480, 0), Size::new(2160, 3840), Rotation::Left)
        );
        assert_eq!(
            out("DP-1-3").crtc,
            Some(4),
            "takes the CRTC HDMI-1-0 released"
        );
        assert!(out("eDP-1").primary);
        assert!(!out("DP-1-2").primary);
        assert_eq!(snap.screen.unwrap().current, Size::new(6640, 3840));
    }

    #[test]
    fn fixture_apply_warns_like_xrandr() {
        let backend = FixtureBackend::demo();
        let before = backend.query().unwrap();
        let outcome = backend.apply(&args("--output eDP-9 --off")).unwrap();
        assert!(outcome.success);
        assert_eq!(
            outcome.stderr,
            "warning: output eDP-9 not found; ignoring\n"
        );
        assert_eq!(backend.query().unwrap(), before);

        let outcome = backend.apply(&args("--output eDP-1 --bogus")).unwrap();
        assert!(!outcome.success);
        assert_eq!(
            backend.query().unwrap(),
            before,
            "a failed call changes nothing"
        );
    }

    #[test]
    fn fixture_apply_scale_and_transform_reset() {
        let backend = FixtureBackend::demo();
        backend
            .apply(&args("--output eDP-1 --scale 1.5x1.5"))
            .unwrap();
        let snap = backend.query().unwrap();
        let edp = snap.outputs[snap.find("eDP-1").unwrap()]
            .active
            .clone()
            .unwrap();
        assert_eq!(edp.size, Size::new(2880, 1620));
        assert_eq!(edp.scaling.transform().unwrap().filter, "bilinear");
        backend
            .apply(&args("--output eDP-1 --transform none"))
            .unwrap();
        let snap = backend.query().unwrap();
        let edp = snap.outputs[snap.find("eDP-1").unwrap()]
            .active
            .clone()
            .unwrap();
        assert_eq!(edp.size, Size::new(1920, 1080));
        assert!(edp.scaling.is_identity());
    }
}
