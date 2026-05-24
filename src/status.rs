use crate::config::CentralConfig;

/// Filter options passed to [`run_status`].
pub struct StatusOpts {
    pub host_patterns: Vec<String>,
}

pub fn run_status(_config: &CentralConfig, _opts: StatusOpts) -> anyhow::Result<()> {
    Ok(())
}
