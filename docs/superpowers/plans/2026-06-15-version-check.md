# Background Version-Check Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tell the user when a newer GitHub release of `git-supervisor` exists, via an explicit `version` subcommand and a cached background check during `watch` — never download or replace anything.

**Architecture:** A new single-purpose module `src/version_check.rs` holds pure helpers (`parse_release`, `compare_versions`, `notify_line`, `cache_is_stale`) plus one thin networked function (`fetch_latest_release`) and two orchestrators (`run_version_check` for the subcommand, `maybe_notify_update` for the auto path). The auto path is cache-gated (24h) and swallows all errors; the subcommand always fetches fresh and surfaces errors.

**Tech Stack:** Rust, clap, serde_json (existing), dirs (existing), and a new dependency `ureq` (blocking HTTP, rustls TLS).

**Design doc:** `docs/superpowers/specs/2026-06-15-version-check-design.md`

---

## File Structure

- **Create:** `src/version_check.rs` — the whole feature: parsing, version comparison, caching, network fetch, and the two entry points.
- **Modify:** `Cargo.toml` — add `ureq`.
- **Modify:** `src/lib.rs` — declare the module, re-export entry points, call `maybe_notify_update` from `run_watch`.
- **Modify:** `src/main.rs` — add the `version` subcommand.
- **Modify:** `README.md` — document the subcommand and auto-check.

---

## Task 1: Module scaffold + `parse_release`

**Files:**
- Modify: `Cargo.toml`
- Create: `src/version_check.rs`

- [ ] **Step 1: Add the ureq dependency**

In `Cargo.toml`, under `[dependencies]` (keep the list alphabetical — insert after `tokio`), add:

```toml
ureq = "2"
```

- [ ] **Step 2: Create the module with `ReleaseInfo` + `parse_release` and a failing test**

Create `src/version_check.rs` with exactly this content:

```rust
//! Notify-only version check against the GitHub Releases API.
//!
//! This module never downloads or replaces the binary. It only detects whether
//! a newer published release exists and tells the user.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::console;

const GITHUB_REPO: &str = "jfding/git-supervisor";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
const HTTP_TIMEOUT_SECS: u64 = 5;

/// A published release as we care about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub html_url: String,
}

/// Shape of the subset of the GitHub `releases/latest` JSON we read.
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
}

/// Parse a GitHub `releases/latest` JSON body into a `ReleaseInfo`. Pure.
pub fn parse_release(json: &str) -> Result<ReleaseInfo> {
    let r: ApiRelease = serde_json::from_str(json).context("parsing GitHub release JSON")?;
    Ok(ReleaseInfo {
        tag: r.tag_name,
        html_url: r.html_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_release_extracts_tag_and_url() {
        let json = r#"{
            "tag_name": "v2.3.0",
            "html_url": "https://github.com/jfding/git-supervisor/releases/tag/v2.3.0",
            "name": "v2.3.0",
            "draft": false,
            "prerelease": false
        }"#;
        let info = parse_release(json).unwrap();
        assert_eq!(info.tag, "v2.3.0");
        assert_eq!(
            info.html_url,
            "https://github.com/jfding/git-supervisor/releases/tag/v2.3.0"
        );
    }

    #[test]
    fn parse_release_errors_on_malformed_json() {
        assert!(parse_release("not json").is_err());
        assert!(parse_release(r#"{"name": "no tag here"}"#).is_err());
    }
}
```

- [ ] **Step 3: Wire the module into the crate so it compiles**

In `src/lib.rs`, add the module declaration alongside the other `pub mod` lines (after `pub mod status;`):

```rust
pub mod version_check;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib version_check::tests::parse_release`
Expected: 2 tests PASS. (First run also compiles `ureq`; this may take a moment.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/version_check.rs src/lib.rs
git commit -m "feat(version-check): add module scaffold and parse_release"
```

---

## Task 2: `compare_versions`

**Files:**
- Modify: `src/version_check.rs`

- [ ] **Step 1: Add a failing test for version comparison**

In `src/version_check.rs`, inside the `mod tests` block, add:

```rust
    #[test]
    fn compare_versions_orders_correctly() {
        assert_eq!(compare_versions("2.1.8", "v2.1.9"), Ordering::Less);
        assert_eq!(compare_versions("2.1.8", "v2.1.8"), Ordering::Equal);
        assert_eq!(compare_versions("2.2.0", "v2.1.9"), Ordering::Greater);
        // leading v on either side is ignored
        assert_eq!(compare_versions("v2.1.8", "2.1.8"), Ordering::Equal);
        // minor/major take precedence over patch
        assert_eq!(compare_versions("2.1.10", "v2.2.0"), Ordering::Less);
        // malformed components parse as 0, never panic
        assert_eq!(compare_versions("garbage", "v0.0.0"), Ordering::Equal);
        // trailing pre-release suffix on a component is truncated to its digits
        assert_eq!(compare_versions("2.1.8", "v2.1.8-rc1"), Ordering::Equal);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib version_check::tests::compare_versions_orders_correctly`
Expected: FAIL — `cannot find function compare_versions in this scope`.

- [ ] **Step 3: Implement `compare_versions` and its helper**

In `src/version_check.rs`, add after the `parse_release` function (before the `#[cfg(test)]` block):

```rust
/// Parse a version string into a `(major, minor, patch)` tuple.
/// Strips a leading `v`/`V`. Each component keeps only its leading digits
/// (so `8-rc1` becomes `8`); anything unparseable becomes `0`.
fn parse_semver(s: &str) -> (u64, u64, u64) {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let mut parts = s.split('.').map(|p| {
        let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u64>().unwrap_or(0)
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Compare a current version against a release tag. Pure.
/// `Ordering::Less` means the current version is behind `latest_tag`.
pub fn compare_versions(current: &str, latest_tag: &str) -> Ordering {
    parse_semver(current).cmp(&parse_semver(latest_tag))
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib version_check::tests::compare_versions_orders_correctly`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/version_check.rs
git commit -m "feat(version-check): add semver comparison"
```

---

## Task 3: `notify_line`

**Files:**
- Modify: `src/version_check.rs`

- [ ] **Step 1: Add a failing test**

In `src/version_check.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn notify_line_only_when_behind() {
        let latest = ReleaseInfo {
            tag: "v2.2.0".to_string(),
            html_url: "https://example.com/rel".to_string(),
        };
        let line = notify_line("2.1.8", &latest).expect("should notify when behind");
        assert!(line.contains("v2.2.0"));
        assert!(line.contains("2.1.8"));

        // up to date or ahead => no notice
        assert!(notify_line("2.2.0", &latest).is_none());
        assert!(notify_line("2.3.0", &latest).is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib version_check::tests::notify_line_only_when_behind`
Expected: FAIL — `cannot find function notify_line in this scope`.

- [ ] **Step 3: Implement `notify_line`**

In `src/version_check.rs`, add after `compare_versions`:

```rust
/// Build a one-line update notice if `latest` is newer than `current`,
/// otherwise `None`. Pure (no I/O, no color).
pub fn notify_line(current: &str, latest: &ReleaseInfo) -> Option<String> {
    if compare_versions(current, &latest.tag) == Ordering::Less {
        Some(format!(
            "A new git-supervisor release is available: {} (current {}). Download: {}",
            latest.tag, current, latest.html_url
        ))
    } else {
        None
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib version_check::tests::notify_line_only_when_behind`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/version_check.rs
git commit -m "feat(version-check): add notify_line renderer"
```

---

## Task 4: Cache (`cache_is_stale` + read/write)

**Files:**
- Modify: `src/version_check.rs`

- [ ] **Step 1: Add a failing test for staleness**

In `src/version_check.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn cache_is_stale_respects_ttl() {
        let ttl = 24 * 60 * 60;
        // 1 hour old => fresh
        assert!(!cache_is_stale(1_000_000, 1_000_000 + 3_600, ttl));
        // exactly ttl old => stale
        assert!(cache_is_stale(1_000_000, 1_000_000 + ttl, ttl));
        // older than ttl => stale
        assert!(cache_is_stale(1_000_000, 1_000_000 + ttl + 1, ttl));
        // clock skew (now < checked_at) must not panic and counts as fresh
        assert!(!cache_is_stale(1_000_000, 999_000, ttl));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib version_check::tests::cache_is_stale_respects_ttl`
Expected: FAIL — `cannot find function cache_is_stale in this scope`.

- [ ] **Step 3: Implement the cache helpers**

In `src/version_check.rs`, add after `notify_line`:

```rust
/// Cached result of a previous version check.
#[derive(Serialize, Deserialize)]
struct CacheData {
    checked_at: u64,
    latest_tag: String,
}

/// Path to the cache file, or `None` if no cache dir is available.
fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("git-supervisor").join("version-check.json"))
}

/// Current unix time in seconds (0 on the impossible pre-epoch error).
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether a cache entry stamped at `checked_at` is older than `ttl` at `now`. Pure.
fn cache_is_stale(checked_at: u64, now: u64, ttl: u64) -> bool {
    now.saturating_sub(checked_at) >= ttl
}

/// Read the cache, returning `None` on any error (missing, unreadable, corrupt).
fn read_cache() -> Option<CacheData> {
    let path = cache_path()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Write the cache, ignoring all errors (best-effort).
fn write_cache(tag: &str) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let data = CacheData {
        checked_at: now_unix(),
        latest_tag: tag.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&data) {
        let _ = std::fs::write(path, json);
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib version_check::tests::cache_is_stale_respects_ttl`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/version_check.rs
git commit -m "feat(version-check): add cache read/write and staleness check"
```

---

## Task 5: Network fetch + entry points

**Files:**
- Modify: `src/version_check.rs`

> Note: `fetch_latest_release` makes a real network call and is intentionally kept thin (all tested logic lives in the pure helpers it calls), so there is no unit test for it. The orchestrators `run_version_check` / `maybe_notify_update` are likewise thin glue; they are exercised via the subcommand wiring test in Task 6 and manually.

- [ ] **Step 1: Implement `fetch_latest_release`**

In `src/version_check.rs`, add after the cache helpers:

```rust
/// Fetch the latest published release from the GitHub API. Networked.
/// Times out after `HTTP_TIMEOUT_SECS` so it can never hang the caller.
fn fetch_latest_release() -> Result<ReleaseInfo> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );
    let user_agent = format!("git-supervisor/{}", env!("CARGO_PKG_VERSION"));
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build();
    let body = agent
        .get(&url)
        .set("User-Agent", &user_agent)
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("requesting latest release from GitHub")?
        .into_string()
        .context("reading GitHub response body")?;
    parse_release(&body)
}
```

- [ ] **Step 2: Implement `maybe_notify_update` (auto path)**

In `src/version_check.rs`, add after `fetch_latest_release`:

```rust
/// Background check for `watch`: cache-gated and silent on every error.
/// Prints at most one highlighted line when a newer release exists.
pub fn maybe_notify_update(current: &str) {
    let latest_tag = match read_cache() {
        Some(c) if !cache_is_stale(c.checked_at, now_unix(), CACHE_TTL_SECS) => c.latest_tag,
        _ => match fetch_latest_release() {
            Ok(info) => {
                write_cache(&info.tag);
                info.tag
            }
            Err(e) => {
                console::log_debug(format!("version check skipped: {}", e));
                return;
            }
        },
    };
    let info = ReleaseInfo {
        html_url: format!("https://github.com/{}/releases/tag/{}", GITHUB_REPO, latest_tag),
        tag: latest_tag,
    };
    if let Some(line) = notify_line(current, &info) {
        console::log_highlight(line);
    }
}
```

- [ ] **Step 3: Implement `run_version_check` (explicit subcommand)**

In `src/version_check.rs`, add after `maybe_notify_update`:

```rust
/// Explicit `version` subcommand: always fetch fresh, refresh the cache,
/// and print the full result. Returns `Err` on network/parse failure.
pub fn run_version_check(current: &str) -> Result<()> {
    println!("git-supervisor {}", current);
    let latest = fetch_latest_release().context("checking GitHub for the latest release")?;
    write_cache(&latest.tag);
    match compare_versions(current, &latest.tag) {
        Ordering::Less => {
            println!(
                "{}",
                console::highlight(format!("A newer release is available: {}", latest.tag))
            );
            println!("Release page: {}", latest.html_url);
            println!(
                "Download binaries: https://github.com/{}/releases",
                GITHUB_REPO
            );
        }
        Ordering::Equal => {
            println!("{}", console::highlight("You are on the latest release."));
        }
        Ordering::Greater => {
            println!(
                "You are ahead of the latest published release ({}).",
                latest.tag
            );
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Verify the crate compiles cleanly**

Run: `cargo build`
Expected: builds with no errors and no warnings about the new module.

- [ ] **Step 5: Commit**

```bash
git add src/version_check.rs
git commit -m "feat(version-check): add network fetch and entry points"
```

---

## Task 6: `version` subcommand wiring

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Re-export the entry points from the crate root**

In `src/lib.rs`, after the existing `pub use cleanup::{run_cleanup, CleanupOpts};` line, add:

```rust
pub use version_check::{maybe_notify_update, run_version_check};
```

- [ ] **Step 2: Add a failing clap-wiring test**

In `src/main.rs`, inside the `#[cfg(test)] mod tests` block, add:

```rust
    #[test]
    fn cli_version_subcommand_parses() {
        let cli = Cli::try_parse_from(["supervisor", "version"]).unwrap();
        assert!(matches!(cli.command, Command::Version));
    }
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --bin git-supervisor cli_version_subcommand_parses`
Expected: FAIL — `no variant named Version found for enum Command`.

- [ ] **Step 4: Add the subcommand variant**

In `src/main.rs`, in the `enum Command` block, add a new variant after `PrintScript`:

```rust
    /// Print the current version and check GitHub for a newer release
    Version,
```

- [ ] **Step 5: Import the entry point and handle the variant**

In `src/main.rs`, update the `use git_supervisor::{...}` line (line 3) to include `run_version_check`:

```rust
use git_supervisor::{run_check, run_cleanup, run_local_watch, run_status, run_version_check, run_watch, CentralConfig, CleanupOpts, StatusOpts, WatchOpts, CHECK_PUSH_SCRIPT};
```

Then, in the `match &cli.command` block in `main()`, add an arm (place it right after the `Command::PrintScript => { ... }` arm):

```rust
        Command::Version => run_version_check(env!("CARGO_PKG_VERSION")),
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --bin git-supervisor cli_version_subcommand_parses`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/main.rs
git commit -m "feat(version-check): add version subcommand"
```

---

## Task 7: Auto-check during `watch`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Fire the background check at watch startup**

In `src/lib.rs`, inside `pub async fn run_watch`, add the following immediately after the opening lines that set up `interval`/`deadline`/`round` and before the `if !opts.skip_prepare {` block (around line 466). `opts.version` is cloned here because it is moved into the webhook server later in the function:

```rust
    // Background version check: cache-gated, never blocks the loop, swallows
    // all errors. Runs on a blocking thread because the HTTP client is sync.
    let current_version = opts.version.clone();
    tokio::task::spawn_blocking(move || version_check::maybe_notify_update(&current_version));
```

- [ ] **Step 2: Verify it compiles and all existing tests still pass**

Run: `cargo test`
Expected: full test suite PASS, including the new `version_check` and `main` tests. No new warnings.

- [ ] **Step 3: Manually sanity-check the subcommand against the live API**

Run: `cargo run -- version`
Expected: prints `git-supervisor 2.1.8` followed by either "You are on the latest release." or a "newer release available" notice with a URL. (If offline, it prints an error and exits non-zero — that is correct behavior for the explicit command.)

- [ ] **Step 4: Commit**

```bash
git add src/lib.rs
git commit -m "feat(version-check): auto-check for updates on watch startup"
```

---

## Task 8: Documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Find where subcommands are documented**

Run: `grep -n "PrintScript\|print-script\|## Usage\|### " README.md`
Expected: shows the headings/subcommand list so you can place the new section consistently with the existing ones.

- [ ] **Step 2: Add a `version` subcommand section**

In `README.md`, add a short section near the other subcommand docs (match the surrounding heading style). Use this content:

```markdown
### `version` — check for updates

```bash
git-supervisor version
```

Prints the current version and checks the [GitHub Releases](https://github.com/jfding/git-supervisor/releases)
API for the latest published release. If a newer release exists, it shows the
new tag and the release page URL. This is **notify-only** — it never downloads
or replaces the binary.

The `watch` command runs the same check once at startup (cached for 24h under
`~/.cache/git-supervisor/version-check.json`) and prints a one-line notice if a
newer release is available. The check is best-effort: any network error is
ignored and never interrupts watching.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document version subcommand and watch auto-check"
```

---

## Self-Review Notes

- **Spec coverage:** detect+notify only (no download anywhere) ✓; explicit `version` subcommand (Task 6) ✓; cached 24h auto-check in `watch` (Tasks 4 + 7) ✓; ureq blocking client (Task 1) ✓; `/releases/latest` + serde_json parse (Tasks 1, 5) ✓; numeric semver compare, no semver crate (Task 2) ✓; cache at `~/.cache/git-supervisor/version-check.json` (Task 4) ✓; auto path swallows errors / explicit path surfaces them (Task 5) ✓; tests for compare/parse/staleness + clap wiring (Tasks 1–4, 6) ✓.
- **Type consistency:** `ReleaseInfo { tag, html_url }`, `compare_versions(current, latest_tag) -> Ordering`, `notify_line(current, &ReleaseInfo) -> Option<String>`, `cache_is_stale(checked_at, now, ttl) -> bool`, `run_version_check(current) -> Result<()>`, `maybe_notify_update(current)` — names and signatures are identical everywhere they appear.
- **No placeholders:** every code step contains complete, compilable code.
