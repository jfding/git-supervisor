# Git Supervisor — monitor git repos and deploy to working environments

git-supervisor: central controller to manage git repos deployments on remote hosts.

This cli tool will read a single YAML config and, for each configured remote host, creates dirs and prepares git repos (clone or fetch) over SSH.

## Download released binaries

Pre-built **git-supervisor** binaries are published on [GitHub Releases](https://github.com/jfding/git-supervisor/releases) for:

- **Linux** (x86_64): `git-supervisor-x86_64-unknown-linux-gnu-<tag>.tar.gz`
- **macOS** (Apple Silicon): `git-supervisor-aarch64-apple-darwin-<tag>.tar.gz`

**macOS:** If the binary is blocked or you see a security warning, clear extended attributes once after download:

```bash
xattr -c git-supervisor
```

## YAML config schema

- **Top level:** `defaults` (optional), `repos` (optional), `hosts` (required).
- **Defaults:** `dir_base`, `branches`.
- **Repos:** map of repo name → definition (`git_url` only). Hosts reference these by name. Branches are not set here.
- **Per host:** `ssh_target` (e.g. `user@host`), optional `ssh_port`, `ssh_identity_file`, `dir_base`, optional `release_count`, optional `release_tag_pattern`, optional `release_tag_exclude_pattern`; `repos`: list of repo names or `{ name, branches? }` entries. Branches are configured only here (per host, per repo). When set, `release_count` is passed as env `RELEASE_TAG_TOPN` (script default 4). When set, `release_tag_pattern` and `release_tag_exclude_pattern` are passed as `RELEASE_TAG_PATTERN` and `RELEASE_TAG_EXCLUDE_PATTERN` (ERE; script default pattern: `^v[0-9Q.]+$`).

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
    release_count: 8    # optional, default 4
    release_tag_pattern: "^v[0-9]+\\.0$"      # optional; passed as RELEASE_TAG_PATTERN
    release_tag_exclude_pattern: "^v0\\."     # optional; passed as RELEASE_TAG_EXCLUDE_PATTERN
    repos:
      - webapp
      - name: api
        branches: [main, release]
```

## Usage

```bash
# Check config, SSH/git connectivity, and repo existence on remotes
git-supervisor check [--config deployments.yaml]

# Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop
git-supervisor watch [--config deployments.yaml] [--interval SECS] [--timeout SECS] [-I | --ignore-missing]
                     [--webhook-secret SECRET] [--webhook-port PORT]
```

- Config is an optional argument to each subcommand; default: `deployments.yaml`.
- **check**: load and validate the config, then for each host verify SSH/git is available and that each configured repo directory exists under `dir_repos` with a `.git` directory.
- **watch**: first prepares each remote (create dirs, init empty repos by cloning when missing unless `-I`/`--ignore-missing`). Then, on the supervisor machine, it polls upstream refs for all configured repos. It only runs remote `check-push` on hosts whose configured repos have upstream changes (first round always runs on all hosts). `--interval` (default 120) controls polling cadence, optional `--timeout` stops after SECS, `-I`/`--ignore-missing` skips cloning (only create dirs; missing repos are ignored). Run until Ctrl+C if no timeout.
- Remotes must have **SSH** access (key-based) and **git** installed. For local hosts (`localhost`, `127.0.0.1`, `::1`, including forms like `user@localhost`), supervisor runs commands directly on the local machine and does not require an SSH daemon.

### GitHub Webhook settings

Use `watch` with `--webhook-port` and a secret (`--webhook-secret` or `GITHUB_WEBHOOK_SECRET`).

```bash
# Watch loop + webhook server on :9870
git-supervisor watch --webhook-port 9870 --webhook-secret MY_SECRET

# Secret from env var
GITHUB_WEBHOOK_SECRET=MY_SECRET git-supervisor watch --webhook-port 8080
```

If `--webhook-port` is set without a secret, a warning is printed and webhook listening is skipped.

### Local mode (no deployments.yaml)

When `watch` cannot find a config file (`--config`, `~/.config/git-supervisor/deployments.yaml`,
or `./deployments.yaml`), it automatically falls back to local mode:

- Runs embedded `check-push.sh` locally (no SSH)
- Reuses `watch` flags: `--interval` and `--timeout`
- `--interval 0` means run once and exit
- Inherits `check-push.sh` env vars from current process (for example `DIR_BASE`, `BR_WHITELIST`, `LOGLEVEL`)
- If `REPO_WHITELIST` is exported but empty, it is treated as unset

```bash
# Local one-shot run (no config file present)
git-supervisor watch --interval 0

# Local loop every 60s, stop after 10 minutes
git-supervisor watch --interval 60 --timeout 600
```

### Run by Docker

- Sample settings in docker-compose.yml in the code tree.
- Volume `<work>` to store all the data: git_repos, (code)copies, scripts.
- Volume `<keys>` to store the ssh keys to access github.com repos.

### Dot file triggers in copy directories

Inside each copy directory (`<dir_base>/copies/<repo>.<branch>/`), several dot files control the behavior of the `check-push` update cycle. **User-managed** files are placed or removed by the operator; **auto-managed** files are written by the script itself.

| Dot file | Managed by | Purpose |
|---|---|---|
| `.skipping` | user | **Skip updates.** The branch copy is ignored during update cycles. Created automatically for non-whitelisted branches on first init; remove it to opt-in to updates. |
| `.debugging` | user | **Freeze for debugging.** Prevents any update to the copy, even if upstream has new commits. Place it when you need to inspect or modify the copy without it being overwritten. |
| `.trigger` | user | **Force an update.** Triggers a refresh of the copy even when no upstream changes are detected. Burn-after-reading: the script deletes it once consumed. |
| `.no-cleanup` | user | **Overlay-only updates.** Normally an update does a full refresh (remove old files, extract fresh). With this file present, new files are extracted on top of the existing copy without deleting extra files. |
| `.stopping` | user | **Mark for deprecation.** The script removes all content in the directory, then recreates it with `.skipping` so no further updates occur. Use this to retire a branch copy cleanly. |
| `.git-rev` | auto | Stores the deployed commit hash. Used to detect whether upstream has new commits since the last update. |
| `.living` | auto | Heartbeat marker written each cycle after a branch/release is processed. Copies without `.living` at cleanup time are considered stale and moved to `*.to-be-removed`. |

All dot files inside a copy directory are **preserved across updates** (both full-refresh and overlay modes).


### Docker restart and pre/post hook jobs

When a copy path has a docker restart config file (`*.docker`), `check-push.sh` can run optional hook jobs around restart:

- **Pre hook**: `*.docker.pre` runs before `docker restart`
- **Post hook**: `*.docker.post` runs after a successful `docker restart`

Examples:

- Branch copy: `/work/copies/webapp.main.docker` + optional `/work/copies/webapp.main.docker.pre` / `/work/copies/webapp.main.docker.post`
- Latest release copy: `/work/copies/webapp.prod.docker` + optional `/work/copies/webapp.prod.docker.pre` / `/work/copies/webapp.prod.docker.post`

Hook job scripts are executed with `bash`, from the copy directory as working directory, and receive:

- `DOCKER_HOOK_STAGE` (`pre` or `post`)
- `DOCKER_NAME` (container name from `*.docker`)
- `DOCKER_HOOK_FILE` (resolved hook script path)

### (Legacy) How to run original shell script loop

Besides running `watch` command without any deployments config, which will do the same work as the original shell check-push.sh script.
And the script was also removed from the release artifacts, but there does have a method to find it back:

```
git-supervisor print-script
```

## Development

### Design

The logic in the central check-push.sh script:

**Flow**

![Flow](docs/imgs/flowchart.png)

**Sequence**

![Sequence](docs/imgs/seqdiagram.png)

### Versioning

The project version is defined solely in `Cargo.toml`. `git-supervisor --version` reflects it directly.

To bump the version (updates `Cargo.toml`, `Cargo.lock`, and the docker-compose image tag), run:

```bash
./scripts/set-version.sh 1.2.3
```

### Testing

- Run `cargo teset` to run the basic test cases
- And `cargo test -- --ignored` to run embedded shell script integration testings
- Or to run the shell script testing directly:
    - First time to launch all tests: `./core/tests/launch-testing.sh`
    - If testing env is ready, to run: `./core/tests/scripts/test-check-push.sh`
    - To clean up test env, to run: `./core/tests/cleanup-test.sh`
