//! RandR events from the X server, so `outlay watch` reads the displays when they change instead
//! of polling. Pure Rust (`x11rb`'s `RustConnection`): no libxcb.

use std::time::Duration;

use anyhow::{Context, Result};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use x11rb::connection::Connection;
use x11rb::protocol::randr::{ConnectionExt, NotifyMask};
use x11rb::rust_connection::RustConnection;

/// A connection that receives the RandR events of the root window.
pub struct Events {
    conn: RustConnection,
}

impl Events {
    /// Connects to `display` (`None`: `$DISPLAY`) and asks for screen, output and output property
    /// changes. The property mask catches an EDID that goes while the port stays connected.
    pub fn listen(display: Option<&str>) -> Result<Self> {
        let (conn, screen) = x11rb::connect(display).context("cannot connect to the X server")?;
        // As xrandr and i3 do: the server sends 1.2+ events only to a client that said its version.
        conn.randr_query_version(1, 5)?
            .reply()
            .context("the X server has no RandR extension")?;
        let root = conn.setup().roots[screen].root;
        let mask =
            NotifyMask::SCREEN_CHANGE | NotifyMask::OUTPUT_CHANGE | NotifyMask::OUTPUT_PROPERTY;
        conn.randr_select_input(root, mask)?
            .check()
            .context("cannot listen to RandR events")?;
        Ok(Self { conn })
    }

    /// Waits up to `timeout` for events and takes them all. `true` when one came. A signal ends
    /// the wait with `false`; a lost connection is an error.
    pub fn wait(&self, timeout: Duration) -> Result<bool> {
        // x11rb may already hold events that the socket no longer shows.
        if self.drain()? {
            return Ok(true);
        }
        let timeout = Timespec::try_from(timeout).context("timeout too long")?;
        let fd = self.conn.stream();
        match poll(&mut [PollFd::new(fd, PollFlags::IN)], Some(&timeout)) {
            Ok(0) | Err(Errno::INTR) => Ok(false),
            Ok(_) => self.drain(),
            Err(err) => Err(err).context("cannot wait for X events"),
        }
    }

    /// Takes every queued event; the contents do not matter, only that the displays changed.
    fn drain(&self) -> Result<bool> {
        let mut any = false;
        while self
            .conn
            .poll_for_event()
            .context("lost the connection to the X server")?
            .is_some()
        {
            any = true;
        }
        Ok(any)
    }
}
