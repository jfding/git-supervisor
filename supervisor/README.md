# supervisor

Central supervisor for the auto-reloader: reads a single YAML config and, for each configured remote host, creates dirs and prepares git repos (clone or fetch) over SSH. The supervisor does **not** start or restart the check-push daemon on remotes; that is left to systemd or your process manager.

See the [design doc](../../docs/plans/2025-02-22-central-supervisor-design.md) for architecture and flow.

## YAML schema

- **Top level:** `defaults` (optional), `repos` (optional), `hosts` (required).
- **Defaults:** `dir_base`, `branches` (optional; used when a host repo entry doesn't set branches).
- **Repos:** map of repo name → definition (`git_url` only). Hosts reference these by name. Branches are not set here.
- **Per host:** `ssh_target` (e.g. `user@host`), optional `ssh_port`, `ssh_identity_file`, `dir_base`, optional `release_count`; `repos`: list of repo names or `{ name, branches? }` entries. Branches are configured only here (per host, per repo). When set, `release_count` is passed to the remote check-push script as env `RELEASE_TAG_TOPN` (number of release tags to consider; script default is 4).

Example:

```yaml
defaults:
  dir_base: /work
  branches: [main, master]

repos:
  webapp:
    git_url: git@github.com:org/webapp.git
  api:
    git_url: git@github.com:org/api.git

hosts:
  app-server:
    ssh_target: deploy@app-server.example.com
    ssh_identity_file: ~/.ssh/deploy_key
    release_count: 8   # optional; passed as RELEASE_TAG_TOPN on remote
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
```

- Config is an optional argument to each subcommand; default: `deployments.yaml`.
- **check**: load and validate the config, then for each host verify SSH/git is available and that each configured repo directory exists under `dir_repos` with a `.git` directory.
- **watch**: first prepares each remote (create dirs, init empty repos by cloning when missing unless `-I`/`--ignore-missing`), then repeatedly runs the check-push script on each host; `--interval` (default 120) seconds between rounds, optional `--timeout` to stop after SECS seconds, `-I`/`--ignore-missing` to skip cloning (only create dirs; missing repos are ignored). Run until Ctrl+C if no timeout.
- Remotes must have **SSH** access (key-based) and **git** installed. The supervisor only creates `dir_repos`/`dir_copies` and ensures each listed repo is cloned or fetched; it does not push any daemon config or start the daemon.

## Integration test (optional)

`cargo test --test integration_ssh` runs an integration test that uses SSH to localhost. It is marked `#[ignore]` by default. To run it: `cargo test --test integration_ssh -- --ignored`. Requires `ssh localhost` to work and a temporary directory.
