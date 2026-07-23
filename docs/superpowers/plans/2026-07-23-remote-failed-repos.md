# Remote Failed-Repos Reporting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After each remote `check-push.sh --once` run, the controller learns which repos hard-failed and retries them next cycle (same role as `ls-remote` `failed_repos`).

**Architecture:** Human logs move to stderr; stdout carries only `result\tfail\t<repo>` TSV lines. A new SSH helper inherits stderr (live logs) and captures stdout. `run_check_push_remote` returns `CheckPushReport`. Watch loop keeps host-scoped `deploy_failures` across cycles and unions them into the existing retry/whitelist path.

**Tech Stack:** Bash (`core/check-push.sh`), Rust (`src/ssh.rs`, `src/ops.rs`, `src/lib.rs`). No new crate dependencies. Reuses status/cleanup TSV conventions.

**Spec:** `docs/superpowers/specs/2026-07-23-remote-failed-repos-design.md`

## Global Constraints

- Hard failures only in v1 (`fetch_and_check` `return 1`).
- Soft failures (copy `continue`, docker ignoring) unchanged — no RESULT lines.
- RESULT schema: `result<TAB>fail<TAB><repo>` only; no reason column.
- `--once` exits `1` if any hard fail, else `0`.
- Live human logs must remain visible (stderr inherit).

---

## File Structure

- **Modify** `core/check-push.sh` — logs → stderr; FAIL_DIR collection; RESULT emission; `--once` exit code.
- **Create** `core/tests/scripts/test-check-push-result.sh` — shell tests for RESULT + exit + stderr logs.
- **Modify** `src/ssh.rs` — `ssh_run_inherit_stderr_capture_stdout`.
- **Modify** `src/ops.rs` — `CheckPushReport`, parse helper, `run_check_push_remote` returns report.
- **Modify** `src/lib.rs` — `deploy_failures` state through `run_watch` / `run_cycle`.

---

### Task 1: Shell RESULT protocol + stderr logging

**Files:**
- Modify: `core/check-push.sh`
- Create: `core/tests/scripts/test-check-push-result.sh`

**Interfaces:**
- Produces: stdout RESULT lines; stderr human logs; `--once` exit 0/1 as in the spec.

- [ ] **Step 1: Write the failing shell test**

Create `core/tests/scripts/test-check-push-result.sh`:

```bash
#!/usr/bin/env bash
# Unit-ish tests for RESULT protocol without full repo fixtures.
# Sources check-push helpers by running the script in a temp DIR_BASE with a
# fake broken remote (or mocks fetch_and_check via embedding). Prefer the
# lightest path that still exercises main_loop's FAIL_DIR + exit path.

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT/check-push.sh"
fails=0
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }

# --- parse_result helper exercised against canned stdout ---
parse_ok=$(printf 'result\tfail\tmy-repo\n' | awk -F'\t' '$1=="result" && $2=="fail" {print $3}')
[[ "$parse_ok" == "my-repo" ]] && pass "RESULT line shape" || fail "RESULT line shape"

# --- logging goes to stderr ---
# Extract _logging by running a tiny wrapper: we verify a successful --once
# with empty REPO_WHITELIST / empty repos still puts no RESULT on stdout.
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/git_repos" "$TMP/copies"
out=$(mktemp); err=$(mktemp)
set +e
DIR_BASE="$TMP" SLEEP_TIME=0 CI_LOCK="$TMP/lock.d" LOGLEVEL=1 \
  bash "$SCRIPT" --once >"$out" 2>"$err"
rc=$?
set -e
[[ $rc -eq 0 ]] && pass "empty run exit 0" || fail "empty run exit 0 (rc=$rc)"
[[ ! -s "$out" ]] && pass "empty run empty stdout" || fail "empty run stdout not empty: $(cat "$out")"
# If LOGLEVEL allowed any line, it must be on stderr not stdout — already checked.

# --- hard fail: create a non-git dir named as whitelist repo so fetch path fails,
#     OR create a git repo with an unreachable remote ---
mkdir -p "$TMP/git_repos/badrepo"
# bare incomplete .git so _repo is selected but fetch fails
mkdir -p "$TMP/git_repos/badrepo/.git"
out=$(mktemp); err=$(mktemp)
set +e
DIR_BASE="$TMP" SLEEP_TIME=0 CI_LOCK="$TMP/lock.d" LOGLEVEL=0 \
  REPO_WHITELIST="badrepo" \
  bash "$SCRIPT" --once >"$out" 2>"$err"
rc=$?
set -e
[[ $rc -eq 1 ]] && pass "hard fail exit 1" || fail "hard fail exit 1 (rc=$rc)"
grep -qx $'result\tfail\tbadrepo' "$out" && pass "RESULT for badrepo" || fail "RESULT missing: $(cat "$out")"
# Human noise must not be on stdout
! grep -v $'^result\tfail\t' "$out" | grep -q . && pass "stdout only RESULT" || fail "stdout polluted"

[[ $fails -eq 0 ]]
```

Make executable. Adjust the “hard fail” fixture if the real script skips incomplete `.git` — the implementer must force a true `fetch_and_check` `return 1` (e.g. real repo + `git remote set-url origin` to `unreachable`).

- [ ] **Step 2: Run test — expect FAIL** (RESULT / exit 1 / stderr not implemented)

```bash
bash core/tests/scripts/test-check-push-result.sh
```

Expected: FAIL on hard-fail exit 1 and/or missing RESULT.

- [ ] **Step 3: Implement shell changes**

In `core/check-push.sh`:

1. In `_logging`, after building `_line`, print to stderr:

```bash
      if _color_enabled; then
        printf '%b\n' "${_line}" >&2
      else
        printf '%s\n' "${_line}" >&2
      fi
```

2. In `main_loop`, before the worker spawn loop:

```bash
    local _fail_dir
    _fail_dir=$(mktemp -d "${TMPDIR:-/tmp}/gs-fail.XXXXXX") || {
      err "failed to create FAIL_DIR"; exit 1
    }
```

3. Change worker spawn to record failures:

```bash
    for _repo in $REPOS_TO_CHECK; do
      info "[${_repo}] checking git upstream changes ..."
      (
        LOG_PREFIX="[${_repo}]"
        if ! fetch_and_check "${_repo}"; then
          printf '%s\n' "${_repo}" > "${_fail_dir}/${_repo}"
          exit 1
        fi
      ) &
    done
```

4. After waits, emit RESULTS and set exit flag; replace the vague-only path:

```bash
    local _any_hard_fail=0
    for _worker_pid in $(jobs -pr); do
      wait "${_worker_pid}" || _any_hard_fail=1
    done
    local _f _failed_repo
    for _f in "${_fail_dir}"/*; do
      [[ -e "$_f" ]] || continue
      _failed_repo=$(basename "$_f")
      printf 'result\tfail\t%s\n' "${_failed_repo}"
      _any_hard_fail=1
    done
    rm -rf "${_fail_dir}"
    [[ "${_any_hard_fail}" == "1" ]] && err "one or more repo workers failed in this round"
```

5. `--once` / sleep-0 exit:

```bash
    if [[ "${1:-}" == "once" ]]; then
      [[ "${_any_hard_fail}" == "1" ]] && exit 1
      exit 0
    fi
    [[ $SLEEP_TIME == "" ]] || [[ $SLEEP_TIME == "0" ]] && {
      [[ "${_any_hard_fail}" == "1" ]] && exit 1
      exit 0
    }
```

Ensure `_any_hard_fail` is in scope for those exits (declare at top of the `while` iteration).

- [ ] **Step 4: Re-run shell test — expect PASS**

```bash
bash core/tests/scripts/test-check-push-result.sh
```

- [ ] **Step 5: Commit**

```bash
git add core/check-push.sh core/tests/scripts/test-check-push-result.sh
git commit -m "$(cat <<'EOF'
feat(check-push): emit RESULT TSV for hard-failed repos on stdout

Move human logs to stderr and exit 1 on --once when any worker hard-fails
so the controller can parse failures without scraping logs.
EOF
)"
```

---

### Task 2: SSH stream helper + RESULT parser

**Files:**
- Modify: `src/ssh.rs`
- Modify: `src/ops.rs`

**Interfaces:**
- Produces:
  - `ssh::ssh_run_inherit_stderr_capture_stdout(host, command, stdin) -> Result<(ExitStatus, String)>`
  - `ops::CheckPushReport { failed_repos: Vec<String> }`
  - `ops::parse_check_push_result(stdout: &str) -> CheckPushReport`
  - `ops::run_check_push_remote(...) -> Result<CheckPushReport>`

- [ ] **Step 1: Write failing unit tests**

In `src/ops.rs` (or a `#[cfg(test)]` module):

```rust
#[test]
fn parse_check_push_result_collects_fail_lines() {
    let stdout = "result\tfail\trepo-a\nresult\tfail\trepo-b\n";
    let report = parse_check_push_result(stdout);
    assert_eq!(report.failed_repos, vec!["repo-a", "repo-b"]);
}

#[test]
fn parse_check_push_result_ignores_noise_and_dedups() {
    let stdout = "hello\nresult\tfail\tx\nresult\tfail\tx\n";
    let report = parse_check_push_result(stdout);
    assert_eq!(report.failed_repos, vec!["x"]);
}

#[test]
fn parse_check_push_result_empty() {
    assert!(parse_check_push_result("").failed_repos.is_empty());
}
```

In `src/ssh.rs` tests:

```rust
#[test]
fn inherit_stderr_capture_stdout_localhost() {
    let h = host("localhost");
    let (status, out) = ssh_run_inherit_stderr_capture_stdout(
        &h,
        "echo err >&2; echo result$'\t'fail$'\t'r1; exit 1",
        b"",
    )
    .unwrap();
    assert!(!status.success());
    assert!(out.contains("result\tfail\tr1"));
}
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
cargo test --lib parse_check_push_result inherit_stderr_capture_stdout -- --nocapture
```

- [ ] **Step 3: Implement**

`src/ssh.rs`:

```rust
use std::process::ExitStatus;

/// Pipe stdin; inherit stderr (live logs); capture stdout.
pub fn ssh_run_inherit_stderr_capture_stdout(
    host: &Host,
    command: &str,
    stdin_data: &[u8],
) -> Result<(ExitStatus, String)> {
    let mut cmd = build_ssh_command(host)?;
    cmd.arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().context("Failed to execute ssh")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data)
            .context("Failed to write ssh stdin")?;
    }
    let output = child.wait_with_output().context("Failed to wait for ssh")?;
    // Note: wait_with_output with stderr inherit — verify on localhost that
    // stderr still streams. If the stdlib requires stderr piped for
    // wait_with_output, use: take stdout, spawn a reader thread, wait on child.
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok((output.status, stdout))
}
```

If `wait_with_output` conflicts with inherited stderr, use explicit:

```rust
let mut stdout_pipe = child.stdout.take().unwrap();
let handle = std::thread::spawn(move || {
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut stdout_pipe, &mut buf).ok();
    buf
});
// drop stdin already written
let status = child.wait()?;
let stdout = handle.join().unwrap_or_default();
Ok((status, stdout))
```

`src/ops.rs`:

```rust
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CheckPushReport {
    pub failed_repos: Vec<String>,
}

pub fn parse_check_push_result(stdout: &str) -> CheckPushReport {
    let mut set = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        let mut parts = line.split('\t');
        if parts.next() != Some("result") {
            continue;
        }
        if parts.next() != Some("fail") {
            continue;
        }
        if let Some(repo) = parts.next() {
            if !repo.is_empty() && parts.next().is_none() {
                set.insert(repo.to_string());
            }
        }
    }
    CheckPushReport {
        failed_repos: set.into_iter().collect(),
    }
}
```

Update `run_check_push_remote` to use the new SSH helper and return `Result<CheckPushReport>`. Map exit codes per spec:

```rust
pub fn run_check_push_remote(...) -> Result<CheckPushReport> {
    // ... build command as today ...
    let (status, stdout) = ssh::ssh_run_inherit_stderr_capture_stdout(host, &command, script.as_bytes())
        .context("run check-push on remote failed")?;
    let report = parse_check_push_result(&stdout);
    if status.success() {
        return Ok(report);
    }
    let code = status.code().unwrap_or(1);
    if code == 1 {
        // Caller may expand empty report to full whitelist.
        return Ok(report);
    }
    anyhow::bail!("ssh exited with {}: {}", status, stdout.trim())
}
```

Update call sites that expected `Result<()>` to handle `CheckPushReport` (temporary: `let _ = report` until Task 3), or fix compile errors in Task 3 immediately if preferred.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test --lib parse_check_push_result inherit_stderr_capture_stdout
```

- [ ] **Step 5: Commit**

```bash
git add src/ssh.rs src/ops.rs
git commit -m "$(cat <<'EOF'
feat: capture check-push RESULT lines while streaming stderr logs

Add ssh_run_inherit_stderr_capture_stdout and parse CheckPushReport from
result\\tfail\\t<repo> lines.
EOF
)"
```

---

### Task 3: Wire `deploy_failures` into the watch loop

**Files:**
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `ops::CheckPushReport`, `ops::run_check_push_remote -> Result<CheckPushReport>`
- Produces: cross-cycle `deploy_failures: HashMap<String, HashSet<String>>` merged into retry/whitelist logic.

- [ ] **Step 1: Write failing unit tests for merge helpers**

Add pure helpers (easy to test) in `src/lib.rs`:

```rust
fn merge_deploy_failures(
    deploy_failures: &mut HashMap<String, HashSet<String>>,
    host_id: &str,
    whitelist: &HashSet<String>,
    report: &ops::CheckPushReport,
    exit_was_fail_with_empty_result: bool,
) {
    let entry = deploy_failures.entry(host_id.to_string()).or_default();
    // Only repos in this run's whitelist (W) are updated.
    let failed: HashSet<String> = if exit_was_fail_with_empty_result {
        whitelist.clone()
    } else {
        report
            .failed_repos
            .iter()
            .filter(|r| whitelist.contains(*r))
            .cloned()
            .collect()
    };
    for repo in whitelist {
        if failed.contains(repo) {
            entry.insert(repo.clone());
        } else {
            entry.remove(repo);
        }
    }
    if entry.is_empty() {
        deploy_failures.remove(host_id);
    }
}

fn failure_set_for_host<'a>(
    probe_failed: &'a HashSet<String>,
    deploy_failures: &'a HashMap<String, HashSet<String>>,
    host_id: &str,
) -> HashSet<String> {
    let mut s = probe_failed.clone();
    if let Some(df) = deploy_failures.get(host_id) {
        s.extend(df.iter().cloned());
    }
    s
}
```

Tests:

```rust
#[test]
fn merge_deploy_failures_inserts_and_clears() {
    let mut df = HashMap::new();
    let wl: HashSet<_> = ["a", "b"].into_iter().map(String::from).collect();
    let report = ops::CheckPushReport {
        failed_repos: vec!["a".into()],
    };
    merge_deploy_failures(&mut df, "h1", &wl, &report, false);
    assert_eq!(df.get("h1").unwrap(), &HashSet::from(["a".into()]));
    let report_ok = ops::CheckPushReport { failed_repos: vec![] };
    merge_deploy_failures(&mut df, "h1", &wl, &report_ok, false);
    assert!(df.get("h1").is_none());
}

#[test]
fn merge_deploy_failures_empty_result_marks_whitelist() {
    let mut df = HashMap::new();
    let wl: HashSet<_> = ["a", "b"].into_iter().map(String::from).collect();
    let report = ops::CheckPushReport { failed_repos: vec![] };
    merge_deploy_failures(&mut df, "h1", &wl, &report, true);
    assert_eq!(df.get("h1").unwrap().len(), 2);
}
```

- [ ] **Step 2: Run tests — expect FAIL** (helpers missing)

```bash
cargo test --lib merge_deploy_failures -- --nocapture
```

- [ ] **Step 3: Implement helpers + wire `run_cycle` / `run_watch`**

1. Add `deploy_failures: &mut HashMap<String, HashSet<String>>` param to `run_cycle`.
2. When computing `failed_repos` for a host, use `failure_set_for_host(&failed_repos, deploy_failures, &host_id)` for `should_run_host_remote` and `effective_filter`.
3. After `run_check_push_remote`, call `merge_deploy_failures` with the whitelist set actually sent (`effective_filter` or full host repos).
4. For empty RESULT + exit 1: `run_check_push_remote` should expose whether the report was empty on failure — either return `CheckPushReport` always on exit 1 and let caller check `report.failed_repos.is_empty()`, or add a flag. Spec: empty RESULT + exit 1 → mark all of `W`.
5. Log warning when host runs due to deploy failures (mirror probe-failure warning).
6. In `run_watch`, create `let mut deploy_failures = HashMap::new();`, pass through `spawn_blocking` like `last_remote_refs` (take/return both maps).

Thread-scope note: today remote runs are `s.spawn` fire-and-forget. Change to collect reports (e.g. return `(host_id, whitelist, Result<CheckPushReport>)` via channel or scoped join handles) before merging into `deploy_failures` after the scope, **or** use a `Mutex` around `deploy_failures` inside the scope. Prefer collect-then-merge after `thread::scope` for clarity:

```rust
let mut outcomes = Vec::new();
std::thread::scope(|s| {
    // ...
    s.spawn(|| {
        let report = ops::run_check_push_remote(...);
        // send via mpsc or push under Mutex
    });
});
// merge outcomes into deploy_failures
```

Simplest robust pattern: `Mutex<Vec<(String, HashSet<String>, Result<CheckPushReport>)>>`.

- [ ] **Step 4: Run unit tests + `cargo test --lib`**

```bash
cargo test --lib merge_deploy_failures
cargo test --lib
```

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/ops.rs
git commit -m "$(cat <<'EOF'
feat(watch): retry repos that hard-failed on remote check-push

Track host-scoped deploy_failures across cycles and union them with
ls-remote probe failures when deciding remotes and whitelists.
EOF
)"
```

---

### Task 4: Integration smoke (localhost)

**Files:**
- Create or extend: `tests/integration_ssh.rs` (or new `tests/integration_check_push_result.rs`)

- [ ] **Step 1: Write a localhost test** that runs embedded check-push against a temp `DIR_BASE` with one unreachable-remote repo, asserts `CheckPushReport.failed_repos` contains that repo name, and that a second logical merge leaves it in `deploy_failures`.

If full integration is too heavy for CI, keep the shell test as the primary E2E and add only a Rust test calling `parse_check_push_result` + `merge_deploy_failures` on canned data — do **not** skip Task 1 shell coverage.

- [ ] **Step 2: Run**

```bash
cargo test --test integration_ssh
# and/or
bash core/tests/scripts/test-check-push-result.sh
```

- [ ] **Step 3: Commit if new test file added**

```bash
git add tests/
git commit -m "test: cover check-push RESULT reporting end-to-end"
```

---

## Spec coverage checklist

| Spec requirement | Task |
|------------------|------|
| Logs → stderr | Task 1 |
| RESULT TSV on stdout | Task 1 |
| FAIL_DIR per-worker files | Task 1 |
| `--once` exit 1 on hard fail | Task 1 |
| Live stderr inherit + stdout capture | Task 2 |
| `CheckPushReport` parse | Task 2 |
| `run_check_push_remote` returns report | Task 2 |
| `deploy_failures` cross-cycle | Task 3 |
| Union with probe `failed_repos` | Task 3 |
| Empty RESULT + exit 1 → mark whitelist | Task 3 |
| Soft failures unchanged | Task 1 (no code for them) |
| Shell + Rust tests | Tasks 1–4 |
