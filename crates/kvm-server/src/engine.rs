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
