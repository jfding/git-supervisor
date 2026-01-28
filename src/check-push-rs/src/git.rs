use crate::error::{Error, Result};
use git2::{Branch, BranchType, Repository, Signature};
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;

pub struct GitRepo {
    repo: Repository,
}

impl GitRepo {
    pub fn open(path: &Path) -> Result<Self> {
        let repo = Repository::open(path)
            .map_err(|e| Error::Git(e))?;

        Ok(GitRepo { repo })
    }

    /// Remove stale index lock file
    pub fn remove_index_lock(&self) -> Result<()> {
        let lock_path = self.repo.path().join("index.lock");
        if lock_path.exists() {
            std::fs::remove_file(&lock_path)
                .map_err(|e| Error::file(format!("Failed to remove index.lock: {}", e)))?;
        }
        Ok(())
    }

    /// Fetch all branches and tags from all remotes
    pub fn fetch_all_tags(&self) -> Result<()> {
        let mut remote = self.repo.find_remote("origin")
            .or_else(|_| {
                // Try to add origin if it doesn't exist
                self.repo.remote("origin", self.repo.path().to_str().unwrap())
            })?;

        remote.fetch(&["--all", "--tags", "--prune"], None, None)
            .map_err(|e| Error::Git(e))?;

        Ok(())
    }

    /// Get list of remote branch names (excluding HEAD)
    pub fn get_remote_branches(&self) -> Result<Vec<String>> {
        let mut branches = Vec::new();

        let remote_branches = self.repo.branches(Some(BranchType::Remote))
            .map_err(|e| Error::Git(e))?;

        for branch_result in remote_branches {
            let (branch, _) = branch_result.map_err(|e| Error::Git(e))?;

            if let Ok(Some(name)) = branch.name() {
                let name_str = name.to_string();

                // Skip HEAD
                if name_str == "HEAD" {
                    continue;
                }

                // Extract branch name (remove "origin/" prefix)
                if let Some(branch_name) = name_str.strip_prefix("origin/") {
                    // Skip branches with slashes (not top-level branches)
                    if !branch_name.contains('/') {
                        branches.push(branch_name.to_string());
                    }
                }
            }
        }

        Ok(branches)
    }

    /// Get list of release tags matching pattern ^v[Q0-9.]+$
    pub fn get_release_tags(&self) -> Result<Vec<String>> {
        use regex::Regex;

        let pattern = Regex::new(r"^v[Q0-9.]+$")
            .map_err(|e| Error::version(format!("Invalid regex pattern: {}", e)))?;

        let mut tags = Vec::new();

        self.repo.tag_foreach(|_id, name| {
            if let Ok(name_str) = std::str::from_utf8(name) {
                if pattern.is_match(name_str) {
                    tags.push(name_str.to_string());
                }
            }
            true
        })?;

        Ok(tags)
    }

    /// Checkout a branch with force
    pub fn checkout_branch(&self, branch_name: &str) -> Result<()> {
        let refname = format!("origin/{}", branch_name);
        let obj = self.repo.revparse_single(&refname)
            .map_err(|e| Error::Git(e))?;

        self.repo.checkout_tree(&obj, None)
            .map_err(|e| Error::Git(e))?;

        // Update HEAD to point to the branch
        if self.repo.set_head(&refname).is_err() {
            // If branch doesn't exist locally, create it
            let commit = obj.as_commit()
                .ok_or_else(|| Error::Git(git2::Error::from_str("Not a commit")))?;

            self.repo.branch(branch_name, commit, false)
                .map_err(|e| Error::Git(e))?;

            self.repo.set_head(&format!("refs/heads/{}", branch_name))
                .map_err(|e| Error::Git(e))?;
        }

        Ok(())
    }

    /// Checkout and reset branch to match remote
    pub fn checkout_and_reset_branch(&self, branch_name: &str) -> Result<()> {
        let refname = format!("origin/{}", branch_name);
        let obj = self.repo.revparse_single(&refname)
            .map_err(|e| Error::Git(e))?;

        let commit = obj.as_commit()
            .ok_or_else(|| Error::Git(git2::Error::from_str("Not a commit")))?;

        // Create or update local branch
        let _branch = self.repo.branch(branch_name, commit, true)
            .map_err(|e| Error::Git(e))?;

        // Checkout the branch
        self.repo.checkout_tree(&obj, None)
            .map_err(|e| Error::Git(e))?;

        // Update HEAD
        self.repo.set_head(&format!("refs/heads/{}", branch_name))
            .map_err(|e| Error::Git(e))?;

        // Reset to match remote
        self.repo.reset(&obj, git2::ResetType::Hard, None)
            .map_err(|e| Error::Git(e))?;

        Ok(())
    }

    /// Checkout a tag
    pub fn checkout_tag(&self, tag_name: &str) -> Result<()> {
        let obj = self.repo.revparse_single(tag_name)
            .map_err(|e| Error::Git(e))?;

        self.repo.checkout_tree(&obj, None)
            .map_err(|e| Error::Git(e))?;

        // Detach HEAD at the tag
        self.repo.set_head_detached(obj.id())
            .map_err(|e| Error::Git(e))?;

        Ok(())
    }

    /// Compare local branch with remote branch, return list of changed files
    pub fn diff_branch(&self, branch_name: &str) -> Result<Vec<String>> {
        let local_ref = format!("refs/heads/{}", branch_name);
        let remote_ref = format!("refs/remotes/origin/{}", branch_name);

        let local_commit = self.repo.revparse_single(&local_ref).ok();
        let remote_commit = self.repo.revparse_single(&remote_ref)
            .map_err(|e| Error::Git(e))?;

        if let Some(local) = local_commit {
            let local_tree = local.as_commit()
                .and_then(|c| c.tree().ok());
            let remote_tree = remote_commit.as_commit()
                .and_then(|c| c.tree().ok());

            if let (Some(local_tree), Some(remote_tree)) = (local_tree, remote_tree) {
                let diff = self.repo.diff_tree_to_tree(
                    Some(&local_tree),
                    Some(&remote_tree),
                    None,
                ).map_err(|e| Error::Git(e))?;

                let mut changed_files = Vec::new();
                diff.foreach(
                    &mut |delta, _| {
                        if let Some(file) = delta.new_file().path() {
                            changed_files.push(file.to_string_lossy().to_string());
                        }
                        true
                    },
                    None,
                    None,
                    None,
                ).map_err(|e| Error::Git(e))?;

                return Ok(changed_files);
            }
        }

        Ok(vec![])
    }

    /// Get the repository path
    pub fn path(&self) -> &Path {
        self.repo.path().parent().unwrap_or_else(|| Path::new("."))
    }
}
