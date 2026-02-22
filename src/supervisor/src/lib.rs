pub mod config;

pub use config::{CentralConfig, Defaults, Host, Repo};

/// Stub: push config to remotes (create dirs + ensure repos). No-op when dry_run.
pub fn run_push(config: &CentralConfig, dry_run: bool) -> Result<(), anyhow::Error> {
    let _ = (config, dry_run);
    Ok(())
}
