//! Which build is actually running.
//!
//! `bundle-mac.sh` overwrites `Contents/MacOS/kvm-client` in place. A process
//! that is already running keeps its original image, so a rebundle without a
//! relaunch leaves the old code running while the file on disk looks current —
//! new features appear to be missing with nothing in the UI to explain it.
//!
//! So we snapshot the executable's mtime at startup (that is the build time of
//! the *running* image) and re-stat it later: if it moved, a newer build is
//! installed and the app needs a relaunch.

use std::process::Command;
use std::time::SystemTime;

pub struct BuildStamp {
    /// mtime of our executable as it was when this process started.
    running: Option<SystemTime>,
    /// `running`, formatted for display.
    pub built_at: String,
}

impl BuildStamp {
    pub fn capture() -> BuildStamp {
        let running = exe_mtime();
        BuildStamp { running, built_at: format_time(running) }
    }

    /// The commit this image was compiled from (stamped by `build.rs`).
    pub fn commit() -> &'static str {
        env!("KVM_BUILD_COMMIT")
    }

    /// True once a newer binary has been written over the one we are running.
    pub fn superseded(&self) -> bool {
        match (self.running, exe_mtime()) {
            (Some(running), Some(on_disk)) => on_disk > running,
            _ => false,
        }
    }
}

fn exe_mtime() -> Option<SystemTime> {
    let path = std::env::current_exe().ok()?;
    std::fs::metadata(path).ok()?.modified().ok()
}

/// `date -r <epoch>` — avoids pulling in a date-formatting dependency for the
/// one string we need. Called twice per process, not per frame.
fn format_time(t: Option<SystemTime>) -> String {
    let Some(t) = t else { return "알 수 없음".into() };
    let Ok(epoch) = t.duration_since(SystemTime::UNIX_EPOCH) else {
        return "알 수 없음".into();
    };
    Command::new("date")
        .args(["-r", &epoch.as_secs().to_string(), "+%Y-%m-%d %H:%M"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "알 수 없음".into())
}
