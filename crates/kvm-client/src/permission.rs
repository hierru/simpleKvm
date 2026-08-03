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
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .status();
}
