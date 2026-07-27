use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::process::{Command, ExitStatus, Stdio};

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

/// Resolve the effective SSH identity file for a host.
/// `ssh_key_name` (managed key) takes precedence over `ssh_identity_file` (explicit path).
fn resolve_identity_file(host: &Host) -> Result<Option<String>> {
    if let Some(ref name) = host.ssh_key_name {
        let path = crate::keys::resolve(name)?;
        return Ok(Some(path.to_string_lossy().into_owned()));
    }
    Ok(host.ssh_identity_file.clone())
}

pub fn is_local_ssh_target(ssh_target: &str) -> bool {
    matches!(
        normalize_ssh_target_host(ssh_target).as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// Build an unconfigured Command for `host`. For local targets returns `sh -lc`;
/// otherwise `ssh ... <target>`. Caller appends the shell snippet to execute and
/// configures stdio.
fn build_ssh_command(host: &Host) -> Result<Command> {
    if is_local_ssh_target(&host.ssh_target) {
        let mut cmd = Command::new("sh");
        cmd.arg("-lc");
        return Ok(cmd);
    }
    let identity = resolve_identity_file(host)?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-o").arg("StrictHostKeyChecking=no");
    if host.ssh_forward_agent == Some(true) {
        cmd.arg("-A");
    }
    if let Some(ref id) = identity {
        cmd.arg("-i").arg(expand_tilde(id));
    }
    if let Some(p) = host.ssh_port {
        cmd.arg("-p").arg(p.to_string());
    }
    cmd.arg(&host.ssh_target);
    Ok(cmd)
}

/// Run a shell command on the remote host via SSH.
/// `command` is the full shell snippet executed on the remote (e.g. "mkdir -p /work/git_repos").
pub fn ssh_run(host: &Host, command: &str) -> Result<()> {
    let mut cmd = build_ssh_command(host)?;
    cmd.arg(command);
    let status = cmd.status().context("Failed to execute ssh")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("ssh exited with {}", status)
    }
}

/// Run a remote command with stdin data (e.g. pipe a script into bash).
pub fn ssh_run_with_stdin(host: &Host, command: &str, stdin_data: &[u8]) -> Result<()> {
    let mut cmd = build_ssh_command(host)?;
    cmd.arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().context("Failed to execute ssh")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data)
            .context("Failed to write ssh stdin")?;
    }
    let status = child.wait().context("Failed to wait for ssh")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("ssh exited with {}", status)
    }
}

/// Pipe stdin; inherit stderr (live logs); capture stdout.
///
/// Does not auto-bail on non-zero exit — caller interprets status and RESULT lines.
/// Uses an explicit stdout reader thread because `wait_with_output` requires piped
/// stderr and would conflict with inherited live logs.
pub fn ssh_run_inherit_stderr_capture_stdout(
    host: &Host,
    command: &str,
    stdin_data: &[u8],
) -> Result<(ExitStatus, String)> {
    let mut cmd = build_ssh_command(host)?;
    cmd.arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().context("Failed to execute ssh")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data)
            .context("Failed to write ssh stdin")?;
    }
    let mut stdout_pipe = child
        .stdout
        .take()
        .context("Failed to take ssh stdout pipe")?;
    let handle = std::thread::spawn(move || {
        let mut buf = String::new();
        Read::read_to_string(&mut stdout_pipe, &mut buf).ok();
        buf
    });
    let status = child.wait().context("Failed to wait for ssh")?;
    let stdout = handle.join().unwrap_or_default();
    Ok((status, stdout))
}

/// Run `command` on `host` with `stdin_data` piped in; capture stdout and stderr.
/// Returns captured stdout on success. On non-zero exit, `Err` includes the status
/// code and trimmed stderr in its message.
pub fn ssh_run_capture(host: &Host, command: &str, stdin_data: &[u8]) -> Result<String> {
    let mut cmd = build_ssh_command(host)?;
    cmd.arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("Failed to execute ssh")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_data)
            .context("Failed to write ssh stdin")?;
    }
    // wait_with_output drains stdout and stderr concurrently via internal threads,
    // avoiding deadlock when remote stderr volume exceeds the pipe buffer.
    let output = child.wait_with_output().context("Failed to wait for ssh")?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        anyhow::bail!("ssh exited with {}: {}", output.status, stderr.trim());
    }
    if !stderr.trim().is_empty() {
        // Non-fatal warnings from the remote — let callers surface them.
        crate::console::log_verbose(format!("ssh stderr: {}", stderr.trim()));
    }
    Ok(stdout)
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
            github_ssh_key: None,
            ssh_forward_agent: None,
            dir_base: None,
            repos: Some(Vec::<HostRepoRef>::new()),
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

    #[test]
    fn ssh_run_capture_localhost_returns_stdout() {
        let h = host("localhost");
        let out = ssh_run_capture(&h, "cat", b"hello\nworld\n").unwrap();
        assert_eq!(out, "hello\nworld\n");
    }

    #[test]
    fn ssh_run_capture_localhost_propagates_failure_with_stderr() {
        let h = host("localhost");
        let err = ssh_run_capture(&h, "echo boom >&2; exit 7", b"").unwrap_err();
        let s = err.to_string();
        assert!(s.contains("7"), "exit code in error: {}", s);
        assert!(s.contains("boom"), "stderr in error: {}", s);
    }

    #[test]
    fn inherit_stderr_capture_stdout_localhost() {
        let h = host("localhost");
        // Use printf (portable): localhost path is `sh -lc`, and dash lacks $'\t'.
        let (status, out) = ssh_run_inherit_stderr_capture_stdout(
            &h,
            "echo err >&2; printf 'result\\tfail\\tr1\\n'; exit 1",
            b"",
        )
        .unwrap();
        assert!(!status.success());
        assert!(out.contains("result\tfail\tr1"));
    }
}
