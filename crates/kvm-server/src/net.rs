//! TCP listener: accepts one macOS client at a time, forwards captured input
//! events from the hook thread's channel, and processes client messages
//! (return-to-server, heartbeats).

use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use kvm_protocol::{read_message, write_message, Message, PROTOCOL_VERSION};

use crate::hooks::{self, CONNECTED, REMOTE};

pub fn run(port: u16, rx: Receiver<Message>, server_name: String) {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .unwrap_or_else(|e| panic!("failed to bind port {port}: {e}"));
    println!("kvm-server: listening on 0.0.0.0:{port}");

    loop {
        let (mut stream, addr) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("accept failed: {e}");
                continue;
            }
        };

        let client_name = match handshake(&mut stream, &server_name) {
            Ok(name) => name,
            Err(e) => {
                eprintln!("handshake with {addr} failed: {e}");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        println!("client connected: {client_name} ({addr})");

        // Drop events that piled up while nobody was connected.
        while rx.try_recv().is_ok() {}
        CONNECTED.store(true, Ordering::SeqCst);

        let reader_stream = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("stream clone failed: {e}");
                CONNECTED.store(false, Ordering::SeqCst);
                continue;
            }
        };
        let reader = std::thread::spawn(move || read_loop(reader_stream));

        // Forward events until the connection dies.
        loop {
            if !CONNECTED.load(Ordering::SeqCst) {
                break;
            }
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(msg) => {
                    if write_message(&mut stream, &msg).is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if write_message(&mut stream, &Message::Heartbeat).is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        CONNECTED.store(false, Ordering::SeqCst);
        if REMOTE.load(Ordering::SeqCst) {
            // Never leave the user stranded without a cursor on Windows.
            hooks::leave_remote(None, false);
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = reader.join();
        println!("client disconnected: {client_name}");
    }
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
    // The client only talks when the cursor returns; block indefinitely and
    // rely on our outgoing heartbeats to detect a dead connection.
    stream.set_read_timeout(None)?;
    Ok(name)
}

fn read_loop(mut stream: TcpStream) {
    loop {
        match read_message(&mut stream) {
            Ok(Message::ReturnToServer { y_ratio }) => hooks::leave_remote(Some(y_ratio), false),
            Ok(Message::Heartbeat) => {}
            Ok(_) => {}
            Err(_) => {
                CONNECTED.store(false, Ordering::SeqCst);
                break;
            }
        }
    }
}
