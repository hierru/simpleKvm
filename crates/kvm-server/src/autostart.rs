//! Windows 시작(로그온) 시 자동 실행 — 작업 스케줄러 방식.
//!
//! 서버는 관리자 권한을 요구하므로(build.rs 매니페스트) 레지스트리 Run 항목으로는
//! 시작할 수 없다(Windows가 상승 필요한 앱의 Run 항목을 차단). 대신 "로그온 시,
//! 최고 권한"으로 등록한 예약 작업은 UAC 확인창 없이 자동 실행된다.
//! 등록되는 명령은 이 실행 파일에 `--minimized`를 붙인 것(트레이로 조용히 시작).
//!
//! 예전 버전이 남긴 HKCU\...\Run 항목은 설정 변경 시 함께 제거한다.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
};

use crate::applog;

const TASK_NAME: &str = "simpleKvm Server";
const LEGACY_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const LEGACY_VALUE_NAME: &str = "simpleKvm";
/// 콘솔 창이 깜빡이지 않도록 (CREATE_NO_WINDOW).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default()
}

fn schtasks(args: &[&str]) -> std::io::Result<(bool, String)> {
    let out = Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text.trim().to_string()))
}

pub fn is_enabled() -> bool {
    matches!(schtasks(&["/Query", "/TN", TASK_NAME]), Ok((true, _)))
}

pub fn set(enabled: bool) -> std::io::Result<()> {
    remove_legacy_run_value();
    if enabled {
        let action = format!("\"{}\" --minimized", exe_path());
        let (ok, text) = schtasks(&[
            "/Create", "/F",
            "/TN", TASK_NAME,
            "/SC", "ONLOGON",
            "/RL", "HIGHEST",
            "/TR", &action,
        ])?;
        if ok {
            applog::write(format!("자동 실행 등록 (작업 스케줄러): {action}"));
            Ok(())
        } else {
            applog::write(format!("자동 실행 등록 실패: {text}"));
            Err(std::io::Error::other(format!(
                "작업 스케줄러 등록 실패 (관리자 권한 필요): {text}"
            )))
        }
    } else {
        let (ok, text) = schtasks(&["/Delete", "/F", "/TN", TASK_NAME])?;
        // 없는 작업을 지우는 것은 성공으로 본다.
        if ok || !is_enabled() {
            applog::write("자동 실행 해제 (작업 스케줄러)");
            Ok(())
        } else {
            applog::write(format!("자동 실행 해제 실패: {text}"));
            Err(std::io::Error::other(format!("작업 스케줄러 삭제 실패: {text}")))
        }
    }
}

/// 예전 버전의 레지스트리 Run 항목 제거 (없으면 아무 일도 하지 않음).
fn remove_legacy_run_value() {
    let sub: Vec<u16> = LEGACY_RUN_KEY.encode_utf16().chain(std::iter::once(0)).collect();
    let name: Vec<u16> = LEGACY_VALUE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), 0, KEY_SET_VALUE, &mut key).is_ok() {
            if RegDeleteValueW(key, PCWSTR(name.as_ptr())).is_ok() {
                applog::write("예전 레지스트리 Run 자동 실행 항목 제거");
            }
            let _ = RegCloseKey(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실제 작업 스케줄러에 등록했다 지운다. `/RL HIGHEST` 등록은 관리자 권한이
    /// 필요하므로 일반 권한에서는 "미검증"으로 건너뛴다.
    /// `cargo test -p kvm-server -- --ignored autostart`
    #[test]
    #[ignore = "실제 작업 스케줄러를 변경"]
    fn roundtrip() {
        let was_enabled = is_enabled();
        match set(true) {
            Ok(()) => {
                assert!(is_enabled(), "등록 후 조회 실패");
                set(false).unwrap();
                assert!(!is_enabled(), "해제 후에도 남아 있음");
                set(was_enabled).unwrap();
            }
            Err(e) => {
                eprintln!("미검증: 관리자 권한이 아니어서 등록 불가 — {e}");
            }
        }
    }
}
