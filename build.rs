//! Stamps the build number into the binary as `HYPRCIRCL_VERSION`.
//!
//! Every green push to master is tagged `vN` by CI. The version is, in order:
//! `HYPRCIRCL_BUILD` from the environment (CI or a packager), the `vN` tag on
//! the checked-out commit, or `dev (<short hash>)` for any other local build.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.trim().to_string())
}

fn is_build_tag(tag: &str) -> bool {
    match tag.strip_prefix('v') {
        Some(n) => !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

fn version() -> String {
    if let Ok(build) = std::env::var("HYPRCIRCL_BUILD") {
        let build = build.trim();
        if !build.is_empty() {
            return build.to_string();
        }
    }
    if let Some(tags) = git(&["tag", "--points-at", "HEAD"]) {
        if let Some(tag) = tags.lines().find(|t| is_build_tag(t)) {
            return tag.to_string();
        }
    }
    match git(&["rev-parse", "--short=7", "HEAD"]) {
        Some(hash) if !hash.is_empty() => format!("dev ({hash})"),
        _ => "dev".to_string(),
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=HYPRCIRCL_BUILD");
    for path in [".git/HEAD", ".git/refs", ".git/packed-refs"] {
        if Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!("cargo:rustc-env=HYPRCIRCL_VERSION={}", version());
}
