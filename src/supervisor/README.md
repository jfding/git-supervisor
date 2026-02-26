# supervisor

Central supervisor for the auto-reloader: reads a single YAML config and, for each configured remote host, creates dirs and prepares git repos (clone or fetch) over SSH. The supervisor does **not** start or restart the check-push daemon on remotes; that is left to systemd or your process manager.

See the [design doc](../../docs/plans/2025-02-22-central-supervisor-design.md) for architecture and flow.

## YAML schema

- **Top level:** `defaults` (optional), `hosts` (required).
- **Defaults:** `dir_base`, `branch_whitelist` (for documentation/future use).
- **Per host:** `ssh_target` (e.g. `user@host`), optional `ssh_port`, `ssh_identity_file`, `dir_base`; `repos` (list of repo entries).
- **Per repo:** `name`, `git_url`, optional `branch_whitelist`.

Example:

```yaml
defaults:
  dir_base: /work

hosts:
  app-server:
    ssh_target: deploy@app-server.example.com
    ssh_identity_file: ~/.ssh/deploy_key
    repos:
      - name: webapp
        git_url: git@github.com:org/webapp.git
```

## Build

```bash
cargo build --release
```

Binary: `target/release/supervisor`.

## Run

```bash
# Validate config and print what would be done (no SSH)
supervisor validate [CONFIG]

# Push to remotes: create dirs and ensure repos
supervisor push [CONFIG]
```

- Config is an optional argument to each subcommand; default: `deployments.yaml`.
- **validate**: load and validate the config, then print what push would do per host (no SSH).
- **push**: create dirs and ensure repos on each remote over SSH.
- Remotes must have **SSH** access (key-based) and **git** installed. The supervisor only creates `dir_repos`/`dir_copies` and ensures each listed repo is cloned or fetched; it does not push any daemon config or start the daemon.

## Integration test (optional)

`cargo test --test integration_ssh` runs an integration test that uses SSH to localhost. It is marked `#[ignore]` by default. To run it: `cargo test --test integration_ssh -- --ignored`. Requires `ssh localhost` to work and a temporary directory.
