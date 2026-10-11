//! 진단 샘플러: 엔진이 켜져 있는 동안 2초마다 다음을 관찰해 변화가 있을 때만
//! server.log에 기록한다.
//!
//! - 커서 클립 영역(GetClipCursor): 다른 프로그램이 커서를 특정 모니터/창에 가둔 경우
//! - 포그라운드 프로세스와 그 프로세스에 접근 가능한지(거부 = 관리자 권한·보호 프로세스)
//! - 훅 누락 의심: 커서는 움직였는데 훅이 받은 마우스 이벤트가 0건
//! - 엣지 도달했으나 전환 없음: 클라이언트 연결 중인데 Mac 쪽 엣지에서 원격 진입이 안 됨
//! - 원격 모드 중: 훅 수신·전송 건수와 SetCursorPos 실패 수 (매 샘플)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kvm_protocol::Side;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, POINT, RECT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClipCursor, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId,
};

use crate::applog;
use crate::hooks::{
    self, CONNECTED, KEY_EVENTS, MOUSE_EVENTS, REMOTE, SENT_MESSAGES, SETCURSOR_FAILS,
};

const INTERVAL: Duration = Duration::from_secs(2);
/// 같은 경고를 반복해서 찍지 않기 위한 최소 간격.
const REPEAT_GAP: Duration = Duration::from_secs(10);

/// 포그라운드 창의 (pid, 프로세스 이름 또는 접근 거부 표시).
fn foreground() -> (u32, String) {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid, process_name(pid))
    }
}

/// 프로세스 실행 파일 이름. 열 수 없으면 "접근거부" — 관리자 권한으로 떠 있거나
/// 안티치트 보호 프로세스라는 뜻이며, 그 창이 활성일 때 일반 권한 훅은 입력을 못 받는다.
fn process_name(pid: u32) -> String {
    if pid == 0 {
        return "(없음)".into();
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return "접근거부(관리자/보호 프로세스)".into();
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        match r {
            Ok(()) => {
                let full = String::from_utf16_lossy(&buf[..len as usize]);
                full.rsplit(['\\', '/']).next().unwrap_or(&full).to_string()
            }
            Err(_) => "접근거부(관리자/보호 프로세스)".into(),
        }
    }
}

fn clip_rect() -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClipCursor(&mut r);
    }
    r
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

fn same_rect(a: &RECT, b: &RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

fn sleep_unless_stopped(stop: &AtomicBool, dur: Duration) {
    let step = Duration::from_millis(100);
    let mut left = dur;
    while left > Duration::ZERO && !stop.load(Ordering::SeqCst) {
        let s = step.min(left);
        std::thread::sleep(s);
        left = left.saturating_sub(s);
    }
}

pub fn run(stop: Arc<AtomicBool>) {
    applog::write("진단 샘플러 시작 (2초 주기)");
    let mut last_clip: Option<RECT> = None;
    let mut last_fg: Option<(u32, String)> = None;
    let mut last_cursor = cursor();
    let mut last_remote = REMOTE.load(Ordering::SeqCst);
    let long_ago = Instant::now() - REPEAT_GAP;
    let mut last_blind_log = long_ago;
    let mut last_edge_log = long_ago;

    while !stop.load(Ordering::SeqCst) {
        sleep_unless_stopped(&stop, INTERVAL);
        if stop.load(Ordering::SeqCst) {
            break;
        }

        let v = hooks::virt_screen();
        let full = RECT { left: v.x, top: v.y, right: v.x + v.w, bottom: v.y + v.h };

        // 커서 클립 변화
        let clip = clip_rect();
        if last_clip.as_ref().map_or(true, |c| !same_rect(c, &clip)) {
            let note = if same_rect(&clip, &full) { "전체 화면 (클립 없음)" } else { "⚠ 일부 영역에 갇힘" };
            applog::write(format!(
                "커서 클립: ({},{})-({},{}) {note}",
                clip.left, clip.top, clip.right, clip.bottom
            ));
            last_clip = Some(clip);
        }

        // 포그라운드 변화
        let fg = foreground();
        if last_fg.as_ref() != Some(&fg) {
            applog::write(format!("포그라운드: pid={} {}", fg.0, fg.1));
            last_fg = Some(fg.clone());
        }

        // 지난 2초간 훅 통계
        let mouse = MOUSE_EVENTS.swap(0, Ordering::Relaxed);
        let keys = KEY_EVENTS.swap(0, Ordering::Relaxed);
        let sent = SENT_MESSAGES.swap(0, Ordering::Relaxed);
        let fails = SETCURSOR_FAILS.swap(0, Ordering::Relaxed);
        let remote = REMOTE.load(Ordering::SeqCst);
        let connected = CONNECTED.load(Ordering::SeqCst);
        let cur = cursor();
        let moved = cur.x != last_cursor.x || cur.y != last_cursor.y;
        let now = Instant::now();

        if remote != last_remote {
            applog::write(format!("원격 상태: {}", if remote { "Mac 제어 중" } else { "Windows 로컬" }));
            last_remote = remote;
        }

        if remote {
            // 원격 중에는 매 샘플 기록: 입력이 Mac에 안 먹을 때 어느 단계가 끊겼는지 본다.
            applog::write(format!(
                "원격 중: 훅 마우스={mouse} 키={keys} 전송={sent} SetCursorPos실패={fails} 커서=({},{}) 포그라운드={}",
                cur.x, cur.y, fg.1
            ));
        } else {
            if moved && mouse == 0 && now.duration_since(last_blind_log) >= REPEAT_GAP {
                applog::write(format!(
                    "⚠ 훅 누락 의심: 커서 ({},{})→({},{}) 이동했으나 훅 마우스 이벤트 0건 (연결={connected}, 포그라운드={})",
                    last_cursor.x, last_cursor.y, cur.x, cur.y, fg.1
                ));
                last_blind_log = now;
            }
            let at_edge = match hooks::mac_side() {
                Side::Left => cur.x <= v.x,
                Side::Right => cur.x >= v.x + v.w - 1,
            };
            if connected && at_edge && now.duration_since(last_edge_log) >= REPEAT_GAP {
                applog::write(format!(
                    "⚠ 엣지 도달했으나 전환 없음: 커서=({},{}) 훅 마우스={mouse} 포그라운드={}",
                    cur.x, cur.y, fg.1
                ));
                last_edge_log = now;
            }
        }
        last_cursor = cur;
    }
    applog::write("진단 샘플러 종료");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 샘플러가 돌면서 로그를 쓰는지 확인 (실제 %APPDATA% 로그 파일 사용).
    /// `cargo test -p kvm-server -- --ignored diag`
    #[test]
    #[ignore = "실제 로그 파일에 기록"]
    fn sampler_writes_log() {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let t = std::thread::spawn(move || run(s));
        std::thread::sleep(Duration::from_millis(4500));
        stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
        let p = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("simpleKvm/server.log");
        let log = std::fs::read_to_string(p).unwrap();
        assert!(log.contains("진단 샘플러 시작"));
        assert!(log.contains("커서 클립:"));
        assert!(log.contains("포그라운드:"));
        assert!(log.contains("진단 샘플러 종료"));
    }
}
