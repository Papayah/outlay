//! A fake i3/sway IPC socket for the focus tests. It knows one focused container, answers
//! `get_tree` with it and `[con_id=N] focus` by moving the focus, as i3 does. Every message goes
//! into a log file that hooks can write to as well, so a test sees what came in which order.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use outlay::wm::Wm;

/// A container that no longer exists: focusing it fails, as for a closed window.
pub const GONE: u64 = 404;

pub struct FakeWm {
    dir: PathBuf,
    socket: PathBuf,
    log: PathBuf,
    focused: Arc<Mutex<u64>>,
}

impl FakeWm {
    /// Listens in a fresh scratch directory, with `focused` holding the focus.
    pub fn start(name: &str, focused: u64) -> Self {
        let dir = std::env::temp_dir().join(format!("outlay-wm-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("ipc.sock");
        let log = dir.join("log");
        std::fs::write(&log, "").unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        let focused = Arc::new(Mutex::new(focused));
        let (shared, path) = (Arc::clone(&focused), log.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                serve(stream, &shared, &path);
            }
        });
        FakeWm {
            dir,
            socket,
            log,
            focused,
        }
    }

    pub fn wm(&self) -> Wm {
        Wm::new(&self.socket)
    }

    /// The user (or a changed screen) puts the focus on `id`.
    pub fn set_focus(&self, id: u64) {
        *self.focused.lock().unwrap() = id;
    }

    /// Moves the focus to `id` when called, as a changed screen can: the backend calls it on
    /// every apply.
    pub fn mover(&self, id: u64) -> impl Fn() + Send + 'static {
        let focused = Arc::clone(&self.focused);
        move || *focused.lock().unwrap() = id
    }

    pub fn focused(&self) -> u64 {
        *self.focused.lock().unwrap()
    }

    /// `get_tree`, or the command text, per message; `hook` where the hook below ran.
    pub fn log(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// A `post_apply` hook that writes `hook` into the log.
    pub fn hook(&self) -> String {
        format!("echo hook >> '{}'", self.log.display())
    }
}

impl Drop for FakeWm {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Answers messages on one connection until the client closes it.
fn serve(mut stream: UnixStream, focused: &Mutex<u64>, log: &PathBuf) {
    loop {
        let mut header = [0; 14];
        if stream.read_exact(&mut header).is_err() {
            return;
        }
        assert_eq!(&header[..6], b"i3-ipc");
        let word = |at: usize| u32::from_ne_bytes(header[at..at + 4].try_into().unwrap());
        let (length, kind) = (word(6) as usize, word(10));
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).unwrap();
        let payload = String::from_utf8(payload).unwrap();
        let reply = match kind {
            4 => {
                append(log, "get_tree");
                let id = *focused.lock().unwrap();
                format!(
                    r#"{{"id": 1, "focused": false, "nodes": [{{"id": 2, "focused": false,
                    "nodes": [], "floating_nodes": [{{"id": {id}, "focused": true}}]}}]}}"#
                )
            }
            0 => {
                append(log, &payload);
                let id = payload
                    .strip_prefix("[con_id=")
                    .and_then(|rest| rest.strip_suffix("] focus"))
                    .and_then(|id| id.parse::<u64>().ok());
                match id {
                    Some(id) if id != GONE => {
                        *focused.lock().unwrap() = id;
                        r#"[{"success": true}]"#.to_owned()
                    }
                    _ => r#"[{"success": false, "error": "No matching node."}]"#.to_owned(),
                }
            }
            other => panic!("unexpected message type {other}"),
        };
        let mut message = b"i3-ipc".to_vec();
        message.extend_from_slice(&u32::try_from(reply.len()).unwrap().to_ne_bytes());
        message.extend_from_slice(&kind.to_ne_bytes());
        message.extend_from_slice(reply.as_bytes());
        stream.write_all(&message).unwrap();
    }
}

fn append(log: &PathBuf, line: &str) {
    let mut file = std::fs::OpenOptions::new().append(true).open(log).unwrap();
    writeln!(file, "{line}").unwrap();
}
