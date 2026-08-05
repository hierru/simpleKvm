//! Background worker: runs the connect/handshake/inject loop off the UI
//! thread. The GUI starts it on "connect" and stops it by flipping `stop` and
//! shutting the socket, which unblocks the blocking read. Status is published
//! through a shared `Status` the UI polls each frame.

use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui;
use kvm_protocol::clipboard::ClipboardState;
use kvm_protocol::{read_message, write_message, DisplayRect, Message, Side, PROTOCOL_VERSION};

use crate::config::ClientConfig;
use crate::inject::Injector;

/// Poll the OS clipboard and forward local changes to the server. Exits when the
/// connection dies (`alive` cleared) or the worker stops.
fn clipboard_loop(
    clip: ClipboardState,
    writer: Arc<Mutex<TcpStream>>,
    alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    while alive.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
        sleep_stop(&stop, Duration::from_millis(600));
        if !alive.load(Ordering::SeqCst) {
            break;
        }
        if let Some(text) = clip.poll_local_change() {
            let mut w = writer.lock().unwrap();
            if write_message(&mut *w, &Message::Clipboard { text }).is_err() {
                break;
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Idle,
    Connecting,
    Connected,
}

pub struct Status {
    pub state: ConnState,
    pub server_name: Option<String>,
    pub displays: Vec<String>,
    /// The server's monitor arrangement and which side of it this Mac sits on,
    /// for the combined map.
    pub server_layout: Option<(Vec<DisplayRect>, Side)>,
    pub log: Vec<String>,
}

impl Status {
    fn new() -> Self {
        Status {
            state: ConnState::Idle,
            server_name: None,
            displays: Vec::new(),
            server_layout: None,
            log: Vec::new(),
        }
    }
}

pub struct Worker {
    stop: Arc<AtomicBool>,
    stream_slot: Arc<Mutex<Option<TcpStream>>>,
    pub status: Arc<Mutex<Status>>,
    join: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(cfg: ClientConfig, ctx: egui::Context) -> Worker {
        let stop = Arc::new(AtomicBool::new(false));
        let stream_slot = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(Status::new()));

        let (w_stop, w_slot, w_status) = (stop.clone(), stream_slot.clone(), status.clone());
        let join = std::thread::spawn(move || run(cfg, w_stop, w_slot, w_status, ctx));

        Worker { stop, stream_slot, status, join: Some(join) }
    }

    /// Signal the worker to stop, unblock its read, and join it.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(s) = self.stream_slot.lock().unwrap().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn push_log(s: &mut Status, line: String) {
    s.log.push(line);
    let len = s.log.len();
    if len > 200 {
        s.log.drain(0..len - 200);
    }
}

fn sleep_stop(stop: &AtomicBool, dur: Duration) {
    let step = Duration::from_millis(100);
    let mut left = dur;
    while left > Duration::ZERO && !stop.load(Ordering::SeqCst) {
        let s = step.min(left);
        std::thread::sleep(s);
        left = left.saturating_sub(s);
    }
}

fn run(
    cfg: ClientConfig,
    stop: Arc<AtomicBool>,
    slot: Arc<Mutex<Option<TcpStream>>>,
    status: Arc<Mutex<Status>>,
    ctx: egui::Context,
) {
    let addr = format!("{}:{}", cfg.server, cfg.port);

    macro_rules! update {
        ($body:expr) => {{
            {
                let mut s = status.lock().unwrap();
                let s: &mut Status = &mut s;
                $body(s);
            }
            ctx.request_repaint();
        }};
    }

    while !stop.load(Ordering::SeqCst) {
        update!(|s: &mut Status| {
            s.state = ConnState::Connecting;
            s.server_name = None;
            push_log(s, format!("connecting to {addr} ..."));
        });

        let mut stream = match TcpStream::connect(&addr) {
            Ok(s) => s,
            Err(e) => {
                update!(|s: &mut Status| push_log(s, format!("connect failed: {e}; retrying in 3s")));
                sleep_stop(&stop, Duration::from_secs(3));
                continue;
            }
        };
        let _ = stream.set_nodelay(true);

        if let Err(e) = write_message(
            &mut stream,
            &Message::Hello { version: PROTOCOL_VERSION, name: cfg.name.clone() },
        ) {
            update!(|s: &mut Status| push_log(s, format!("handshake write failed: {e}")));
            sleep_stop(&stop, Duration::from_secs(3));
            continue;
        }

        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        match read_message(&mut stream) {
            Ok(Message::HelloAck { version, name }) if version == PROTOCOL_VERSION => {
                update!(|s: &mut Status| {
                    s.state = ConnState::Connected;
                    s.server_name = Some(name.clone());
                    push_log(s, format!("connected to '{name}' (protocol v{version})"));
                });
            }
            Ok(other) => {
                update!(|s: &mut Status| push_log(s, format!("unexpected handshake reply: {other:?}")));
                sleep_stop(&stop, Duration::from_secs(3));
                continue;
            }
            Err(e) => {
                update!(|s: &mut Status| push_log(s, format!("handshake read failed: {e}")));
                sleep_stop(&stop, Duration::from_secs(3));
                continue;
            }
        }

        // Share our display arrangement so the server can draw the combined map.
        let displays: Vec<DisplayRect> = crate::inject::ui_display_rects()
            .into_iter()
            .map(|(x, y, w, h)| DisplayRect { x, y, w, h })
            .collect();
        let _ = write_message(&mut stream, &Message::ClientLayout { displays });

        // Blocking reads from here; stop() unblocks us via socket shutdown.
        let _ = stream.set_read_timeout(None);
        if let Ok(clone) = stream.try_clone() {
            *slot.lock().unwrap() = Some(clone);
        }

        // Shared write half so the read loop (ReturnToServer) and the clipboard
        // thread (Clipboard) never interleave partial frames on the socket.
        let writer = match stream.try_clone() {
            Ok(w) => Arc::new(Mutex::new(w)),
            Err(e) => {
                update!(|s: &mut Status| push_log(s, format!("stream clone failed: {e}")));
                sleep_stop(&stop, Duration::from_secs(1));
                continue;
            }
        };

        let mut injector = Injector::new(cfg.speed, cfg.ctrl_as_cmd);
        let displays = injector.display_lines();
        update!(|s: &mut Status| s.displays = displays.clone());

        // Injection is silently ignored without the Accessibility permission.
        if !crate::permission::is_trusted() {
            update!(|s: &mut Status| push_log(
                s,
                "경고: 손쉬운 사용 권한 없음 — 입력이 주입되지 않습니다".to_string()
            ));
        }

        // Clipboard sync: a poller pushes local changes; incoming Clipboard
        // messages are applied in the read loop below.
        let clip = ClipboardState::primed();
        let alive = Arc::new(AtomicBool::new(true));
        let clip_thread = {
            let (clip, writer, alive, stop) =
                (clip.clone(), writer.clone(), alive.clone(), stop.clone());
            std::thread::spawn(move || clipboard_loop(clip, writer, alive, stop))
        };

        loop {
            match read_message(&mut stream) {
                Ok(msg) => {
                    match &msg {
                        Message::Enter { .. } => {
                            update!(|s: &mut Status| push_log(s, "→ 제어 이동 받음 (Enter)".to_string()))
                        }
                        Message::Leave => {
                            update!(|s: &mut Status| push_log(s, "← 제어 회수 (Leave)".to_string()))
                        }
                        Message::Clipboard { text } => {
                            let text = text.clone();
                            update!(|s: &mut Status| push_log(s, "클립보드 수신".to_string()));
                            clip.apply_remote(text);
                            continue;
                        }
                        Message::ServerLayout { monitors, mac_side } => {
                            let layout = (monitors.clone(), *mac_side);
                            update!(|s: &mut Status| s.server_layout = Some(layout.clone()));
                            continue;
                        }
                        _ => {}
                    }
                    if let Some(out) = injector.handle(msg) {
                        let mut w = writer.lock().unwrap();
                        if write_message(&mut *w, &out).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    if !stop.load(Ordering::SeqCst) {
                        update!(|s: &mut Status| push_log(s, format!("connection lost: {e}")));
                    }
                    break;
                }
            }
        }

        alive.store(false, Ordering::SeqCst);
        let _ = clip_thread.join();
        injector.release_everything();
        *slot.lock().unwrap() = None;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        update!(|s: &mut Status| {
            s.state = ConnState::Connecting;
            s.server_layout = None;
        });
        sleep_stop(&stop, Duration::from_secs(1));
    }

    update!(|s: &mut Status| {
        s.state = ConnState::Idle;
        s.server_name = None;
        s.displays.clear();
        s.server_layout = None;
        push_log(s, "disconnected".to_string());
    });
}
