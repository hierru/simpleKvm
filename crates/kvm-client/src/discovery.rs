//! LAN auto-discovery of simpleKvm servers via mDNS/DNS-SD browsing.
//! Runs for the app's lifetime; the settings UI reads `servers` each frame.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use kvm_protocol::MDNS_SERVICE_TYPE;
use mdns_sd::{ServiceDaemon, ServiceEvent};

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredServer {
    /// Instance name the server advertises (its configured name).
    pub name: String,
    pub addr: String,
    pub port: u16,
    /// Full mDNS instance name — dedup/removal key.
    fullname: String,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    pub servers: Arc<Mutex<Vec<DiscoveredServer>>>,
    /// When we last received *any* mDNS event for our service type. Reported
    /// as-is in the diagnostics pane: it says traffic is arriving, nothing
    /// more. It is deliberately NOT used to judge the Local Network
    /// permission — a browse that returns nothing has been observed both with
    /// and without that permission, so it cannot tell the two apart.
    pub last_event: Arc<Mutex<Option<Instant>>>,
}

impl Discovery {
    pub fn start() -> Option<Discovery> {
        let daemon = ServiceDaemon::new().ok()?;
        let rx = daemon.browse(MDNS_SERVICE_TYPE).ok()?;
        let servers: Arc<Mutex<Vec<DiscoveredServer>>> = Arc::new(Mutex::new(Vec::new()));
        let last_event: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

        {
            let servers = servers.clone();
            let last_event = last_event.clone();
            // Exits when the daemon shuts down (recv starts failing).
            std::thread::spawn(move || {
                while let Ok(event) = rx.recv() {
                    // Only responses count. `SearchStarted` fires on every
                    // query round even when the LAN answers with nothing, so
                    // stamping it would make "last heard" always look fresh.
                    if matches!(
                        event,
                        ServiceEvent::ServiceFound(..)
                            | ServiceEvent::ServiceResolved(..)
                            | ServiceEvent::ServiceRemoved(..)
                    ) {
                        *last_event.lock().unwrap() = Some(Instant::now());
                    }
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let addrs = info.get_addresses();
                            let Some(addr) = addrs
                                .iter()
                                .find(|a| a.is_ipv4())
                                .or_else(|| addrs.iter().next())
                                .map(|a| a.to_string())
                            else {
                                continue;
                            };
                            let fullname = info.get_fullname().to_string();
                            let name = fullname
                                .strip_suffix(&format!(".{MDNS_SERVICE_TYPE}"))
                                .unwrap_or(&fullname)
                                .to_string();
                            let entry = DiscoveredServer {
                                name,
                                addr,
                                port: info.get_port(),
                                fullname,
                            };
                            let mut list = servers.lock().unwrap();
                            match list.iter_mut().find(|e| e.fullname == entry.fullname) {
                                Some(existing) => *existing = entry,
                                None => list.push(entry),
                            }
                        }
                        ServiceEvent::ServiceRemoved(_ty, fullname) => {
                            servers.lock().unwrap().retain(|e| e.fullname != fullname);
                        }
                        _ => {}
                    }
                }
            });
        }

        Some(Discovery { daemon, servers, last_event })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}
