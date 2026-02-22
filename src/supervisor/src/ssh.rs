use anyhow::{Context, Result};
use std::process::Command;

use crate::config::Host;

/// Expand `~` in path to home directory if present.
fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        dirs::home_dir()
            .map(|h| h.join(&path[2..]).to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    } else if path == "~" {
        dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    }
}

/// Run a shell command on the remote host via SSH.
/// `command` is the full shell snippet executed on the remote (e.g. "mkdir -p /work/git_repos").
pub fn ssh_run(host: &Host, command: &str) -> Result<()> {
    let mut cmd = Command::new("ssh");
    if let Some(ref id) = host.ssh_identity_file {
        cmd.arg("-i").arg(expand_tilde(id));
    }
    if let Some(p) = host.ssh_port {
        cmd.arg("-p").arg(p.to_string());
    }
    cmd.arg(&host.ssh_target).arg(command);

    let status = cmd
        .status()
        .context("Failed to execute ssh")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("ssh exited with {}", status)
    }
}
