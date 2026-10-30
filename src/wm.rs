//! The window manager's keyboard focus, through the IPC protocol that i3 and sway share: what
//! has it, and giving it back. A change to the screens can move the focus (i3 and
//! sway move the workspaces of an output that goes off, and the window that ends up under the
//! pointer takes it), so the session puts it back after every change it makes. Then the answer
//! to "Keep this layout?" lands in outlay.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

const MAGIC: &[u8; 6] = b"i3-ipc";
const RUN_COMMAND: u32 = 0;
const GET_TREE: u32 = 4;
/// How long the window manager may take to answer. It answers at once; a stuck one must not
/// hold up the apply.
const TIMEOUT: Duration = Duration::from_millis(500);

/// What has the keyboard focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    /// A window, by its container id.
    Container(u64),
    /// A workspace with no window, by its name: sway's criteria do not match workspaces.
    Workspace(String),
}

impl Focus {
    /// The command that gives the focus back.
    fn command(&self) -> String {
        match self {
            Focus::Container(id) => format!("[con_id={id}] focus"),
            // A plain `workspace` command on the focused workspace would go back to the
            // previous one with workspace_auto_back_and_forth.
            Focus::Workspace(name) => format!(
                "workspace --no-auto-back-and-forth \"{}\"",
                name.replace('\\', "\\\\").replace('"', "\\\"")
            ),
        }
    }
}

/// An i3 or sway IPC socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wm {
    socket: PathBuf,
}

impl Wm {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    /// The window manager the environment names: `$SWAYSOCK`, else `$I3SOCK`. sway sets both;
    /// a `SWAYSOCK` that points at another sway (the headless one of the tests) wins over the
    /// `I3SOCK` of the session around it.
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        ["SWAYSOCK", "I3SOCK"]
            .into_iter()
            .find_map(|name| env(name).filter(|v| !v.is_empty()))
            .map(Self::new)
    }

    /// What has the keyboard focus: a window, or a workspace with none.
    pub fn focused(&self) -> Result<Focus> {
        let tree: Value = serde_json::from_slice(&self.request(GET_TREE, "")?)
            .context("the window manager's tree is not JSON")?;
        find_focused(&tree).context("nothing has the focus")
    }

    /// Gives the keyboard focus back. A window on a workspace that is not on any screen brings
    /// its workspace up.
    pub fn focus(&self, focus: &Focus) -> Result<()> {
        let reply: Value = serde_json::from_slice(&self.request(RUN_COMMAND, &focus.command())?)
            .context("the window manager's answer is not JSON")?;
        // One result per command: [{"success": false, "error": "…"}] when it failed.
        match reply.get(0) {
            Some(result) if result["success"] == true => Ok(()),
            Some(result) => bail!(
                "{}",
                result["error"]
                    .as_str()
                    .unwrap_or("the focus command failed")
            ),
            None => bail!("the window manager did not answer the focus command"),
        }
    }

    /// Sends one message on a fresh connection and returns the payload of the reply. Nothing
    /// subscribes to events, so the next message is the reply.
    fn request(&self, kind: u32, payload: &str) -> Result<Vec<u8>> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("could not connect to {}", self.socket.display()))?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        stream.write_all(&encode(kind, payload.as_bytes())?)?;
        let mut header = [0; 14];
        stream.read_exact(&mut header)?;
        let (length, reply) = decode_header(&header)?;
        if reply != kind {
            bail!("the window manager answered message {kind} with message {reply}");
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        Ok(body)
    }
}

/// `i3-ipc`, the payload's length and the message type (both in native byte order), the payload.
fn encode(kind: u32, payload: &[u8]) -> Result<Vec<u8>> {
    let length = u32::try_from(payload.len()).context("the message is too long")?;
    let mut message = Vec::with_capacity(14 + payload.len());
    message.extend_from_slice(MAGIC);
    message.extend_from_slice(&length.to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(payload);
    Ok(message)
}

/// The payload's length and the message type from a reply's header.
fn decode_header(header: &[u8; 14]) -> Result<(usize, u32)> {
    if &header[..6] != MAGIC {
        bail!("not an i3 IPC reply");
    }
    let word = |at: usize| {
        u32::from_ne_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Ok((usize::try_from(word(6))?, word(10)))
}

/// The focused node in a `get_tree` answer: tiled windows are in `nodes`, floating ones in
/// `floating_nodes`.
fn find_focused(node: &Value) -> Option<Focus> {
    if node["focused"] == true {
        return if node["type"] == "workspace" {
            node["name"]
                .as_str()
                .map(|name| Focus::Workspace(name.to_owned()))
        } else {
            node["id"].as_u64().map(Focus::Container)
        };
    }
    ["nodes", "floating_nodes"]
        .into_iter()
        .filter_map(|key| node[key].as_array())
        .flatten()
        .find_map(find_focused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sway_wins_over_i3() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(
            Wm::from_env(&env(&[("I3SOCK", "/run/i3")])),
            Some(Wm::new("/run/i3"))
        );
        assert_eq!(
            Wm::from_env(&env(&[("I3SOCK", "/run/i3"), ("SWAYSOCK", "/run/sway")])),
            Some(Wm::new("/run/sway"))
        );
        assert_eq!(
            Wm::from_env(&env(&[("I3SOCK", "/run/i3"), ("SWAYSOCK", "")])),
            Some(Wm::new("/run/i3")),
            "an empty variable is not set"
        );
        assert_eq!(Wm::from_env(&env(&[])), None);
    }

    #[test]
    fn messages_have_the_i3_header() {
        let message = encode(RUN_COMMAND, b"[con_id=7] focus").unwrap();
        assert_eq!(&message[..6], b"i3-ipc");
        let header: [u8; 14] = message[..14].try_into().unwrap();
        assert_eq!(decode_header(&header).unwrap(), (16, RUN_COMMAND));
        assert_eq!(&message[14..], b"[con_id=7] focus");

        let mut bad = header;
        bad[0] = b'x';
        assert!(decode_header(&bad).is_err());
    }

    #[test]
    fn the_focused_node_may_float() {
        let tree = json!({
            "id": 1, "type": "root", "focused": false, "nodes": [
                {"id": 2, "type": "output", "focused": false, "nodes": [
                    {"id": 3, "type": "workspace", "name": "1", "focused": false, "nodes": [],
                     "floating_nodes": [
                        {"id": 9, "type": "floating_con", "focused": true, "nodes": []}
                    ]},
                    {"id": 4, "type": "workspace", "name": "2", "focused": false}
                ]}
            ]
        });
        assert_eq!(find_focused(&tree), Some(Focus::Container(9)));
        assert_eq!(find_focused(&json!({"id": 1, "focused": false})), None);
        assert_eq!(
            find_focused(&json!({"id": 1, "nodes": [{"id": 5, "type": "con", "focused": true}]})),
            Some(Focus::Container(5))
        );
    }

    #[test]
    fn an_empty_workspace_is_focused_by_name() {
        let tree = json!({"id": 1, "focused": false, "nodes": [
            {"id": 4, "type": "workspace", "name": "2: web", "focused": true, "nodes": []}
        ]});
        assert_eq!(
            find_focused(&tree),
            Some(Focus::Workspace("2: web".to_owned()))
        );
        assert_eq!(
            Focus::Workspace(r#"3 "a\b""#.to_owned()).command(),
            r#"workspace --no-auto-back-and-forth "3 \"a\\b\"""#
        );
        assert_eq!(Focus::Container(7).command(), "[con_id=7] focus");
    }
}
