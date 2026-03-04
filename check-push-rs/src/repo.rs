use crate::branch::BranchProcessor;
use crate::cleanup;
use crate::config::Config;
use crate::error::{Error, Result};
use crate::file_ops;
use crate::git::GitRepo;
use crate::tag::TagProcessor;

pub struct RepoProcessor {
    config: Config,
    branch_processor: BranchProcessor,
    tag_processor: TagProcessor,
}

impl RepoProcessor {
    pub fn new(config: Config) -> Self {
        RepoProcessor {
            branch_processor: BranchProcessor::new(config.clone()),
            tag_processor: TagProcessor::new(config.clone()),
            config,
        }
    }

    /// Process a single repository
    pub fn process_repository(&self, repo_name: &str) -> Result<()> {
        let repo_path = self.config.dir_repos.join(repo_name);

        if !repo_path.join(".git").exists() {
            return Ok(()); // Not a git repository
        }

        let git_repo = GitRepo::open(&repo_path)?;

        // Remove stale index lock
        git_repo.remove_index_lock()?;

        // Fetch all branches and tags
        tracing::info!("..fetching repo ...");
        git_repo.fetch_all_tags()?;

        // Process branches
        let branches = git_repo.get_remote_branches()?;
        for branch in branches {
            // Check whitelist or if copy directory exists
            if self.config.is_branch_whitelisted(&branch)
                || self.config.dir_copies.join(format!("{}.{}", repo_name, branch)).exists()
            {
                if let Err(e) = self.branch_processor.process_branch(&git_repo, repo_name, &branch) {
                    tracing::error!("Failed to process branch {}: {}", branch, e);
                    // Continue with other branches
                }

                // Update heartbeat
                let cp_path = self.config.dir_copies.join(format!("{}.{}", repo_name, branch));
                if cp_path.exists() {
                    crate::file_ops::touch_file(&cp_path.join(".living"))?;
                }
            }
        }

        // Process tags
        let tags = git_repo.get_release_tags()?;
        for tag in tags {
            if let Err(e) = self.tag_processor.process_tag(&git_repo, repo_name, &tag) {
                tracing::error!("Failed to process tag {}: {}", tag, e);
                // Continue with other tags
            }

            // Update heartbeat
            let cp_path = self.config.dir_copies.join(format!("{}.prod.{}", repo_name, tag));
            if cp_path.exists() {
                crate::file_ops::touch_file(&cp_path.join(".living"))?;
            }
        }

        // Cleanup deprecated directories
        cleanup::cleanup_deprecated_dirs(repo_name, &self.config.dir_copies)?;

        Ok(())
    }
}
