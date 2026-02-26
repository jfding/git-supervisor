use anyhow::Context;

pub mod config;
pub mod ops;
pub mod ssh;

pub use config::{CentralConfig, Defaults, Host, Repo};

/// Validate config: print what push would do without running SSH.
pub fn run_validate(config: &CentralConfig) -> Result<(), anyhow::Error> {
    for host_id in config.hosts.keys() {
        let repos = config.repos_for_host(host_id);
        eprintln!("{}: would create dirs and ensure {} repo(s)", host_id, repos.len());
    }
    Ok(())
}

/// Push to remotes: create dirs and ensure repos.
/// Returns Err if any host failed (create_dirs or any ensure_repo).
pub fn run_push(config: &CentralConfig) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        let dir_repos = config.dir_repos_for_host(host_id);
        let dir_copies = config.dir_copies_for_host(host_id);

        if let Err(e) = ops::create_dirs(host, &dir_repos, &dir_copies)
            .context(format!("host {}: create_dirs", host_id))
        {
            eprintln!("Error: {}: {}", host_id, e);
            failures.push(format!("{}: {}", host_id, e));
            continue;
        }

        for repo in config.repos_for_host(host_id) {
            if let Err(e) = ops::ensure_repo(host, &dir_repos, &repo) {
                eprintln!("Warning: {}: {} (continuing)", host_id, e);
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
