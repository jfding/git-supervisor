# Design: remote hard-fail repo reporting for check-push

Date: 2026-07-23

## Goal

After each remote `check-push.sh --once` run, the controller must know **which
repos hard-failed**, and retry those repos on the next watch cycle the same way
it already retries `ls-remote` probe failures (`failed_repos`).

Today the controller only sees a vague host-level outcome (often not even that):
parallel workers set `_worker_failed=1` and log `"one or more repo workers
failed"`, `--once` still exits `0`, and `run_check_push_remote` uses
`ssh_run_with_stdin` (inherit streams, success/fail only).

## Decisions (from brainstorming)

| Topic | Choice |
|-------|--------|
| Primary consumer | **Controller retry** (not operator-only logging) |
| Failure scope (v1) | **Hard failures only** (`return 1` from `fetch_and_check`: invalid name, `cd` fail, `git fetch` fail). Soft failures (copy `continue`, docker “ignoring”) deferred. |
| Transport | **Approach 2:** human logs on **stderr**, machine RESULT lines on **stdout** |
| Live logs | Yes — stderr inherited so operators still see streaming output |

## Non-goals (v1)

- Soft-failure reporting (copy / docker)
- Per-branch or per-tag failure detail
- JSON payloads
- Changing daemon-loop control flow beyond emitting RESULT lines each round
- Reworking `run_check_push_local` retry state (optional reuse of the parser later)

## Wire protocol

### Channels

| Stream | Content |
|--------|---------|
| stderr | All human logs (`info` / `warn` / `err` / …) — live via SSH inherit |
| stdout | Only machine RESULT lines (empty on full success) |

### RESULT schema

One TSV line per hard-failed repo (no header), same spirit as status/cleanup probes:

```text
result<TAB>fail<TAB><repo>
```

- Full success → empty stdout, exit `0`.
- Repo names are already constrained by `_unsafe_path_segment` (no tabs/newlines).
- No reason column in v1 (may add as column 4 later).

### Exit code (`--once`)

| Code | Meaning |
|------|---------|
| `0` | No hard failures |
| `1` | One or more hard failures (RESULT lines also present when known) |
| other | Infrastructure abort (lock, missing `DIR_REPOS`, etc.); stdout may be empty |

Daemon mode (no `--once`): emit the same RESULT lines each round; keep looping.
Exit code only matters for `--once` / the controller.

Standalone operators: logs still appear on a TTY (stderr). Scripts that previously
captured stdout for “all output” may need `2>&1`.

## `check-push.sh` changes

### Logging

`_logging` (and any other human-facing `printf` that is not a RESULT line) writes
to **stderr** (`>&2`).

### Parallel-safe failure collection

1. Before spawning workers: create `FAIL_DIR=$(mktemp -d)` (under `$TMPDIR` or
   adjacent to `CI_LOCK`).
2. Each background worker: if `fetch_and_check` returns non-zero, write
   `printf '%s\n' "$_repo" > "$FAIL_DIR/$_repo"` (one file per repo — no
   concurrent append races).
3. After all `wait`s: for each file in `FAIL_DIR`, print
   `result\tfail\t$repo` on **stdout**; remember that hard failures occurred.
4. `rm -rf "$FAIL_DIR"`.
5. `--once`: `exit 1` if any hard failure, else `exit 0`.

Hard fail = today’s `return 1` from `fetch_and_check` only.

### Unknown worker failure

If `wait` fails but no file appears in `FAIL_DIR` (e.g. worker killed before
writing), still treat the round as failed (`exit 1`). The controller then marks
**all repos in this run’s whitelist** as failed for that host (see below).

## Controller / SSH

### New helper

Alongside `ssh_run_with_stdin` / `ssh_run_capture`:

```text
ssh_run_stream_result(host, cmd, stdin) -> Result<(exit_status, stdout_string)>
```

- stderr → inherit (live logs)
- stdout → piped and fully drained
- Returns captured stdout and the process exit status (do not auto-bail solely
  on exit `1` — caller interprets RESULT lines)

Alternatively a thin wrapper that parses immediately; either is fine as long as
live stderr inherit is preserved.

### Types

```rust
pub struct CheckPushReport {
    pub failed_repos: Vec<String>, // sorted, unique
}
```

Parser: keep lines matching `^result\tfail\t([^\t]+)$`; ignore anything else on
stdout (defensive). Drop RESULT repos not in the host’s configured / whitelisted
set.

### `run_check_push_remote`

Returns `Result<CheckPushReport>` instead of `Result<()>`.

Interpretation:

| Exit | RESULT lines | Outcome |
|------|--------------|---------|
| 0 | empty | `Ok(CheckPushReport { failed_repos: [] })` |
| 1 | non-empty | `Ok(report)` with parsed failures |
| 1 | empty | Treat as host-level failure: mark **all repos in this run’s whitelist** as failed |
| SSH/spawn failure | — | `Err(...)` (do not clear prior `deploy_failures` for that host) |
| other non-zero | — | `Err(...)` or same whitelist-all fallback; prefer `Err` for infra |

`run_check_push_local` may stay fire-and-forget in v1.

## Watch-loop retry wiring

Mirror the cross-cycle role of `last_remote_refs`:

```text
deploy_failures: HashMap<String /* host_id */, HashSet<String /* repo */>>
```

Host-scoped: the same repo name can fail on one host and succeed on another.

### Per cycle

1. When building `should_run_host_remote` / `effective_filter`, union this host’s
   `deploy_failures` with probe `failed_repos` (same path as today).
2. After each remote run for host `H` with whitelist `W`:
   - For each repo in `report.failed_repos` ∩ configured repos → insert into
     `deploy_failures[H]`.
   - For each repo in `W` that is **not** in `report.failed_repos` and the run
     was a parseable success/fail report → remove from `deploy_failures[H]`
     (cleared on successful deploy attempt).
   - Empty-stdout + exit 1 → insert all of `W` into `deploy_failures[H]`.
3. Log a warning when a host runs solely (or partly) because of deploy failures,
   analogous to the existing “probe failures” warning.

Webhook / first round: still send the full whitelist; still update
`deploy_failures` from the report so the next timer round can narrow retries.

`deploy_failures` lives in `run_watch` and is passed into / returned from
`run_cycle` the same way as `last_remote_refs`.

## Edge cases

| Case | Behavior |
|------|----------|
| RESULT repo not in config | Ignore line |
| Concurrent hosts | Independent SSH sessions; no shared remote state |
| Empty whitelist / no repos | No RESULT, exit 0 |
| Soft failures | Unchanged; do not write FAIL_DIR / RESULT |

## Testing

- **Shell:** force a hard fail → RESULT line on stdout + exit 1; all ok → empty
  stdout + exit 0; human logs on stderr only.
- **Rust unit:** parse RESULT lines; ignore non-matching stdout; whitelist filter.
- **Integration (if feasible):** localhost remote run with forced fetch failure →
  `deploy_failures` drives next-cycle whitelist.

## Follow-ups (out of scope)

- Soft-failure RESULT lines (`result\tsoft\t<repo>\t<reason>`).
- Reason column on hard fails.
- Feed local-watch the same report type.
