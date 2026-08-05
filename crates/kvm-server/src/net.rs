//! TCP listener: accepts one macOS client at a time, forwards captured input
//! events from the hook thread's channel, syncs the clipboard both ways, and
//! processes client messages (return-to-server, heartbeats). Stoppable and
//! status-reporting so the GUI can start/stop it.

use std::io::ErrorKind;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kvm_protocol::clipboard::ClipboardState;
use kvm_protocol::{read_message, write_message, DisplayRect, Message, Side, PROTOCOL_VERSION};

use crate::hooks::{self, CONNECTED, REMOTE};

pub struct ServerStatus {
    pub listening: bool,
    pub client: Option<String>,
    /// The connected client's display arrangement, for the combined map.
    pub client_layout: Option<Vec<DisplayRect>>,
    pub log: Vec<String>,
}

impl ServerStatus {
    pub fn new() -> Self {
        ServerStatus { listening: false, client: None, client_layout: None, log: Vec::new() }
    }
}

impl Default for ServerStatus {
    fn default() -> Self {
        Self::new()
    }
}

fn push_log(status: &Arc<Mutex<ServerStatus>>, line: String) {
    let mut s = status.lock().unwrap();
    s.log.push(line);
    let len = s.log.len();
    if len > 200 {
        s.log.drain(0..len - 200);
    }
}

pub fn run(
    port: u16,
    rx: Receiver<Message>,
    server_name: String,
    mac_side: Side,
    status: Arc<Mutex<ServerStatus>>,
    stop: Arc<AtomicBool>,
    clip: ClipboardState,
) {
    let listener = match TcpListener::bind(("0.0.0.0", port)) {
        Ok(l) => l,
        Err(e) => {
            push_log(
                &status,
                format!("bind {port} 실패: {e} — 다른 simpleKvm 서버가 이미 실행 중인지 확인하세요"),
            );
            status.lock().unwrap().listening = false;
            return;
        }
    };
    // Non-blocking accept so we can poll the stop flag while idle.
    let _ = listener.set_nonblocking(true);
    status.lock().unwrap().listening = true;
    push_log(&status, format!("0.0.0.0:{port} 에서 대기 중"));

    while !stop.load(Ordering::SeqCst) {
        let (mut stream, addr) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => {
                push_log(&status, format!("accept 실패: {e}"));
                continue;
            }
        };
        // Blocking I/O for this session; we stop it via shutdown + the stop flag.
        let _ = stream.set_nonblocking(false);

        let client_name = match handshake(&mut stream, &server_name) {
            Ok(name) => name,
            Err(e) => {
                push_log(&status, format!("{addr} 핸드셰이크 실패: {e}"));
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        push_log(&status, format!("클라이언트 연결됨: {client_name} ({addr})"));
        status.lock().unwrap().client = Some(client_name.clone());

        // Share our monitor arrangement so the client can draw the combined map.
        let monitors: Vec<DisplayRect> = hooks::ui_monitor_rects()
            .into_iter()
            .map(|(x, y, w, h)| DisplayRect { x, y, w, h })
            .collect();
        let _ = write_message(&mut stream, &Message::ServerLayout { monitors, mac_side });

        // Drop events that piled up while nobody was connected.
        while rx.try_recv().is_ok() {}
        CONNECTED.store(true, Ordering::SeqCst);

        let writer = match stream.try_clone() {
            Ok(w) => Arc::new(Mutex::new(w)),
            Err(e) => {
                push_log(&status, format!("stream clone 실패: {e}"));
                CONNECTED.store(false, Ordering::SeqCst);
                continue;
            }
        };

        // Reader: return-to-server + inbound clipboard + client layout.
        let reader = match stream.try_clone() {
            Ok(s) => {
                let clip = clip.clone();
                let status = status.clone();
                Some(std::thread::spawn(move || read_loop(s, clip, status)))
            }
            Err(e) => {
                push_log(&status, format!("reader clone 실패: {e}"));
                None
            }
        };

        // Clipboard poller: outbound local clipboard changes.
        let alive = Arc::new(AtomicBool::new(true));
        let clip_thread = {
            let (clip, writer, alive, stop) =
                (clip.clone(), writer.clone(), alive.clone(), stop.clone());
            std::thread::spawn(move || clipboard_loop(clip, writer, alive, stop))
        };

        // Forward captured events until the connection dies or we stop.
        loop {
            if !CONNECTED.load(Ordering::SeqCst) || stop.load(Ordering::SeqCst) {
                break;
            }
            let out = match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(msg) => msg,
                Err(RecvTimeoutError::Timeout) => Message::Heartbeat,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let mut w = writer.lock().unwrap();
            if write_message(&mut *w, &out).is_err() {
                break;
            }
        }

        CONNECTED.store(false, Ordering::SeqCst);
        alive.store(false, Ordering::SeqCst);
        if REMOTE.load(Ordering::SeqCst) {
            // Never leave the user stranded without a cursor on Windows.
            hooks::leave_remote(None, false);
        }
        let _ = stream.shutdown(Shutdown::Both);
        if let Some(r) = reader {
            let _ = r.join();
        }
        let _ = clip_thread.join();
        {
            let mut s = status.lock().unwrap();
            s.client = None;
            s.client_layout = None;
        }
        push_log(&status, format!("클라이언트 연결 해제: {client_name}"));
    }

    status.lock().unwrap().listening = false;
    push_log(&status, "중지됨".to_string());
}

fn handshake(stream: &mut TcpStream, server_name: &str) -> std::io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let name = match read_message(stream)? {
        Message::Hello { version, name } if version == PROTOCOL_VERSION => name,
        Message::Hello { version, .. } => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("protocol version mismatch: client {version}, server {PROTOCOL_VERSION}"),
            ))
        }
        other => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("expected Hello, got {other:?}"),
            ))
        }
    };
    write_message(
        stream,
        &Message::HelloAck { version: PROTOCOL_VERSION, name: server_name.to_string() },
    )?;
    // The client only talks when the cursor returns or the clipboard changes;
    // block indefinitely and rely on our outgoing heartbeats to detect death.
    stream.set_read_timeout(None)?;
    Ok(name)
}

fn read_loop(mut stream: TcpStream, clip: ClipboardState, status: Arc<Mutex<ServerStatus>>) {
    loop {
        match read_message(&mut stream) {
            Ok(Message::ReturnToServer { y_ratio }) => hooks::leave_remote(Some(y_ratio), false),
            Ok(Message::Clipboard { text }) => clip.apply_remote(text),
            Ok(Message::ClientLayout { displays }) => {
                status.lock().unwrap().client_layout = Some(displays);
            }
            Ok(_) => {}
            Err(_) => {
                CONNECTED.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
}

/// Poll the OS clipboard and forward local changes to the connected client.
fn clipboard_loop(
    clip: ClipboardState,
    writer: Arc<Mutex<TcpStream>>,
    alive: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    while alive.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
        for _ in 0..6 {
            if !alive.load(Ordering::SeqCst) || stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Some(text) = clip.poll_local_change() {
            let mut w = writer.lock().unwrap();
            if write_message(&mut *w, &Message::Clipboard { text }).is_err() {
                break;
            }
        }
    }
}
