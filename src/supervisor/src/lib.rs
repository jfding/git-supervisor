use anyhow::Context;

pub mod config;
pub mod ops;
pub mod ssh;

pub use config::{CentralConfig, Defaults, Host, Repo};

/// Push to remotes: create dirs and ensure repos. No-op when dry_run.
/// Returns Err if any host failed (create_dirs or any ensure_repo).
pub fn run_push(config: &CentralConfig, dry_run: bool) -> Result<(), anyhow::Error> {
    let mut failures: Vec<String> = Vec::new();

    for (host_id, host) in &config.hosts {
        let dir_repos = config.dir_repos_for_host(host_id);
        let dir_copies = config.dir_copies_for_host(host_id);

        if dry_run {
            eprintln!("[dry-run] {}: would create dirs and ensure {} repo(s)", host_id, host.repos.len());
            continue;
        }

        if let Err(e) = ops::create_dirs(host, &dir_repos, &dir_copies)
            .context(format!("host {}: create_dirs", host_id))
        {
            eprintln!("Error: {}: {}", host_id, e);
            failures.push(format!("{}: {}", host_id, e));
            continue;
        }

        for repo in &host.repos {
            if let Err(e) = ops::ensure_repo(host, &dir_repos, repo) {
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
