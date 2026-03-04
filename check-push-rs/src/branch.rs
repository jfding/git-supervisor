use crate::config::Config;
use crate::error::{Error, Result};
use crate::file_ops::{check_flag_file, copy_repo_files, create_directory, is_directory_empty, touch_file};
use crate::git::GitRepo;
use crate::scripts;
use std::path::{Path, PathBuf};

pub struct BranchProcessor {
    config: Config,
}

impl BranchProcessor {
    pub fn new(config: Config) -> Self {
        BranchProcessor { config }
    }

    /// Process a branch
    pub fn process_branch(&self, repo: &GitRepo, repo_name: &str, branch: &str) -> Result<()> {
        let cp_path = self.config.dir_copies.join(format!("{}.{}", repo_name, branch));
        let post_path = self.config.dir_copies.join(format!("{}.{}.post", repo_name, branch));
        let docker_path = self.config.dir_copies.join(format!("{}.{}.docker", repo_name, branch));

        // If no copy directory exists, create it with .skipping flag
        if !cp_path.exists() {
            create_directory(&cp_path)?;
            touch_file(&cp_path.join(".skipping"))?;
            tracing::info!("..init dir of [ {} ]", branch);
        }

        // Check flags
        if check_flag_file(&cp_path, ".debugging") {
            tracing::debug!("..skip debugging work copy of branch [ {} ]", branch);
            return Ok(());
        }

        if check_flag_file(&cp_path, ".skipping") {
            tracing::debug!("..skip unused branch [ {} ]", branch);
            return Ok(());
        }

        // Checkout the branch
        repo.checkout_branch(branch)?;

        // Initialize files if directory is empty
        if is_directory_empty(&cp_path) {
            copy_repo_files(repo.path(), &cp_path, true)?;
            tracing::info!("..copy files for [ {} ]", branch);
        }

        // Check for changes
        let diff = repo.diff_branch(branch)?;

        // Check for .trigger file
        let trigger_file = cp_path.join(".trigger");
        let mut should_update = !diff.is_empty();

        if trigger_file.exists() {
            // Burn after reading
            std::fs::remove_file(&trigger_file)
                .map_err(|e| Error::file(format!("Failed to remove trigger file: {}", e)))?;

            if diff.is_empty() {
                tracing::info!("..having a debug try");
                should_update = true;
            }
        }

        if should_update {
            tracing::info!("..UPDATING branch [ {} ]", branch);

            // Checkout and reset branch to match remote
            repo.checkout_and_reset_branch(branch)
                .map_err(|e| {
                    tracing::error!("..failed git checkout and skip: {}", e);
                    e
                })?;

            // Copy files
            let no_cleanup = check_flag_file(&cp_path, ".no-cleanup");
            copy_repo_files(repo.path(), &cp_path, !no_cleanup)?;

            // Run post scripts
            scripts::run_post_script(&self.config, &post_path, &cp_path)?;

            // Restart docker
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(scripts::restart_docker(&self.config, &docker_path))?;
        } else {
            tracing::debug!("..no change of branch [ {} ], skip", branch);
        }

        // Update heartbeat
        touch_file(&cp_path.join(".living"))?;

        Ok(())
    }
}
