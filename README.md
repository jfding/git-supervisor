# Git Supervisor — monitor git repos and deploy to working environments

## Contents

git-supervisor: central controller to manage git repos deployments on remote hosts

## Download released binaries

Pre-built **git-supervisor** binaries are published on [GitHub Releases](https://github.com/jfding/git-supervisor/releases) for:

- **Linux** (x86_64): `git-supervisor-x86_64-unknown-linux-gnu-<tag>.tar.gz`
- **macOS** (Apple Silicon): `git-supervisor-aarch64-apple-darwin-<tag>.tar.gz`

**macOS:** If the binary is blocked or you see a security warning, clear extended attributes once after download:

```bash
xattr -c git-supervisor
```

## Usage

### Command `watch`

Run git-supervisor cli to watch status of repos on multiple hosts

- Specify the **command** as `/git-supervisor watch ...` for docker-run
- Additional args can be appended in above line
- To prepare the proper defined **deployments.yaml** for the target repos

#### Github Webhook settings

Webhook serving is now part of `watch` command, to enable it by:

Use `watch` with `--webhook-port` and a secret (`--webhook-secret` or `GITHUB_WEBHOOK_SECRET`).

```bash
# Watch loop + webhook server on :9870
/git-supervisor watch --webhook-port 9870 --webhook-secret MY_SECRET

# Secret from env var
GITHUB_WEBHOOK_SECRET=MY_SECRET /git-supervisor watch --webhook-port 8080
```

If `--webhook-port` is set without a secret, the command exits with an argument error.

#### Local mode (no deployments.yaml)

When `watch` cannot find a config file (`--config`, `~/.config/git-supervisor/deployments.yaml`,
or `./deployments.yaml`), it automatically falls back to local mode:

- Runs embedded `check-push.sh` locally (no SSH)
- Reuses `watch` flags: `--interval` and `--timeout`
- `--interval 0` means run once and exit
- Inherits `check-push.sh` env vars from current process (for example `DIR_BASE`, `BR_WHITELIST`, `LOGLEVEL`)
- If `REPO_WHITELIST` is exported but empty, it is treated as unset

```bash
# Local one-shot run (no config file present)
/git-supervisor watch --interval 0

# Local loop every 60s, stop after 10 minutes
/git-supervisor watch --interval 60 --timeout 600
```

#### Run by Docker

- Sample settings in docker-compose.yml in the code tree.
- Volume <work> to store all the data: git_repos, (code)copies, scripts.
- Volumn <keys> to store the ssh keys to access github.com repos.

#### (Legacy) Run original shell script loop to check status of repos on local

- Must set SLEEP_TIME env for docker-run, to specify the timeout values(seconds)
- Specify the **command** as `/srcripts/check-push.sh` for docker-run
- If no SLEEP_TIME env, the script will be run as one-shot checking.
- ENV **BR_WHITELIST**: Space-separated branch names to track and copy by default (e.g. `main master dev`). Override via env; default in script: `main master dev test alpha`. Whitelisted branches get their copy dir created and populated on first run; other branches are only tracked if a copy dir already exists (and then start with a `.skipping` flag until you remove it).

#### Docker restart pre/post hook jobs

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

## Development

### Design

The logic in the central check-push.sh script:

**Flow**

![Flow](docs/imgs/flowchart.png)

**Sequence**

![Sequence](docs/imgs/seqdiagram.png)

### Versioning

The project uses a single source of truth for version: the **`VERSION`** file at the repo root (e.g. `1.0.0`).

- **Scripts**: Run `check-push.sh --version` / `-V` prints it. In the Docker image, `VERSION` is copied to `/scripts/VERSION`.
- **supervisor** (Rust): Build reads `VERSION` from the repo root and sets the binary version; `supervisor --version` shows it. The `gh-webhook` subcommand serves `GET /version` and includes `version` in webhook responses. If `VERSION` is missing, `Cargo.toml` package version is used.

To set the version everywhere (e.g. for a release), run:

```bash
./scripts/set-version.sh 1.2.3
```

This updates `VERSION`, `supervisor/Cargo.toml`, etc.

### how to test

- first time to launch all tests: `./tests/launch-testing.sh`
- if testing env is ready, to run: `./tests/scripts/test-check-push.sh`
- to clean up test env, to run: `./tests/cleanup-test.sh`

After everytime to run the test scripts, the results can be checked in `./tests/work.test/copies/`
