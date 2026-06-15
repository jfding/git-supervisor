//! Notify-only version check against the GitHub release tags.
//!
//! This module never downloads or replaces the binary. It only detects whether
//! a newer published release exists and tells the user.
//!
//! Detection uses `git ls-remote --tags` against the GitHub repo via the system
//! `git` binary (already a hard runtime dependency), so there is no new Rust
//! dependency, no C compiler, and no TLS stack in the statically-linked binary.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::console;

const GITHUB_REPO: &str = "jfding/git-supervisor";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

/// Git clone URL used for `git ls-remote`.
fn git_url() -> String {
    format!("https://github.com/{}.git", GITHUB_REPO)
}

/// Releases page URL, synthesized for the update notice.
fn releases_url() -> String {
    format!("https://github.com/{}/releases", GITHUB_REPO)
}

/// Parse `git ls-remote` output into a de-duplicated list of tag names. Pure.
///
/// Each line looks like `<sha>\trefs/tags/<name>`. We strip the `refs/tags/`
/// prefix and any `^{}` peel suffix (defensive — `--refs` already removes them).
pub fn parse_ls_remote_tags(output: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for line in output.lines() {
        let Some((_, refname)) = line.split_once('\t') else {
            continue;
        };
        let Some(name) = refname.trim().strip_prefix("refs/tags/") else {
            continue;
        };
        let name = name.strip_suffix("^{}").unwrap_or(name).to_string();
        if !name.is_empty() && !tags.contains(&name) {
            tags.push(name);
        }
    }
    tags
}

/// Whether a tag names a stable release: optional leading `v`, then
/// dot-separated all-numeric components (e.g. `v2.1.8`). Rejects pre-releases
/// like `v2.2.0-rc1` and non-version tags. Pure.
fn is_stable_release_tag(tag: &str) -> bool {
    let s = tag.trim().trim_start_matches(['v', 'V']);
    !s.is_empty()
        && s.split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Highest stable release tag among `tags`, or `None` if there are none. Pure.
pub fn latest_stable_tag(tags: &[String]) -> Option<String> {
    tags.iter()
        .filter(|t| is_stable_release_tag(t))
        .max_by(|a, b| compare_versions(a, b))
        .cloned()
}

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

/// Build a one-line update notice if `latest_tag` is newer than `current`,
/// otherwise `None`. Pure (no I/O, no color).
pub fn notify_line(current: &str, latest_tag: &str) -> Option<String> {
    if compare_versions(current, latest_tag) == Ordering::Less {
        Some(format!(
            "A new git-supervisor release is available: {} (current {}). Download: {}",
            latest_tag,
            current,
            releases_url()
        ))
    } else {
        None
    }
}

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

/// Fetch the latest stable release tag via `git ls-remote`. Side-effecting.
///
/// `GIT_TERMINAL_PROMPT=0` ensures a credential prompt can never hang the call.
fn fetch_latest_tag() -> Result<String> {
    let output = std::process::Command::new("git")
        .arg("ls-remote")
        .arg("--tags")
        .arg("--refs")
        .arg(git_url())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("running git ls-remote")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git ls-remote failed: {}",
            if stderr.is_empty() {
                format!("exit {}", output.status)
            } else {
                stderr
            }
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let tags = parse_ls_remote_tags(&stdout);
    latest_stable_tag(&tags).context("no stable release tags found via git ls-remote")
}

/// Background check for `watch`: cache-gated and silent on every error.
/// Prints at most one highlighted line when a newer release exists.
pub fn maybe_notify_update(current: &str) {
    let latest_tag = match read_cache() {
        Some(c) if !cache_is_stale(c.checked_at, now_unix(), CACHE_TTL_SECS) => c.latest_tag,
        _ => match fetch_latest_tag() {
            Ok(tag) => {
                write_cache(&tag);
                tag
            }
            Err(e) => {
                console::log_debug(format!("version check skipped: {}", e));
                return;
            }
        },
    };
    if let Some(line) = notify_line(current, &latest_tag) {
        console::log_highlight(line);
    }
}

/// Explicit `version` subcommand: always fetch fresh, refresh the cache,
/// and print the full result. Returns `Err` on lookup failure.
pub fn run_version_check(current: &str) -> Result<()> {
    println!("git-supervisor {}", current);
    let latest = fetch_latest_tag().context("checking GitHub for the latest release")?;
    write_cache(&latest);
    match compare_versions(current, &latest) {
        Ordering::Less => {
            println!(
                "{}",
                console::highlight(format!("A newer release is available: {}", latest))
            );
            println!("Download binaries: {}", releases_url());
        }
        Ordering::Equal => {
            println!("{}", console::highlight("You are on the latest release."));
        }
        Ordering::Greater => {
            println!(
                "You are ahead of the latest published release ({}).",
                latest
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_remote_tags_extracts_and_dedupes() {
        let output = "\
deadbeef\trefs/tags/v2.1.8
cafef00d\trefs/tags/v2.2.0
cafef00d\trefs/tags/v2.2.0^{}
abc123\trefs/tags/v2.0.0
";
        let tags = parse_ls_remote_tags(output);
        assert_eq!(tags, vec!["v2.1.8", "v2.2.0", "v2.0.0"]);
    }

    #[test]
    fn parse_ls_remote_tags_ignores_garbage_lines() {
        let output = "not-a-ref-line\n\tno-sha-but-tab\nsha\trefs/heads/master\n";
        assert!(parse_ls_remote_tags(output).is_empty());
    }

    #[test]
    fn latest_stable_tag_picks_highest_stable() {
        let tags = vec![
            "v2.1.8".to_string(),
            "v2.2.0-rc1".to_string(),
            "v2.10.0".to_string(),
            "nightly".to_string(),
            "v2.2.0".to_string(),
        ];
        assert_eq!(latest_stable_tag(&tags), Some("v2.10.0".to_string()));
    }

    #[test]
    fn latest_stable_tag_none_when_no_stable() {
        let tags = vec!["v2.2.0-rc1".to_string(), "latest".to_string()];
        assert_eq!(latest_stable_tag(&tags), None);
        assert_eq!(latest_stable_tag(&[]), None);
    }

    #[test]
    fn compare_versions_orders_correctly() {
        assert_eq!(compare_versions("2.1.8", "v2.1.9"), Ordering::Less);
        assert_eq!(compare_versions("2.1.8", "v2.1.8"), Ordering::Equal);
        assert_eq!(compare_versions("2.2.0", "v2.1.9"), Ordering::Greater);
        assert_eq!(compare_versions("v2.1.8", "2.1.8"), Ordering::Equal);
        assert_eq!(compare_versions("2.1.10", "v2.2.0"), Ordering::Less);
        assert_eq!(compare_versions("garbage", "v0.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.1.8", "v2.1.8-rc1"), Ordering::Equal);
    }

    #[test]
    fn notify_line_only_when_behind() {
        let line = notify_line("2.1.8", "v2.2.0").expect("should notify when behind");
        assert!(line.contains("v2.2.0"));
        assert!(line.contains("2.1.8"));
        assert!(line.contains("github.com/jfding/git-supervisor/releases"));

        assert!(notify_line("2.2.0", "v2.2.0").is_none());
        assert!(notify_line("2.3.0", "v2.2.0").is_none());
    }

    #[test]
    fn cache_is_stale_respects_ttl() {
        let ttl = 24 * 60 * 60;
        assert!(!cache_is_stale(1_000_000, 1_000_000 + 3_600, ttl));
        assert!(cache_is_stale(1_000_000, 1_000_000 + ttl, ttl));
        assert!(cache_is_stale(1_000_000, 1_000_000 + ttl + 1, ttl));
        // clock skew (now < checked_at) must not panic and counts as fresh
        assert!(!cache_is_stale(1_000_000, 999_000, ttl));
    }
}
