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
- **Detection method:** `git ls-remote --tags --refs https://github.com/jfding/git-supervisor.git`,
  using the system `git` binary that git-supervisor already requires at runtime.
  This adds **no new Rust dependency, no C compiler, and no TLS stack** to the
  statically-linked musl binary (git handles HTTPS). We filter the tags to
  stable releases ourselves and pick the highest by semver.

  > **Why not the GitHub Releases API + an HTTP client?** The original plan was
  > `ureq` + the `/releases/latest` API. `ureq`'s rustls backend pulls in `ring`,
  > which compiles C/assembly and needs a musl C compiler — a new build
  > dependency for a project whose build is otherwise pure Rust, and binary
  > bloat. Since `git` is already a hard runtime dependency and the project
  > already has a release-tag convention (`release_tag_pattern`), `git ls-remote`
  > is the cleaner fit. Trade-off: no release-notes URL (we synthesize the
  > releases page URL) and we must filter pre-releases ourselves.

## New module: `src/version_check.rs`

Single-purpose module, exported from `lib.rs`. Public surface:

- `parse_ls_remote_tags(output: &str) -> Vec<String>` — **pure.** Parses
  `git ls-remote` output (`<sha>\trefs/tags/<name>` lines) into a de-duplicated
  list of tag names, stripping the `refs/tags/` prefix and any `^{}` peel
  suffix.
- `latest_stable_tag(tags: &[String]) -> Option<String>` — **pure.** Keeps only
  stable release tags (optional `v`, then dot-separated all-numeric components,
  e.g. `v2.1.8`; rejects `v2.2.0-rc1`) and returns the highest by semver.
- `compare_versions(current: &str, latest_tag: &str) -> std::cmp::Ordering` —
  **pure.** Strips a leading `v` from each, parses `major.minor.patch` into a
  numeric tuple, and compares. No semver crate needed. Malformed components
  parse as `0`.
- `fetch_latest_tag() -> Result<String>` — the only function with side effects.
  Runs `git ls-remote --tags --refs https://github.com/jfding/git-supervisor.git`
  (matching `ops::remote_refs_fingerprint`'s invocation style), with
  `GIT_TERMINAL_PROMPT=0` so a credential prompt can never hang it. Parses the
  output and returns the latest stable tag.
- `run_version_check(current: &str) -> Result<()>` — for the explicit
  subcommand. Always fetches fresh, refreshes the cache, prints the full
  result.
- `maybe_notify_update(current: &str)` — for the auto path. Cache-gated,
  swallows all errors, prints at most one styled line.

The repository slug is a module constant: `GITHUB_REPO = "jfding/git-supervisor"`.
The release page URL (`https://github.com/<repo>/releases`) is synthesized for
the notice, since `git ls-remote` does not provide one.

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

**None.** No new Rust dependency. Detection uses the system `git` binary via
`std::process::Command`. `serde`/`serde_json` (cache) and `dirs` (cache path)
are already present.

## Testing

Unit tests (no real network, no git invocation):

- `compare_versions`: behind / equal / ahead / `v`-prefixed / malformed input.
- `parse_ls_remote_tags`: sample `ls-remote` output → tag list; `^{}` peel
  suffix stripped; duplicates removed.
- `latest_stable_tag`: a mix of stable + pre-release + junk tags → the highest
  stable tag; empty/no-stable → `None`.
- `cache_is_stale`: fresh vs expired, with an injected `now`.

Integration / wiring:

- A clap parse test for the new `version` subcommand, matching the existing
  `main.rs` test style.

The side-effecting `fetch_latest_tag` is kept thin; the tested logic lives in
the pure functions it calls.
