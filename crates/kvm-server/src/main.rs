//! simpleKvm server — runs on the Windows PC that owns the physical
//! keyboard/mouse. Captures input with low-level hooks and streams it to the
//! macOS client when the cursor crosses the configured screen edge.

use clap::Parser;
use kvm_protocol::{Side, DEFAULT_PORT};

#[cfg(windows)]
mod hooks;
#[cfg(windows)]
mod net;

#[derive(Parser, Debug)]
#[command(name = "kvm-server", about = "simpleKvm server (Windows side)")]
struct Args {
    /// TCP port to listen on.
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,

    /// Which side of this Windows screen the Mac sits on: left | right.
    #[arg(long, default_value = "left")]
    mac_side: Side,

    /// Name reported to the client during the handshake.
    #[arg(long, default_value = "windows-pc")]
    name: String,

    /// Print the detected monitor arrangement and shared edge, then exit.
    #[arg(long, default_value_t = false)]
    list_monitors: bool,
}

#[cfg(windows)]
fn main() {
    let args = Args::parse();

    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        // Without this, cursor coordinates are virtualized on scaled displays.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    if args.list_monitors {
        hooks::print_layout(args.mac_side);
        return;
    }

    let (tx, rx) = std::sync::mpsc::channel();
    hooks::init(tx, args.mac_side);

    let port = args.port;
    let name = args.name.clone();
    std::thread::spawn(move || net::run(port, rx, name));

    println!(
        "kvm-server: mac is on the {:?} edge; move the cursor past that edge to switch.",
        args.mac_side
    );
    println!("kvm-server: press Ctrl+Alt+F12 to force control back to Windows.");

    hooks::run_message_loop();
}

#[cfg(not(windows))]
fn main() {
    let _ = Args::parse();
    eprintln!("kvm-server only runs on Windows. Build kvm-client on the Mac instead.");
    std::process::exit(1);
}
