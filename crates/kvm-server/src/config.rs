//! Persisted server settings (JSON at %APPDATA%\simpleKvm\server.json).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use kvm_protocol::{Side, DEFAULT_PORT};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// TCP port to listen on.
    pub port: u16,
    /// Which side of this Windows screen the Mac sits on.
    pub mac_side: Side,
    /// Name reported to the client during the handshake.
    pub name: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, mac_side: Side::Left, name: "windows-pc".to_string() }
    }
}

fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    let mut p = PathBuf::from(base);
    p.push("simpleKvm");
    Some(p)
}

impl ServerConfig {
    pub fn load() -> Self {
        let Some(file) = config_dir().map(|d| d.join("server.json")) else {
            return Self::default();
        };
        match std::fs::read_to_string(&file) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    // Used by the Windows GUI; absent from the non-Windows stub build.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn save(&self) -> std::io::Result<()> {
        let Some(dir) = config_dir() else {
            return Ok(());
        };
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(dir.join("server.json"), json)
    }
}
