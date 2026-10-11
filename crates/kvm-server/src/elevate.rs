//! 관리자 권한 자기 상승.
//!
//! 관리자 권한(상위 무결성) 창이 활성일 때 일반 권한 프로세스의 저수준 입력 훅은
//! 그 입력을 받지 못한다(UIPI). 게임처럼 관리자 권한으로 뜨는 프로그램 위에서도
//! KVM이 동작하려면 서버 자체가 관리자 권한이어야 하므로, 일반 권한으로 시작되면
//! 같은 인자로 자신을 `runas`(UAC) 재실행하고 원래 프로세스는 종료한다.
//!
//! 링커 매니페스트(requireAdministrator) 대신 이 방식을 쓰는 이유: 매니페스트는
//! 테스트 실행 파일까지 상승을 요구해 `cargo test`가 돌지 않는다(os error 740).
//! 개발 중에는 `KVM_NO_ELEVATE=1`로 건너뛸 수 있다.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::applog;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut info = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut info as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && info.TokenIsElevated != 0
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 같은 실행 파일·같은 인자로 관리자 권한 재실행을 요청한다. 성공(UAC 허용)하면 true —
/// 호출자는 즉시 종료해야 한다. 사용자가 UAC를 거부하면 false.
pub fn relaunch_elevated() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let args: Vec<String> = std::env::args()
        .skip(1)
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a })
        .collect();
    let exe_w = wide(&exe.to_string_lossy());
    let args_w = wide(&args.join(" "));
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("runas"),
            PCWSTR(exe_w.as_ptr()),
            PCWSTR(args_w.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW는 32보다 큰 값이면 성공이다.
    let ok = (result.0 as isize) > 32;
    applog::write(format!(
        "관리자 권한 재실행 요청: {} (인자={:?})",
        if ok { "허용됨 → 원래 프로세스 종료" } else { "거부/실패 → 일반 권한으로 계속" },
        args
    ));
    ok
}
