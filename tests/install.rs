#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
//! `install.sh`, piped into `sh` the way users run it, against a fake release on disk. Nothing
//! here uses the network or leaves its sandbox directory.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The Rust target the installer picks on this machine.
fn target() -> String {
    format!("{}-unknown-linux-musl", std::env::consts::ARCH)
}

/// The other architecture, listed in every SHA256SUMS so the installer has to skip it.
fn other_target() -> &'static str {
    if std::env::consts::ARCH == "x86_64" {
        "aarch64-unknown-linux-musl"
    } else {
        "x86_64-unknown-linux-musl"
    }
}

/// The shells the installer writes completions for, when this system has them.
fn shells() -> Vec<&'static str> {
    ["bash", "zsh", "fish"]
        .into_iter()
        .filter(|sh| Path::new("/usr/bin").join(sh).exists() || Path::new("/bin").join(sh).exists())
        .collect()
}

struct Run {
    ok: bool,
    stderr: String,
}

/// A home directory and a release server (`file://`) with v0.1.0 and v0.2.0; latest is v0.2.0.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let root =
            std::env::temp_dir().join(format!("outlay-install-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home")).unwrap();
        let sandbox = Sandbox { root };
        sandbox.release("0.1.0");
        sandbox.release("0.2.0");
        let latest = sandbox.releases().join("latest/download");
        fs::create_dir_all(&latest).unwrap();
        fs::copy(
            sandbox.releases().join("download/v0.2.0/SHA256SUMS"),
            latest.join("SHA256SUMS"),
        )
        .unwrap();
        sandbox
    }

    fn releases(&self) -> PathBuf {
        self.root.join("releases")
    }

    fn bin(&self) -> PathBuf {
        self.root.join("home/.local/bin")
    }

    fn data_home(&self) -> PathBuf {
        self.root.join("xdg-data")
    }

    fn config_home(&self) -> PathBuf {
        self.root.join("xdg-config")
    }

    fn asset(&self, version: &str) -> PathBuf {
        self.releases().join(format!(
            "download/v{version}/outlay-{version}-{}.tar.gz",
            target()
        ))
    }

    /// `download/v{version}/` with a tarball laid out like the real one, whose `outlay` is a
    /// script that knows `--version` and `completions`, and a SHA256SUMS for it.
    fn release(&self, version: &str) {
        let name = format!("outlay-{version}-{}", target());
        let stage = self.root.join("stage").join(&name);
        fs::create_dir_all(&stage).unwrap();
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'outlay {version}' ;;\n  \
             completions) echo \"# outlay {version} completions for $2\" ;;\n  *) exit 2 ;;\nesac\n"
        );
        fs::write(stage.join("outlay"), script).unwrap();
        fs::set_permissions(stage.join("outlay"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(stage.join("README.md"), "readme\n").unwrap();
        fs::write(stage.join("LICENSE"), "license\n").unwrap();

        let dir = self.releases().join(format!("download/v{version}"));
        fs::create_dir_all(&dir).unwrap();
        let tarball = format!("{name}.tar.gz");
        let tar = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&tarball))
            .arg("-C")
            .arg(self.root.join("stage"))
            .arg(&name)
            .status()
            .unwrap();
        assert!(tar.success());
        let sum = Command::new("sha256sum")
            .arg(&tarball)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(sum.status.success());
        let other = format!("{:064x}  outlay-{version}-{}.tar.gz\n", 0, other_target());
        let sums = other + &String::from_utf8(sum.stdout).unwrap();
        fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    }

    fn completion_file(&self, shell: &str) -> PathBuf {
        match shell {
            "bash" => self.data_home().join("bash-completion/completions/outlay"),
            "zsh" => self.data_home().join("zsh/site-functions/_outlay"),
            "fish" => self.config_home().join("fish/completions/outlay.fish"),
            _ => unreachable!(),
        }
    }

    fn installed_version(&self, bin: &Path) -> String {
        let out = Command::new(bin.join("outlay"))
            .arg("--version")
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn run(&self, args: &[&str]) -> Run {
        self.run_in(&self.root, args)
    }

    /// `sh -s -- ARGS < install.sh` with a clean environment: nothing from the developer's shell
    /// leaks in, and every path the installer writes to is inside the sandbox. `OUTLAY_TEST_SH`
    /// names another shell to run it with (`/bin/sh` is bash on Arch, dash on Debian).
    fn run_in(&self, cwd: &Path, args: &[&str]) -> Run {
        let script = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh")).unwrap();
        let shell = std::env::var("OUTLAY_TEST_SH").unwrap_or_else(|_| "sh".to_string());
        let mut child = Command::new(shell)
            .args(["-s", "--"])
            .args(args)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("XDG_DATA_HOME", self.data_home())
            .env("XDG_CONFIG_HOME", self.config_home())
            .env("XDG_STATE_HOME", self.root.join("xdg-state"))
            .env(
                "OUTLAY_RELEASES_URL",
                format!("file://{}", self.releases().display()),
            )
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin().display()))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run sh");
        child.stdin.take().unwrap().write_all(&script).unwrap();
        let out = child.wait_with_output().unwrap();
        Run {
            ok: out.status.success(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }

    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(self.bin())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".outlay.new."))
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_fresh_install_takes_the_latest_release_with_completions() {
    let sb = Sandbox::new("fresh");
    let run = sb.run(&[]);
    assert!(run.ok, "{}", run.stderr);
    let path = sb.bin().join("outlay");
    assert!(
        run.stderr
            .contains(&format!("Installed outlay 0.2.0 to {}", path.display())),
        "{}",
        run.stderr
    );
    assert_eq!(sb.installed_version(&sb.bin()), "outlay 0.2.0");
    for shell in shells() {
        let file = sb.completion_file(shell);
        let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file:?}: {e}"));
        assert_eq!(text, format!("# outlay 0.2.0 completions for {shell}\n"));
    }
    assert!(sb.leftovers().is_empty());
}

#[test]
fn a_second_run_is_up_to_date_and_downloads_nothing() {
    let sb = Sandbox::new("rerun");
    assert!(sb.run(&[]).ok);
    fs::remove_file(sb.asset("0.2.0")).unwrap();
    let run = sb.run(&[]);
    assert!(run.ok, "{}", run.stderr);
    assert!(
        run.stderr.contains(&format!(
            "outlay 0.2.0 is up to date ({})",
            sb.bin().join("outlay").display()
        )),
        "{}",
        run.stderr
    );
}

#[test]
fn a_pinned_install_updates_to_the_latest() {
    let sb = Sandbox::new("update");
    let run = sb.run(&["--version", "0.1.0"]);
    assert!(run.ok, "{}", run.stderr);
    assert_eq!(sb.installed_version(&sb.bin()), "outlay 0.1.0");
    let run = sb.run(&[]);
    assert!(run.ok, "{}", run.stderr);
    assert!(
        run.stderr.contains("Updated outlay 0.1.0 -> 0.2.0"),
        "{}",
        run.stderr
    );
    assert_eq!(sb.installed_version(&sb.bin()), "outlay 0.2.0");
    // The completions come from the new binary.
    for shell in shells() {
        let text = fs::read_to_string(sb.completion_file(shell)).unwrap();
        assert!(text.contains("outlay 0.2.0"), "{shell}: {text}");
    }
}

#[test]
fn pinning_an_older_version_downgrades_and_force_reinstalls() {
    let sb = Sandbox::new("downgrade");
    assert!(sb.run(&[]).ok);
    let run = sb.run(&["--version", "v0.1.0"]);
    assert!(run.ok, "{}", run.stderr);
    assert!(
        run.stderr.contains("Updated outlay 0.2.0 -> 0.1.0"),
        "{}",
        run.stderr
    );
    assert_eq!(sb.installed_version(&sb.bin()), "outlay 0.1.0");

    let run = sb.run(&["--version=0.1.0"]);
    assert!(
        run.stderr.contains("outlay 0.1.0 is up to date"),
        "{}",
        run.stderr
    );
    let run = sb.run(&["--version", "0.1.0", "--force"]);
    assert!(run.ok, "{}", run.stderr);
    assert!(
        run.stderr.contains("Reinstalled outlay 0.1.0"),
        "{}",
        run.stderr
    );
}

#[test]
fn an_unreleased_version_is_an_error() {
    let sb = Sandbox::new("unreleased");
    let run = sb.run(&["--version", "9.9.9"]);
    assert!(!run.ok);
    assert!(
        run.stderr.contains("is 9.9.9 a released version?"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_corrupt_download_fails_and_keeps_the_old_binary() {
    let sb = Sandbox::new("corrupt");
    assert!(sb.run(&["--version", "0.1.0"]).ok);
    let mut tarball = fs::OpenOptions::new()
        .append(true)
        .open(sb.asset("0.2.0"))
        .unwrap();
    tarball.write_all(b"tampered").unwrap();
    drop(tarball);

    let run = sb.run(&[]);
    assert!(!run.ok);
    assert!(run.stderr.contains("checksum mismatch"), "{}", run.stderr);
    assert_eq!(sb.installed_version(&sb.bin()), "outlay 0.1.0");
    assert!(sb.leftovers().is_empty(), "{:?}", sb.leftovers());
}

#[test]
fn a_release_without_this_architecture_points_to_cargo() {
    let sb = Sandbox::new("noarch");
    let sums = sb.releases().join("latest/download/SHA256SUMS");
    let only_other: String = fs::read_to_string(&sums)
        .unwrap()
        .lines()
        .filter(|line| !line.contains(&target()))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&sums, only_other).unwrap();

    let run = sb.run(&[]);
    assert!(!run.ok);
    assert!(run.stderr.contains(&target()), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("cargo install --git https://github.com/Papayah/outlay --locked"),
        "{}",
        run.stderr
    );
    assert!(!sb.bin().join("outlay").exists());
}

#[test]
fn uninstall_removes_the_binary_and_completions_but_keeps_the_config() {
    let sb = Sandbox::new("uninstall");
    assert!(sb.run(&[]).ok);
    let config = sb.config_home().join("outlay/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "nudge_step = 5\n").unwrap();

    let run = sb.run(&["--uninstall"]);
    assert!(run.ok, "{}", run.stderr);
    assert!(!sb.bin().join("outlay").exists());
    for shell in shells() {
        assert!(!sb.completion_file(shell).exists(), "{shell}");
    }
    assert!(config.exists());
    assert!(
        run.stderr
            .contains(&format!("kept {}", config.parent().unwrap().display())),
        "{}",
        run.stderr
    );
}

#[test]
fn a_custom_directory_skips_completions() {
    let sb = Sandbox::new("custom");
    let bin = sb.root.join("opt/bin");
    let run = sb.run(&["--to", bin.to_str().unwrap()]);
    assert!(run.ok, "{}", run.stderr);
    assert_eq!(sb.installed_version(&bin), "outlay 0.2.0");
    assert!(
        run.stderr.contains("outlay completions bash|zsh|fish"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("is not on your PATH"), "{}", run.stderr);
    for shell in ["bash", "zsh", "fish"] {
        assert!(!sb.completion_file(shell).exists(), "{shell}");
    }
}

#[test]
fn a_relative_directory_is_taken_from_the_working_directory() {
    let sb = Sandbox::new("relative");
    let work = sb.root.join("work");
    fs::create_dir_all(&work).unwrap();
    let run = sb.run_in(&work, &["--to", "tools/bin/"]);
    assert!(run.ok, "{}", run.stderr);
    let bin = work.join("tools/bin");
    assert_eq!(sb.installed_version(&bin), "outlay 0.2.0");
    assert!(
        run.stderr.contains(&format!(
            "Installed outlay 0.2.0 to {}/outlay",
            bin.display()
        )),
        "{}",
        run.stderr
    );
}
