//! LAN auto-discovery of simpleKvm servers via mDNS/DNS-SD browsing.
//! Runs for the app's lifetime; the settings UI reads `servers` each frame.

use std::sync::{Arc, Mutex};

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
}

impl Discovery {
    pub fn start() -> Option<Discovery> {
        let daemon = ServiceDaemon::new().ok()?;
        let rx = daemon.browse(MDNS_SERVICE_TYPE).ok()?;
        let servers: Arc<Mutex<Vec<DiscoveredServer>>> = Arc::new(Mutex::new(Vec::new()));

        {
            let servers = servers.clone();
            // Exits when the daemon shuts down (recv starts failing).
            std::thread::spawn(move || {
                while let Ok(event) = rx.recv() {
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

        Some(Discovery { daemon, servers })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}
