use std::path::PathBuf;
use std::process::Command;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .to_path_buf()
}

fn run_script(path: &std::path::Path) -> std::process::ExitStatus {
    Command::new("bash")
        .arg(path)
        .status()
        .unwrap_or_else(|e| panic!("failed to run {}: {}", path.display(), e))
}

/// Full integration test for check-push.sh: sets up fake git repos, creates test
/// scenarios (docker hooks, .skipping, .trigger, etc.), runs the script, then
/// verifies release tag ordering and docker hook outputs.
///
/// Run with: cargo test -- --ignored
#[test]
#[ignore]
fn check_push_integration() {
    let scripts = project_root().join("core/tests/scripts");

    let status = run_script(&scripts.join("setup-test-repos.sh"));
    assert!(status.success(), "setup-test-repos.sh failed");

    let status = run_script(&scripts.join("create-test-scenarios.sh"));
    assert!(status.success(), "create-test-scenarios.sh failed");

    let status = run_script(&scripts.join("test-check-push.sh"));
    assert!(status.success(), "test-check-push.sh failed");
}
