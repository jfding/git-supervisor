use anyhow::{Context, Result};
use std::path::Path;

use crate::config::{Host, Repo};
use crate::ssh;

/// Escape a path for use inside single quotes in a remote shell.
/// Any single quote in the path becomes '\''.
fn escape_single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Check that `git` is available on the remote host (run `git --version`).
pub fn check_git_available(host: &Host) -> Result<()> {
    ssh::ssh_run(host, "git --version > /dev/null 2>&1").context("git not found or not runnable on remote (is git installed?)")
}

/// Check that `docker` is available on the remote host (run `docker --version`).
/// Returns Err if docker is not found or not runnable; used for optional warning only.
pub fn check_docker_available(host: &Host) -> Result<()> {
    ssh::ssh_run(host, "docker --version > /dev/null 2>&1").context("docker not found or not runnable")
}

/// Create dir_repos and dir_copies on the remote host.
pub fn create_dirs(
    host: &Host,
    dir_repos: &Path,
    dir_copies: &Path,
) -> Result<()> {
    let r = escape_single_quoted(&dir_repos.to_string_lossy());
    let c = escape_single_quoted(&dir_copies.to_string_lossy());
    let command = format!("mkdir -p {} {} 2>/dev/null", r, c);
    ssh::ssh_run(host, &command).context("create_dirs failed")
}

/// Ensure the repo exists on the remote: clone if missing; if `fetch_existing` is true, also fetch when repo exists.
/// dir_repos is the path to the git_repos directory on the remote.
pub fn ensure_repo(
    host: &Host,
    dir_repos: &Path,
    repo: &Repo,
    fetch_existing: bool,
) -> Result<()> {
    // Sanitize: name and git_url must not be used in shell eval. We pass them as
    // arguments to a single-quoted script fragment. The only way to get out of
    // single quotes is a closing quote, so we must not allow ' in name or git_url
    // when we embed them, or we escape. Use double quotes on remote and escape
    // any " and $ and ` and \ in the values.
    let dir = dir_repos.to_string_lossy();
    let name = &repo.name;
    let url = &repo.git_url;
    // Avoid injection: run a small script that uses the variables. We pass name and url
    // via the command string but we need to escape for the remote shell.
    // Simpler: use single-quoted script and close quote + pass safe args.
    // ssh host 'cd /work/git_repos && if [ ! -d name/.git ]; then git clone url name; else cd name && git fetch --all --tags --prune; fi'
    // So we need name and url substituted. If name/url contain ' we break. Replace ' in name/url with '\'' for remote.
    let name_esc = name.replace('\'', "'\\''");
    let url_esc = url.replace('\'', "'\\''");
    let dir_esc = dir.replace('\'', "'\\''");

    // Build remote command: cd to dir_repos, then clone if missing; optionally fetch if existing.
    let command = if fetch_existing {
        format!(
            "cd '{}' && \
if [ ! -d '{}/.git' ]; then \
  echo -n 'New repo [{}]: '; git clone '{}' '{}'; \
else \
  echo -n 'Existing repo [{}]: '; (cd '{}' && git fetch --all --tags --prune); \
fi",
            dir_esc,
            name_esc,
            name_esc,
            url_esc,
            name_esc,
            name_esc,
            name_esc,
        )
    } else {
        format!(
            "cd '{}' && \
if [ ! -d '{}/.git' ]; then \
  echo -n 'New repo [{}]: '; git clone '{}' '{}'; \
else \
  echo 'Existing repo [{}]: (ignored)'; \
fi",
            dir_esc,
            name_esc,
            name_esc,
            url_esc,
            name_esc,
            name_esc,
        )
    };

    ssh::ssh_run(host, &command).with_context(|| format!("clone & [optional]fetch {} failed", repo.name))
}

/// Sandbox env defaults for running check-push.sh on the remote (one-shot, no daemon loop).
const CHECK_PUSH_VERB: u8 = 1;
const CHECK_PUSH_TIMEOUT: u32 = 600;
const CHECK_PUSH_CI_LOCK: &str = "/tmp/.auto-reloader-lock.d";

/// Run the embedded check-push.sh script on the remote host with sandbox env.
/// dir_base is the host's work dir (e.g. /work); script runs with DIR_BASE set and --once.
pub fn run_check_push_remote(host: &Host, host_id: &str, dir_base: &Path, script: &str) -> Result<()> {
    let dir_base_esc = escape_single_quoted(&dir_base.to_string_lossy());
    // Export env vars then run script via stdin; script expects --once for one-shot.
    let command = format!(
        "env DIR_BASE={} VERB={} TIMEOUT={} SLEEP_TIME=0 CI_LOCK='{}' HOST_ID={} bash -s -- --once",
        dir_base_esc,
        CHECK_PUSH_VERB,
        CHECK_PUSH_TIMEOUT,
        CHECK_PUSH_CI_LOCK,
        host_id
    );
    ssh::ssh_run_with_stdin(host, &command, script.as_bytes())
        .context("run check-push on remote failed")
}
