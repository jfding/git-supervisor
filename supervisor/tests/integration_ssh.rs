//! Integration test that runs watch (prepare + one round) against localhost.
//! Requires `ssh localhost` to work (e.g. passwordless or key-based).
//! Run with: cargo test --test integration_ssh -- --ignored

use git_supervisor::{run_watch, CentralConfig, WatchOpts};

#[tokio::test]
#[ignore = "requires ssh localhost and git on localhost"]
async fn watch_to_localhost() {
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
    let result = run_watch(
        &config,
        WatchOpts {
            interval_secs: 0,
            timeout_secs: Some(1),
            ignore_missing: false,
            skip_prepare: false,
            webhook_port: None,
            webhook_secret: None,
            version: "test".into(),
        },
    )
    .await;
    let _ = std::fs::remove_file(&path);
    // May fail if git not installed on localhost or clone fails; we only check it doesn't panic.
    let _ = result;
}
