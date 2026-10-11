//! Append-only diagnostic log at %APPDATA%\simpleKvm\server.log, so tray and
//! window problems can be traced after the fact — release builds have no
//! console. Kept small on purpose: timestamped lines, bounded file size.

use std::io::Write;
use std::path::PathBuf;

const MAX_BYTES: u64 = 2_000_000;

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    let mut p = PathBuf::from(base);
    p.push("simpleKvm");
    std::fs::create_dir_all(&p).ok()?;
    p.push("server.log");
    Some(p)
}

fn timestamp() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

pub fn write(line: impl AsRef<str>) {
    let Some(p) = path() else {
        return;
    };
    // Roll over past the size cap so the log can't grow without bound.
    if std::fs::metadata(&p).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {}", timestamp(), line.as_ref());
    }
}
