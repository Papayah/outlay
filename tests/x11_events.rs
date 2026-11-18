//! RandR events from a real X server: Xvfb, started per test. Every test passes with "skipped"
//! unless `OUTLAY_TEST_XVFB` names the Xvfb binary:
//!
//! ```sh
//! OUTLAY_TEST_XVFB=$(command -v Xvfb) cargo test --test x11_events -- --nocapture
//! ```

mod common;

use std::time::Duration;

use common::xvfb::Xvfb;
use outlay::xrandr::events::Events;

const QUIET: Duration = Duration::from_millis(200);

#[test]
fn a_screen_change_wakes_the_wait_and_a_lost_server_is_an_error() {
    let Some(mut xvfb) = Xvfb::start() else {
        return;
    };
    let events = Events::listen(Some(&xvfb.display)).unwrap();
    assert!(!events.wait(QUIET).unwrap(), "nothing changed yet");

    // Xvfb has one mode and a maximum screen of that size: a new physical size is the change
    // it accepts, and it sends RRScreenChangeNotify for it.
    let out = xvfb.xrandr(&["--fbmm", "300x200"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(events.wait(Duration::from_secs(5)).unwrap(), "no event");
    assert!(!events.wait(QUIET).unwrap(), "the events were all taken");

    xvfb.kill();
    assert!(events.wait(Duration::from_secs(5)).is_err());
}
