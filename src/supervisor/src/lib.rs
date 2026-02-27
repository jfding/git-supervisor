use anyhow::Context;
use std::time::{Duration, Instant};
use std::thread;

pub mod config;
pub mod ops;
pub mod ssh;

pub use config::{CentralConfig, Defaults, Host, Repo};

/// Embedded check-push.sh script (from repo src/check-push.sh), run on remote with sandbox env.
const CHECK_PUSH_SCRIPT: &str = include_str!("../../../src/check-push.sh");

fn escape_single_quoted(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Check config and remotes: validate SSH/git connectivity and repo existence on each host.
pub fn run_check(config: &CentralConfig) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        eprintln!("Check host {{ {} }} -->", host_id);

        if let Err(e) = ops::check_git_available(host).context("check git/ssh available") {
            eprintln!("Error {{ {} }}: {}", host_id, e);
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        let dir_repos = config.dir_repos_for_host(host_id);

        for repo in config.repos_for_host(host_id) {
            let repo_dir = dir_repos.join(&repo.name);
            let repo_dir_str = repo_dir.to_string_lossy();
            let repo_dir_esc = format!("'{}'", escape_single_quoted(&repo_dir_str));

            let command = format!(
                "if [ -d {}/.git ]; then \
  echo 'OK repo [{}] at {}'; \
else \
  echo 'MISSING repo [{}] at {}'; \
  exit 1; \
fi",
                repo_dir_esc,
                repo.name,
                repo_dir_str,
                repo.name,
                repo_dir_str,
            );

            if let Err(e) = crate::ssh::ssh_run(host, &command) {
                eprintln!("Error {{ {} }}: {}", host_id, e);
                failures.push(format!("{{ {} }}: {}", host_id, e));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{} host/repo failure(s):\n{}", failures.len(), failures.join("\n"))
    }
}

/// Push to remotes: create dirs and ensure repos.
/// If `checkout` is true, after preparing repos run the embedded check-push.sh on each host with sandbox env.
/// If `no_fetch` is true, skip git fetch for existing repos (only clone when missing).
/// Returns Err if any host failed (create_dirs or any ensure_repo, or check-push when checkout is true).
pub fn run_push(config: &CentralConfig, checkout: bool, no_fetch: bool) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        println!("Push host {{ {} }} -->", host_id);

        let dir_repos = config.dir_repos_for_host(host_id);
        let dir_copies = config.dir_copies_for_host(host_id);
        let dir_base = config.dir_base_for_host(host_id);

        if let Err(e) = ops::check_git_available(host)
            .context("check git available")
        {
            eprintln!("Error {{ {} }}: {}", host_id, e);
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        if let Err(e) = ops::check_docker_available(host) {
            eprintln!("Warning {{ {} }}: {} (optional)", host_id, e);
        }

        if let Err(e) = ops::create_dirs(host, &dir_repos, &dir_copies)
            .context("create_dirs")
        {
            eprintln!("Error {{ {} }}: {}", host_id, e);
            failures.push(format!("{{ {} }}: {}", host_id, e));
            continue;
        }

        for repo in config.repos_for_host(host_id) {
            // When running check-push (--checkout) or --no-fetch, skip fetch on existing repos.
            let fetch_existing = !checkout && !no_fetch;
            if let Err(e) = ops::ensure_repo(host, &dir_repos, &repo, fetch_existing) {
                eprintln!("Warning {{ {} }}: {} (continuing)", host_id, e);
                failures.push(format!("{{ {} }}: {}", host_id, e));
            }
        }

        if checkout {
            if let Err(e) = ops::run_check_push_remote(host, host_id, &dir_base, CHECK_PUSH_SCRIPT) {
                eprintln!("Error {{ {} }}: {}", host_id, e);
                failures.push(format!("{{ {} }}: {}", host_id, e));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{} host/repo failure(s):\n{}", failures.len(), failures.join("\n"))
    }
}

/// Run check-push on each host in a loop. Sleeps `interval_secs` between rounds.
/// If `timeout_secs` is Some, stops after that many seconds; if None, runs until interrupted.
pub fn run_watch(
    config: &CentralConfig,
    interval_secs: u64,
    timeout_secs: Option<u64>,
) -> Result<(), anyhow::Error> {
    let interval = Duration::from_secs(interval_secs);
    let deadline = timeout_secs.map(|s| Instant::now() + Duration::from_secs(s));
    let mut round: u64 = 0;

    loop {
        round += 1;
        eprintln!("watch round {} (hosts: {})", round, config.hosts.len());

        std::thread::scope(|s| {
            for (host_id, host) in &config.hosts {
                let host_id = host_id.clone();
                let dir_base = config.dir_base_for_host(&host_id).clone();
                s.spawn(move || {
                    if let Err(e) = ops::run_check_push_remote(host, &host_id, &dir_base, CHECK_PUSH_SCRIPT) {
                        eprintln!("Error: {}: {}", host_id, e);
                    }
                });
            }
        });

        let sleep_duration = match deadline {
            Some(d) => {
                let remaining = d.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    eprintln!("watch timeout reached, stopping");
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
