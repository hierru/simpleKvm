//! Dev tool: pretends to be the macOS client so the server can be tested
//! without a Mac. Connects, handshakes, and prints every received message.
//!
//!     cargo run -p kvm-protocol --example fake_client -- 127.0.0.1 [seconds]

use std::net::TcpStream;
use std::time::{Duration, Instant};

use kvm_protocol::{read_message, write_message, Message, DEFAULT_PORT, PROTOCOL_VERSION};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".into());
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);

    let mut stream = TcpStream::connect((host.as_str(), DEFAULT_PORT)).expect("connect failed");
    stream.set_nodelay(true).unwrap();
    write_message(
        &mut stream,
        &Message::Hello { version: PROTOCOL_VERSION, name: "fake-client".into() },
    )
    .expect("hello failed");

    match read_message(&mut stream).expect("handshake read failed") {
        Message::HelloAck { version, name } => {
            println!("handshake ok: server '{name}' protocol v{version}")
        }
        other => panic!("unexpected reply: {other:?}"),
    }

    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        match read_message(&mut stream) {
            Ok(msg) => println!("recv: {msg:?}"),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => {
                println!("connection closed: {e}");
                return;
            }
        }
    }
    println!("done");
}
