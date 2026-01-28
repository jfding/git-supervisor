use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub dir_repos: PathBuf,
    pub dir_copies: PathBuf,
    pub dir_scripts: PathBuf,
    pub ci_lock: PathBuf,
    pub verbosity: u8,
    pub timeout: u64,
    pub sleep_time: Option<u64>,
    pub branch_whitelist: Vec<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        // Try to load from config file first
        let config = Self::load_from_file()
            .or_else(|_| Self::load_from_env())
            .map_err(|e| Error::config(format!("Failed to load configuration: {}", e)))?;

        // Validate paths
        config.validate()?;

        Ok(config)
    }

    fn load_from_file() -> Result<Self> {
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
        let config_paths = vec![
            PathBuf::from("/work/.check-push.conf"),
            PathBuf::from(format!("{}/.check-push.conf", home_dir)),
            PathBuf::from(".check-push.conf"),
        ];

        for path in config_paths {
            if path.exists() {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| Error::config(format!("Failed to read config file {:?}: {}", path, e)))?;
                let config: Config = toml::from_str(&content)
                    .map_err(|e| Error::config(format!("Failed to parse config file {:?}: {}", path, e)))?;
                return Ok(config);
            }
        }

        Err(Error::config("No config file found"))
    }

    fn load_from_env() -> Result<Self> {
        let verbosity = env::var("VERB")
            .ok()
            .and_then(|v| u8::from_str(&v).ok())
            .unwrap_or(1);

        let timeout = env::var("TIMEOUT")
            .ok()
            .and_then(|v| u64::from_str(&v).ok())
            .unwrap_or(600);

        let sleep_time = env::var("SLEEP_TIME")
            .ok()
            .and_then(|v| u64::from_str(&v).ok());

        let dir_repos = env::var("DIR_REPOS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/work/git_repos"));

        let dir_copies = env::var("DIR_COPIES")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/work/copies"));

        let dir_scripts = env::var("DIR_SCRIPTS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/work/scripts"));

        let ci_lock = env::var("CI_LOCK")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/.ci-lock"));

        let branch_whitelist = env::var("BR_WHITELIST")
            .unwrap_or_else(|_| "main master dev test alpha".to_string())
            .split_whitespace()
            .map(String::from)
            .collect();

        Ok(Config {
            dir_repos,
            dir_copies,
            dir_scripts,
            ci_lock,
            verbosity,
            timeout,
            sleep_time,
            branch_whitelist,
        })
    }

    pub fn validate(&self) -> Result<()> {
        // Ensure dir_repos exists or can be created
        if !self.dir_repos.exists() {
            std::fs::create_dir_all(&self.dir_repos)
                .map_err(|e| Error::config(format!("Cannot create DIR_REPOS {:?}: {}", self.dir_repos, e)))?;
        }

        // Ensure dir_copies exists or can be created
        if !self.dir_copies.exists() {
            std::fs::create_dir_all(&self.dir_copies)
                .map_err(|e| Error::config(format!("Cannot create DIR_COPIES {:?}: {}", self.dir_copies, e)))?;
        }

        // Ensure dir_scripts exists or can be created
        if !self.dir_scripts.exists() {
            std::fs::create_dir_all(&self.dir_scripts)
                .map_err(|e| Error::config(format!("Cannot create DIR_SCRIPTS {:?}: {}", self.dir_scripts, e)))?;
        }

        // Validate verbosity is 0, 1, or 2
        if self.verbosity > 2 {
            return Err(Error::config("VERB must be 0, 1, or 2"));
        }

        Ok(())
    }

    pub fn is_branch_whitelisted(&self, branch: &str) -> bool {
        self.branch_whitelist.iter().any(|b| b == branch)
    }
}
