//! Persisted client settings (JSON at
//! ~/Library/Application Support/simpleKvm/client.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use kvm_protocol::DEFAULT_PORT;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientConfig {
    /// Windows server address (IP or hostname).
    pub server: String,
    /// Server TCP port.
    pub port: u16,
    /// Mouse speed multiplier applied to incoming deltas.
    pub speed: f64,
    /// Map the Windows Ctrl key to macOS Command.
    pub ctrl_as_cmd: bool,
    /// Name reported to the server during the handshake.
    pub name: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            port: DEFAULT_PORT,
            speed: 1.0,
            ctrl_as_cmd: false,
            name: "mac".to_string(),
        }
    }
}

fn config_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push("Library/Application Support/simpleKvm");
    Some(p)
}

impl ClientConfig {
    pub fn load() -> Self {
        let Some(file) = config_dir().map(|d| d.join("client.json")) else {
            return Self::default();
        };
        match std::fs::read_to_string(&file) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    // Used by the macOS GUI; absent from the non-macOS stub build.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn save(&self) -> std::io::Result<()> {
        let Some(dir) = config_dir() else {
            return Ok(());
        };
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(dir.join("client.json"), json)
    }
}
