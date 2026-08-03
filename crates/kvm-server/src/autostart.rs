//! Run-at-login via the per-user registry key
//! HKCU\Software\Microsoft\Windows\CurrentVersion\Run (value "simpleKvm").
//!
//! The stored command launches this binary with `--minimized`, so an
//! autostarted server comes up quietly in the tray.

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "simpleKvm";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The command to relaunch at login (this binary, minimized to the tray).
fn command() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default();
    format!("\"{exe}\" --minimized")
}

fn open_run_key(sam: REG_SAM_FLAGS) -> Option<HKEY> {
    let sub = wide(RUN_KEY);
    let mut key = HKEY::default();
    let err = unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), 0, sam, &mut key)
    };
    err.is_ok().then_some(key)
}

pub fn is_enabled() -> bool {
    let Some(key) = open_run_key(KEY_QUERY_VALUE) else {
        return false;
    };
    let name = wide(VALUE_NAME);
    let err = unsafe { RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, None, None) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    err.is_ok()
}

pub fn set(enabled: bool) -> std::io::Result<()> {
    let Some(key) = open_run_key(KEY_SET_VALUE) else {
        return Err(std::io::Error::other("Run 레지스트리 키를 열 수 없습니다"));
    };
    let name = wide(VALUE_NAME);
    let result = if enabled {
        let data = wide(&command());
        // REG_SZ payload is the UTF-16 string bytes including the terminator.
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2)
        };
        let err = unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) };
        err.is_ok()
            .then_some(())
            .ok_or_else(|| std::io::Error::other(format!("레지스트리 쓰기 실패: {err:?}")))
    } else {
        let err = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
        // Deleting an absent value is fine.
        if err.is_ok() || err.0 == 2 {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("레지스트리 삭제 실패: {err:?}")))
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes to the real HKCU Run key and removes the value again.
    #[test]
    #[ignore = "touches the real registry; run explicitly"]
    fn roundtrip() {
        let was_enabled = is_enabled();
        set(true).unwrap();
        assert!(is_enabled());
        set(false).unwrap();
        assert!(!is_enabled());
        // Leave the machine as we found it.
        set(was_enabled).unwrap();
    }
}
