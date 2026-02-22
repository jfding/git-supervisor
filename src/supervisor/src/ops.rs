use anyhow::{Context, Result};
use std::path::Path;

use crate::config::{Host, Repo};
use crate::ssh;

/// Escape a path for use inside single quotes in a remote shell.
/// Any single quote in the path becomes '\''.
fn escape_single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Create dir_repos and dir_copies on the remote host.
pub fn create_dirs(
    host: &Host,
    dir_repos: &Path,
    dir_copies: &Path,
) -> Result<()> {
    let r = escape_single_quoted(&dir_repos.to_string_lossy());
    let c = escape_single_quoted(&dir_copies.to_string_lossy());
    let command = format!("mkdir -p {} {}", r, c);
    ssh::ssh_run(host, &command).context("create_dirs failed")
}

/// Ensure the repo exists on the remote: clone if missing, else fetch.
/// dir_repos is the path to the git_repos directory on the remote.
pub fn ensure_repo(
    host: &Host,
    dir_repos: &Path,
    repo: &Repo,
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
    let command = format!(
        "cd '{}' && if [ ! -d '{}/.git' ]; then git clone '{}' '{}'; else (cd '{}' && git fetch --all --tags --prune); fi",
        dir_esc, name_esc, url_esc, name_esc, name_esc
    );
    ssh::ssh_run(host, &command).with_context(|| format!("ensure_repo {} failed", repo.name))
}
