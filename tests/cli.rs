//! The binary, run against fixtures only.

use std::process::Command;

fn outlay(args: &[&str]) -> (bool, String, String) {
    // Keep the developer's own config file out of the tests.
    let out = Command::new(env!("CARGO_BIN_EXE_outlay"))
        .args(args)
        .env("XDG_CONFIG_HOME", "/nonexistent/outlay-test-config")
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
