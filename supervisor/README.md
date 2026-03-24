# supervisor

Central supervisor for Git Supervisor: reads a single YAML config and, for each configured remote host, creates dirs and prepares git repos (clone or fetch) over SSH. The supervisor does **not** start or restart the check-push daemon on remotes; that is left to systemd or your process manager.

See the [design doc](../../docs/plans/2025-02-22-central-supervisor-design.md) for architecture and flow.

## YAML schema

- **Top level:** `defaults` (optional), `repos` (optional), `hosts` (required).
- **Defaults:** `dir_base`, `branches`, optional `log-level` (passed to the remote script as `LOGLEVEL`; script default 2 when omitted).
- **Repos:** map of repo name → definition (`git_url` only). Hosts reference these by name. Branches are not set here.
- **Per host:** `ssh_target` (e.g. `user@host`), optional `ssh_port`, `ssh_identity_file`, `dir_base`, optional `release_count`, optional `release_tag_pattern`, optional `release_tag_exclude_pattern`; `repos`: list of repo names or `{ name, branches? }` entries. Branches are configured only here (per host, per repo). When set, `release_count` is passed as env `RELEASE_TAG_TOPN` (script default 4). When set, `release_tag_pattern` and `release_tag_exclude_pattern` are passed as `RELEASE_TAG_PATTERN` and `RELEASE_TAG_EXCLUDE_PATTERN` (ERE; script default pattern: `^v[0-9Q.]+$`).

Example:

```yaml
defaults:
  dir_base: /work
  branches: [main, master]
  log-level: 2

repos:
  webapp:
    git_url: git@github.com:org/webapp.git
  api:
    git_url: git@github.com:org/api.git

hosts:
  app-server:
    ssh_target: deploy@app-server.example.com
    ssh_identity_file: ~/.ssh/deploy_key
    release_count: 8    # optional, default 4
    release_tag_pattern: "^v[0-9]+\\.0$"      # optional; passed as RELEASE_TAG_PATTERN
    release_tag_exclude_pattern: "^v0\\."     # optional; passed as RELEASE_TAG_EXCLUDE_PATTERN
    repos:
      - webapp
      - name: api
        branches: [main, release]
```

## Build

```bash
cargo build --release
```

Binary: `target/release/supervisor`.

## Run

```bash
# Check config, SSH/git connectivity, and repo existence on remotes
supervisor check [CONFIG]

# Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop
supervisor watch [CONFIG] [--interval SECS] [--timeout SECS] [-I | --ignore-missing]

# Start GitHub webhook server that triggers check-push on push events
supervisor hook [CONFIG] --secret SECRET [--port PORT] [--script PATH]
```

- Config is an optional argument to each subcommand; default: `deployments.yaml`.
- **check**: load and validate the config, then for each host verify SSH/git is available and that each configured repo directory exists under `dir_repos` with a `.git` directory.
- **watch**: first prepares each remote (create dirs, init empty repos by cloning when missing unless `-I`/`--ignore-missing`), then repeatedly runs the check-push script on each host; `--interval` (default 120) seconds between rounds, optional `--timeout` to stop after SECS seconds, `-I`/`--ignore-missing` to skip cloning (only create dirs; missing repos are ignored). Run until Ctrl+C if no timeout.
- **hook**: starts an HTTP server (default port `9870`) that listens for GitHub webhook push events. Verifies `X-Hub-Signature-256` using the provided `--secret` (or `GITHUB_WEBHOOK_SECRET` env var). On push events, runs a one-shot watch cycle on all configured hosts (equivalent to `watch --interval 0 --skip-prepare`). Use `--script PATH` to run an external script instead (e.g. `/scripts/check-push.sh`). Endpoints: `GET /version` returns the app version; `POST /webhook` handles GitHub events.
- Remotes must have **SSH** access (key-based) and **git** installed. The supervisor only creates `dir_repos`/`dir_copies` and ensures each listed repo is cloned or fetched; it does not push any daemon config or start the daemon.

## Integration test (optional)

`cargo test --test integration_ssh` runs an integration test that uses SSH to localhost. It is marked `#[ignore]` by default. To run it: `cargo test --test integration_ssh -- --ignored`. Requires `ssh localhost` to work and a temporary directory.
