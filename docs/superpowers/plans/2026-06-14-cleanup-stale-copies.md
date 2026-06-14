# `cleanup` Subcommand Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `git-supervisor cleanup` subcommand that reaps the stale `*.to-be-removed` copy directories that `check-push.sh` moves aside but never deletes, across the controlled target hosts.

**Architecture:** Mirror the existing `status` command. A dedicated embedded shell script (`cleanup_probe.sh`) runs once per host over SSH: it enumerates `*.to-be-removed` dirs under `$DIR_COPIES`, emits a fixed-column TSV, and — only when `APPLY=1` — deletes each one itself using a safe-rm guard ported from `check-push.sh`. The Rust side (`cleanup.rs`) fans out across hosts in parallel, parses the TSV into `CleanupReport`s, and renders a status-style, age-annotated report. Dry-run is the default; `--apply` performs deletion.

**Tech Stack:** Rust (clap, anyhow, chrono, std::thread::scope), embedded Bash script (`include_str!`), reuse of `status::{format_relative_time, glob_match}` and `console::{paint, Color}`.

---

## Background (read before starting)

Lifecycle of a "stale copy" (verified in `core/check-push.sh:645-674`):

1. Each `check-push` cycle touches `.living` in every copy it processes.
2. Copies that are NOT refreshed (no `.living`) get `mv`'d to `<copy>.to-be-removed` (line 672).
3. The actual `rm -rf` is commented out (line 670) — so `*.to-be-removed` dirs accumulate forever.
4. `status` already surfaces these as `ReportKind::Stale` rows.

This command finishes the deletion check-push deferred. **Scope: `*.to-be-removed` dirs only** — not `unknown` dirs, not active reaping of `.living`-less copies.

Reference implementations to mirror:
- `src/status.rs` — `STATUS_PROBE_SCRIPT`, `Report::parse_line`, `collect_host`, `run_status`, parallel fanout, rendering.
- `src/status_probe.sh` — env contract (`DIR_BASE`, `HOST_ID`), `mtime_or_zero`, repo-name matching (`REPO_NAMES` + `match_repo`), TSV `emit`.
- `core/check-push.sh:107-141` — `_safe_rm_rf_copies` (the guard to port).
- `tests/integration_status.rs` — pattern for binary-against-localhost integration tests (`localhost` targets run via `sh -lc`, no real SSH — see `src/ssh.rs:55-72`).

**TSV contract (fixed 6 columns, tab-separated):**

```
<host>\t<repo>\t<name>\t<mtime_unix>\t<outcome>\t<reason>
```

- `repo`: matched repo name, or `-` if the dir matches no known repo.
- `name`: the directory name, e.g. `webapp.prod.v10.0.to-be-removed`.
- `mtime_unix`: dir mtime in Unix seconds, or `0`.
- `outcome`: `would-remove` (dry-run) | `removed` | `failed` (apply).
- `reason`: `-` normally; a single-line error string when `outcome=failed`.

---

## File Structure

- **Create** `src/cleanup_probe.sh` — embedded probe/reaper script (enumerate + dry-run + apply).
- **Create** `src/cleanup.rs` — `CLEANUP_PROBE_SCRIPT`, `Outcome`, `CleanupReport`, `CleanupOpts`, `run_cleanup`, parsing/grouping/rendering helpers, unit tests.
- **Create** `core/tests/scripts/test-cleanup-probe.sh` — self-contained shell test for `cleanup_probe.sh` (dry-run, apply, safe-rm refusal).
- **Create** `tests/integration_cleanup.rs` — binary-against-localhost integration tests.
- **Modify** `src/lib.rs` — `pub mod cleanup;` + re-exports.
- **Modify** `src/main.rs` — `Cleanup(CleanupArgs)` subcommand + handler + CLI parse tests.
- **Modify** `README.md` — document the subcommand; note it reaps the `.living`/`to-be-removed` lifecycle.

---

## Task 1: `cleanup_probe.sh` — enumerate stale dirs (dry-run only)

Build the script's skeleton: env contract, helpers, repo matching, and the dry-run path that lists `*.to-be-removed` dirs without deleting anything. Deletion (APPLY) comes in Task 2.

**Files:**
- Create: `src/cleanup_probe.sh`
- Create: `core/tests/scripts/test-cleanup-probe.sh`

- [ ] **Step 1: Write the failing test**

Create `core/tests/scripts/test-cleanup-probe.sh`:

```bash
#!/usr/bin/env bash
# Self-contained tests for src/cleanup_probe.sh. Creates a temp DIR_BASE,
# runs the probe in dry-run and apply modes, asserts behavior.
set -u
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROBE="$(cd "$SCRIPT_DIR/../../../src" && pwd)/cleanup_probe.sh"
fails=0
check() { # desc, actual, expected
  if [[ "$2" == "$3" ]]; then echo "ok   - $1"; else echo "FAIL - $1: got [$2] want [$3]"; fails=$((fails+1)); fi
}
contains() { # desc, haystack, needle
  if [[ "$2" == *"$3"* ]]; then echo "ok   - $1"; else echo "FAIL - $1: [$2] missing [$3]"; fails=$((fails+1)); fi
}

# ---- fixture ----
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/git_repos/webapp" "$TMP/git_repos/api-service"
mkdir -p "$TMP/copies/webapp.main"                       # live branch copy
mkdir -p "$TMP/copies/webapp.prod.v10.0.to-be-removed"   # stale
mkdir -p "$TMP/copies/api-service.dev.to-be-removed"     # stale

# ---- dry-run (APPLY unset → default 0) ----
out=$(DIR_BASE="$TMP" HOST_ID="h1" bash "$PROBE")
contains "dry-run lists webapp stale" "$out" $'h1\twebapp\twebapp.prod.v10.0.to-be-removed\t'
contains "dry-run lists api stale"    "$out" $'h1\tapi-service\tapi-service.dev.to-be-removed\t'
contains "dry-run marks would-remove" "$out" "would-remove"
check    "dry-run does NOT list live copy" "$(echo "$out" | grep -c 'webapp.main')" "0"
check    "dry-run deleted nothing (stale dir still present)" \
         "$([[ -d "$TMP/copies/webapp.prod.v10.0.to-be-removed" ]] && echo yes || echo no)" "yes"

echo "---- $fails failure(s) ----"
exit $((fails > 0))
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash core/tests/scripts/test-cleanup-probe.sh`
Expected: FAIL — script not found / no output (e.g. `bash: .../cleanup_probe.sh: No such file or directory`, assertions FAIL).

- [ ] **Step 3: Write minimal implementation**

Create `src/cleanup_probe.sh`:

```bash
#!/usr/bin/env bash
# Reaps stale "*.to-be-removed" copy dirs under $DIR_COPIES.
# Dry-run by default (APPLY=0): lists what WOULD be removed, deletes nothing.
# APPLY=1: deletes each stale dir via a safe-rm guard, reports per-dir outcome.
# Inputs (env): DIR_BASE, HOST_ID, APPLY.
# Schema: <host>\t<repo>\t<name>\t<mtime_unix>\t<outcome>\t<reason>
#   outcome ∈ would-remove | removed | failed ; reason is "-" unless failed.
# Exit: 0 on success (incl. zero findings); non-zero if any deletion failed
#       or $DIR_COPIES is unreadable.
set -u
export LC_ALL=C

: "${HOST_ID:=unknown}"
: "${DIR_BASE:=/work}"
: "${APPLY:=0}"
DIR_REPOS="${DIR_BASE}/git_repos"
DIR_COPIES="${DIR_BASE}/copies"

# Missing copies dir → nothing to clean (host not yet bootstrapped).
[[ -d "$DIR_COPIES" ]] || exit 0
if [[ ! -r "$DIR_COPIES" ]]; then
  echo "cleanup: $DIR_COPIES not readable" >&2
  exit 1
fi

emit() {
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "$6"
}

# Print Unix-seconds mtime, or 0 if missing/unreadable.
mtime_or_zero() {
  local f=$1
  [[ -e "$f" ]] || { echo 0; return; }
  local t
  t=$(stat -c %Y "$f" 2>/dev/null) && { echo "$t"; return; }
  t=$(stat -f %m "$f" 2>/dev/null) && { echo "$t"; return; }
  echo 0
}

# Build the repo-name list, longest first, so multi-dot repo names ("my.api")
# match before shorter prefixes ("my"). Mirrors status_probe.sh.
REPO_NAMES=()
if [[ -d "$DIR_REPOS" ]]; then
  while IFS= read -r r; do
    [[ -n "$r" ]] && REPO_NAMES+=("$r")
  done < <(ls -1 "$DIR_REPOS" 2>/dev/null \
            | awk '{print length, $0}' \
            | sort -rn -k1,1 \
            | cut -d' ' -f2-)
fi

# Echo the matching repo name for a copy-dir name, or "-" if none matches.
match_repo() {
  local d=$1 r
  for r in "${REPO_NAMES[@]}"; do
    if [[ "$d" == "$r" || "$d" == "$r".* ]]; then
      echo "$r"; return 0
    fi
  done
  echo "-"; return 0
}

cd "$DIR_COPIES" || { echo "cleanup: cd $DIR_COPIES failed" >&2; exit 1; }

rc=0
shopt -s nullglob
for d in *.to-be-removed/; do
  d=${d%/}
  [[ -d "$d" ]] || continue
  repo=$(match_repo "$d")
  mtime=$(mtime_or_zero "$d")
  # Dry-run only for now; APPLY handling added in Task 2.
  emit "$HOST_ID" "$repo" "$d" "$mtime" would-remove "-"
done

exit $rc
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash core/tests/scripts/test-cleanup-probe.sh`
Expected: PASS — `---- 0 failure(s) ----`, exit 0.

- [ ] **Step 5: Commit**

```bash
git add src/cleanup_probe.sh core/tests/scripts/test-cleanup-probe.sh
git commit -m "feat: add cleanup_probe.sh dry-run enumeration of stale copies"
```

---

## Task 2: `cleanup_probe.sh` — APPLY deletion with safe-rm guard

Add the `APPLY=1` path: delete each stale dir via a ported `_safe_rm_rf_copies` guard, emit `removed`/`failed` outcomes, and refuse anything not strictly under `$DIR_COPIES`.

**Files:**
- Modify: `src/cleanup_probe.sh`
- Test: `core/tests/scripts/test-cleanup-probe.sh`

- [ ] **Step 1: Write the failing test**

Append to `core/tests/scripts/test-cleanup-probe.sh`, immediately before the `echo "---- $fails ..."` summary line:

```bash
# ---- apply ----
out=$(DIR_BASE="$TMP" HOST_ID="h1" APPLY=1 bash "$PROBE")
ap_rc=$?
contains "apply marks webapp removed" "$out" $'h1\twebapp\twebapp.prod.v10.0.to-be-removed\t0\tremoved\t-'
check    "apply deleted webapp stale dir" \
         "$([[ -d "$TMP/copies/webapp.prod.v10.0.to-be-removed" ]] && echo yes || echo no)" "no"
check    "apply deleted api stale dir" \
         "$([[ -d "$TMP/copies/api-service.dev.to-be-removed" ]] && echo yes || echo no)" "no"
check    "apply left live copy untouched" \
         "$([[ -d "$TMP/copies/webapp.main" ]] && echo yes || echo no)" "yes"
check    "apply exit code 0 on full success" "$ap_rc" "0"

# ---- safe-rm refusal: a symlink pointing outside DIR_COPIES must not be followed/deleted ----
OUTSIDE=$(mktemp -d)
mkdir -p "$OUTSIDE/precious"
ln -s "$OUTSIDE/precious" "$TMP/copies/evil.to-be-removed"
out=$(DIR_BASE="$TMP" HOST_ID="h1" APPLY=1 bash "$PROBE")
ev_rc=$?
contains "refuses to delete outside copies tree" "$out" "failed"
check    "outside target NOT deleted" \
         "$([[ -d "$OUTSIDE/precious" ]] && echo yes || echo no)" "yes"
check    "apply exit code non-zero when a deletion fails" "$([[ $ev_rc -ne 0 ]] && echo nz || echo z)" "nz"
rm -rf "$OUTSIDE"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash core/tests/scripts/test-cleanup-probe.sh`
Expected: FAIL — apply assertions fail (the dry-run script emits `would-remove`, never `removed`; stale dirs are not deleted).

- [ ] **Step 3: Write minimal implementation**

In `src/cleanup_probe.sh`, add the safe-rm function after `match_repo` (before the `cd "$DIR_COPIES"` line):

```bash
# rm -rf only when $1 resolves strictly under $DIR_COPIES (real path). Refuses
# the copies root itself and anything outside the tree. Ported from
# check-push.sh:_safe_rm_rf_copies. On failure: echo a one-line reason, return 1.
safe_rm_rf_copies() {
  local _target=$1 _base _resolved _err
  [[ -n "$_target" ]] || { echo "empty target"; return 1; }
  _base=$(cd "$DIR_COPIES" && pwd -P) || { echo "cannot resolve DIR_COPIES"; return 1; }
  # Resolve the target's own real path. A symlink resolves to its destination,
  # so a link pointing outside the tree is correctly refused below.
  if [[ -e "$_target" || -L "$_target" ]]; then
    _resolved=$(cd "$_target" 2>/dev/null && pwd -P) || { echo "cannot resolve target"; return 1; }
  else
    echo "target missing"; return 1
  fi
  [[ "$_resolved" == "$_base" ]] && { echo "refusing copies root"; return 1; }
  case "${_resolved}/" in
    "${_base}/"*) ;;
    *) echo "outside copies tree"; return 1 ;;
  esac
  _err=$(rm -rf -- "$_target" 2>&1) || { echo "${_err:-rm failed}"; return 1; }
  return 0
}

# Collapse tabs/CR/newlines in a reason string to spaces so it stays one TSV field.
sanitize() { printf '%s' "$1" | tr '\t\r\n' '   '; }
```

Then replace the `emit ... would-remove "-"` line inside the loop with:

```bash
  if [[ "$APPLY" == "1" ]]; then
    if reason=$(safe_rm_rf_copies "$d"); then
      emit "$HOST_ID" "$repo" "$d" "$mtime" removed "-"
    else
      emit "$HOST_ID" "$repo" "$d" "$mtime" failed "$(sanitize "$reason")"
      rc=1
    fi
  else
    emit "$HOST_ID" "$repo" "$d" "$mtime" would-remove "-"
  fi
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bash core/tests/scripts/test-cleanup-probe.sh`
Expected: PASS — `---- 0 failure(s) ----`, exit 0.

- [ ] **Step 5: Commit**

```bash
git add src/cleanup_probe.sh core/tests/scripts/test-cleanup-probe.sh
git commit -m "feat: add APPLY deletion with safe-rm guard to cleanup_probe.sh"
```

---

## Task 3: `cleanup.rs` — TSV parser

Create the Rust module with the embedded script constant, the `Outcome` enum, the `CleanupReport` struct, and `CleanupReport::parse_line` with unit tests.

**Files:**
- Create: `src/cleanup.rs`
- Modify: `src/lib.rs` (add `pub mod cleanup;` so the module compiles and tests run)

- [ ] **Step 1: Write the failing test**

Create `src/cleanup.rs`:

```rust
use crate::config::CentralConfig;
use crate::config::Host;
use crate::console::{self, paint, Color};
use crate::ssh;
use crate::status::{format_relative_time, glob_match};
use std::collections::BTreeMap;
use std::path::Path;

/// Embedded cleanup probe/reaper script, run on remotes with `DIR_BASE`/`HOST_ID`/`APPLY` env.
pub const CLEANUP_PROBE_SCRIPT: &str = include_str!("cleanup_probe.sh");

/// What happened (or would happen) to a stale dir.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    WouldRemove,
    Removed,
    Failed,
}

/// One parsed row from the cleanup probe. One per `*.to-be-removed` dir.
#[derive(Debug, Clone)]
pub struct CleanupReport {
    pub host: String,
    pub repo: String,
    pub name: String,
    pub mtime_unix: u64,
    pub outcome: Outcome,
    pub reason: String,
}

impl CleanupReport {
    /// Parse one TSV line. Strict 6-column contract; returns None on any mismatch.
    pub fn parse_line(line: &str) -> Option<Self> {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 6 {
            return None;
        }
        let outcome = match cols[4] {
            "would-remove" => Outcome::WouldRemove,
            "removed" => Outcome::Removed,
            "failed" => Outcome::Failed,
            _ => return None,
        };
        // Non-numeric or negative mtimes → 0.
        let mtime_unix = cols[3]
            .parse::<i64>()
            .ok()
            .filter(|n| *n >= 0)
            .map(|n| n as u64)
            .unwrap_or(0);
        let reason = if cols[5] == "-" { String::new() } else { cols[5].to_string() };
        Some(Self {
            host: cols[0].to_string(),
            repo: cols[1].to_string(),
            name: cols[2].to_string(),
            mtime_unix,
            outcome,
            reason,
        })
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parses_would_remove_line() {
        let line = "app1\twebapp\twebapp.prod.v10.0.to-be-removed\t1747720000\twould-remove\t-";
        let r = CleanupReport::parse_line(line).expect("parse");
        assert_eq!(r.host, "app1");
        assert_eq!(r.repo, "webapp");
        assert_eq!(r.name, "webapp.prod.v10.0.to-be-removed");
        assert_eq!(r.mtime_unix, 1747720000);
        assert_eq!(r.outcome, Outcome::WouldRemove);
        assert!(r.reason.is_empty());
    }

    #[test]
    fn parses_removed_line() {
        let line = "h\tapi\tapi.dev.to-be-removed\t0\tremoved\t-";
        let r = CleanupReport::parse_line(line).unwrap();
        assert_eq!(r.outcome, Outcome::Removed);
        assert_eq!(r.mtime_unix, 0);
    }

    #[test]
    fn parses_failed_line_with_reason() {
        let line = "h\t-\tevil.to-be-removed\t0\tfailed\toutside copies tree";
        let r = CleanupReport::parse_line(line).unwrap();
        assert_eq!(r.outcome, Outcome::Failed);
        assert_eq!(r.repo, "-");
        assert_eq!(r.reason, "outside copies tree");
    }

    #[test]
    fn rejects_wrong_column_count() {
        assert!(CleanupReport::parse_line("a\tb\tc").is_none());
        assert!(CleanupReport::parse_line("a\tb\tc\td\te\tf\tg").is_none());
    }

    #[test]
    fn rejects_unknown_outcome() {
        assert!(CleanupReport::parse_line("h\tr\tn\t0\tbogus\t-").is_none());
    }

    #[test]
    fn negative_mtime_maps_to_zero() {
        let r = CleanupReport::parse_line("h\tr\tn\t-5\twould-remove\t-").unwrap();
        assert_eq!(r.mtime_unix, 0);
    }
}
```

Add to `src/lib.rs` in the module list (after `pub mod ssh;`, before `pub mod status;`):

```rust
pub mod cleanup;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cleanup::parser_tests 2>&1 | tail -20`
Expected: FAIL — compile error: unused imports (`CentralConfig`, `Host`, `ssh`, `paint`, `Color`, `BTreeMap`, `Path`, `format_relative_time`, `glob_match`, `console`) are not yet used. (They are consumed in Task 4.)

> NOTE: To keep this task green on its own, the imports not yet used trigger warnings, not errors, *unless* `-D warnings` is set. If the build fails on unused imports, temporarily prefix the module with `#![allow(unused_imports)]` and remove it in Task 4. Verify which applies: `cargo test --lib cleanup 2>&1 | grep -c "error\[" || true`.

- [ ] **Step 3: Write minimal implementation**

If Step 2 showed hard errors from unused imports, add this as the first line of `src/cleanup.rs`:

```rust
#![allow(unused_imports)] // removed in Task 4 once render/run land
```

Otherwise no change is needed — the parser code above already satisfies the tests.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib cleanup::parser_tests 2>&1 | tail -20`
Expected: PASS — 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/cleanup.rs src/lib.rs
git commit -m "feat: add CleanupReport TSV parser"
```

---

## Task 4: `cleanup.rs` — grouping, rendering, and `run_cleanup`

Add the host-probe fanout, per-host grouping/rendering, and the `run_cleanup` entry point (parallel across hosts, best-effort, aggregate failure into the exit code).

**Files:**
- Modify: `src/cleanup.rs`

- [ ] **Step 1: Write the failing test**

Add this test module at the end of `src/cleanup.rs`:

```rust
#[cfg(test)]
mod render_tests {
    use super::*;

    fn rep(repo: &str, name: &str, outcome: Outcome) -> CleanupReport {
        CleanupReport {
            host: "h".into(),
            repo: repo.into(),
            name: name.into(),
            mtime_unix: 0,
            outcome,
            reason: String::new(),
        }
    }

    #[test]
    fn group_by_repo_sorts_names() {
        let reports = vec![
            rep("webapp", "webapp.prod.v2.to-be-removed", Outcome::WouldRemove),
            rep("webapp", "webapp.prod.v1.to-be-removed", Outcome::WouldRemove),
            rep("api", "api.dev.to-be-removed", Outcome::WouldRemove),
        ];
        let grouped = group_by_repo(&reports);
        let webapp = grouped.get("webapp").unwrap();
        let names: Vec<&str> = webapp.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["webapp.prod.v1.to-be-removed", "webapp.prod.v2.to-be-removed"]);
        assert!(grouped.contains_key("api"));
    }

    #[test]
    fn summary_dry_run_counts_would_remove() {
        let reports = vec![
            rep("a", "a.x.to-be-removed", Outcome::WouldRemove),
            rep("b", "b.y.to-be-removed", Outcome::WouldRemove),
        ];
        let s = summarize(&reports, false, 1);
        assert!(s.contains("would remove 2"), "got: {s}");
        assert!(s.contains("--apply"), "dry-run summary should hint --apply; got: {s}");
    }

    #[test]
    fn summary_apply_counts_removed_and_failed() {
        let reports = vec![
            rep("a", "a.x.to-be-removed", Outcome::Removed),
            rep("b", "b.y.to-be-removed", Outcome::Removed),
            rep("c", "c.z.to-be-removed", Outcome::Failed),
        ];
        let s = summarize(&reports, true, 1);
        assert!(s.contains("removed 2"), "got: {s}");
        assert!(s.contains("failed 1"), "got: {s}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cleanup::render_tests 2>&1 | tail -20`
Expected: FAIL — `cannot find function group_by_repo` / `summarize` in this scope.

- [ ] **Step 3: Write minimal implementation**

If you added `#![allow(unused_imports)]` in Task 3, remove it now. Then add the following to `src/cleanup.rs`, after the `impl CleanupReport` block (before the test modules):

```rust
/// Filter options passed to [`run_cleanup`].
pub struct CleanupOpts {
    pub host_patterns: Vec<String>,
    /// When false (default), dry-run: list what would be removed, delete nothing.
    pub apply: bool,
}

// ─── Pure helpers ────────────────────────────────────────────────────────────

/// Group reports by repo name, each repo's rows sorted by dir name ascending.
pub fn group_by_repo(reports: &[CleanupReport]) -> BTreeMap<String, Vec<CleanupReport>> {
    let mut grouped: BTreeMap<String, Vec<CleanupReport>> = BTreeMap::new();
    for r in reports {
        grouped.entry(r.repo.clone()).or_default().push(r.clone());
    }
    for rows in grouped.values_mut() {
        rows.sort_by(|a, b| a.name.cmp(&b.name));
    }
    grouped
}

/// Build the trailing summary line. `host_count` is the number of hosts that
/// contributed at least one report.
pub fn summarize(reports: &[CleanupReport], apply: bool, host_count: usize) -> String {
    if apply {
        let removed = reports.iter().filter(|r| r.outcome == Outcome::Removed).count();
        let failed = reports.iter().filter(|r| r.outcome == Outcome::Failed).count();
        format!("cleanup: removed {}, failed {} across {} host(s)", removed, failed, host_count)
    } else {
        let n = reports.iter().filter(|r| r.outcome == Outcome::WouldRemove).count();
        format!(
            "cleanup: would remove {} stale copies across {} host(s) (run with --apply to delete)",
            n, host_count
        )
    }
}

fn host_filter_matches(patterns: &[String], host_id: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| glob_match(p, host_id))
}

// ─── Host-level probe ────────────────────────────────────────────────────────

enum HostOutcome {
    Ok(Vec<CleanupReport>),
    Empty,          // probe succeeded, no stale dirs (or $DIR_COPIES missing)
    Failed(String), // SSH/probe failed; message includes stderr
}

fn collect_host(host_id: &str, host: &Host, dir_base: &Path, apply: bool) -> HostOutcome {
    let dir_esc = dir_base.to_string_lossy().replace('\'', "'\\''");
    let host_esc = host_id.replace('\'', "'\\''");
    let command = format!(
        "DIR_BASE='{}' HOST_ID='{}' APPLY={} bash -s",
        dir_esc,
        host_esc,
        if apply { 1 } else { 0 },
    );
    match ssh::ssh_run_capture(host, &command, CLEANUP_PROBE_SCRIPT.as_bytes()) {
        Err(e) => HostOutcome::Failed(e.to_string()),
        Ok(out) => {
            let reports: Vec<CleanupReport> =
                out.lines().filter_map(CleanupReport::parse_line).collect();
            if reports.is_empty() {
                HostOutcome::Empty
            } else {
                HostOutcome::Ok(reports)
            }
        }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render_host(host_id: &str, reports: &[CleanupReport], now: u64) {
    println!("host: {}", host_id);
    for (repo, rows) in group_by_repo(reports) {
        let repo_label = if repo == "-" { "(unmatched)" } else { repo.as_str() };
        println!("  {}", repo_label);
        for r in rows {
            let suffix = match r.outcome {
                Outcome::WouldRemove => {
                    paint(format_relative_time(r.mtime_unix, now), Color::Grey)
                }
                Outcome::Removed => paint("removed".to_string(), Color::Green),
                Outcome::Failed => paint(format!("failed: {}", r.reason), Color::Red),
            };
            println!("    {:<40}  {}", r.name, suffix);
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Reap stale `*.to-be-removed` copies on each configured host.
/// Dry-run by default; deletes only when `opts.apply` is true. Best-effort:
/// renders everything it can, then returns Err if any host or any deletion failed.
pub fn run_cleanup(config: &CentralConfig, opts: CleanupOpts) -> anyhow::Result<()> {
    // Filter & skip empty-repos hosts (matches run_status's behavior).
    let mut targets: Vec<(String, &Host)> = Vec::new();
    for (host_id, host) in &config.hosts {
        if !host.is_wildcard() && config.repos_for_host(host_id).is_empty() {
            console::log_info(format!(
                "cleanup host {{ {} }} --> skipped (repos: [] is empty)",
                host_id
            ));
            continue;
        }
        if !host_filter_matches(&opts.host_patterns, host_id) {
            continue;
        }
        targets.push((host_id.clone(), host));
    }

    if !opts.host_patterns.is_empty() && targets.is_empty() {
        anyhow::bail!("no hosts matched: {:?}", opts.host_patterns);
    }

    // Parallel fanout across hosts.
    let apply = opts.apply;
    let outcomes: Vec<(String, HostOutcome)> = std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|(host_id, host)| {
                let host_id = host_id.clone();
                let dir_base = config.dir_base_for_host(&host_id);
                let host_ref: &Host = host;
                s.spawn(move || {
                    let outcome = collect_host(&host_id, host_ref, &dir_base, apply);
                    (host_id, outcome)
                })
            })
            .collect();
        let mut out: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().expect("thread panicked"))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    });

    let now = chrono::Local::now().timestamp().max(0) as u64;
    let mut any_failed = false;
    let mut all_reports: Vec<CleanupReport> = Vec::new();
    let mut host_count = 0usize;

    for (host_id, outcome) in &outcomes {
        match outcome {
            HostOutcome::Ok(reports) => {
                host_count += 1;
                render_host(host_id, reports, now);
                if reports.iter().any(|r| r.outcome == Outcome::Failed) {
                    any_failed = true;
                }
                all_reports.extend(reports.iter().cloned());
            }
            HostOutcome::Empty => {
                println!("host: {}  {}", host_id, paint("(nothing to clean)", Color::Grey));
            }
            HostOutcome::Failed(msg) => {
                any_failed = true;
                println!("{}", paint(format!("host: {}  ERROR", host_id), Color::Red));
                let first_line = msg.lines().next().unwrap_or("");
                if !first_line.is_empty() {
                    println!("  {}", paint(first_line, Color::Red));
                }
            }
        }
    }

    println!("{}", summarize(&all_reports, opts.apply, host_count));

    if any_failed {
        anyhow::bail!("cleanup failed on one or more hosts/dirs");
    }
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib cleanup 2>&1 | tail -20`
Expected: PASS — parser_tests (6) + render_tests (3) all pass; no unused-import warnings.

- [ ] **Step 5: Commit**

```bash
git add src/cleanup.rs
git commit -m "feat: add run_cleanup with parallel fanout and status-style rendering"
```

---

## Task 5: Export `cleanup` from the library

Wire the new module's public items into the crate root so `main.rs` can call them.

**Files:**
- Modify: `src/lib.rs:16` (the `pub use` block after `pub use config::...`)

- [ ] **Step 1: Write the failing test**

Add this test to the `#[cfg(test)] mod tests` block at the bottom of `src/lib.rs`:

```rust
    #[test]
    fn cleanup_reexports_are_public() {
        // Compile-time check that the re-exports exist at crate root.
        let _opts = crate::CleanupOpts { host_patterns: vec![], apply: false };
        let _f: fn(&CentralConfig, crate::CleanupOpts) -> Result<(), anyhow::Error> =
            crate::run_cleanup;
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib cleanup_reexports_are_public 2>&1 | tail -20`
Expected: FAIL — `cannot find ... CleanupOpts in crate root` / `run_cleanup` unresolved.

- [ ] **Step 3: Write minimal implementation**

In `src/lib.rs`, immediately after the line `pub use status::{run_status, StatusOpts};`, add:

```rust
pub use cleanup::{run_cleanup, CleanupOpts};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib cleanup_reexports_are_public 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs
git commit -m "feat: re-export run_cleanup and CleanupOpts from crate root"
```

---

## Task 6: Wire the `cleanup` subcommand into the CLI

Add the `Cleanup` subcommand variant, its args struct, the dispatch handler, and CLI parse tests.

**Files:**
- Modify: `src/main.rs` (imports, `Command` enum, args struct, `match` arm, tests)

- [ ] **Step 1: Write the failing test**

Add these tests to the `#[cfg(test)] mod tests` block at the bottom of `src/main.rs`:

```rust
    #[test]
    fn cli_cleanup_parses_defaults() {
        let cli = Cli::try_parse_from(["supervisor", "cleanup"]).unwrap();
        match cli.command {
            Command::Cleanup(args) => {
                assert!(args.host.is_empty());
                assert!(!args.apply, "apply must default to false (dry-run)");
            }
            _ => panic!("expected Cleanup"),
        }
    }

    #[test]
    fn cli_cleanup_parses_apply_and_hosts() {
        let cli = Cli::try_parse_from([
            "supervisor", "cleanup", "--host", "prod-*", "--host", "bastion", "--apply",
        ])
        .unwrap();
        match cli.command {
            Command::Cleanup(args) => {
                assert_eq!(args.host, vec!["prod-*".to_string(), "bastion".to_string()]);
                assert!(args.apply);
            }
            _ => panic!("expected Cleanup"),
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin git-supervisor cli_cleanup 2>&1 | tail -20`
Expected: FAIL — `no variant ... Cleanup` / `Command::Cleanup` unresolved.

- [ ] **Step 3: Write minimal implementation**

In `src/main.rs`:

(a) Extend the `use` import to include the cleanup symbols:

```rust
use git_supervisor::{run_check, run_cleanup, run_local_watch, run_status, run_watch, CentralConfig, CleanupOpts, StatusOpts, WatchOpts, CHECK_PUSH_SCRIPT};
```

(b) Add a variant to the `Command` enum (after the `Status(StatusArgs)` variant):

```rust
    /// Remove stale `*.to-be-removed` copies on each host. Dry-run by default; use --apply to delete.
    Cleanup(CleanupArgs),
```

(c) Add the args struct (after `StatusArgs`):

```rust
#[derive(clap::Args)]
struct CleanupArgs {
    /// Limit to hosts whose ID matches this glob (`*`, `?`). Repeatable; union semantics.
    #[arg(long)]
    host: Vec<String>,
    /// Actually delete the stale copies. Without this flag, only list what would be removed.
    #[arg(long)]
    apply: bool,
}
```

(d) Add a match arm in `main()` (after the `Command::Status(args) => { ... }` arm):

```rust
        Command::Cleanup(args) => {
            let path = config_path.unwrap_or_else(|| {
                console::log_error(
                    "no config file found; use --config or create ~/.config/git-supervisor/deployments.yaml"
                );
                std::process::exit(1);
            });
            let config = load_config_or_exit(&path);
            run_cleanup(&config, CleanupOpts {
                host_patterns: args.host.clone(),
                apply: args.apply,
            })
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --bin git-supervisor cli_cleanup 2>&1 | tail -20`
Expected: PASS — both `cli_cleanup_*` tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire cleanup subcommand into CLI"
```

---

## Task 7: End-to-end integration tests (binary against localhost)

Exercise the full path: real binary → local `sh -lc` exec of the embedded script → tempdir copies. Verify dry-run lists without deleting, `--apply` deletes the right dirs, and `--host` with no match exits non-zero.

**Files:**
- Create: `tests/integration_cleanup.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/integration_cleanup.rs`:

```rust
use std::fs;
use std::process::Command;

fn write_config(path: &std::path::Path, dir_base: &str) {
    let yaml = format!(
        "repos:\n  webapp:\n    git_url: https://example.invalid/webapp.git\n  api:\n    git_url: https://example.invalid/api.git\nhosts:\n  local:\n    ssh_target: localhost\n    dir_base: {dir_base}\n    repos: [webapp, api]\n"
    );
    fs::write(path, yaml).unwrap();
}

fn fixture(base: &std::path::Path) {
    fs::create_dir_all(base.join("git_repos/webapp")).unwrap();
    fs::create_dir_all(base.join("git_repos/api")).unwrap();
    fs::create_dir_all(base.join("copies/webapp.main")).unwrap(); // live
    fs::create_dir_all(base.join("copies/webapp.prod.v1.0.to-be-removed")).unwrap(); // stale
    fs::create_dir_all(base.join("copies/api.dev.to-be-removed")).unwrap(); // stale
}

fn run(cfg: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["--config", cfg.to_str().unwrap(), "cleanup"];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_git-supervisor"))
        .args(&args)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn cleanup_dry_run_lists_without_deleting() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &[]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("webapp.prod.v1.0.to-be-removed"), "stdout: {stdout}");
    assert!(stdout.contains("api.dev.to-be-removed"), "stdout: {stdout}");
    assert!(stdout.contains("would remove 2"), "summary missing; stdout: {stdout}");
    assert!(stdout.contains("--apply"), "dry-run should hint --apply; stdout: {stdout}");
    // Nothing deleted.
    assert!(tmp.path().join("copies/webapp.prod.v1.0.to-be-removed").is_dir());
    assert!(tmp.path().join("copies/api.dev.to-be-removed").is_dir());
}

#[test]
fn cleanup_apply_deletes_only_stale() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &["--apply"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("removed 2"), "summary missing; stdout: {stdout}");
    // Stale dirs gone, live dir intact.
    assert!(!tmp.path().join("copies/webapp.prod.v1.0.to-be-removed").exists());
    assert!(!tmp.path().join("copies/api.dev.to-be-removed").exists());
    assert!(tmp.path().join("copies/webapp.main").is_dir(), "live copy must survive");
}

#[test]
fn cleanup_host_filter_no_match_errors() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &["--host", "nope-*"]);
    assert!(!out.status.success(), "expected non-zero exit for zero matches");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("no hosts matched"), "stderr: {stderr}");
}

#[test]
fn cleanup_empty_host_reports_nothing_to_clean() {
    let tmp = tempfile::tempdir().unwrap();
    // git_repos + copies exist but no *.to-be-removed dirs.
    fs::create_dir_all(tmp.path().join("git_repos/webapp")).unwrap();
    fs::create_dir_all(tmp.path().join("copies/webapp.main")).unwrap();
    fs::create_dir_all(tmp.path().join("git_repos/api")).unwrap();
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &[]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("(nothing to clean)"), "stdout: {stdout}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test integration_cleanup 2>&1 | tail -25`
Expected: At this point the binary already supports `cleanup` (Tasks 1–6), so these should actually PASS. If any fail, fix the implementation — do not weaken the test. Run once to confirm the suite is green.

- [ ] **Step 3: Write minimal implementation**

No production code expected here — the feature is complete. If a test fails, debug against the relevant task (probe script behavior → Tasks 1–2; rendering/summary → Task 4; CLI → Task 6).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test integration_cleanup 2>&1 | tail -25`
Expected: PASS — all 4 integration tests pass.

- [ ] **Step 5: Commit**

```bash
git add tests/integration_cleanup.rs
git commit -m "test: add end-to-end integration tests for cleanup subcommand"
```

---

## Task 8: Documentation

Document the new subcommand in the README and note its place in the copy lifecycle.

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Add the subcommand section**

In `README.md`, add a new section after the `### Dot file triggers in copy directories` section (before `### Docker restart and pre/post hook jobs`):

````markdown
### Cleaning up stale copies

When a branch or release copy stops being deployed, `check-push` moves it aside to
`<copy>.to-be-removed` (see the `.living` row in the dot-file table above) but never
deletes it — these stale dirs accumulate over time. The `cleanup` subcommand reaps them
across the controlled hosts.

```bash
# Dry-run (default): list the stale copies that WOULD be removed, delete nothing
git-supervisor cleanup

# Limit the blast radius to specific hosts (same glob semantics as `status`)
git-supervisor cleanup --host 'prod-*'

# Actually delete the stale copies
git-supervisor cleanup --apply
```

- Targets only `*.to-be-removed` directories under `<dir_base>/copies/`. Live copies and
  unrecognized directories are never touched.
- Runs in parallel across hosts; deletion is guarded so it refuses anything that does not
  resolve strictly under `<dir_base>/copies/`.
- Exits non-zero if any host is unreachable or any deletion fails (best-effort: the other
  hosts/dirs are still processed).
````

- [ ] **Step 2: Cross-reference from the dot-file table**

In `README.md`, update the `.living` row of the dot-file table. Replace:

```
| `.living` | auto | Heartbeat marker written each cycle after a branch/release is processed. Copies without `.living` at cleanup time are considered stale and moved to `*.to-be-removed`. |
```

with:

```
| `.living` | auto | Heartbeat marker written each cycle after a branch/release is processed. Copies without `.living` at cleanup time are considered stale and moved to `*.to-be-removed` (reap these with `git-supervisor cleanup`). |
```

- [ ] **Step 3: Verify the docs render and reference real flags**

Run: `grep -n "git-supervisor cleanup" README.md`
Expected: shows the new usage lines (`cleanup`, `cleanup --host`, `cleanup --apply`) and the table cross-reference.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document the cleanup subcommand"
```

---

## Final verification

- [ ] **Run the full Rust suite**

Run: `cargo test 2>&1 | tail -30`
Expected: all unit + integration tests pass (incl. `cleanup::*`, `cli_cleanup_*`, `integration_cleanup`).

- [ ] **Run the shell probe test**

Run: `bash core/tests/scripts/test-cleanup-probe.sh`
Expected: `---- 0 failure(s) ----`, exit 0.

- [ ] **Manual smoke (optional, local mode)**

```bash
mkdir -p /tmp/gs-demo/git_repos/webapp /tmp/gs-demo/copies/webapp.prod.v1.to-be-removed /tmp/gs-demo/copies/webapp.main
printf 'repos:\n  webapp:\n    git_url: x\nhosts:\n  local:\n    ssh_target: localhost\n    dir_base: /tmp/gs-demo\n    repos: [webapp]\n' > /tmp/gs-demo/cfg.yaml
cargo run -- --config /tmp/gs-demo/cfg.yaml cleanup            # lists, deletes nothing
cargo run -- --config /tmp/gs-demo/cfg.yaml cleanup --apply    # deletes the stale dir
ls /tmp/gs-demo/copies                                          # webapp.main remains; stale gone
rm -rf /tmp/gs-demo
```

---

## Self-Review notes

- **Spec coverage:** Q1 scope (`*.to-be-removed` only) → Task 1 glob `*.to-be-removed/`. Q2 name `cleanup` → Task 6. Q3 dry-run default → Task 6 `apply: bool` default false + Task 1 `APPLY:=0`. Q4 dedicated script w/ embedded safe-rm → Tasks 1–2. Q5 `--apply` → Task 6. Q6 `--host` reuse → Task 4 `host_filter_matches` via `status::glob_match` + Task 6. Q7 age-annotated status-style render + per-dir removed/failed → Task 4 `render_host`. Q8 best-effort + aggregate exit code → Task 4 `any_failed`/`bail!`. Q9 parallel fanout → Task 4 `thread::scope`. Q10 no `--repo` → not added. Q11 single 6-col schema w/ outcome → Tasks 1–3. Q12 unit + shell + safe-rm refusal tests → Tasks 1–7.
- **Type consistency:** `CleanupReport`, `Outcome::{WouldRemove,Removed,Failed}`, `CleanupOpts{host_patterns, apply}`, `run_cleanup`, `group_by_repo`, `summarize`, `collect_host`, `render_host` used consistently across tasks. TSV is 6 columns everywhere (script `emit`, `parse_line`, shell test, integration assertions).
- **Reuse:** `format_relative_time`, `glob_match` imported from `status` (both already `pub`); `paint`/`Color`/`log_*` from `console`. No duplication of those.
</content>
</invoke>
