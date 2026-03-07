use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CentralConfig {
    pub defaults: Option<Defaults>,
    /// Top-level repo definitions (name -> definition). Hosts reference these by name.
    #[serde(default)]
    pub repos: HashMap<String, RepoDef>,
    pub hosts: HashMap<String, Host>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Defaults {
    pub dir_base: Option<String>,
    #[serde(alias = "branch_whitelist")]
    pub branches: Option<Vec<String>>,
}

/// One repo reference in a host's repo list. Can be a plain name or `{ name, branches? }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum HostRepoRef {
    Simple(String),
    Full {
        name: String,
        #[serde(default)]
        #[serde(alias = "branch_whitelist")]
        branches: Option<Vec<String>>,
    },
}

impl HostRepoRef {
    pub fn name(&self) -> &str {
        match self {
            HostRepoRef::Simple(s) => s.as_str(),
            HostRepoRef::Full { name, .. } => name.as_str(),
        }
    }
    pub fn branches(&self) -> Option<&[String]> {
        match self {
            HostRepoRef::Simple(_) => None,
            HostRepoRef::Full { branches, .. } => branches.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Host {
    pub ssh_target: String,
    pub ssh_port: Option<u16>,
    pub ssh_identity_file: Option<String>,
    pub dir_base: Option<String>,
    /// List of repo refs (name or { name, branches? }). Must exist in top-level `repos`.
    #[serde(default)]
    pub repos: Vec<HostRepoRef>,
    /// Per-host: number of release tags to consider (top-N). Passed to remote script as RELEASE_TAG_TOPN.
    #[serde(default)]
    pub release_count: Option<u32>,
    /// Per-host: ERE pattern for release tags. Passed to remote as RELEASE_TAG_PATTERN (script default: ^v[0-9Q.]+$).
    #[serde(default)]
    pub release_tag_pattern: Option<String>,
    /// Per-host: ERE pattern to exclude from release tags. Passed to remote as RELEASE_TAG_EXCLUDE_PATTERN.
    #[serde(default)]
    pub release_tag_exclude_pattern: Option<String>,
}

/// Repo definition (git_url only). Key in `repos` map is the repo name. Branches are set per host/repo.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RepoDef {
    pub git_url: String,
}

/// Resolved repo: name + definition, used when operating on a host. Branches come from host repo entry or defaults.
#[derive(Debug, Clone)]
pub struct Repo {
    pub name: String,
    pub git_url: String,
    pub branches: Option<Vec<String>>,
}

impl CentralConfig {
    /// Load central config from a YAML file path.
    pub fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: CentralConfig =
            serde_yaml::from_str(&content).context("Failed to parse YAML config")?;
        if config.hosts.is_empty() {
            anyhow::bail!("Config must have at least one host under 'hosts'");
        }
        for (host_id, host) in &config.hosts {
            for ref_ in &host.repos {
                let name = ref_.name();
                if !config.repos.contains_key(name) {
                    anyhow::bail!(
                        "Host '{}' references unknown repo '{}'; define it under top-level 'repos'",
                        host_id,
                        name
                    );
                }
            }
        }
        Ok(config)
    }

    /// Resolve repo refs for a host into full Repo values (from top-level `repos`). Branches from host repo entry or defaults.
    pub fn repos_for_host(&self, host_id: &str) -> Vec<Repo> {
        let host = match self.hosts.get(host_id) {
            Some(h) => h,
            None => return vec![],
        };
        let default_branches = self.defaults.as_ref().and_then(|d| d.branches.as_ref());
        host.repos
            .iter()
            .filter_map(|ref_| {
                let name = ref_.name();
                self.repos.get(name).map(|def| {
                    let branches = ref_
                        .branches()
                        .map(|b| b.to_vec())
                        .or_else(|| default_branches.cloned());
                    Repo {
                        name: name.to_string(),
                        git_url: def.git_url.clone(),
                        branches,
                    }
                })
            })
            .collect()
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
  branches: [main, master, dev]

repos:
  webapp:
    git_url: git@github.com:org/webapp.git

hosts:
  app-server:
    ssh_target: deploy@app-server.example.com
    repos: [webapp]
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.defaults.as_ref().unwrap().dir_base.as_deref(),
            Some("/work")
        );
        let host = config.hosts.get("app-server").unwrap();
        assert_eq!(host.ssh_target, "deploy@app-server.example.com");
        assert_eq!(
            host.repos.iter().map(|r| r.name()).collect::<Vec<_>>(),
            vec!["webapp"]
        );
        let repos = config.repos_for_host("app-server");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "webapp");
        assert_eq!(repos[0].git_url, "git@github.com:org/webapp.git");
        assert_eq!(
            repos[0].branches,
            Some(vec![
                "main".to_string(),
                "master".to_string(),
                "dev".to_string()
            ])
        );
    }

    #[test]
    fn empty_hosts_fails_validation() {
        let yaml = r#"
defaults:
  dir_base: /work
repos: {}
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
repos: {}
hosts:
  empty-host:
    ssh_target: user@host
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.repos_for_host("empty-host").is_empty());
    }

    #[test]
    fn defaults_dir_base_applied_when_host_has_none() {
        let yaml = r#"
defaults:
  dir_base: /var/work

repos: {}
hosts:
  h:
    ssh_target: u@h
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.dir_base_for_host("h"), PathBuf::from("/var/work"));
        assert_eq!(
            config.dir_repos_for_host("h"),
            PathBuf::from("/var/work/git_repos")
        );
        assert_eq!(
            config.dir_copies_for_host("h"),
            PathBuf::from("/var/work/copies")
        );
    }

    #[test]
    fn host_dir_base_overrides_defaults() {
        let yaml = r#"
defaults:
  dir_base: /work

repos: {}
hosts:
  h:
    ssh_target: u@h
    dir_base: /var/work
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.dir_base_for_host("h"), PathBuf::from("/var/work"));
    }

    #[test]
    fn unknown_repo_name_fails_validation() {
        let yaml = r#"
repos:
  webapp:
    git_url: git@github.com:org/webapp.git
hosts:
  app-server:
    ssh_target: deploy@host
    repos:
      - webapp
      - nonexistent
"#;
        let _config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        let path = std::env::temp_dir().join("supervisor-unknown-repo.yaml");
        std::fs::write(&path, yaml).unwrap();
        let result = CentralConfig::load(&path);
        let _ = std::fs::remove_file(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    #[test]
    fn invalid_yaml_fails() {
        let yaml = "hosts:\n  bad: [unclosed";
        let result: Result<CentralConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn branches_per_host_repo_entry() {
        let yaml = r#"
defaults:
  dir_base: /work
  branches: [main, master]

repos:
  webapp:
    git_url: git@github.com:org/webapp.git
  api:
    git_url: git@github.com:org/api.git

hosts:
  app-server:
    ssh_target: deploy@host
    repos:
      - name: webapp
        branches: [main, release]
      - name: api
  other-host:
    ssh_target: other@host
    repos: [webapp]
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        let app_repos = config.repos_for_host("app-server");
        assert_eq!(app_repos.len(), 2);
        assert_eq!(app_repos[0].name, "webapp");
        assert_eq!(
            app_repos[0].branches,
            Some(vec!["main".to_string(), "release".to_string()])
        );
        assert_eq!(app_repos[1].name, "api");
        assert_eq!(
            app_repos[1].branches,
            Some(vec!["main".to_string(), "master".to_string()])
        );
        let other_repos = config.repos_for_host("other-host");
        assert_eq!(other_repos.len(), 1);
        assert_eq!(
            other_repos[0].branches,
            Some(vec!["main".to_string(), "master".to_string()])
        );
    }

    #[test]
    fn host_release_count_parsed() {
        let yaml = r#"
repos: {}
hosts:
  with-count:
    ssh_target: u@h
    repos: []
    release_count: 10
  without-count:
    ssh_target: u@h2
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.hosts.get("with-count").unwrap().release_count,
            Some(10)
        );
        assert_eq!(
            config.hosts.get("without-count").unwrap().release_count,
            None
        );
    }

    #[test]
    fn host_release_tag_patterns_parsed() {
        let yaml = r#"
repos: {}
hosts:
  with-patterns:
    ssh_target: u@h
    repos: []
    release_tag_pattern: "^v[0-9]+\\.0$"
    release_tag_exclude_pattern: "^v0\\."
  without-patterns:
    ssh_target: u@h2
    repos: []
"#;
        let config: CentralConfig = serde_yaml::from_str(yaml).unwrap();
        let with_p = config.hosts.get("with-patterns").unwrap();
        assert_eq!(with_p.release_tag_pattern.as_deref(), Some("^v[0-9]+\\.0$"));
        assert_eq!(
            with_p.release_tag_exclude_pattern.as_deref(),
            Some("^v0\\.")
        );
        let without_p = config.hosts.get("without-patterns").unwrap();
        assert_eq!(without_p.release_tag_pattern, None);
        assert_eq!(without_p.release_tag_exclude_pattern, None);
    }
}
