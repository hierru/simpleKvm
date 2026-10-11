//! macOS Accessibility (손쉬운 사용) permission check. CGEvent injection is
//! silently dropped unless this process is trusted, so the UI surfaces it.
//!
//! Note: the permission is per-binary-identity. An ad-hoc signed .app gets a new
//! identity on every rebuild, which revokes a previously granted permission —
//! after each rebuild the user must re-add it in System Settings.

// `AXIsProcessTrusted` — true when this process may inject input events.
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
}

pub fn is_trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Open System Settings at Privacy & Security → Accessibility.
pub fn open_settings() {
    open_pane("com.apple.preference.security?Privacy_Accessibility");
}

/// Open System Settings at Privacy & Security → Local Network.
///
/// There is no counterpart to `AXIsProcessTrusted` for this permission: it is
/// enforced by a Network Extension packet filter rather than TCC, and Apple
/// exposes no way to read its state without triggering the request. Probing it
/// functionally (browse mDNS and see whether anything answers) was tried and
/// rejected — the same bundle, at the same grant state, produced both "nothing
/// found" and "resolved fine" minutes apart. So the UI links here instead of
/// claiming a verdict it cannot support.
pub fn open_local_network_settings() {
    open_pane("com.apple.settings.PrivacySecurity.extension?Privacy_LocalNetwork");
}

fn open_pane(url: &str) {
    let _ = std::process::Command::new("open")
        .arg(format!("x-apple.systempreferences:{url}"))
        .status();
}
