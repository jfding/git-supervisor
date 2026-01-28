use crate::error::{Error, Result};
use crate::file_ops::{check_flag_file, create_directory, touch_file};
use std::fs;
use std::path::{Path, PathBuf};

/// Cleanup deprecated directories for a repository
pub fn cleanup_deprecated_dirs(repo_name: &str, copies_dir: &Path) -> Result<()> {
    let pattern = format!("{}.", repo_name);

    // Find all directories matching the pattern
    let entries: Vec<PathBuf> = fs::read_dir(copies_dir)
        .map_err(|e| Error::file(format!("Failed to read copies directory: {}", e)))?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name()?.to_string_lossy();
                if name.starts_with(&pattern) {
                    Some(path)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    for dir_path in entries {
        let dir_name = dir_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Skip .to-be-removed and .latest directories
        if dir_name.contains("to-be-removed") || dir_name.contains(".latest") {
            continue;
        }

        // Handle .stopping flag
        if check_flag_file(&dir_path, ".stopping") {
            // Clean up all content
            if dir_path.exists() {
                fs::remove_dir_all(&dir_path)
                    .map_err(|e| Error::file(format!("Failed to remove directory {:?}: {}", dir_path, e)))?;
            }

            // Recreate directory and mark as skipping
            create_directory(&dir_path)?;
            touch_file(&dir_path.join(".skipping"))?;
            touch_file(&dir_path.join(".living"))?;
            continue;
        }

        // Check for .living file
        let living_file = dir_path.join(".living");
        if living_file.exists() {
            // Remove .living file (will be recreated if still active)
            fs::remove_file(&living_file)
                .map_err(|e| Error::file(format!("Failed to remove .living file: {}", e)))?;
        } else {
            // No .living file, mark for removal
            tracing::info!("..cleaning up deprecated dir: {:?}", dir_path);

            let new_name = format!("{}.to-be-removed", dir_path.to_string_lossy());
            let new_path = PathBuf::from(new_name);

            fs::rename(&dir_path, &new_path)
                .map_err(|e| Error::file(format!("Failed to rename directory {:?}: {}", dir_path, e)))?;
        }
    }

    Ok(())
}
