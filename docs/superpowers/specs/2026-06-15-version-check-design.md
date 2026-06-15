# Design: background version-check for git-supervisor

Date: 2026-06-15

## Goal

Detect when a newer GitHub release of `git-supervisor` exists and tell the
user. This feature is **notify-only**: it never downloads, replaces, or
modifies the running binary.

## Decisions (from brainstorming)

- **Scope:** detect + notify only. No self-update / download.
- **Triggers:** an explicit subcommand *and* a cached background check during
  `watch`.
- **HTTP client:** `ureq` (blocking, rustls). Fits the mostly-synchronous CLI
  and keeps the dependency tree lean.
- **Detection method:** GitHub Releases API `GET /releases/latest`, parsed with
  `serde_json` (already a dependency). The `latest` endpoint already excludes
  pre-releases and drafts.

## New module: `src/version_check.rs`

Single-purpose module, exported from `lib.rs`. Public surface:

- `ReleaseInfo { tag: String, html_url: String }` — parsed result.
- `parse_release(json: &str) -> Result<ReleaseInfo>` — **pure.** Extracts
  `tag_name` and `html_url` from the API JSON. Errors on malformed JSON or
  missing fields.
- `compare_versions(current: &str, latest_tag: &str) -> std::cmp::Ordering` —
  **pure.** Strips a leading `v` from each, parses `major.minor.patch` into a
  numeric tuple, and compares. No semver crate: `/releases/latest` excludes
  pre-releases, so plain `x.y.z` ordering is sufficient. Malformed components
  parse as `0`.
- `fetch_latest_release() -> Result<ReleaseInfo>` — the only networked
  function. `ureq` GET to
  `https://api.github.com/repos/jfding/git-supervisor/releases/latest` with:
  - `User-Agent: git-supervisor/<version>` (GitHub returns 403 without it)
  - `Accept: application/vnd.github+json`
  - ~5s timeout so it can never hang the caller.
- `run_version_check(current: &str) -> Result<()>` — for the explicit
  subcommand. Always fetches fresh, refreshes the cache, prints the full
  result.
- `maybe_notify_update(current: &str)` — for the auto path. Cache-gated,
  swallows all errors, prints at most one styled line.

The repository slug is a module constant: `GITHUB_REPO = "jfding/git-supervisor"`.

## Caching

- Location: `~/.cache/git-supervisor/version-check.json` (via
  `dirs::cache_dir()`; fall back to skipping the cache if no cache dir).
- Contents: `{ "checked_at": <unix_seconds>, "latest_tag": "v2.1.8" }`.
- TTL: **24 hours.**
- `cache_is_stale(checked_at, now, ttl) -> bool` is a **pure** function (clock
  injected) so it is testable without real time.
- Auto-check (`maybe_notify_update`): only hits the network when the cache is
  missing or stale; otherwise compares against the cached tag. After a network
  fetch, rewrites the cache.
- Explicit subcommand (`run_version_check`): always fetches fresh, then
  refreshes the cache.

## Triggers

### 1. Explicit subcommand: `git-supervisor version`

- Prints the current version.
- Fetches the latest release (fresh).
- If behind: prints the new tag, the release `html_url`, and a line directing
  the user to download from GitHub Releases.
- If up to date: prints a confirmation.
- On network/parse failure: prints a warning and exits non-zero.

### 2. Auto-check during `watch`

- At `watch` startup, run `maybe_notify_update` inside `tokio::spawn_blocking`
  (ureq is blocking) so it never delays the first check-push round.
- If behind: prints one styled line via the `console` module.
- On any error (network, parse, cache): silent — nothing is printed.

## Error handling

Notify-only must never break `watch`. The auto path swallows all
network/parse/cache errors. The explicit subcommand surfaces them to the user.

## Dependencies

Add `ureq` (v2, rustls TLS). This is the only new dependency. `serde_json` and
`dirs` are already present.

## Testing

Unit tests (no real network):

- `compare_versions`: behind / equal / ahead / `v`-prefixed / malformed input.
- `parse_release`: a sample API JSON string → `ReleaseInfo`; malformed JSON →
  error.
- `cache_is_stale`: fresh vs expired, with an injected `now`.

Integration / wiring:

- A clap parse test for the new `version` subcommand, matching the existing
  `main.rs` test style.

The networked `fetch_latest_release` is kept thin; the tested logic lives in the
pure functions it calls.
