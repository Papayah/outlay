//! Headless sway for the live Wayland tests. It runs only when `OUTLAY_TEST_SWAY` names the sway
//! binary; otherwise a test prints "skipped" and passes, as `OUTLAY_TEST_SH` does for the
//! installer tests.
//!
//! sway never gets `DISPLAY` or `WAYLAND_DISPLAY`, so it cannot open a window on the developer's
//! screen, and it runs in a private runtime directory. That directory stays short: a socket path
//! is limited to 108 bytes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub struct Sway {
    child: Child,
    sway: PathBuf,
    /// The private `XDG_RUNTIME_DIR`.
    pub runtime: PathBuf,
    /// `wayland-1`.
    pub display: String,
    /// The compositor's socket.
    pub socket: PathBuf,
    /// sway's IPC socket, for `swaymsg`.
    pub swaysock: PathBuf,
}

static STARTED: AtomicUsize = AtomicUsize::new(0);

impl Sway {
    /// Starts sway with `outputs` headless outputs, or says "skipped" and returns `None` when
    /// `OUTLAY_TEST_SWAY` is not set.
    pub fn start(outputs: u32) -> Option<Self> {
        let Some(sway) = std::env::var_os("OUTLAY_TEST_SWAY") else {
            println!("skipped: set OUTLAY_TEST_SWAY=/path/to/sway to run against headless sway");
            return None;
        };
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        let n = STARTED.fetch_add(1, Ordering::SeqCst);
        let runtime = base.join(format!("outlay-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = runtime.join("config");
        std::fs::write(&config, "").unwrap();
        let log = std::fs::File::create(runtime.join("sway.log")).unwrap();
        let child = Command::new(&sway)
            .arg("-c")
            .arg(&config)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("SWAYSOCK")
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("WLR_HEADLESS_OUTPUTS", outputs.to_string())
            .env("WLR_LIBINPUT_NO_DEVICES", "1")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap_or_else(|e| panic!("could not start {}: {e}", Path::new(&sway).display()));
        let mut sway = Sway {
            child,
            sway: PathBuf::from(sway),
            runtime,
            display: String::new(),
            socket: PathBuf::new(),
            swaysock: PathBuf::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let found = |prefix: &str, suffix: &str| {
                std::fs::read_dir(&sway.runtime)
                    .unwrap()
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find(|n| n.starts_with(prefix) && n.ends_with(suffix))
            };
            if let (Some(display), Some(ipc)) = (
                found("wayland-", "").filter(|n| !n.ends_with(".lock")),
                found("sway-ipc.", ".sock"),
            ) {
                sway.socket = sway.runtime.join(&display);
                sway.display = display;
                sway.swaysock = sway.runtime.join(ipc);
                // The IPC socket exists before sway answers on it.
                if sway.try_swaymsg(&["-t", "get_version"]).is_some() {
                    return Some(sway);
                }
            }
            if let Ok(Some(status)) = sway.child.try_wait() {
                panic!("sway exited with {status}:\n{}", sway.log());
            }
            assert!(
                Instant::now() < deadline,
                "sway did not start within 5 s:\n{}",
                sway.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.runtime.join("sway.log")).unwrap_or_default()
    }

    /// A command that talks to this sway and nothing else: no `DISPLAY`, this runtime directory.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_remove("DISPLAY")
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("WAYLAND_DISPLAY", &self.display)
            .env("SWAYSOCK", &self.swaysock)
            .stdin(Stdio::null());
        cmd
    }

    fn try_swaymsg(&self, args: &[&str]) -> Option<String> {
        let out = self
            .command(self.sway.with_file_name("swaymsg"))
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Runs `swaymsg ARGS` and returns what it printed.
    pub fn swaymsg(&self, args: &[&str]) -> String {
        self.try_swaymsg(args)
            .unwrap_or_else(|| panic!("swaymsg {args:?} failed:\n{}", self.log()))
    }

    /// `swaymsg -t get_outputs`, parsed.
    pub fn outputs(&self) -> Vec<serde_json::Value> {
        serde_json::from_str(&self.swaymsg(&["-t", "get_outputs", "-r"])).unwrap()
    }

    /// The logical rectangle sway gives output `name`: x, y, width, height.
    pub fn rect(&self, name: &str) -> (i64, i64, i64, i64) {
        let outputs = self.outputs();
        let out = outputs
            .iter()
            .find(|o| o["name"] == name)
            .unwrap_or_else(|| panic!("sway has no output {name}"));
        let r = &out["rect"];
        let n = |k: &str| r[k].as_i64().unwrap();
        (n("x"), n("y"), n("width"), n("height"))
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}
