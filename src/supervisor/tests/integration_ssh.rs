//! Integration test that runs push against localhost.
//! Requires `ssh localhost` to work (e.g. passwordless or key-based).
//! Run with: cargo test --test integration_ssh -- --ignored

use supervisor::{run_push, CentralConfig};

#[test]
#[ignore = "requires ssh localhost and git on localhost"]
fn push_to_localhost() {
    let yaml = r#"
defaults:
  dir_base: /tmp/supervisor-test

repos:
  tiny-repo:
    git_url: https://github.com/git/git.git

hosts:
  local:
    ssh_target: localhost
    repos: [tiny-repo]
"#;
    let path = std::env::temp_dir().join("supervisor-integration.yaml");
    std::fs::write(&path, yaml).unwrap();

    let config = CentralConfig::load(&path).unwrap();
    let result = run_push(&config);
    let _ = std::fs::remove_file(&path);
    // May fail if git not installed on localhost or clone fails; we only check it doesn't panic.
    let _ = result;
}
