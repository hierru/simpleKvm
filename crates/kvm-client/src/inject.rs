//! Injects received input events into macOS via CGEvent, tracking a virtual
//! cursor position, button/modifier state, and click counts.
//!
//! Multi-display aware: the virtual cursor may roam across every active
//! display, movement into dead zones (gaps in the arrangement) is clamped to
//! the current display, and enter/return positions are mapped against the
//! display(s) that own the shared edge.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use kvm_protocol::{Message, MouseButton, Side};

use crate::keymap::{map_vk, Mapped, MappedKey};

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_SLOP: f64 = 6.0;

/// Private window-server API (same call Synergy/Barrier use): without the
/// `SetsCursorInBackground` connection property, cursor moves posted by a
/// background process relocate the pointer but the cursor is not redrawn —
/// it looks like the cursor vanished while we drive it.
mod cgs {
    use std::os::raw::{c_int, c_void};
    extern "C" {
        pub fn _CGSDefaultConnection() -> c_int;
        pub fn CGSSetConnectionProperty(
            cid: c_int,
            target_cid: c_int,
            key: *const c_void,   // CFStringRef
            value: *const c_void, // CFTypeRef
        ) -> c_int;
    }
}

fn enable_background_cursor_drawing() {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::string::CFString;

    let key = CFString::from_static_string("SetsCursorInBackground");
    let value = CFBoolean::true_value();
    unsafe {
        let cid = cgs::_CGSDefaultConnection();
        let _ = cgs::CGSSetConnectionProperty(
            cid,
            cid,
            key.as_concrete_TypeRef() as *const _,
            value.as_CFTypeRef(),
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.w
    }
    fn bottom(&self) -> f64 {
        self.y + self.h
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Bounds of every active display in global display coordinates
/// (origin at the main display's top-left, y grows downward — the same
/// coordinate space CGEvent uses).
fn active_display_rects() -> Vec<Rect> {
    let mut rects: Vec<Rect> = CGDisplay::active_displays()
        .unwrap_or_default()
        .into_iter()
        .map(|id| {
            let b = CGDisplay::new(id).bounds();
            Rect { x: b.origin.x, y: b.origin.y, w: b.size.width, h: b.size.height }
        })
        .collect();
    if rects.is_empty() {
        let b = CGDisplay::main().bounds();
        rects.push(Rect { x: b.origin.x, y: b.origin.y, w: b.size.width, h: b.size.height });
    }
    rects
}

pub struct Injector {
    source: CGEventSource,
    displays: Vec<Rect>,
    /// Global desktop extremes across all displays.
    min_x: f64,
    max_x: f64,
    /// Vertical span (top, bottom) of the display(s) owning the shared edge;
    /// `y_ratio` on the wire is relative to this span.
    edge_span: (f64, f64),
    pos: (f64, f64),
    in_control: bool,
    edge: Side,
    flags: CGEventFlags,
    left_down: bool,
    right_down: bool,
    other_down: bool,
    held_keys: HashSet<u16>,
    last_click: Option<(Instant, MouseButton, (f64, f64))>,
    click_count: i64,
    speed: f64,
    ctrl_as_cmd: bool,
}

impl Injector {
    pub fn new(speed: f64, ctrl_as_cmd: bool) -> Self {
        enable_background_cursor_drawing();
        let mut injector = Injector {
            source: CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                .expect("failed to create CGEventSource"),
            displays: Vec::new(),
            min_x: 0.0,
            max_x: 0.0,
            edge_span: (0.0, 0.0),
            pos: (0.0, 0.0),
            in_control: false,
            edge: Side::Right,
            flags: CGEventFlags::empty(),
            left_down: false,
            right_down: false,
            other_down: false,
            held_keys: HashSet::new(),
            last_click: None,
            click_count: 1,
            speed,
            ctrl_as_cmd,
        };
        injector.refresh_layout();
        for d in &injector.displays {
            println!(
                "display: {:.0}x{:.0} at ({:.0}, {:.0})",
                d.w, d.h, d.x, d.y
            );
        }
        injector
    }

    /// Human-readable summary of every active display, for the GUI status pane.
    pub fn display_lines(&self) -> Vec<String> {
        self.displays
            .iter()
            .map(|d| format!("{:.0}x{:.0} at ({:.0}, {:.0})", d.w, d.h, d.x, d.y))
            .collect()
    }

    /// Re-read the display arrangement (it can change while we run) and
    /// recompute the globals derived from it.
    fn refresh_layout(&mut self) {
        self.displays = active_display_rects();
        self.min_x = self.displays.iter().map(|d| d.x).fold(f64::MAX, f64::min);
        self.max_x = self.displays.iter().map(|d| d.right()).fold(f64::MIN, f64::max);

        let mut top = f64::MAX;
        let mut bottom = f64::MIN;
        for d in self.displays.iter().filter(|d| self.owns_edge(d)) {
            top = top.min(d.y);
            bottom = bottom.max(d.bottom());
        }
        self.edge_span = if top < bottom { (top, bottom) } else { (0.0, 0.0) };
    }

    fn owns_edge(&self, d: &Rect) -> bool {
        match self.edge {
            Side::Right => d.right() >= self.max_x - 0.5,
            Side::Left => d.x <= self.min_x + 0.5,
        }
    }

    fn display_at(&self, x: f64, y: f64) -> Rect {
        self.displays
            .iter()
            .copied()
            .find(|d| d.contains(x, y))
            .unwrap_or(self.displays[0])
    }

    /// Process one incoming message. Returns a message to send back to the
    /// server (currently only `ReturnToServer`), if any.
    pub fn handle(&mut self, msg: Message) -> Option<Message> {
        match msg {
            Message::Enter { edge, y_ratio } => {
                self.edge = edge;
                self.refresh_layout();

                let (top, bottom) = self.edge_span;
                let y = top + y_ratio as f64 * (bottom - top);
                // Land on the edge-owning display closest to that height.
                let target = self
                    .displays
                    .iter()
                    .copied()
                    .filter(|d| self.owns_edge(d))
                    .min_by(|a, b| {
                        let da = dist_to_range(y, a.y, a.bottom());
                        let db = dist_to_range(y, b.y, b.bottom());
                        da.partial_cmp(&db).unwrap()
                    })
                    .unwrap_or_else(|| self.displays[0]);
                let x = match edge {
                    Side::Right => target.right() - 2.0,
                    Side::Left => target.x + 1.0,
                };
                self.pos = (x, y.clamp(target.y, target.bottom() - 1.0));
                self.in_control = true;
                // The cursor may be in a hidden state (e.g. after typing);
                // make sure it is drawn where we just warped it.
                let _ = CGDisplay::main().show_cursor();
                eprintln!(
                    "enter: edge={:?} ratio={y_ratio:.3} span=({top:.0},{bottom:.0}) -> pos=({:.0},{:.0})",
                    edge, self.pos.0, self.pos.1
                );
                self.post_move();
            }
            Message::Leave => {
                self.in_control = false;
                self.release_everything();
            }
            Message::MouseMove { dx, dy } if self.in_control => {
                let nx = self.pos.0 + dx as f64 * self.speed;
                let ny = self.pos.1 + dy as f64 * self.speed;

                if self.displays.iter().any(|d| d.contains(nx, ny)) {
                    self.pos = (nx, ny);
                    self.post_move();
                } else {
                    let cur = self.display_at(self.pos.0, self.pos.1);
                    let crossed = match self.edge {
                        Side::Right => nx > self.max_x - 1.0 && self.owns_edge(&cur),
                        Side::Left => nx < self.min_x && self.owns_edge(&cur),
                    };
                    if crossed {
                        let (top, bottom) = self.edge_span;
                        let span_h = (bottom - top).max(1.0);
                        let y_ratio = ((ny - top) / span_h).clamp(0.0, 1.0) as f32;
                        self.in_control = false;
                        self.release_everything();
                        return Some(Message::ReturnToServer { y_ratio });
                    } else {
                        // Dead zone or outer edge: stay on the current display.
                        self.pos = (
                            nx.clamp(cur.x, cur.right() - 1.0),
                            ny.clamp(cur.y, cur.bottom() - 1.0),
                        );
                        self.post_move();
                    }
                }
            }
            Message::MouseButton { button, pressed } if self.in_control => {
                self.post_button(button, pressed);
            }
            Message::Wheel { dx, dy } if self.in_control => {
                // Windows: 120 per notch; macOS line units: ~3 lines per notch.
                // core-graphics has no scroll constructor, so build a blank
                // event and set the type/fields by hand.
                let lines_y = dy * 3 / 120;
                let lines_x = dx * 3 / 120;
                let lines_y = if lines_y != 0 { lines_y } else { dy.signum() };
                let lines_x = if lines_x != 0 { lines_x } else { dx.signum() };
                if let Ok(ev) = CGEvent::new(self.source.clone()) {
                    ev.set_type(CGEventType::ScrollWheel);
                    ev.set_integer_value_field(
                        EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1,
                        lines_y as i64,
                    );
                    ev.set_integer_value_field(
                        EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2,
                        lines_x as i64,
                    );
                    ev.set_flags(self.flags);
                    ev.post(CGEventTapLocation::HID);
                }
            }
            Message::Key { vk, pressed } if self.in_control => {
                self.post_key(vk, pressed);
            }
            Message::Heartbeat => {}
            _ => {}
        }
        None
    }

    fn current_move_type(&self) -> CGEventType {
        if self.left_down {
            CGEventType::LeftMouseDragged
        } else if self.right_down {
            CGEventType::RightMouseDragged
        } else if self.other_down {
            CGEventType::OtherMouseDragged
        } else {
            CGEventType::MouseMoved
        }
    }

    fn post_move(&self) {
        let ev = CGEvent::new_mouse_event(
            self.source.clone(),
            self.current_move_type(),
            CGPoint::new(self.pos.0, self.pos.1),
            CGMouseButton::Left,
        );
        if let Ok(ev) = ev {
            ev.set_flags(self.flags);
            ev.post(CGEventTapLocation::HID);
        }
    }

    fn post_button(&mut self, button: MouseButton, pressed: bool) {
        let (cg_button, down_type, up_type, button_number) = match button {
            MouseButton::Left => (
                CGMouseButton::Left,
                CGEventType::LeftMouseDown,
                CGEventType::LeftMouseUp,
                0,
            ),
            MouseButton::Right => (
                CGMouseButton::Right,
                CGEventType::RightMouseDown,
                CGEventType::RightMouseUp,
                1,
            ),
            MouseButton::Middle => (
                CGMouseButton::Center,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
                2,
            ),
            MouseButton::X1 => (
                CGMouseButton::Center,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
                3,
            ),
            MouseButton::X2 => (
                CGMouseButton::Center,
                CGEventType::OtherMouseDown,
                CGEventType::OtherMouseUp,
                4,
            ),
        };

        if pressed {
            let now = Instant::now();
            self.click_count = match self.last_click {
                Some((t, b, p))
                    if b == button
                        && now.duration_since(t) < DOUBLE_CLICK_WINDOW
                        && (p.0 - self.pos.0).abs() < DOUBLE_CLICK_SLOP
                        && (p.1 - self.pos.1).abs() < DOUBLE_CLICK_SLOP =>
                {
                    self.click_count + 1
                }
                _ => 1,
            };
            self.last_click = Some((now, button, self.pos));
        }

        match button {
            MouseButton::Left => self.left_down = pressed,
            MouseButton::Right => self.right_down = pressed,
            _ => self.other_down = pressed,
        }

        let ev_type = if pressed { down_type } else { up_type };
        if let Ok(ev) = CGEvent::new_mouse_event(
            self.source.clone(),
            ev_type,
            CGPoint::new(self.pos.0, self.pos.1),
            cg_button,
        ) {
            ev.set_flags(self.flags);
            ev.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, self.click_count);
            ev.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, button_number);
            ev.post(CGEventTapLocation::HID);
        }
    }

    fn post_key(&mut self, vk: u16, pressed: bool) {
        let Some(mapped) = map_vk(vk, self.ctrl_as_cmd) else {
            if pressed {
                eprintln!("unmapped Windows virtual-key: 0x{vk:02X}");
            }
            return;
        };
        let MappedKey { code, modifier } = match mapped {
            Mapped::Key(k) => k,
            Mapped::Combo { code, flags } => {
                // One-shot chord (e.g. 한/영 -> Ctrl+Space): press and release
                // with exactly the chord's flags, leaving tracked state alone.
                if pressed {
                    for down in [true, false] {
                        if let Ok(ev) =
                            CGEvent::new_keyboard_event(self.source.clone(), code, down)
                        {
                            ev.set_flags(flags);
                            ev.post(CGEventTapLocation::HID);
                        }
                    }
                }
                return;
            }
        };

        if let Some(flag) = modifier {
            if pressed {
                self.flags |= flag;
            } else {
                self.flags &= !flag;
            }
        }
        if pressed {
            self.held_keys.insert(code);
        } else {
            self.held_keys.remove(&code);
        }

        if let Ok(ev) = CGEvent::new_keyboard_event(self.source.clone(), code, pressed) {
            ev.set_flags(self.flags);
            ev.post(CGEventTapLocation::HID);
        }
    }

    /// Release any held keys/buttons so nothing sticks when control leaves
    /// this machine or the connection drops.
    pub fn release_everything(&mut self) {
        for code in std::mem::take(&mut self.held_keys) {
            if let Ok(ev) = CGEvent::new_keyboard_event(self.source.clone(), code, false) {
                ev.set_flags(CGEventFlags::empty());
                ev.post(CGEventTapLocation::HID);
            }
        }
        self.flags = CGEventFlags::empty();

        let point = CGPoint::new(self.pos.0, self.pos.1);
        if self.left_down {
            if let Ok(ev) = CGEvent::new_mouse_event(
                self.source.clone(),
                CGEventType::LeftMouseUp,
                point,
                CGMouseButton::Left,
            ) {
                ev.post(CGEventTapLocation::HID);
            }
            self.left_down = false;
        }
        if self.right_down {
            if let Ok(ev) = CGEvent::new_mouse_event(
                self.source.clone(),
                CGEventType::RightMouseUp,
                point,
                CGMouseButton::Right,
            ) {
                ev.post(CGEventTapLocation::HID);
            }
            self.right_down = false;
        }
        if self.other_down {
            if let Ok(ev) = CGEvent::new_mouse_event(
                self.source.clone(),
                CGEventType::OtherMouseUp,
                point,
                CGMouseButton::Center,
            ) {
                ev.post(CGEventTapLocation::HID);
            }
            self.other_down = false;
        }
    }
}

/// Human-readable dump of the detected display arrangement, for the settings
/// UI — lets the user compare against 시스템 설정 > 디스플레이 정렬.
pub fn layout_string() -> String {
    let rects = active_display_rects();
    let min_x = rects.iter().map(|d| d.x).fold(f64::MAX, f64::min);
    let max_x = rects.iter().map(|d| d.right()).fold(f64::MIN, f64::max);
    let mut s = String::new();
    let mut left_owners = Vec::new();
    let mut right_owners = Vec::new();
    for (i, d) in rects.iter().enumerate() {
        s.push_str(&format!(
            "디스플레이 {}: {:.0}x{:.0} @ ({:.0}, {:.0})\n",
            i, d.w, d.h, d.x, d.y
        ));
        if d.x <= min_x + 0.5 {
            left_owners.push(i.to_string());
        }
        if d.right() >= max_x - 0.5 {
            right_owners.push(i.to_string());
        }
    }
    s.push_str(&format!(
        "가로 범위: {min_x:.0} .. {max_x:.0}\n왼쪽 엣지 소유: 디스플레이 {}\n오른쪽 엣지 소유: 디스플레이 {}",
        left_owners.join(","),
        right_owners.join(",")
    ));
    s
}

/// Distance from `v` to the closed range [lo, hi] (0 when inside).
fn dist_to_range(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo - v
    } else if v > hi {
        v - hi
    } else {
        0.0
    }
}
