//! Low-level mouse/keyboard hooks and remote-mode state.
//!
//! Both hook callbacks run on the thread that installed them (the main
//! thread's message loop). While `REMOTE` is set, every physical input event
//! is swallowed locally and forwarded over the channel to the network thread.
//!
//! Mouse capture strategy: on entering remote mode the cursor is parked at the
//! center of the virtual screen; each subsequent move event yields a delta
//! from that center and the cursor is snapped back. Events injected by our own
//! `SetCursorPos` carry `LLMHF_INJECTED` and are passed through untouched.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use kvm_protocol::{Message, MouseButton, Side};
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_CONTROL, VK_F12, VK_LCONTROL, VK_LMENU, VK_MENU, VK_RCONTROL, VK_RMENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, GetSystemMetrics, SetCursorPos,
    SetWindowsHookExW, TranslateMessage, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_XBUTTONDOWN, WM_XBUTTONUP,
};

struct Shared {
    tx: Sender<Message>,
    mac_side: Side,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// True while a handshaken client is attached.
pub static CONNECTED: AtomicBool = AtomicBool::new(false);
/// True while input is being forwarded to the client.
pub static REMOTE: AtomicBool = AtomicBool::new(false);

static CENTER_X: AtomicI32 = AtomicI32::new(0);
static CENTER_Y: AtomicI32 = AtomicI32::new(0);

// Modifier tracking for the Ctrl+Alt+F12 release hotkey while remote.
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);
static ALT_DOWN: AtomicBool = AtomicBool::new(false);

pub fn init(tx: Sender<Message>, mac_side: Side) {
    if SHARED.set(Shared { tx, mac_side }).is_err() {
        panic!("hooks::init called twice");
    }
}

fn send(msg: Message) {
    if let Some(s) = SHARED.get() {
        let _ = s.tx.send(msg);
    }
}

#[derive(Clone, Copy)]
struct VirtScreen {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn virt_screen() -> VirtScreen {
    unsafe {
        VirtScreen {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            w: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            h: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        }
    }
}

fn monitor_rects() -> Vec<RECT> {
    unsafe extern "system" fn cb(_mon: HMONITOR, _hdc: HDC, rc: *mut RECT, lp: LPARAM) -> BOOL {
        let rects = unsafe { &mut *(lp.0 as *mut Vec<RECT>) };
        rects.push(unsafe { *rc });
        BOOL(1)
    }
    let mut rects: Vec<RECT> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(cb),
            LPARAM(&mut rects as *mut _ as isize),
        );
    }
    rects
}

/// The monitor rect under `pt` (nearest if `pt` is off-screen).
fn monitor_at(pt: POINT) -> RECT {
    unsafe {
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            info.rcMonitor
        } else {
            let v = virt_screen();
            RECT { left: v.x, top: v.y, right: v.x + v.w, bottom: v.y + v.h }
        }
    }
}

/// Vertical span (top, bottom) covered by the monitor(s) that own the shared
/// edge. `y_ratio` on the wire is always relative to this span, so positions
/// map correctly even when the monitors have different resolutions/offsets.
fn edge_span(side: Side) -> (i32, i32) {
    let v = virt_screen();
    let edge_x = match side {
        Side::Left => v.x,
        Side::Right => v.x + v.w,
    };
    let mut top = i32::MAX;
    let mut bottom = i32::MIN;
    for rc in monitor_rects() {
        let owns_edge = match side {
            Side::Left => rc.left == edge_x,
            Side::Right => rc.right == edge_x,
        };
        if owns_edge {
            top = top.min(rc.top);
            bottom = bottom.max(rc.bottom);
        }
    }
    if top >= bottom {
        (v.y, v.y + v.h)
    } else {
        (top, bottom)
    }
}

/// Diagnostic for `--list-monitors`: prints the detected arrangement and
/// where the shared edge lands.
pub fn print_layout(mac_side: Side) {
    let v = virt_screen();
    println!("virtual screen: {}x{} at ({}, {})", v.w, v.h, v.x, v.y);
    for (i, rc) in monitor_rects().iter().enumerate() {
        println!(
            "monitor {}: {}x{} at ({}, {})",
            i,
            rc.right - rc.left,
            rc.bottom - rc.top,
            rc.left,
            rc.top
        );
    }
    let (top, bottom) = edge_span(mac_side);
    println!("mac-side edge ({mac_side:?}): y span {top}..{bottom} — crossing this edge switches to the Mac");
}

fn enter_remote(cursor: POINT) {
    let s = SHARED.get().unwrap();
    let (span_top, span_bottom) = edge_span(s.mac_side);
    let span_h = (span_bottom - span_top).max(1);
    let y_ratio = ((cursor.y - span_top) as f32 / span_h as f32).clamp(0.0, 1.0);

    // Park the cursor at the center of the monitor it is currently on; the
    // center of the virtual-screen bounding box can fall in a dead zone
    // between monitors, which would break delta extraction.
    let mon = monitor_at(cursor);
    let cx = (mon.left + mon.right) / 2;
    let cy = (mon.top + mon.bottom) / 2;
    CENTER_X.store(cx, Ordering::Relaxed);
    CENTER_Y.store(cy, Ordering::Relaxed);
    CTRL_DOWN.store(false, Ordering::Relaxed);
    ALT_DOWN.store(false, Ordering::Relaxed);

    REMOTE.store(true, Ordering::SeqCst);
    send(Message::Enter { edge: s.mac_side.opposite(), y_ratio });
    unsafe {
        let _ = SetCursorPos(cx, cy);
    }
    println!("-> control moved to the Mac");
}

/// Reclaim local control. `y_ratio` positions the cursor next to the shared
/// edge (used when the client reports the cursor coming back); `notify` sends
/// a `Leave` so the client can release anything it holds.
pub fn leave_remote(y_ratio: Option<f32>, notify: bool) {
    if !REMOTE.swap(false, Ordering::SeqCst) {
        return;
    }
    if notify {
        send(Message::Leave);
    }
    let s = SHARED.get().unwrap();
    let v = virt_screen();
    let (span_top, span_bottom) = edge_span(s.mac_side);
    let span_h = (span_bottom - span_top).max(1);
    let y = span_top + (y_ratio.unwrap_or(0.5).clamp(0.0, 1.0) * span_h as f32) as i32;
    let x = match s.mac_side {
        Side::Left => v.x + 5,
        Side::Right => v.x + v.w - 6,
    };
    unsafe {
        let _ = SetCursorPos(x, y.clamp(span_top, span_bottom - 1));
    }
    println!("<- control back on Windows");
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
    if info.flags & LLMHF_INJECTED != 0 {
        // Our own SetCursorPos re-centering (or other synthetic input).
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    let msg = wparam.0 as u32;

    if REMOTE.load(Ordering::SeqCst) {
        match msg {
            WM_MOUSEMOVE => {
                let cx = CENTER_X.load(Ordering::Relaxed);
                let cy = CENTER_Y.load(Ordering::Relaxed);
                let dx = info.pt.x - cx;
                let dy = info.pt.y - cy;
                if dx != 0 || dy != 0 {
                    send(Message::MouseMove { dx, dy });
                    let _ = SetCursorPos(cx, cy);
                }
            }
            WM_LBUTTONDOWN => send(Message::MouseButton { button: MouseButton::Left, pressed: true }),
            WM_LBUTTONUP => send(Message::MouseButton { button: MouseButton::Left, pressed: false }),
            WM_RBUTTONDOWN => send(Message::MouseButton { button: MouseButton::Right, pressed: true }),
            WM_RBUTTONUP => send(Message::MouseButton { button: MouseButton::Right, pressed: false }),
            WM_MBUTTONDOWN => send(Message::MouseButton { button: MouseButton::Middle, pressed: true }),
            WM_MBUTTONUP => send(Message::MouseButton { button: MouseButton::Middle, pressed: false }),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let button = if (info.mouseData >> 16) as u16 == 1 {
                    MouseButton::X1
                } else {
                    MouseButton::X2
                };
                send(Message::MouseButton { button, pressed: msg == WM_XBUTTONDOWN });
            }
            WM_MOUSEWHEEL => {
                let delta = ((info.mouseData >> 16) as u16) as i16 as i32;
                send(Message::Wheel { dx: 0, dy: delta });
            }
            WM_MOUSEHWHEEL => {
                let delta = ((info.mouseData >> 16) as u16) as i16 as i32;
                send(Message::Wheel { dx: delta, dy: 0 });
            }
            _ => {}
        }
        return LRESULT(1); // swallow locally
    }

    // Local mode: watch for the cursor touching the Mac-side edge.
    if msg == WM_MOUSEMOVE && CONNECTED.load(Ordering::SeqCst) {
        let s = SHARED.get().unwrap();
        let v = virt_screen();
        let hit = match s.mac_side {
            Side::Left => info.pt.x <= v.x,
            Side::Right => info.pt.x >= v.x + v.w - 1,
        };
        if hit {
            enter_remote(info.pt);
            return LRESULT(1);
        }
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }
    let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    if info.flags.0 & LLKHF_INJECTED.0 != 0 {
        return CallNextHookEx(HHOOK::default(), code, wparam, lparam);
    }

    if REMOTE.load(Ordering::SeqCst) {
        let msg = wparam.0 as u32;
        let pressed = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let vk = info.vkCode as u16;

        match vk {
            v if v == VK_LCONTROL.0 || v == VK_RCONTROL.0 || v == VK_CONTROL.0 => {
                CTRL_DOWN.store(pressed, Ordering::Relaxed)
            }
            v if v == VK_LMENU.0 || v == VK_RMENU.0 || v == VK_MENU.0 => {
                ALT_DOWN.store(pressed, Ordering::Relaxed)
            }
            _ => {}
        }

        if pressed
            && vk == VK_F12.0
            && CTRL_DOWN.load(Ordering::Relaxed)
            && ALT_DOWN.load(Ordering::Relaxed)
        {
            leave_remote(None, true);
            return LRESULT(1);
        }

        send(Message::Key { vk, pressed });
        return LRESULT(1);
    }
    CallNextHookEx(HHOOK::default(), code, wparam, lparam)
}

pub fn run_message_loop() {
    unsafe {
        let _mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), HINSTANCE::default(), 0)
            .expect("failed to install mouse hook");
        let _kbd_hook =
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), HINSTANCE::default(), 0)
                .expect("failed to install keyboard hook");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
