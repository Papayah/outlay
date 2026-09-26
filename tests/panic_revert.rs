//! The panic hook runs `revert.sh` while an applied layout waits for its answer. The hook is
//! global to the process, so this file holds a single test.

use std::path::{Path, PathBuf};

fn script(dir: &Path, marker: &Path) -> PathBuf {
    let path = dir.join("revert.sh");
    std::fs::write(&path, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    path
}

#[test]
fn a_panic_during_the_countdown_runs_revert_sh() {
    let dir = std::env::temp_dir().join(format!("outlay-panic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("reverted");
    let revert = script(&dir, &marker);

    outlay::tui::install_panic_hook(false);
    outlay::tui::arm_panic_revert(Some(revert));
    assert!(std::panic::catch_unwind(|| panic!("boom during the countdown")).is_err());
    assert!(marker.exists(), "revert.sh ran");

    std::fs::remove_file(&marker).unwrap();
    outlay::tui::arm_panic_revert(None);
    assert!(std::panic::catch_unwind(|| panic!("boom after keeping")).is_err());
    assert!(!marker.exists(), "a disarmed hook runs nothing");

    std::fs::remove_dir_all(dir).unwrap();
}
