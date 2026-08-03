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
use kvm_protocol::{read_message, write_message, Message, PROTOCOL_VERSION};

use crate::config::ClientConfig;
use crate::inject::Injector;

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
    pub log: Vec<String>,
}

impl Status {
    fn new() -> Self {
        Status { state: ConnState::Idle, server_name: None, displays: Vec::new(), log: Vec::new() }
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

        // Blocking reads from here; stop() unblocks us via socket shutdown.
        let _ = stream.set_read_timeout(None);
        if let Ok(clone) = stream.try_clone() {
            *slot.lock().unwrap() = Some(clone);
        }

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
                        _ => {}
                    }
                    if let Err(e) = injector.handle(msg, &mut stream) {
                        update!(|s: &mut Status| push_log(s, format!("send failed: {e}")));
                        break;
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

        injector.release_everything();
        *slot.lock().unwrap() = None;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        update!(|s: &mut Status| s.state = ConnState::Connecting);
        sleep_stop(&stop, Duration::from_secs(1));
    }

    update!(|s: &mut Status| {
        s.state = ConnState::Idle;
        s.server_name = None;
        s.displays.clear();
        push_log(s, "disconnected".to_string());
    });
}
