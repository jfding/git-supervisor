use anyhow::Context;
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

pub mod config;
pub mod console;
pub mod hook;
pub mod keys;
pub mod ops;
pub mod ssh;

pub use config::{CentralConfig, Defaults, Host, Repo};

/// Embedded check-push.sh script, run on remote with sandbox env.
const CHECK_PUSH_SCRIPT: &str = include_str!("../embed/check-push.sh");

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
) -> (Option<String>, Option<String>) {
    let repos = config.repos_for_host(host_id);
    let default_branches = config.defaults.as_ref().and_then(|d| d.branches.as_deref());

    let repo_whitelist: String = repos
        .iter()
        .map(|r| r.name.clone())
        .collect::<Vec<_>>()
        .join(" ");
    let br_whitelist_per_host = repos
        .iter()
        .filter_map(|r| {
            let branches = r.branches.as_deref().or(default_branches)?;
            let mut s = r.name.clone();
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
) -> (HashSet<String>, HashSet<String>) {
    let mut changed_repos = HashSet::new();
    let mut failed_repos = HashSet::new();

    for (repo_name, repo_def) in &config.repos {
        match ops::remote_refs_fingerprint(&repo_def.git_url) {
            Ok(fingerprint) => {
                if last_refs.get(repo_name) != Some(&fingerprint) {
                    changed_repos.insert(repo_name.clone());
                }
                last_refs.insert(repo_name.clone(), fingerprint);
            }
            Err(e) => {
                eprintln!(
                    "{}",
                    console::warning(format!(
                        "watch probe failed for repo [{}]: {}",
                        repo_name, e
                    ))
                );
                failed_repos.insert(repo_name.clone());
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
    if first_round || host_repo_names.is_empty() {
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
        eprintln!(
            "{}",
            console::highlight(format!("Check host {{ {} }} -->", host_id))
        );

        if let Err(e) = ops::check_git_available(host).context("check git/ssh available") {
            eprintln!(
                "{}",
                console::error(format!("Error {{ {} }}: {}", host_id, e))
            );
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        let dir_repos = config.dir_repos_for_host(host_id);

        for repo in config.repos_for_host(host_id) {
            let repo_dir = dir_repos.join(&repo.name);
            let repo_dir_str = repo_dir.to_string_lossy();
            let repo_dir_esc = format!("'{}'", escape_single_quoted(&repo_dir_str));
            let ok_line = console::shell_printf(
                &format!("OK repo [{}] at {}", repo.name, repo_dir_str),
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
                eprintln!(
                    "{}",
                    console::error(format!("Error {{ {} }}: {}", host_id, e))
                );
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
        eprintln!(
            "{}",
            console::info(format!("Prepare host {{ {} }} -->", host_id))
        );

        let dir_repos = config.dir_repos_for_host(host_id);
        let dir_copies = config.dir_copies_for_host(host_id);

        if let Err(e) = ops::check_git_available(host).context("check git available") {
            eprintln!(
                "{}",
                console::error(format!("Error {{ {} }}: {}", host_id, e))
            );
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        if let Err(e) = ops::check_docker_available(host) {
            eprintln!(
                "{}",
                console::warning(format!("Warning {{ {} }}: {} (optional)", host_id, e))
            );
        }

        if let Err(e) = ops::create_dirs(host, &dir_repos, &dir_copies).context("create_dirs") {
            eprintln!(
                "{}",
                console::error(format!("Error {{ {} }}: {}", host_id, e))
            );
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        for repo in config.repos_for_host(host_id) {
            if let Err(e) = ops::ensure_repo(host, &dir_repos, &repo, ignore_missing) {
                eprintln!(
                    "{}",
                    console::error(format!("Error {{ {} }}: {} (continuing)", host_id, e))
                );
                failures.push(format!("{{ {} }}: {}", host_id, e));
            }
        }
    }
    println!("{}", console::info("Prepare DONE\n"));

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
) {
    let (changed_repos, failed_repos) = if skip_poll {
        eprintln!(
            "{}",
            console::info(format!(
                "watch round {} [webhook] (hosts: {})",
                round,
                config.hosts.len()
            ))
        );
        eprintln!(
            "{}",
            console::info("watch: webhook triggered, running remote check-push for all hosts")
        );
        (HashSet::new(), HashSet::new())
    } else {
        let (changed, failed) = poll_changed_repos(config, last_remote_refs);
        eprintln!(
            "{}",
            console::info(format!(
                "watch round {} (hosts: {})",
                round,
                config.hosts.len()
            ))
        );
        if !first_round {
            if changed.is_empty() {
                eprintln!(
                    "{}",
                    console::info("watch: no upstream repo changes detected in this round")
                );
            } else {
                let mut changed_sorted: Vec<_> = changed.iter().cloned().collect();
                changed_sorted.sort();
                eprintln!(
                    "{}",
                    console::info(format!(
                        "watch: upstream repo change detected: {}",
                        changed_sorted.join(", ")
                    ))
                );
            }
        } else {
            eprintln!(
                "{}",
                console::info("watch: initial round, running remote check-push for all hosts")
            );
        }
        (changed, failed)
    };

    std::thread::scope(|s| {
        for (host_id, host) in &config.hosts {
            let host_id = host_id.clone();
            let dir_base = config.dir_base_for_host(&host_id).clone();
            let host_repo_names: Vec<String> = config
                .repos_for_host(&host_id)
                .into_iter()
                .map(|r| r.name)
                .collect();
            let (repo_whitelist, br_whitelist_per_host) =
                whitelists_from_config(config, &host_id);
            let check_push_env = ops::CheckPushEnv {
                repo_whitelist,
                repo_branches: br_whitelist_per_host,
                log_level: config.defaults.as_ref().and_then(|d| d.log_level),
                release_tag_topn: host.release_count,
                release_tag_pattern: host.release_tag_pattern.clone(),
                release_tag_exclude_pattern: host.release_tag_exclude_pattern.clone(),
            };

            // Webhook-triggered cycles always run all hosts
            let should_run_remote = if skip_poll {
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
                    eprintln!(
                        "{}",
                        console::info(format!(
                            "watch: skip host {{{}}} (no remote repo changes)",
                            host_id
                        ))
                    );
                }
                if has_probe_failure && !first_round && !has_changed_repo && should_run {
                    eprintln!(
                        "{}",
                        console::warning(format!(
                            "watch: host {{{}}} has probe failures, running remote check-push defensively",
                            host_id
                        ))
                    );
                }
                should_run
            };

            if !should_run_remote {
                continue;
            }
            s.spawn(move || {
                if let Err(e) = ops::run_check_push_remote(
                    host,
                    &host_id,
                    &dir_base,
                    CHECK_PUSH_SCRIPT,
                    &check_push_env,
                ) {
                    eprintln!("{}", console::error(format!("Error: {}: {}", host_id, e)));
                }
            });
        }
    });
}

/// Prepare remotes (create dirs, init empty repos unless --ignore-missing), then run check-push on each host in a loop.
/// Sleeps `interval_secs` between rounds. If `timeout_secs` is Some, stops after that many seconds.
pub fn run_watch(
    config: &CentralConfig,
    interval_secs: u64,
    timeout_secs: Option<u64>,
    ignore_missing: bool,
    skip_prepare: bool,
    webhook_port: Option<u16>,
    webhook_secret: Option<String>,
) -> Result<(), anyhow::Error> {
    let _ = (webhook_port, webhook_secret);
    let interval = Duration::from_secs(interval_secs);
    let deadline = timeout_secs.map(|s| Instant::now() + Duration::from_secs(s));
    let mut round: u64 = 0;
    let mut last_remote_refs: HashMap<String, String> = HashMap::new();

    if !skip_prepare {
        run_prepare(config, ignore_missing)?;
    }

    loop {
        round += 1;
        let first_round = round == 1;

        run_cycle(config, &mut last_remote_refs, round, first_round, false);

        if interval_secs == 0 {
            eprintln!("{}", console::info("interval is 0, run once and quit"));
            break;
        }

        let sleep_duration = match deadline {
            Some(d) => {
                let remaining = d.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    eprintln!("{}", console::info("watch timeout reached, stopping"));
                    break;
                }
                remaining.min(interval)
            }
            None => interval,
        };
        thread::sleep(sleep_duration);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
    fn should_run_host_remote_runs_for_empty_host_repo_list() {
        let host_repo_names = vec![];
        let changed = HashSet::new();
        let failed = HashSet::new();
        assert!(should_run_host_remote(
            false,
            &host_repo_names,
            &changed,
            &failed
        ));
    }
}
