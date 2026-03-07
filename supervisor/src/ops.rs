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

/// Ensure the repo exists on the remote: clone if missing unless `ignore_missing` is true
/// dir_repos is the path to the git_repos directory on the remote.
pub fn ensure_repo(
    host: &Host,
    dir_repos: &Path,
    repo: &Repo,
    ignore_missing: bool,
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

    // Build remote command: cd to dir_repos, then clone if missing
    let command = if !ignore_missing {
        format!(
            "cd '{}' && \
if [ ! -d '{}/.git' ]; then \
  echo -n '    New repo [{}]: '; git clone '{}' '{}'; \
else \
  echo '    Existing repo [{}]: (ready)'; \
fi",
            dir_esc,
            name_esc,
            name_esc,
            url_esc,
            name_esc,
            name_esc,
        )
    } else {
        format!(
            "cd '{}' && \
if [ ! -d '{}/.git' ]; then \
  echo '    Missing repo [{}]: (ignored)'; \
else \
  echo '    Existing repo [{}]: (ready)'; \
fi",
            dir_esc,
            name_esc,
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

/// Env options for running check-push on a host (REPO_WHITELIST, BR_WHITELIST_PER_REPO, RELEASE_TAG_*).
#[derive(Default)]
pub struct CheckPushEnv {
    pub repo_whitelist: Option<String>,
    pub repo_branches: Option<String>,
    pub release_tag_topn: Option<u32>,
    pub release_tag_pattern: Option<String>,
    pub release_tag_exclude_pattern: Option<String>,
}

fn build_check_push_extra_env(env: &CheckPushEnv) -> String {
    let mut env_parts: Vec<String> = Vec::new();
    if let Some(s) = &env.repo_whitelist {
        env_parts.push(format!("REPO_WHITELIST={}", escape_single_quoted(s)));
    }
    if let Some(s) = &env.repo_branches {
        env_parts.push(format!("BR_WHITELIST_PER_REPO={}", escape_single_quoted(s)));
    }
    if let Some(n) = env.release_tag_topn {
        env_parts.push(format!("RELEASE_TAG_TOPN={}", n));
    }
    if let Some(s) = &env.release_tag_pattern {
        env_parts.push(format!("RELEASE_TAG_PATTERN={}", escape_single_quoted(s)));
    }
    if let Some(s) = &env.release_tag_exclude_pattern {
        env_parts.push(format!("RELEASE_TAG_EXCLUDE_PATTERN={}", escape_single_quoted(s)));
    }
    if env_parts.is_empty() {
        String::new()
    } else {
        format!(" {}", env_parts.join(" "))
    }
}

/// Run the embedded check-push.sh script on the remote host with sandbox env.
/// dir_base is the host's work dir (e.g. /work); script runs with DIR_BASE set and --once.
/// env supplies REPO_WHITELIST, BR_WHITELIST_PER_REPO, RELEASE_TAG_* when set.
pub fn run_check_push_remote(
    host: &Host,
    host_id: &str,
    dir_base: &Path,
    script: &str,
    env: &CheckPushEnv,
) -> Result<()> {
    let dir_base_esc = escape_single_quoted(&dir_base.to_string_lossy());
    let host_id_esc = escape_single_quoted(host_id);
    let extra = build_check_push_extra_env(env);

    // Export env vars then run script via stdin; script expects --once for one-shot.
    let command = format!(
        "env DIR_BASE={} VERB={} TIMEOUT={} SLEEP_TIME=0 CI_LOCK='{}' HOST_ID='{}'{} bash -s -- --once",
        dir_base_esc,
        CHECK_PUSH_VERB,
        CHECK_PUSH_TIMEOUT,
        CHECK_PUSH_CI_LOCK,
        host_id_esc,
        extra
    );
    ssh::ssh_run_with_stdin(host, &command, script.as_bytes())
        .context("run check-push on remote failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_check_push_extra_env_includes_release_tag_topn() {
        let env = CheckPushEnv {
            release_tag_topn: Some(5),
            ..Default::default()
        };
        let extra = build_check_push_extra_env(&env);
        assert!(
            extra.contains("RELEASE_TAG_TOPN=5"),
            "extra env should include RELEASE_TAG_TOPN=5, got: {:?}",
            extra
        );
    }

    #[test]
    fn build_check_push_extra_env_omit_release_tag_topn_when_none() {
        let extra = build_check_push_extra_env(&CheckPushEnv::default());
        assert!(
            !extra.contains("RELEASE_TAG_TOPN"),
            "extra env should not include RELEASE_TAG_TOPN when None, got: {:?}",
            extra
        );
    }

    #[test]
    fn build_check_push_extra_env_includes_release_tag_patterns() {
        let env = CheckPushEnv {
            release_tag_pattern: Some("^v[0-9]+\\.0$".into()),
            release_tag_exclude_pattern: Some("^v0\\.".into()),
            ..Default::default()
        };
        let extra = build_check_push_extra_env(&env);
        assert!(
            extra.contains("RELEASE_TAG_PATTERN="),
            "extra env should include RELEASE_TAG_PATTERN, got: {:?}",
            extra
        );
        assert!(
            extra.contains("RELEASE_TAG_EXCLUDE_PATTERN="),
            "extra env should include RELEASE_TAG_EXCLUDE_PATTERN, got: {:?}",
            extra
        );
    }
}
