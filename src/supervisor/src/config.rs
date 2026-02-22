use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CentralConfig {
    pub defaults: Option<Defaults>,
    pub hosts: HashMap<String, Host>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Defaults {
    pub dir_base: Option<String>,
    pub branch_whitelist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Host {
    pub ssh_target: String,
    pub ssh_port: Option<u16>,
    pub ssh_identity_file: Option<String>,
    pub dir_base: Option<String>,
    pub repos: Vec<Repo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Repo {
    pub name: String,
    pub git_url: String,
    pub branch_whitelist: Option<Vec<String>>,
}

impl CentralConfig {
    /// Load central config from a YAML file path.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: CentralConfig = serde_yaml::from_str(&content)
            .context("Failed to parse YAML config")?;
        if config.hosts.is_empty() {
            anyhow::bail!("Config must have at least one host under 'hosts'");
        }
        Ok(config)
    }

    /// Resolve dir_base for a host (host override or default).
    pub fn dir_base_for_host(&self, host_id: &str) -> PathBuf {
        let host = self.hosts.get(host_id);
        let base = host
            .and_then(|h| h.dir_base.as_deref())
            .or_else(|| self.defaults.as_ref().and_then(|d| d.dir_base.as_deref()))
            .unwrap_or("/work");
        PathBuf::from(base)
    }

    /// dir_repos = dir_base / "git_repos"
    pub fn dir_repos_for_host(&self, host_id: &str) -> PathBuf {
        self.dir_base_for_host(host_id).join("git_repos")
    }

    /// dir_copies = dir_base / "copies"
    pub fn dir_copies_for_host(&self, host_id: &str) -> PathBuf {
        self.dir_base_for_host(host_id).join("copies")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_yaml() {
        let yaml = r#"
defaults:
  dir_base: /work
  branch_whitelist: [main, master, dev]

hosts:
  app-server:
    ssh_target: deploy@app-server.example.com
    repos:
      - name: webapp
        git_url: git@github.com:org/webapp.git
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.defaults.as_ref().unwrap().dir_base.as_deref(), Some("/work"));
        let host = config.hosts.get("app-server").unwrap();
        assert_eq!(host.ssh_target, "deploy@app-server.example.com");
        assert_eq!(host.repos.len(), 1);
        assert_eq!(host.repos[0].name, "webapp");
        assert_eq!(host.repos[0].git_url, "git@github.com:org/webapp.git");
    }

    #[test]
    fn empty_hosts_fails_validation() {
        let yaml = r#"
defaults:
  dir_base: /work
hosts: {}
"#;
        let _config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        let path = std::env::temp_dir().join("supervisor-empty-hosts.yaml");
        std::fs::write(&path, yaml).unwrap();
        let result = CentralConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn empty_repos_list_valid() {
        let yaml = r#"
hosts:
  empty-host:
    ssh_target: user@host
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.hosts.get("empty-host").unwrap().repos.is_empty());
    }

    #[test]
    fn defaults_dir_base_applied_when_host_has_none() {
        let yaml = r#"
defaults:
  dir_base: /var/work

hosts:
  h:
    ssh_target: u@h
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.dir_base_for_host("h"), PathBuf::from("/var/work"));
        assert_eq!(config.dir_repos_for_host("h"), PathBuf::from("/var/work/git_repos"));
        assert_eq!(config.dir_copies_for_host("h"), PathBuf::from("/var/work/copies"));
    }

    #[test]
    fn host_dir_base_overrides_defaults() {
        let yaml = r#"
defaults:
  dir_base: /work

hosts:
  h:
    ssh_target: u@h
    dir_base: /var/work
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.dir_base_for_host("h"), PathBuf::from("/var/work"));
    }
}
