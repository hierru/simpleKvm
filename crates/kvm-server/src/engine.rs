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
    mdns: Option<mdns_sd::ServiceDaemon>,
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
            let (port, name, mac_side, status, stop, clip) =
                (cfg.port, cfg.name.clone(), cfg.mac_side, status.clone(), stop.clone(), clip);
            std::thread::spawn(move || net::run(port, rx, name, mac_side, status, stop, clip))
        };

        // Low-level hooks must be installed and pumped on their own thread.
        let hook_thread = std::thread::spawn(|| {
            let _ = hooks::install_and_run();
        });

        let mdns = advertise_mdns(cfg, &status);

        Engine {
            stop,
            hook_thread: Some(hook_thread),
            net_thread: Some(net_thread),
            mdns,
            status,
        }
    }

    pub fn stop(&mut self) {
        if let Some(m) = self.mdns.take() {
            let _ = m.shutdown();
        }
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

/// Advertise `_simplekvm._tcp` on the LAN so the client can discover this
/// server without typing an IP. Best-effort: returns None (with a log line)
/// if mDNS can't come up — manual IP entry still works.
fn advertise_mdns(
    cfg: &ServerConfig,
    status: &Arc<Mutex<ServerStatus>>,
) -> Option<mdns_sd::ServiceDaemon> {
    let log = |line: String| status.lock().unwrap().log.push(line);
    let daemon = match mdns_sd::ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            log(format!("mDNS 시작 실패 (수동 IP 입력은 계속 가능): {e}"));
            return None;
        }
    };
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "simplekvm".into());
    let host_name = format!("{}.local.", host.to_lowercase());
    let info = match mdns_sd::ServiceInfo::new(
        kvm_protocol::MDNS_SERVICE_TYPE,
        &cfg.name,
        &host_name,
        "",
        cfg.port,
        None::<std::collections::HashMap<String, String>>,
    ) {
        Ok(i) => i.enable_addr_auto(),
        Err(e) => {
            log(format!("mDNS 서비스 정보 생성 실패: {e}"));
            return None;
        }
    };
    match daemon.register(info) {
        Ok(()) => {
            log(format!("mDNS 광고 시작: {} ({host_name})", cfg.name));
            Some(daemon)
        }
        Err(e) => {
            log(format!("mDNS 등록 실패: {e}"));
            None
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

    /// The engine must be discoverable over mDNS while running.
    #[test]
    #[ignore = "installs real hooks and uses the real network stack"]
    fn engine_mdns_discovery() {
        let cfg = ServerConfig {
            port: 24912,
            mac_side: Side::Left,
            name: "mdns-test-server".into(),
        };
        let mut engine = Engine::start(&cfg);

        let daemon = mdns_sd::ServiceDaemon::new().expect("browse daemon");
        let rx = daemon.browse(kvm_protocol::MDNS_SERVICE_TYPE).expect("browse");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                    if info.get_fullname().starts_with("mdns-test-server.")
                        && info.get_port() == cfg.port
                        && !info.get_addresses().is_empty()
                    {
                        found = true;
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }
        let _ = daemon.shutdown();
        engine.stop();
        assert!(found, "advertised service was not discovered within 10s");
    }
}
