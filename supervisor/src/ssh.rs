use anyhow::{Context, Result};
use std::io::Write;
use std::process::{Command, Stdio};

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

    let status = cmd.status().context("Failed to execute ssh")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("ssh exited with {}", status)
    }
}

/// Run a remote command with stdin data (e.g. pipe a script into bash).
pub fn ssh_run_with_stdin(host: &Host, command: &str, stdin_data: &[u8]) -> Result<()> {
    let mut cmd = Command::new("ssh");
    if let Some(ref id) = host.ssh_identity_file {
        cmd.arg("-i").arg(expand_tilde(id));
    }
    if let Some(p) = host.ssh_port {
        cmd.arg("-p").arg(p.to_string());
    }
    cmd.arg(&host.ssh_target)
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().context("Failed to execute ssh")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data)
            .context("Failed to write script to ssh stdin")?;
    }
    let status = child.wait().context("Failed to wait for ssh")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("ssh exited with {}", status)
    }
}
