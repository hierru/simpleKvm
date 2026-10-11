//! Stamps the commit the binary was built from into the executable, so the
//! diagnostics pane can show which build is actually running. The build time
//! is not stamped here: the running image's identity is taken from the
//! executable's mtime at process start (see `app::BuildStamp`), which stays
//! correct even when a rebundle overwrites the file underneath a live process.

use std::process::Command;

fn main() {
    println!("cargo:rustc-env=KVM_BUILD_COMMIT={}", commit());
    // Keep the stamp honest: re-run when HEAD moves or any source changes.
    // `.git/HEAD` alone is not enough: on a branch it just says
    // `ref: refs/heads/main`, so a new commit never touches it. Watch the
    // branch ref (and packed-refs, where it may live) plus the index, which
    // changes on commit and so clears a stale `+dirty`.
    for path in watched_git_paths() {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=src");
}

fn watched_git_paths() -> Vec<String> {
    let mut names = vec!["HEAD".to_string(), "packed-refs".into(), "index".into()];
    if let Some(branch_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
        names.push(branch_ref);
    }
    names
        .iter()
        .filter_map(|n| git(&["rev-parse", "--path-format=absolute", "--git-path", n]))
        .collect()
}

fn commit() -> String {
    let Some(short) = git(&["rev-parse", "--short", "HEAD"]) else {
        return "unknown".into();
    };
    // A dirty tree means the binary does not match the commit exactly.
    match git(&["status", "--porcelain"]) {
        Some(s) if !s.is_empty() => format!("{short}+dirty"),
        _ => short,
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
