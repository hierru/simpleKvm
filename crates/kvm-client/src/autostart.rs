//! Launch-at-login via a per-user LaunchAgent
//! (~/Library/LaunchAgents/com.simplekvm.client.plist).
//!
//! We only write/remove the plist; we deliberately do NOT `launchctl load` on
//! enable (a `RunAtLoad` job starts immediately, spawning a duplicate of the
//! already-running app) nor `unload` on disable (that would kill the running
//! instance if launchd started it at login). The toggle takes effect next login.

use std::path::PathBuf;

const LABEL: &str = "com.simplekvm.client";

fn plist_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push("Library/LaunchAgents");
    p.push(format!("{LABEL}.plist"));
    Some(p)
}

/// The executable to relaunch at login (this app's own binary).
fn program() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default()
}

pub fn is_enabled() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn enable() -> std::io::Result<()> {
    let Some(path) = plist_path() else {
        return Err(std::io::Error::other("no HOME"));
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{prog}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        prog = program()
    );
    std::fs::write(&path, plist)
}

pub fn disable() -> std::io::Result<()> {
    let Some(path) = plist_path() else {
        return Ok(());
    };
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn set(enabled: bool) -> std::io::Result<()> {
    if enabled {
        enable()
    } else {
        disable()
    }
}
