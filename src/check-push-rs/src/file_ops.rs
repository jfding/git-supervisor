use crate::error::{Error, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sanitize a path to prevent directory traversal attacks
pub fn sanitize_path(base: &Path, path: &str) -> Result<PathBuf> {
    let path_buf = PathBuf::from(path);

    // Resolve the path relative to base
    let resolved = base.join(&path_buf);

    // Canonicalize to resolve symlinks and get absolute path
    let canonical = resolved.canonicalize()
        .map_err(|_| Error::path(format!("Path does not exist or cannot be resolved: {:?}", resolved)))?;

    // Ensure the canonical path is within the base directory
    let base_canonical = base.canonicalize()
        .map_err(|_| Error::path(format!("Base path does not exist: {:?}", base)))?;

    if !canonical.starts_with(&base_canonical) {
        return Err(Error::path(format!(
            "Path traversal detected: {:?} is not within {:?}",
            canonical, base_canonical
        )));
    }

    Ok(canonical)
}

/// Create a directory, creating parent directories if needed
pub fn create_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|e| Error::file(format!("Failed to create directory {:?}: {}", path, e)))
}

/// Touch a file (create or update timestamp)
pub fn touch_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_directory(parent)?;
    }

    // Create file if it doesn't exist, or update timestamp if it does
    fs::File::create(path)
        .map_err(|e| Error::file(format!("Failed to touch file {:?}: {}", path, e)))?;

    Ok(())
}

/// Create or update a symlink
pub fn create_symlink(target: &Path, link_path: &Path) -> Result<()> {
    if let Some(parent) = link_path.parent() {
        create_directory(parent)?;
    }

    // Remove existing symlink or file
    if link_path.exists() || link_path.is_symlink() {
        fs::remove_file(link_path)
            .map_err(|e| Error::file(format!("Failed to remove existing link {:?}: {}", link_path, e)))?;
    }

    // Get relative path for symlink
    let link_dir = link_path.parent()
        .ok_or_else(|| Error::file(format!("Invalid link path: {:?}", link_path)))?;

    let relative_target = pathdiff::diff_paths(target, link_dir)
        .ok_or_else(|| Error::file(format!("Cannot create relative path from {:?} to {:?}", link_dir, target)))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(&relative_target, link_path)
            .map_err(|e| Error::file(format!("Failed to create symlink {:?} -> {:?}: {}", link_path, relative_target, e)))?;
    }

    #[cfg(not(unix))]
    {
        // On Windows, use junction or copy
        return Err(Error::file("Symlinks not supported on this platform"));
    }

    Ok(())
}

/// Check if a flag file exists
pub fn check_flag_file(path: &Path, flag: &str) -> bool {
    let flag_path = path.join(flag);
    flag_path.exists() && flag_path.is_file()
}

/// Copy repository files using rsync
pub fn copy_repo_files(source: &Path, dest: &Path, delete: bool) -> Result<()> {
    // Ensure destination directory exists
    create_directory(dest)?;

    // Build rsync command
    let mut cmd = Command::new("rsync");
    cmd.arg("-a"); // Archive mode

    if delete {
        cmd.arg("--delete"); // Delete files in dest that don't exist in source
    }

    cmd.arg("--exclude").arg(".git");

    // Ensure source path ends with / for rsync
    let source_str = source.to_string_lossy();
    let source_arg = if source_str.ends_with('/') {
        source_str.to_string()
    } else {
        format!("{}/", source_str)
    };

    cmd.arg(&source_arg);
    cmd.arg(dest);

    let output = cmd.output()
        .map_err(|e| Error::file(format!("Failed to execute rsync: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::file(format!("rsync failed: {}", stderr)));
    }

    Ok(())
}

/// Check if a directory is empty
pub fn is_directory_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true,
    }
}

/// Read container name from docker file
pub fn read_docker_container(path: &Path) -> Result<String> {
    let content = fs::read_to_string(path)
        .map_err(|e| Error::docker(format!("Failed to read docker file {:?}: {}", path, e)))?;

    let container = content.trim();

    // Validate container name (basic sanitization)
    if container.is_empty() {
        return Err(Error::docker("Docker container name is empty"));
    }

    // Check for potentially dangerous characters
    if container.contains('\n') || container.contains('\r') || container.contains('\0') {
        return Err(Error::docker("Docker container name contains invalid characters"));
    }

    Ok(container.to_string())
}
