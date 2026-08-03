//! Starts/stops the capture engine: a hook thread (low-level hooks + message
//! loop) and a net thread (TCP + forwarding + clipboard), wired by an mpsc
//! channel. The GUI owns an `Engine` while running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use kvm_protocol::clipboard::ClipboardState;

use crate::config::ServerConfig;
use crate::hooks;
use crate::net::{self, ServerStatus};

pub struct Engine {
    stop: Arc<AtomicBool>,
    hook_thread: Option<JoinHandle<()>>,
    net_thread: Option<JoinHandle<()>>,
    pub status: Arc<Mutex<ServerStatus>>,
}

impl Engine {
    pub fn start(cfg: &ServerConfig) -> Engine {
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(ServerStatus::new()));
        let (tx, rx) = mpsc::channel();

        hooks::configure(tx, cfg.mac_side);
        let clip = ClipboardState::primed();

        let net_thread = {
            let (port, name, status, stop, clip) =
                (cfg.port, cfg.name.clone(), status.clone(), stop.clone(), clip);
            std::thread::spawn(move || net::run(port, rx, name, status, stop, clip))
        };

        // Low-level hooks must be installed and pumped on their own thread.
        let hook_thread = std::thread::spawn(|| {
            let _ = hooks::install_and_run();
        });

        Engine { stop, hook_thread: Some(hook_thread), net_thread: Some(net_thread), status }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        hooks::stop_message_loop();
        hooks::clear();
        if let Some(h) = self.hook_thread.take() {
            let _ = h.join();
        }
        if let Some(n) = self.net_thread.take() {
            let _ = n.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kvm_protocol::{read_message, write_message, Message, Side, PROTOCOL_VERSION};
    use std::net::TcpStream;
    use std::time::Duration;

    /// End-to-end check of the real engine: hooks + listener come up, a client
    /// handshakes, and the clipboard syncs in both directions.
    ///
    /// Ignored by default because it installs real input hooks and touches the
    /// OS clipboard (saved and restored). Run with:
    /// `cargo test -p kvm-server -- --ignored`
    #[test]
    #[ignore = "installs real hooks and touches the OS clipboard"]
    fn engine_handshake_and_clipboard_sync() {
        let cfg = ServerConfig {
            port: 24911,
            mac_side: Side::Left,
            name: "test-server".into(),
        };
        let mut engine = Engine::start(&cfg);

        // Wait for the listener to come up.
        let mut stream = None;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(s) = TcpStream::connect(("127.0.0.1", cfg.port)) {
                stream = Some(s);
                break;
            }
        }
        let mut stream = stream.expect("server did not start listening");

        // Handshake.
        write_message(
            &mut stream,
            &Message::Hello { version: PROTOCOL_VERSION, name: "fake-mac".into() },
        )
        .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        match read_message(&mut stream).unwrap() {
            Message::HelloAck { version, .. } => assert_eq!(version, PROTOCOL_VERSION),
            other => panic!("expected HelloAck, got {other:?}"),
        }

        let mut cb = arboard::Clipboard::new().unwrap();
        let saved = cb.get_text().ok();

        // Remote -> local: a Clipboard message from the client must land on
        // the OS clipboard.
        let inbound = "simpleKvm-test-remote-\u{d074}\u{b9bd}\u{bcf4}\u{b4dc}";
        write_message(&mut stream, &Message::Clipboard { text: inbound.into() }).unwrap();
        let mut applied = false;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(100));
            if cb.get_text().ok().as_deref() == Some(inbound) {
                applied = true;
                break;
            }
        }

        // Local -> remote: a local clipboard change must be sent to the client
        // by the poller (600ms interval).
        let outbound = "simpleKvm-test-local-change";
        cb.set_text(outbound).unwrap();
        let mut forwarded = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match read_message(&mut stream) {
                Ok(Message::Clipboard { text }) if text == outbound => {
                    forwarded = true;
                    break;
                }
                Ok(_) => {} // heartbeats etc.
                Err(_) => break,
            }
        }

        // Restore the user's clipboard before asserting.
        match saved {
            Some(t) => {
                let _ = cb.set_text(t);
            }
            None => {
                let _ = cb.clear();
            }
        }
        drop(stream);
        engine.stop();

        assert!(applied, "remote clipboard was not applied to the OS clipboard");
        assert!(forwarded, "local clipboard change was not forwarded to the client");
    }
}
