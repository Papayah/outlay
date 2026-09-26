//! The binary, run against fixtures only.

use std::process::Command;

fn outlay(args: &[&str]) -> (bool, String, String) {
    // Keep the developer's own config file out of the tests, and ~/.screenlayout out of reach.
    let out = Command::new(env!("CARGO_BIN_EXE_outlay"))
        .args(args)
        .env("XDG_CONFIG_HOME", "/nonexistent/outlay-test-config")
        .env("HOME", "/nonexistent/outlay-test-home")
        .output()
        .expect("run outlay");
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/xrandr/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn list_prints_outputs_then_rates() {
    let (ok, stdout, stderr) = outlay(&["--demo", "list"]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with("1  HDMI-1-0  connected · Philips FTV · 1440x810 mm · 34 dpi\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\n    2560x1440  143.91*  119.88  99.95  59.95+\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\n    1920x1080  60.00  60.00i  59.94*  50.00\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\n4  DP-1-3  connected · LG HDR 4K · 600x340 mm\n"),
        "{stdout}"
    );
}

#[test]
fn list_names_the_outputs_that_are_not_connected() {
    let (ok, stdout, _) = outlay(&["list", "--from-file", &fixture("laptop-edp-hdmi")]);
    assert!(ok);
    assert!(
        stdout.contains("    1920x1080  165.01*+  60.01dbl  59.97dbl  59.96  59.93\n"),
        "{stdout}"
    );
    assert!(stdout.ends_with(
        "\nnot connected: DP-1 HDMI-1 DP-2 DP-3 DP-4 DP-5 DP-1-0 DP-1-1 DP-1-2 DP-1-3 DP-1-4\n"
    ));
}

#[test]
fn show_prints_the_table() {
    let (ok, stdout, stderr) = outlay(&["show", "--from-file", &fixture("laptop-edp-hdmi")]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with("outlay · 2 on · 0 off · screen 3840x1080 of max 16384x16384\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("\n1  eDP-1 ★   1920x1080 @ 165.01  1920,0"),
        "{stdout}"
    );
    assert!(
        stdout.contains("AUO B156HAN12.H · 344x193 mm · 142 dpi"),
        "{stdout}"
    );
}

#[test]
fn show_draws_the_layout_to_scale_above_the_table() {
    let (ok, stdout, stderr) = outlay(&["--demo", "show"]);
    assert!(ok, "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[1], "", "{stdout}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("│           1 HDMI-1-0           │")),
        "{stdout}"
    );
    assert!(
        lines.iter().any(|l| l.contains("▸")),
        "the stick link of HDMI-1-0 points at DP-1-2: {stdout}"
    );
    let table = lines
        .iter()
        .position(|l| l.starts_with("#  output"))
        .unwrap();
    assert!(table > 10, "the diagram comes first: {stdout}");
    assert!(
        !stdout.contains('\x1b'),
        "no colour when stdout is not a terminal"
    );
}

#[test]
fn a_bad_capture_is_reported() {
    let (ok, _, stderr) = outlay(&["--from-file", "/nonexistent/capture.txt", "show"]);
    assert!(!ok);
    assert!(
        stderr.starts_with("outlay: could not read /nonexistent/capture.txt"),
        "{stderr}"
    );
}

/// A fresh directory with copies of some fixture profiles; never `~/.screenlayout`.
fn layouts(tag: &str, profiles: &[&str]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("outlay-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for name in profiles {
        let from = format!(
            "{}/tests/fixtures/screenlayout/{name}.sh",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::copy(from, dir.join(format!("{name}.sh"))).unwrap();
    }
    dir
}

#[test]
fn apply_n_prints_the_command_only() {
    let dir = layouts("apply-n", &["tv-home"]);
    let d = dir.to_str().unwrap();
    let (ok, stdout, stderr) = outlay(&["--demo", "--layouts-dir", d, "apply", "tv-home", "-n"]);
    assert!(ok, "{stderr}");
    assert_eq!(
        stdout,
        "xrandr --output HDMI-1-0 --mode 0x1c2 --pos 0x0 --rotate normal --reflect normal \
         --output DP-1-2 --off \
         --output eDP-1 --primary --mode 0x1cf --pos 0x1440 --rotate normal --reflect normal \
         --output DP-1-3 --off\n"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn apply_keeps_at_once_without_a_countdown_and_remaps_missing_outputs() {
    let dir = layouts("apply-keep", &["home-setup"]);
    let d = dir.to_str().unwrap();
    let args = [
        "--demo",
        "--layouts-dir",
        d,
        "--revert-timeout",
        "0",
        "apply",
        "home-setup",
    ];
    let (ok, _, stderr) = outlay(&args);
    assert!(ok, "{stderr}");
    assert!(
        stderr.starts_with("eDP-2 is not connected; using eDP-1 instead.\n"),
        "{stderr}"
    );
    assert!(
        stderr.contains("  HDMI-1-0  pos 0,360 → 0,853\n"),
        "{stderr}"
    );
    assert!(stderr.ends_with("Kept the new layout.\n"), "{stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn apply_reverts_unless_kept() {
    let dir = layouts("apply-revert", &["tv-home"]);
    let d = dir.to_str().unwrap();
    let args = [
        "--demo",
        "--layouts-dir",
        d,
        "--revert-timeout",
        "1",
        "apply",
        "tv-home",
    ];
    let (ok, _, stderr) = outlay(&args);
    assert!(!ok, "{stderr}");
    assert!(
        stderr.contains("Keep this layout? Type y and Enter within 1 s"),
        "{stderr}"
    );
    assert!(stderr.contains("No answer in 1 s: reverted"), "{stderr}");
    assert!(
        stderr.ends_with("outlay: the layout was not kept\n"),
        "{stderr}"
    );

    // A y typed after the first second keeps it.
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_outlay"))
        .args([
            "--demo",
            "--layouts-dir",
            d,
            "--revert-timeout",
            "5",
            "apply",
            "tv-home",
        ])
        .env("XDG_CONFIG_HOME", "/nonexistent/outlay-test-config")
        .env("HOME", "/nonexistent/outlay-test-home")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // Typed while the screens were dark: ignored.
    writeln!(stdin, "y").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1300));
    writeln!(stdin, "y").unwrap();
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.ends_with("Kept the new layout.\n"), "{stderr}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn save_writes_a_script_and_will_not_overwrite_a_different_one_unasked() {
    let dir = layouts("save", &[]);
    let d = dir.to_str().unwrap();
    let (ok, stdout, stderr) = outlay(&["--demo", "--layouts-dir", d, "save", "desk", "-n"]);
    assert!(ok, "{stderr}");
    assert!(
        stdout.starts_with("#!/bin/sh\n# generated by outlay "),
        "{stdout}"
    );
    assert!(!dir.join("desk.sh").exists(), "-n only prints");

    let (ok, _, stderr) = outlay(&["--demo", "--layouts-dir", d, "save", "desk"]);
    assert!(ok, "{stderr}");
    assert!(stderr.starts_with("Saved "), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.join("desk.sh")).unwrap(),
        stdout
    );
    let (ok, _, stderr) = outlay(&["--demo", "--layouts-dir", d, "save", "desk"]);
    assert!(ok, "{stderr}");
    assert!(stderr.ends_with("desk.sh is up to date.\n"), "{stderr}");

    let other = fixture("dock-mst-3");
    let (ok, _, stderr) = outlay(&["--from-file", &other, "--layouts-dir", d, "save", "desk"]);
    assert!(!ok);
    assert!(
        stderr.contains("exists and differs; pass --force"),
        "{stderr}"
    );
    assert!(
        stderr.contains("\n- xrandr --output HDMI-1-0"),
        "the diff: {stderr}"
    );
    let (ok, _, stderr) = outlay(&[
        "--from-file",
        &other,
        "--layouts-dir",
        d,
        "save",
        "-f",
        "desk",
    ]);
    assert!(ok, "{stderr}");
    let text = std::fs::read_to_string(dir.join("desk.sh")).unwrap();
    assert!(text.contains("--output DP-2.2 --primary"), "{text}");
    std::fs::remove_dir_all(dir).unwrap();
}
