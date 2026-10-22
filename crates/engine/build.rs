//! Stamp the engine used by both the CLI and the app with its source revision.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn main() {
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SOLIDCRAFT_ENGINE_COMMIT={commit}");
    // Absolute git paths also work for linked worktrees and packed branch refs.
    for name in ["HEAD", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", &branch])
    {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
