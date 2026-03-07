use crate::cleanup;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::file_ops::{check_flag_file, copy_repo_files, create_directory, create_symlink, touch_file};
use crate::git::GitRepo;
use crate::scripts;
use crate::version::version_less_than;
use std::path::Path;

pub struct TagProcessor {
    config: Config,
}

impl TagProcessor {
    pub fn new(config: Config) -> Self {
        TagProcessor { config }
    }

    /// Process a release tag
    pub fn process_tag(&self, repo: &GitRepo, repo_name: &str, tag: &str) -> Result<()> {
        let cp_path = self.config.dir_copies.join(format!("{}.prod.{}", repo_name, tag));
        let arch_path = self.config.dir_copies.join(".archives").join(format!("{}.prod.{}", repo_name, tag));
        let post_path = self.config.dir_copies.join(format!("{}.prod.post", repo_name));
        let docker_path = self.config.dir_copies.join(format!("{}.prod.docker", repo_name));
        let latest_path = self.config.dir_copies.join(format!("{}.prod.latest", repo_name));

        // If copy path exists, skip
        if cp_path.exists() {
            return Ok(());
        }

        // If archive path exists, skip
        if arch_path.exists() {
            return Ok(());
        }

        // Checkout the tag
        repo.checkout_tag(tag)?;

        // Create directory and copy files
        create_directory(&cp_path)?;
        copy_repo_files(repo.path(), &cp_path, true)?;
        tracing::info!("..copy files for new RELEASE [ {} ]", tag);

        // Update .latest symlink
        self.update_latest_symlink(&latest_path, &cp_path, tag)?;

        // Run post scripts
        scripts::run_post_script(&self.config, &post_path, &cp_path)?;

        // Restart docker
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(scripts::restart_docker(&self.config, &docker_path))?;

        // Update heartbeat
        touch_file(&cp_path.join(".living"))?;

        Ok(())
    }

    fn update_latest_symlink(&self, latest_path: &Path, cp_path: &Path, tag: &str) -> Result<()> {
        if latest_path.is_symlink() {
            // Read current symlink target
            if let Ok(current_target) = std::fs::read_link(latest_path) {
                // Extract tag from path like "repo.prod.v1.0.0"
                if let Some(current_tag) = current_target
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|s| {
                        // Extract tag after ".prod."
                        s.split(".prod.").nth(1).map(String::from)
                    })
                {
                    // Compare versions
                    if let Ok(true) = version_less_than(&current_tag, tag) {
                        // New tag is newer, update symlink
                        std::fs::remove_file(latest_path)?;
                        create_symlink(&cp_path, latest_path)?;
                    }
                } else {
                    // Could not parse current tag, update anyway
                    std::fs::remove_file(latest_path)?;
                    create_symlink(&cp_path, latest_path)?;
                }
            }
        } else {
            // No existing symlink, create one
            create_symlink(&cp_path, latest_path)?;
        }

        Ok(())
    }
}
