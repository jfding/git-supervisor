use anyhow::Context;
use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};

pub mod cleanup;
pub mod config;
pub mod console;
pub mod hook;
pub mod keys;
pub mod ops;
pub mod ssh;
pub mod status;

pub use config::{CentralConfig, Defaults, Host, Repo};
pub use status::{run_status, StatusOpts};
pub use cleanup::{run_cleanup, CleanupOpts};

/// Options for the watch event loop.
pub struct WatchOpts {
    pub interval_secs: u64,
    pub timeout_secs: Option<u64>,
    pub ignore_missing: bool,
    pub skip_prepare: bool,
    pub webhook_port: Option<u16>,
    pub webhook_secret: Option<String>,
    pub version: String,
}

/// Embedded check-push.sh script, run on remote with sandbox env.
pub const CHECK_PUSH_SCRIPT: &str = include_str!("../core/check-push.sh");

const PID_FILE: &str = "/tmp/git-supervisor.pid";

/// RAII guard: writes PID on creation, removes the file on drop.
struct PidFile(PathBuf);

impl PidFile {
    fn create() -> Self {
        let path = PathBuf::from(PID_FILE);
        let pid = std::process::id();
        if let Err(e) = std::fs::write(&path, pid.to_string()) {
            console::log_warning(format!("failed to write pid file {}: {}", path.display(), e));
        }
        Self(path)
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn escape_single_quoted(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Build whitelists from host repos. Returns None for each value when empty.
/// - repo_whitelist: repo names (REPO_WHITELIST), space-separated
/// - br_whitelist_per_host: BR_WHITELIST_PER_REPO string for the script, "repo1 br1 br2|repo2 br3".
///   Uses default_branches when a repo has no branches specified.
fn whitelists_from_config(
    config: &CentralConfig,
    host_id: &str,
    filter_repos: Option<&HashSet<String>>,
) -> (Option<String>, Option<String>) {
    let all_repos = config.repos_for_host(host_id);
    let repos: Vec<_> = match filter_repos {
        Some(filter) => all_repos.into_iter().filter(|r| filter.contains(&r.name)).collect(),
        None => all_repos,
    };
    let default_branches = config.defaults.as_ref().and_then(|d| d.branches.as_deref());

    let repo_whitelist: String = repos
        .iter()
        .map(|r| r.dir_name().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let br_whitelist_per_host = repos
        .iter()
        .filter_map(|r| {
            let branches = r.branches.as_deref().or(default_branches)?;
            let mut s = r.dir_name().to_string();
            for br in branches {
                s.push(' ');
                s.push_str(br);
            }
            Some(s)
        })
        .collect::<Vec<_>>()
        .join("|");

    (
        (!repo_whitelist.is_empty()).then_some(repo_whitelist),
        (!br_whitelist_per_host.is_empty()).then_some(br_whitelist_per_host),
    )
}

/// Poll all configured repos from the local machine and detect which ones changed upstream.
///
/// A repo is considered "changed" when its `git ls-remote` fingerprint differs from the
/// previous watch round, or when it is first seen.
fn poll_changed_repos(
    config: &CentralConfig,
    last_refs: &mut HashMap<String, String>,
    quiet: bool,
) -> (HashSet<String>, HashSet<String>) {
    let mut changed_repos = HashSet::new();
    let mut failed_repos = HashSet::new();

    let referenced = config.repos_referenced_by_hosts();

    if !quiet {
        let mut names: Vec<&str> = referenced.iter().map(|s| s.as_str()).collect();
        names.sort_unstable();
        console::log_info(format!(
            "polling repos: [{}]",
            names.join(", ")
        ));
    }
    let results: Vec<(String, anyhow::Result<String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = config
            .repos
            .iter()
            .filter(|(name, _)| referenced.contains(*name))
            .map(|(repo_name, repo_def)| {
                let repo_name = repo_name.clone();
                let git_url = repo_def.git_url.clone();
                if !quiet {
                    console::log_debug(format!("polling repo [{}]: {}", repo_name, git_url));
                }
                s.spawn(move || (repo_name, ops::remote_refs_fingerprint(&git_url)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread panicked")).collect()
    });

    for (repo_name, result) in results {
        match result {
            Ok(fingerprint) => {
                if last_refs.get(&repo_name) != Some(&fingerprint) {
                    changed_repos.insert(repo_name.clone());
                }
                last_refs.insert(repo_name, fingerprint);
            }
            Err(e) => {
                console::log_warning(format!("polling failed for repo [{}]: {}", repo_name, e));
                failed_repos.insert(repo_name);
            }
        }
    }

    (changed_repos, failed_repos)
}

fn should_run_host_remote(
    first_round: bool,
    host_repo_names: &[String],
    changed_repos: &HashSet<String>,
    failed_repos: &HashSet<String>,
) -> bool {
    if host_repo_names.is_empty() {
        return false;
    }
    if first_round {
        return true;
    }
    host_repo_names
        .iter()
        .any(|repo| changed_repos.contains(repo))
        || host_repo_names
            .iter()
            .any(|repo| failed_repos.contains(repo))
}

/// Check config and remotes: validate SSH/git connectivity and repo existence on each host.
pub fn run_check(config: &CentralConfig) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        if !host.is_wildcard() && config.repos_for_host(host_id).is_empty() {
            console::log_info(format!("Check host {{ {} }} --> skipped (repos: [] is empty)", host_id));
            continue;
        }

        let label = if host.is_wildcard() { " (wildcard)" } else { "" };
        console::log_info(format!("Check host {{ {} }}{} -->", host_id, label));

        if let Err(e) = ops::check_git_available(host).context("check git/ssh available") {
            console::log_error(format!("Error {{ {} }}: {}", host_id, e));
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        let dir_repos = config.dir_repos_for_host(host_id);

        for repo in config.repos_for_host(host_id) {
            let repo_dir = dir_repos.join(repo.dir_name());
            let repo_dir_str = repo_dir.to_string_lossy();
            let repo_dir_esc = format!("'{}'", escape_single_quoted(&repo_dir_str));
            let ok_line = console::shell_printf(
                &format!("READY repo [{}] at {}", repo.name, repo_dir_str),
                Some(console::Color::Green),
            );
            let missing_line = console::shell_printf(
                &format!("MISSING repo [{}] at {}", repo.name, repo_dir_str),
                Some(console::Color::Yellow),
            );

            let command = format!(
                "if [ -d {}/.git ]; then \
  {}; \
else \
  {}; \
fi",
                repo_dir_esc, ok_line, missing_line,
            );

            if let Err(e) = crate::ssh::ssh_run(host, &command) {
                console::log_error(format!("Error {{ {} }}: {}", host_id, e));
                failures.push(format!("{{ {} }}: {}", host_id, e));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} host/repo failure(s):\n{}",
            failures.len(),
            failures.join("\n")
        )
    }
}

/// Prepare remotes: create dirs and optionally ensure repos exist (clone only when missing; no fetch).
/// If `ignore_missing` is true, check each repo and report "ready" or "missing" but do not clone missing ones.
fn run_prepare(config: &CentralConfig, ignore_missing: bool) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        if !host.is_wildcard() && config.repos_for_host(host_id).is_empty() {
            console::log_info(format!("Prepare host {{ {} }} --> skipped (repos: [] is empty)", host_id));
            continue;
        }

        let label = if host.is_wildcard() { " (wildcard)" } else { "" };
        console::log_info(format!("Prepare host {{ {} }}{} -->", host_id, label));

        let dir_repos = config.dir_repos_for_host(host_id);
        let dir_copies = config.dir_copies_for_host(host_id);

        if let Err(e) = ops::check_git_available(host).context("check git available") {
            console::log_error(format!("Error {{ {} }}: {}", host_id, e));
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        if let Err(e) = ops::check_docker_available(host) {
            console::log_warning(format!("Warning {{ {} }}: {} (optional)", host_id, e));
        }

        if let Err(e) = ops::create_dirs(host, &dir_repos, &dir_copies).context("create_dirs") {
            console::log_error(format!("Error {{ {} }}: {}", host_id, e));
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        for repo in config.repos_for_host(host_id) {
            if let Err(e) = ops::ensure_repo(host, &dir_repos, &repo, ignore_missing, host.github_ssh_key.as_deref()) {
                console::log_error(format!("Error {{ {} }}: {} (continuing)", host_id, e));
                failures.push(format!("{{ {} }}: {}", host_id, e));
            }
        }
    }
    console::log_info("Prepare DONE\n");

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} host/repo failure(s):\n{}",
            failures.len(),
            failures.join("\n")
        )
    }
}

/// Run one deployment cycle.
///
/// When `skip_poll` is true (webhook trigger), skip `git ls-remote` polling
/// and run check-push on all hosts. When false (timer trigger), poll and only
/// run hosts with changed repos.
fn run_cycle(
    config: &CentralConfig,
    last_remote_refs: &mut HashMap<String, String>,
    round: u64,
    first_round: bool,
    skip_poll: bool,
    trigger_label: &str,
    wh_tag: &str,
) {
    let (changed_repos, failed_repos) = if skip_poll {
        console::log_info(format!("watch: round {} [{} triggered] refreshing all hosts", round, trigger_label));
        // Update ref fingerprints so the next timer-triggered round won't
        // re-detect the same changes and cause a duplicate refresh.
        let _ = poll_changed_repos(config, last_remote_refs, true);
        (HashSet::new(), HashSet::new())
    } else {
        console::log_info(format!("watch:{} round {} (hosts: {})", wh_tag, round, config.hosts.len()));

        let (changed, failed) = poll_changed_repos(config, last_remote_refs, false);

        if !first_round {
            if changed.is_empty() {
                console::log_highlight("watch: no upstream repo changes detected, skipping all hosts");
            } else {
                let mut changed_sorted: Vec<_> = changed.iter().cloned().collect();
                changed_sorted.sort();
                console::log_highlight(format!(
                    "watch: upstream repo change detected: [{}]",
                    changed_sorted.join(", ")
                ));
            }
        } else {
            console::log_highlight("watch: initial round, refreshing all hosts");
        }
        (changed, failed)
    };

    let mut any_host_ran = false;
    let mut skipped_hosts: Vec<String> = Vec::new();
    std::thread::scope(|s| {
        for (host_id, host) in &config.hosts {
            let host_id = host_id.clone();
            let dir_base = config.dir_base_for_host(&host_id).clone();
            let is_wildcard = host.is_wildcard();
            let host_repo_names: Vec<String> = config
                .repos_for_host(&host_id)
                .into_iter()
                .map(|r| r.name)
                .collect();

            if !is_wildcard && host_repo_names.is_empty() {
                continue;
            }

            // Wildcard hosts always run (we can't poll repos we don't know about).
            // Webhook-triggered cycles always run all hosts.
            let should_run_remote = if is_wildcard || skip_poll {
                true
            } else {
                let has_changed_repo = host_repo_names
                    .iter()
                    .any(|repo| changed_repos.contains(repo));
                let has_probe_failure = host_repo_names
                    .iter()
                    .any(|repo| failed_repos.contains(repo));
                let should_run = should_run_host_remote(
                    first_round,
                    &host_repo_names,
                    &changed_repos,
                    &failed_repos,
                );
                if !should_run {
                    skipped_hosts.push(host_id.clone());
                }
                if has_probe_failure && !first_round && !has_changed_repo && should_run {
                    console::log_warning(format!(
                        "watch: host {{{}}} has probe failures, running remote check-push defensively",
                        host_id
                    ));
                }
                should_run
            };

            if !should_run_remote {
                continue;
            }

            // When we know which repos changed/failed, narrow the whitelist so the
            // remote script only processes relevant repos. First round and webhook
            // cycles always send the full list.
            let effective_filter: Option<HashSet<String>> = if !skip_poll && !first_round {
                Some(
                    host_repo_names
                        .iter()
                        .filter(|name| {
                            changed_repos.contains(*name) || failed_repos.contains(*name)
                        })
                        .cloned()
                        .collect(),
                )
            } else {
                None
            };
            let (repo_whitelist, br_whitelist_per_host) =
                whitelists_from_config(config, &host_id, effective_filter.as_ref());
            let check_push_env = ops::CheckPushEnv {
                repo_whitelist,
                repo_branches: br_whitelist_per_host,
                log_level: Some(console::log_level()),
                release_tag_topn: host.release_count,
                release_tag_pattern: host.release_tag_pattern.clone(),
                release_tag_exclude_pattern: host.release_tag_exclude_pattern.clone(),
                github_ssh_key: host.github_ssh_key.clone(),
            };

            any_host_ran = true;
            s.spawn(move || {
                if let Err(e) = ops::run_check_push_remote(
                    host,
                    &host_id,
                    &dir_base,
                    CHECK_PUSH_SCRIPT,
                    &check_push_env,
                ) {
                    console::log_error(format!("Failed on {{{}}}: {}", host_id, e));
                }
            });
        }
    });

    if any_host_ran && !skipped_hosts.is_empty() {
        console::log_info(format!("watch: skip {{{}}} (no remote repo changes)", skipped_hosts.join(", ")));
    }
}

/// Print a countdown on stderr while sleeping, overwriting the same line on a terminal.
/// Falls back to a plain sleep when stderr is not a terminal (e.g. piped logs).
async fn countdown_wait(duration: Duration, wh_tag: &str) {
    let total_secs = duration.as_secs();
    if console::log_level() >= 2 && std::io::stderr().is_terminal() {
        for remaining in (1..=total_secs).rev() {
            eprint!(
                "\r{}",
                console::info(format!("watch:{} next round in {}s...", wh_tag, remaining))
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        // Clear the countdown line before the next log line is printed
        eprint!("\r{:40}\r", "");
    } else {
        console::log_info(format!("watch:{} next round in {}s...", wh_tag, total_secs));
        tokio::time::sleep(duration).await;
    }
}

/// Prepare remotes (create dirs, init empty repos unless --ignore-missing), then run check-push
/// on each host in an event loop. The loop waits on either a timer tick or a webhook signal.
/// Both trigger a deployment cycle and reset the timer.
pub async fn run_watch(
    config: &CentralConfig,
    opts: WatchOpts,
) -> Result<(), anyhow::Error> {
    let interval = Duration::from_secs(opts.interval_secs);
    let deadline = opts.timeout_secs.map(|s| Instant::now() + Duration::from_secs(s));
    let mut round: u64 = 0;
    let mut last_remote_refs: HashMap<String, String> = HashMap::new();
    let mut first_timer_done = false;

    if !opts.skip_prepare {
        run_prepare(config, opts.ignore_missing)?;
    }

    // Optionally start the webhook server
    let mut webhook_rx = match (opts.webhook_port, opts.webhook_secret) {
        (Some(port), Some(secret)) => {
            Some(hook::start_webhook_server(port, secret, opts.version).await?)
        }
        _ => None,
    };

    let wh_tag = if webhook_rx.is_some() { " [+webhook]" } else { "" };

    let mut sigusr1 = signal(SignalKind::user_defined1())
        .context("failed to register SIGUSR1 handler")?;

    let _pid_guard = PidFile::create();
    console::log_info(format!(
        "watch: pid {} written to {} (send SIGUSR1 to trigger refresh)",
        std::process::id(),
        PID_FILE,
    ));

    loop {
        // For the first iteration, run immediately (no wait).
        // Each iteration yields (skip_poll, trigger_label) so run_cycle can log the source.
        let (skip_poll, trigger_label) = if round == 0 {
            (false, "timer")
        } else {
            if opts.interval_secs == 0 {
                console::log_highlight("watch: interval is 0, run once and quit");
                break;
            }

            let sleep_duration = match deadline {
                Some(d) => {
                    let remaining = d.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        console::log_info("watch: timeout reached, stopping");
                        break;
                    }
                    remaining.min(interval)
                }
                None => interval,
            };

            // Wait for timer, webhook signal, or SIGUSR1
            tokio::select! {
                _ = countdown_wait(sleep_duration, wh_tag) => {
                    (false, "timer")
                }
                Some(()) = async {
                    match webhook_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if std::io::stderr().is_terminal() {
                        eprint!("\r{:40}\r", "");
                    }
                    (true, "webhook")
                }
                _ = sigusr1.recv() => {
                    if std::io::stderr().is_terminal() {
                        eprint!("\r{:40}\r", "");
                    }
                    (true, "signal")
                }
            }
        };

        round += 1;
        // First timer-triggered round always forces full run on all hosts
        let first_round = !first_timer_done && !skip_poll;
        if !skip_poll {
            first_timer_done = true;
        }

        // run_cycle uses std::thread::scope (blocking SSH), so run in spawn_blocking
        let config_clone = config.clone();
        let mut refs = std::mem::take(&mut last_remote_refs);
        let trigger_label = trigger_label.to_string();
        let returned_refs = tokio::task::spawn_blocking(move || {
            run_cycle(&config_clone, &mut refs, round, first_round, skip_poll, &trigger_label, wh_tag);
            refs
        })
        .await?;
        last_remote_refs = returned_refs;
    }

    Ok(())
}

/// Run check-push.sh on the local machine in a watch loop (no config file needed).
/// All check-push.sh env vars (DIR_BASE, BR_WHITELIST, LOGLEVEL, etc.) are read from
/// the process environment, exactly like running check-push.sh directly.
/// interval_secs=0 means run once and exit.
pub fn run_local_watch(interval_secs: u64, timeout_secs: Option<u64>) -> Result<(), anyhow::Error> {
    let interval = Duration::from_secs(interval_secs);
    let deadline = timeout_secs.map(|s| Instant::now() + Duration::from_secs(s));
    let mut round: u64 = 0;

    loop {
        round += 1;
        console::log_info(format!("local watch round {}", round));

        if let Err(e) = ops::run_check_push_local(CHECK_PUSH_SCRIPT) {
            console::log_error(format!("Error: {}", e));
        }

        console::log_info(format!("local watch round {} done", round));

        if interval_secs == 0 {
            break;
        }

        if let Some(d) = deadline {
            let remaining = d.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                console::log_info("watch timeout reached, stopping");
                break;
            }
            console::log_info("waiting for next check ...");
            std::thread::sleep(remaining.min(interval));
        } else {
            console::log_info("waiting for next check ...");
            std::thread::sleep(interval);
        }

        if deadline.is_some_and(|d| Instant::now() >= d) {
            console::log_info("watch timeout reached, stopping");
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn cleanup_reexports_are_public() {
        // Compile-time check that the re-exports exist at crate root.
        let _opts = crate::CleanupOpts { host_patterns: vec![], apply: false };
        let _f: fn(&CentralConfig, crate::CleanupOpts) -> Result<(), anyhow::Error> =
            crate::run_cleanup;
    }

    #[test]
    fn should_run_host_remote_first_round_always_runs() {
        let host_repo_names = vec!["repo-a".to_string()];
        let changed = HashSet::new();
        let failed = HashSet::new();
        assert!(should_run_host_remote(
            true,
            &host_repo_names,
            &changed,
            &failed
        ));
    }

    #[test]
    fn should_run_host_remote_skips_when_no_changes_or_failures() {
        let host_repo_names = vec!["repo-a".to_string()];
        let changed = HashSet::new();
        let failed = HashSet::new();
        assert!(!should_run_host_remote(
            false,
            &host_repo_names,
            &changed,
            &failed
        ));
    }

    #[test]
    fn should_run_host_remote_runs_on_changed_repo() {
        let host_repo_names = vec!["repo-a".to_string(), "repo-b".to_string()];
        let changed: HashSet<String> = ["repo-b".to_string()].into_iter().collect();
        let failed = HashSet::new();
        assert!(should_run_host_remote(
            false,
            &host_repo_names,
            &changed,
            &failed
        ));
    }

    #[test]
    fn should_run_host_remote_runs_on_probe_failure() {
        let host_repo_names = vec!["repo-a".to_string(), "repo-b".to_string()];
        let changed = HashSet::new();
        let failed: HashSet<String> = ["repo-a".to_string()].into_iter().collect();
        assert!(should_run_host_remote(
            false,
            &host_repo_names,
            &changed,
            &failed
        ));
    }

    #[test]
    fn should_run_host_remote_skips_explicitly_empty_host_repo_list() {
        let host_repo_names = vec![];
        let changed = HashSet::new();
        let failed = HashSet::new();
        assert!(!should_run_host_remote(
            false,
            &host_repo_names,
            &changed,
            &failed
        ));
    }

    #[test]
    fn should_run_host_remote_skips_explicitly_empty_even_first_round() {
        let host_repo_names = vec![];
        let changed = HashSet::new();
        let failed = HashSet::new();
        assert!(!should_run_host_remote(
            true,
            &host_repo_names,
            &changed,
            &failed
        ));
    }
}
