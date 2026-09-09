//! Capture git facts at build time so the binary can report which commit it
//! was built from when the build is not sitting on a clean release tag.
//!
//! Emits two env vars, always defined (empty when git info is unavailable, as
//! in a source tarball or the Docker build where `.git` is not in the context):
//!   GIT_SUPERVISOR_COMMIT  - short commit hash, or "" if unknown
//!   GIT_SUPERVISOR_TAGGED  - "1" when HEAD is a clean checkout of a version tag
//!
//! The version string itself is assembled in `version_check::display_version`
//! so that formatting stays unit-testable.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn main() {
    // Emitting any rerun-if-changed replaces cargo's default source watching,
    // so the source paths have to be listed alongside the git ones: the commit
    // matters for the hash, the sources for the dirty flag.
    for path in [
        ".git/HEAD",
        ".git/refs",
        ".git/packed-refs",
        "src",
        "core",
        "build.rs",
        "Cargo.toml",
        "Cargo.lock",
    ] {
        if std::path::Path::new(path).exists() {
            println!("cargo:rerun-if-changed={}", path);
        }
    }

    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some();
    // Any tag pointing at HEAD counts; `--points-at` works on the shallow
    // checkouts CI produces, unlike `describe --exact-match`.
    let on_tag = git(&["tag", "--points-at", "HEAD"]).is_some();
    let tagged = !commit.is_empty() && on_tag && !dirty;

    println!("cargo:rustc-env=GIT_SUPERVISOR_COMMIT={}", commit);
    println!(
        "cargo:rustc-env=GIT_SUPERVISOR_TAGGED={}",
        if tagged { "1" } else { "" }
    );
    println!(
        "cargo:rustc-env=GIT_SUPERVISOR_DIRTY={}",
        if dirty { "1" } else { "" }
    );
}
