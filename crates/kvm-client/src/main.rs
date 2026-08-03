//! simpleKvm client — runs on the Mac. Connects to the Windows server,
//! receives input events, and injects them with CoreGraphics (CGEvent).
//!
//! Requires the Accessibility permission:
//! System Settings > Privacy & Security > Accessibility > add your terminal
//! (or the kvm-client binary).

use clap::Parser;
use kvm_protocol::DEFAULT_PORT;

#[cfg(target_os = "macos")]
mod inject;
#[cfg(target_os = "macos")]
mod keymap;

#[derive(Parser, Debug)]
#[command(name = "kvm-client", about = "simpleKvm client (macOS side)")]
struct Args {
    /// Windows server address (IP or hostname).
    server: String,

    /// Server TCP port.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Mouse speed multiplier applied to incoming deltas.
    #[arg(long, default_value_t = 1.0)]
    speed: f64,

    /// Map the Windows Ctrl key to macOS Command (Ctrl+C -> Cmd+C, ...).
    /// Default is the physical mapping: Ctrl->Control, Win->Command, Alt->Option.
    #[arg(long, default_value_t = false)]
    ctrl_as_cmd: bool,

    /// Name reported to the server during the handshake.
    #[arg(long, default_value = "mac")]
    name: String,
}

#[cfg(target_os = "macos")]
fn main() {
    use kvm_protocol::{read_message, write_message, Message, PROTOCOL_VERSION};
    use std::net::TcpStream;
    use std::time::Duration;

    let args = Args::parse();
    let addr = format!("{}:{}", args.server, args.port);

    loop {
        println!("connecting to {addr} ...");
        let mut stream = match TcpStream::connect(&addr) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("connect failed: {e}; retrying in 3s");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        };
        let _ = stream.set_nodelay(true);

        if let Err(e) = write_message(
            &mut stream,
            &Message::Hello { version: PROTOCOL_VERSION, name: args.name.clone() },
        ) {
            eprintln!("handshake write failed: {e}");
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        // The server heartbeats every 2s; a 15s silence means it is gone.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
        match read_message(&mut stream) {
            Ok(Message::HelloAck { version, name }) if version == PROTOCOL_VERSION => {
                println!("connected to '{name}' (protocol v{version})");
            }
            Ok(other) => {
                eprintln!("unexpected handshake reply: {other:?}");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
            Err(e) => {
                eprintln!("handshake read failed: {e}");
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
        }

        let mut injector = inject::Injector::new(args.speed, args.ctrl_as_cmd);
        loop {
            match read_message(&mut stream) {
                Ok(msg) => {
                    if let Err(e) = injector.handle(msg, &mut stream) {
                        eprintln!("send failed: {e}");
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("connection lost: {e}");
                    break;
                }
            }
        }
        injector.release_everything();
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    let _ = Args::parse();
    eprintln!("kvm-client only runs on macOS. Build kvm-server on the Windows PC instead.");
    std::process::exit(1);
}
