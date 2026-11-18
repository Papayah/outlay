//! Xvfb for the RandR event tests. It runs only when `OUTLAY_TEST_XVFB` names the Xvfb binary;
//! otherwise a test prints "skipped" and passes. Xvfb draws into memory: no window opens.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

pub struct Xvfb {
    child: Child,
    /// `:N`.
    pub display: String,
}

impl Xvfb {
    /// Starts Xvfb on a free display, or says "skipped" and returns `None`.
    pub fn start() -> Option<Self> {
        let Some(xvfb) = std::env::var_os("OUTLAY_TEST_XVFB") else {
            println!("skipped: set OUTLAY_TEST_XVFB=/path/to/Xvfb to run against Xvfb");
            return None;
        };
        // -displayfd: Xvfb picks a free display and writes its number to stdout.
        let mut child = Command::new(&xvfb)
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1280x1024x24",
                "-nolisten",
                "tcp",
            ])
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("could not start {}: {e}", xvfb.to_string_lossy()));
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert!(!line.trim().is_empty(), "Xvfb printed no display number");
        Some(Xvfb {
            child,
            display: format!(":{}", line.trim()),
        })
    }

    /// Runs `xrandr ARGS` on this server only: the real `DISPLAY` never reaches it.
    pub fn xrandr(&self, args: &[&str]) -> std::process::Output {
        Command::new("xrandr")
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("DISPLAY", &self.display)
            .output()
            .expect("xrandr")
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        self.kill();
    }
}
