use anyhow::Context;

pub mod config;
pub mod ops;
pub mod ssh;

pub use config::{CentralConfig, Defaults, Host, Repo};

/// Embedded check-push.sh script (from repo src/check-push.sh), run on remote with sandbox env.
const CHECK_PUSH_SCRIPT: &str = include_str!("../../../src/check-push.sh");

/// Validate config: print what push would do without running SSH.
pub fn run_validate(config: &CentralConfig) -> Result<(), anyhow::Error> {
    for host_id in config.hosts.keys() {
        let repos = config.repos_for_host(host_id);
        eprintln!("{}: would create dirs and ensure {} repo(s)", host_id, repos.len());
    }
    Ok(())
}

/// Push to remotes: create dirs and ensure repos.
/// If `checkout` is true, after preparing repos run the embedded check-push.sh on each host with sandbox env.
/// Returns Err if any host failed (create_dirs or any ensure_repo, or check-push when checkout is true).
pub fn run_push(config: &CentralConfig, checkout: bool) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        let dir_repos = config.dir_repos_for_host(host_id);
        let dir_copies = config.dir_copies_for_host(host_id);
        let dir_base = config.dir_base_for_host(host_id);

        if let Err(e) = ops::create_dirs(host, &dir_repos, &dir_copies)
            .context(format!("host {}: create_dirs", host_id))
        {
            eprintln!("Error: {}: {}", host_id, e);
            failures.push(format!("{}: {}", host_id, e));
            continue;
        }

        for repo in config.repos_for_host(host_id) {
            // When running check-push (--checkout), skip fetch on existing repos.
            let fetch_existing = !checkout;
            if let Err(e) = ops::ensure_repo(host, &dir_repos, &repo, fetch_existing) {
                eprintln!("Warning: {}: {} (continuing)", host_id, e);
                failures.push(format!("{}: {}", host_id, e));
            }
        }

        if checkout {
            if let Err(e) = ops::run_check_push_remote(host, &dir_base, CHECK_PUSH_SCRIPT) {
                eprintln!("Error: {}: {}", host_id, e);
                failures.push(format!("{}: {}", host_id, e));
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("{} host/repo failure(s): {}", failures.len(), failures.join("; "))
    }
}
