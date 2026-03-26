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

fn normalize_ssh_target_host(ssh_target: &str) -> String {
    let trimmed = ssh_target.trim();
    let mut host = trimmed
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(trimmed)
        .trim();

    // Handle the common bracketed IPv6 form like "[::1]" or "[::1]:2222".
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            host = &host[1..end];
        }
    } else if let Some((h, p)) = host.rsplit_once(':') {
        // Handle optional host:port (non-IPv6 form only).
        if p.chars().all(|c| c.is_ascii_digit()) && !h.contains(':') {
            host = h;
        }
    }

    host.to_ascii_lowercase()
}

fn is_local_ssh_target(ssh_target: &str) -> bool {
    matches!(
        normalize_ssh_target_host(ssh_target).as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn local_run(command: &str) -> Result<()> {
    let status = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .status()
        .context("Failed to execute local shell command")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("local command exited with {}", status)
    }
}

fn local_run_with_stdin(command: &str, stdin_data: &[u8]) -> Result<()> {
    let mut child = Command::new("sh")
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to execute local shell command")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data)
            .context("Failed to write script to local stdin")?;
    }
    let status = child.wait().context("Failed to wait for local command")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("local command exited with {}", status)
    }
}

/// Run a shell command on the remote host via SSH.
/// `command` is the full shell snippet executed on the remote (e.g. "mkdir -p /work/git_repos").
pub fn ssh_run(host: &Host, command: &str) -> Result<()> {
    if is_local_ssh_target(&host.ssh_target) {
        return local_run(command);
    }

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
    if is_local_ssh_target(&host.ssh_target) {
        return local_run_with_stdin(command, stdin_data);
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HostRepoRef;

    fn host(ssh_target: &str) -> Host {
        Host {
            ssh_target: ssh_target.to_string(),
            ssh_port: None,
            ssh_identity_file: None,
            ssh_key_name: None,
            dir_base: None,
            repos: Vec::<HostRepoRef>::new(),
            release_count: None,
            release_tag_pattern: None,
            release_tag_exclude_pattern: None,
        }
    }

    #[test]
    fn local_target_detection_supports_common_localhost_forms() {
        assert!(is_local_ssh_target("localhost"));
        assert!(is_local_ssh_target("LOCALHOST"));
        assert!(is_local_ssh_target("127.0.0.1"));
        assert!(is_local_ssh_target("::1"));
        assert!(is_local_ssh_target("[::1]"));
        assert!(is_local_ssh_target("[::1]:2222"));
        assert!(is_local_ssh_target("user@localhost"));
        assert!(is_local_ssh_target("user@[::1]"));
        assert!(!is_local_ssh_target("deploy@example.com"));
        assert!(!is_local_ssh_target("10.0.0.8"));
    }

    #[test]
    fn localhost_runs_without_ssh() {
        let h = host("localhost");
        assert!(ssh_run(&h, "printf ok >/dev/null").is_ok());
    }

    #[test]
    fn localhost_stdin_runs_without_ssh() {
        let h = host("127.0.0.1");
        assert!(ssh_run_with_stdin(&h, "cat >/dev/null", b"hello").is_ok());
    }
}
