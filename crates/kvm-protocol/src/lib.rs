//! Wire protocol shared by the Windows server and the macOS client.
//!
//! Framing: every message is a little-endian u32 length prefix followed by a
//! bincode-encoded [`Message`].

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::str::FromStr;

pub const PROTOCOL_VERSION: u32 = 2;
pub const DEFAULT_PORT: u16 = 24800;

/// Upper bound on a single frame; input events are tiny, this only guards
/// against garbage from a non-protocol peer.
const MAX_FRAME_LEN: u32 = 64 * 1024;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

impl FromStr for Side {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "left" => Ok(Side::Left),
            "right" => Ok(Side::Right),
            other => Err(format!("expected 'left' or 'right', got '{other}'")),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message {
    /// Client -> server, first message after connecting.
    Hello { version: u32, name: String },
    /// Server -> client, handshake reply.
    HelloAck { version: u32, name: String },

    /// Server -> client: the cursor crossed onto the client screen.
    /// `edge` is the client-screen edge the cursor appears on (and the edge
    /// that returns control to the server). `y_ratio` is 0.0 (top) ..= 1.0.
    Enter { edge: Side, y_ratio: f32 },
    /// Server -> client: server reclaimed control (hotkey, disconnect, ...).
    Leave,

    /// Relative mouse movement in device pixels.
    MouseMove { dx: i32, dy: i32 },
    MouseButton { button: MouseButton, pressed: bool },
    /// Wheel deltas in Windows units (one notch = 120). dy > 0 scrolls up.
    Wheel { dx: i32, dy: i32 },
    /// Windows virtual-key code; the client maps it to a macOS key code.
    Key { vk: u16, pressed: bool },

    /// Client -> server: the cursor hit the client's return edge.
    ReturnToServer { y_ratio: f32 },

    /// Either direction: the sender's clipboard text changed. Text-only for now.
    Clipboard { text: String },

    Heartbeat,
}

pub fn write_message(w: &mut impl Write, msg: &Message) -> io::Result<()> {
    let data = bincode::serialize(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(&data)?;
    w.flush()
}

pub fn read_message(r: &mut impl Read) -> io::Result<Message> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf);
    if len == 0 || len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid frame length {len}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    bincode::deserialize(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Clipboard sync helper shared by both ends. Suppresses echo by remembering the
/// last text seen, whether it came from the local OS or the remote peer.
#[cfg(feature = "clipboard")]
pub mod clipboard {
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    pub struct ClipboardState {
        last: Arc<Mutex<Option<String>>>,
    }

    impl ClipboardState {
        /// Seed `last` with the current OS clipboard so a freshly-connected peer
        /// isn't immediately sent (and doesn't swap) the pre-existing contents.
        pub fn primed() -> Self {
            let state = Self::default();
            if let Some(text) = read_os_clipboard() {
                *state.last.lock().unwrap() = Some(text);
            }
            state
        }

        /// Apply text received from the peer to the OS clipboard, recording it so
        /// the local poller won't echo it straight back.
        pub fn apply_remote(&self, text: String) {
            {
                let mut last = self.last.lock().unwrap();
                if last.as_deref() == Some(text.as_str()) {
                    return;
                }
                *last = Some(text.clone());
            }
            if let Ok(mut cb) = arboard::Clipboard::new() {
                let _ = cb.set_text(text);
            }
        }

        /// If the OS clipboard changed locally, return the new text to send to the
        /// peer (and record it). Returns None when unchanged or unreadable.
        pub fn poll_local_change(&self) -> Option<String> {
            let cur = read_os_clipboard()?;
            let mut last = self.last.lock().unwrap();
            if last.as_deref() == Some(cur.as_str()) {
                return None;
            }
            *last = Some(cur.clone());
            Some(cur)
        }
    }

    fn read_os_clipboard() -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let msgs = [
            Message::Hello { version: PROTOCOL_VERSION, name: "mac".into() },
            Message::Enter { edge: Side::Right, y_ratio: 0.42 },
            Message::MouseMove { dx: -3, dy: 17 },
            Message::Key { vk: 0x41, pressed: true },
            Message::ReturnToServer { y_ratio: 1.0 },
            Message::Clipboard { text: "hello 클립보드".into() },
        ];
        let mut buf = Vec::new();
        for m in &msgs {
            write_message(&mut buf, m).unwrap();
        }
        let mut cursor = std::io::Cursor::new(buf);
        for m in &msgs {
            let got = read_message(&mut cursor).unwrap();
            assert_eq!(format!("{m:?}"), format!("{got:?}"));
        }
    }
}
