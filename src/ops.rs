use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::{Host, Repo};
use crate::console::{self, Color};
use crate::ssh;

/// Escape a path for use inside single quotes in a remote shell.
/// Any single quote in the path becomes '\''.
fn escape_single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Wrap a remote `printf` fragment so it completes and flushes before the next command.
/// A bare builtin `printf` can leave the shell's stdout buffered; a subshell exits after
/// `printf`, flushing its stdio, and stderr is usually unbuffered so status lines are not
/// torn by `git` output or interleaved oddly when multiple SSH sessions share a terminal.
fn shell_printf_flush(fragment: String) -> String {
    format!("({}) >&2", fragment)
}

/// Check that `tool` exists on the host via `command -v`.
/// Local targets run the check directly; remote targets use SSH.
fn check_tool_available(host: &Host, tool: &str) -> Result<()> {
    let cmd = format!("command -v {} > /dev/null 2>&1", tool);
    if ssh::is_local_ssh_target(&host.ssh_target) {
        let status = Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .with_context(|| format!("{} not found in local host", tool))?;
        if status.success() {
            Ok(())
        } else {
            anyhow::bail!("{} not found in local host", tool);
        }
    } else {
        ssh::ssh_run(host, &cmd)
            .with_context(|| format!("{} not found in remote host", tool))
    }
}

/// Check that `git` is available on the host.
pub fn check_git_available(host: &Host) -> Result<()> {
    check_tool_available(host, "git")
}

/// Check that `docker` is available on the host.
pub fn check_docker_available(host: &Host) -> Result<()> {
    check_tool_available(host, "docker")
}

/// Create dir_repos and dir_copies on the remote host.
pub fn create_dirs(host: &Host, dir_repos: &Path, dir_copies: &Path) -> Result<()> {
    let r = escape_single_quoted(&dir_repos.to_string_lossy());
    let c = escape_single_quoted(&dir_copies.to_string_lossy());
    let command = format!("mkdir -p {} {} 2>/dev/null", r, c);
    ssh::ssh_run(host, &command).context("create_dirs failed")
}

/// Ensure the repo exists on the remote: clone if missing unless `ignore_missing` is true
/// dir_repos is the path to the git_repos directory on the remote.
/// `github_ssh_key` is an optional path (on the remote host) to the SSH key used for GitHub access.
pub fn ensure_repo(host: &Host, dir_repos: &Path, repo: &Repo, ignore_missing: bool, github_ssh_key: Option<&str>) -> Result<()> {
    // Sanitize: name and git_url must not be used in shell eval. We pass them as
    // arguments to a single-quoted script fragment. The only way to get out of
    // single quotes is a closing quote, so we must not allow ' in name or git_url
    // when we embed them, or we escape. Use double quotes on remote and escape
    // any " and $ and ` and \ in the values.
    let dir = dir_repos.to_string_lossy();
    // Use the name derived from the git URL as the clone directory, not the config alias.
    let clone_dir = repo.dir_name();
    let url = &repo.git_url;
    let clone_dir_esc = clone_dir.replace('\'', "'\\''");
    let url_esc = url.replace('\'', "'\\''");
    let dir_esc = dir.replace('\'', "'\\''");
    // Prefix for git commands: sets GIT_SSH_COMMAND when a GitHub key is provided.
    let git_prefix = github_ssh_key
        .map(|k| shell_git_ssh_command_prefix(k))
        .unwrap_or_default();

    // Build remote command: cd to dir_repos, then clone if missing
    let command = if !ignore_missing {
        let new_repo_line = shell_printf_flush(console::shell_printf_inline(
            &format!("    New repo [{}]: ", clone_dir),
            Some(Color::Green),
        ));
        let existing_repo_line = shell_printf_flush(console::shell_printf(
            &format!("    Existing repo [{}]: (ready)", clone_dir),
            None,
        ));

        format!(
            "cd '{}' && \
if [ ! -d '{}/.git' ]; then \
  {}; {}git clone '{}' '{}'; \
else \
  {}; \
fi",
            dir_esc, clone_dir_esc, new_repo_line, git_prefix, url_esc, clone_dir_esc, existing_repo_line,
        )
    } else {
        let missing_repo_line = shell_printf_flush(console::shell_printf(
            &format!("    Missing repo [{}]: (ignored)", clone_dir),
            Some(Color::Yellow),
        ));
        let existing_repo_line = shell_printf_flush(console::shell_printf(
            &format!("    Existing repo [{}]: (ready)", clone_dir),
            Some(Color::Green),
        ));
        format!(
            "cd '{}' && \
if [ ! -d '{}/.git' ]; then \
  {}; \
else \
  {}; \
fi",
            dir_esc, clone_dir_esc, missing_repo_line, existing_repo_line,
        )
    };

    ssh::ssh_run(host, &command)
        .with_context(|| format!("clone & [optional]fetch {} failed", repo.name))
}

/// Build a `GIT_SSH_COMMAND` assignment for the remote shell that forces git to use `key`.
/// Leading `~/` is converted to `$HOME/` so the remote shell expands it correctly.
/// The result is ready to prepend to a shell command, e.g. `"<result> git clone ..."`.
fn shell_git_ssh_command_prefix(key: &str) -> String {
    let key_path = if key.starts_with("~/") {
        format!("$HOME/{}", &key[2..])
    } else if key == "~" {
        "$HOME".to_string()
    } else {
        key.to_string()
    };
    // Escape for a double-quoted shell string (allow $HOME to expand).
    let escaped = key_path
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`");
    format!(
        "GIT_SSH_COMMAND=\"ssh -o StrictHostKeyChecking=no -i {}\" ",
        escaped
    )
}

/// Sandbox env defaults for running check-push.sh on the remote (one-shot, no daemon loop).
const CHECK_PUSH_VERB: u8 = 1;
const CHECK_PUSH_TIMEOUT: u32 = 600;
const CHECK_PUSH_CI_LOCK: &str = "/tmp/.git-supervisor-lock.d";

/// Env options for running check-push on a host (REPO_WHITELIST, BR_WHITELIST_PER_REPO, RELEASE_TAG_*).
#[derive(Default)]
pub struct CheckPushEnv {
    pub repo_whitelist: Option<String>,
    pub repo_branches: Option<String>,
    pub log_level: Option<u8>,
    pub release_tag_topn: Option<u32>,
    pub release_tag_pattern: Option<String>,
    pub release_tag_exclude_pattern: Option<String>,
    /// SSH key path on the remote host used for GitHub access (sets GIT_SSH_COMMAND).
    pub github_ssh_key: Option<String>,
}

fn build_check_push_extra_env(env: &CheckPushEnv) -> String {
    let mut env_parts: Vec<String> = Vec::new();
    if let Some(s) = &env.repo_whitelist {
        env_parts.push(format!("REPO_WHITELIST={}", escape_single_quoted(s)));
    }
    if let Some(s) = &env.repo_branches {
        env_parts.push(format!("BR_WHITELIST_PER_REPO={}", escape_single_quoted(s)));
    }
    if let Some(n) = env.log_level {
        env_parts.push(format!("LOGLEVEL={}", n));
    }
    if let Some(n) = env.release_tag_topn {
        env_parts.push(format!("RELEASE_TAG_TOPN={}", n));
    }
    if let Some(s) = &env.release_tag_pattern {
        env_parts.push(format!("RELEASE_TAG_PATTERN={}", escape_single_quoted(s)));
    }
    if let Some(s) = &env.release_tag_exclude_pattern {
        env_parts.push(format!(
            "RELEASE_TAG_EXCLUDE_PATTERN={}",
            escape_single_quoted(s)
        ));
    }
    if let Some(key) = &env.github_ssh_key {
        // Use double-quote form so that $HOME (from ~/...) is expanded on the remote shell.
        let key_path = if key.starts_with("~/") {
            format!("$HOME/{}", &key[2..])
        } else {
            key.clone()
        };
        let escaped_path = key_path.replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`");
        env_parts.push(format!(
            "GIT_SSH_COMMAND=\"ssh -o StrictHostKeyChecking=no -i {}\"",
            escaped_path
        ));
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
        "env DIR_BASE={} VERB={} TIMEOUT={} SLEEP_TIME=0 CI_LOCK='{}' HOST_ID='{}'{}{} bash -s -- --once",
        dir_base_esc,
        CHECK_PUSH_VERB,
        CHECK_PUSH_TIMEOUT,
        CHECK_PUSH_CI_LOCK,
        host_id_esc,
        if console::color_enabled() { " FORCE_COLOR=1" } else { "" },
        extra
    );
    ssh::ssh_run_with_stdin(host, &command, script.as_bytes())
        .context("run check-push on remote failed")
}

/// Run the embedded check-push.sh script once on the local machine.
/// All env vars (DIR_BASE, BR_WHITELIST, LOGLEVEL, etc.) are inherited from the process environment.
/// REPO_WHITELIST is explicitly unset if it is set to an empty string, so the script
/// treats it the same as if it were never exported.
pub fn run_check_push_local(script: &str) -> Result<()> {
    let color_env = if console::color_enabled() { "FORCE_COLOR=1 " } else { "" };
    // If REPO_WHITELIST is exported but empty, unset it so the script scans all repos.
    let unset_repo_whitelist = match std::env::var("REPO_WHITELIST") {
        Ok(v) if v.is_empty() => "unset REPO_WHITELIST; ",
        _ => "",
    };
    // Explicitly forward LOGLEVEL so a login shell (-l) cannot shadow the inherited value.
    let loglevel_env = match std::env::var("LOGLEVEL") {
        Ok(v) if v.parse::<u8>().is_ok() => format!("LOGLEVEL={} ", v),
        _ => String::new(),
    };
    let command = format!(
        "{}{}{}SLEEP_TIME=0 CI_LOCK='{}' bash -s -- --once",
        unset_repo_whitelist,
        color_env,
        loglevel_env,
        CHECK_PUSH_CI_LOCK
    );
    let localhost = Host {
        ssh_target: "localhost".to_string(),
        ssh_port: None,
        ssh_identity_file: None,
        ssh_key_name: None,
        github_ssh_key: None,
        ssh_forward_agent: None,
        dir_base: None,
        repos: vec![],
        release_count: None,
        release_tag_pattern: None,
        release_tag_exclude_pattern: None,
    };
    ssh::ssh_run_with_stdin(&localhost, &command, script.as_bytes())
        .context("local check-push failed")
}

/// Query remote refs for a repo URL and return a stable fingerprint string.
///
/// Uses `git ls-remote --heads --tags --refs <url>` so the supervisor can
/// detect upstream changes without maintaining local clones.
pub fn remote_refs_fingerprint(repo_url: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("ls-remote")
        .arg("--heads")
        .arg("--tags")
        .arg("--refs")
        .arg(repo_url)
        .output()
        .with_context(|| format!("failed to run git ls-remote for {}", repo_url))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git failed with status -\n{}",
            if stderr.is_empty() {
                format!("exit {}", output.status)
            } else {
                stderr
            }
        );
    }

    // Normalize refs so a textual compare between rounds is deterministic.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines: BTreeSet<String> = BTreeSet::new();
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        lines.insert(line.to_string());
    }
    Ok(lines.into_iter().collect::<Vec<_>>().join("\n"))
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

    #[test]
    fn build_check_push_extra_env_includes_loglevel() {
        let env = CheckPushEnv {
            log_level: Some(1),
            ..Default::default()
        };
        let extra = build_check_push_extra_env(&env);
        assert!(
            extra.contains("LOGLEVEL=1"),
            "extra env should include LOGLEVEL=1, got: {:?}",
            extra
        );
    }

    #[test]
    fn remote_refs_fingerprint_is_stable_with_reordered_lines() {
        // Mimic two ls-remote outputs with different line order and trailing spaces.
        let a = "bbbb\trefs/heads/dev\naaaa\trefs/heads/main\n";
        let b = "aaaa\trefs/heads/main\nbbbb\trefs/heads/dev   \n";

        let normalize = |s: &str| {
            let mut lines: BTreeSet<String> = BTreeSet::new();
            for line in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
                lines.insert(line.to_string());
            }
            lines.into_iter().collect::<Vec<_>>().join("\n")
        };

        assert_eq!(normalize(a), normalize(b));
    }
}
