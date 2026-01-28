use crate::error::{Error, Result};
use crate::file_ops::{read_docker_container, sanitize_path};
use crate::config::Config;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::timeout as tokio_timeout;

/// Run a post-deployment script
pub fn run_post_script(config: &Config, script_path: &Path, copy_path: &Path) -> Result<()> {
    if !script_path.exists() {
        return Ok(()); // No script to run
    }

    // Validate script path is within expected directory
    let scripts_dir = &config.dir_scripts;
    sanitize_path(scripts_dir, script_path.to_str().unwrap())?;

    // Also check if script is in copies directory (for branch-specific scripts)
    let copies_dir = &config.dir_copies;
    let _ = sanitize_path(copies_dir, script_path.to_str().unwrap());

    tracing::info!("..running post scripts [ {:?} ]", script_path);

    // Change to copy directory and execute script
    let output = Command::new("bash")
        .arg(script_path)
        .current_dir(copy_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| Error::script(format!("Failed to execute post script: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::script(format!("Post script failed: {}", stderr)));
    }

    // Log output if verbose
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        tracing::debug!("Post script output: {}", stdout);
    }

    Ok(())
}

/// Restart a docker container
pub async fn restart_docker(config: &Config, docker_path: &Path) -> Result<()> {
    if !docker_path.exists() {
        return Ok(()); // No docker config
    }

    // Read container name
    let container_name = read_docker_container(docker_path)?;

    tracing::info!("..restarting docker [ {} ]", container_name);

    // Execute docker restart with timeout
    let timeout_duration = Duration::from_secs(config.timeout);

    let restart_future = async {
        let output = Command::new("docker")
            .arg("restart")
            .arg(&container_name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();

        match output {
            Ok(output) => {
                if output.status.success() {
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(Error::docker(format!("Docker restart failed: {}", stderr)))
                }
            }
            Err(e) => Err(Error::docker(format!("Failed to execute docker restart: {}", e))),
        }
    };

    match tokio_timeout(timeout_duration, restart_future).await {
        Ok(result) => result,
        Err(_) => Err(Error::Timeout),
    }
}
