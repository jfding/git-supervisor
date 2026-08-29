use std::fs;
use std::os::unix::fs::symlink;
use std::process::Command;

fn write_release_with_latest(base: &std::path::Path, repo: &str, tag: &str, sha: &str) {
    let release_dir = base.join("copies").join(format!("{repo}.prod.{tag}"));
    fs::create_dir_all(&release_dir).unwrap();
    fs::create_dir_all(base.join("git_repos").join(repo)).unwrap();
    fs::write(release_dir.join(".git-rev"), sha).unwrap();
    let latest = base.join("copies").join(format!("{repo}.prod.latest"));
    symlink(format!("{repo}.prod.{tag}"), &latest).unwrap();
}

fn write_layout(base: &std::path::Path, repo: &str, branch: &str, sha: &str) {
    let copies = base.join("copies").join(format!("{repo}.{branch}"));
    let repos = base.join("git_repos").join(repo);
    fs::create_dir_all(&copies).unwrap();
    fs::create_dir_all(&repos).unwrap();
    fs::write(copies.join(".git-rev"), sha).unwrap();
}

fn write_config(path: &std::path::Path, dir_base: &str) {
    let yaml = format!(
        "repos:\n  demo:\n    git_url: https://example.invalid/demo.git\n  my.api:\n    git_url: https://example.invalid/my.api.git\nhosts:\n  local:\n    ssh_target: localhost\n    dir_base: {dir_base}\n    repos: [demo, my.api]\n"
    );
    fs::write(path, yaml).unwrap();
}

#[test]
fn status_renders_branch_against_localhost() {
    let tmp = tempfile::tempdir().unwrap();
    write_layout(tmp.path(), "demo", "main", "abcdef1234567");
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = Command::new(env!("CARGO_BIN_EXE_git-supervisor"))
        .args(["--config", cfg.to_str().unwrap(), "status"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("host: local"), "stdout: {stdout}");
    assert!(stdout.contains("demo"), "stdout: {stdout}");
    assert!(stdout.contains("main"), "stdout: {stdout}");
    assert!(stdout.contains("abcdef1"), "expected 7-char SHA truncation; stdout: {stdout}");
    assert!(!stdout.contains("abcdef12"), "SHA should be 7 chars exactly; stdout: {stdout}");
    // Regression: the branch row must render a real mtime, not "-".
    // .git-rev was just written, so the relative time should be "just now".
    let branch_row = stdout.lines().find(|l| l.trim().starts_with("main ")).expect("main row");
    assert!(
        !branch_row.contains(" -  "),
        "branch row missing mtime (showing '-'); got: {branch_row:?}"
    );
    assert!(
        branch_row.contains("just now"),
        "expected 'just now' mtime on freshly-written .git-rev; got: {branch_row:?}"
    );
}

#[test]
fn status_handles_dotted_repo_name() {
    let tmp = tempfile::tempdir().unwrap();
    // Repo name "my.api" with a "main" branch dir should parse as repo=my.api, branch=main —
    // not repo=my, branch=api.main.
    write_layout(tmp.path(), "my.api", "main", "1234567890abc");
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = Command::new(env!("CARGO_BIN_EXE_git-supervisor"))
        .args(["--config", cfg.to_str().unwrap(), "status"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("my.api"), "expected repo 'my.api' in output: {stdout}");
    // "main" should appear on its own line as a branch under my.api, not glued to api.
    assert!(stdout.lines().any(|l| l.trim().starts_with("main ")), "expected 'main' branch row; got: {stdout}");
}

#[test]
fn status_latest_symlink_matches_release_row() {
    // Regression: the probe used to emit the latest's full symlink target
    // ("demo.prod.v1.0.0"), but release rows use just the tag ("v1.0.0").
    // The comparison always failed → no `latest` flag on the release row.
    let tmp = tempfile::tempdir().unwrap();
    write_release_with_latest(tmp.path(), "demo", "v1.0.0", "abc1234567");
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = Command::new(env!("CARGO_BIN_EXE_git-supervisor"))
        .args(["--config", cfg.to_str().unwrap(), "status"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("(latest:"),
        "repo header should not carry latest annotation; got: {stdout}"
    );
    let release_row = stdout
        .lines()
        .find(|l| l.trim().starts_with("v1.0.0 "))
        .expect("release row");
    assert!(
        release_row.contains("latest"),
        "latest release row should carry latest flag; got: {release_row:?}"
    );
}

#[test]
fn status_filter_matches_zero_errors() {
    let tmp = tempfile::tempdir().unwrap();
    write_layout(tmp.path(), "demo", "main", "abc1234567");
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = Command::new(env!("CARGO_BIN_EXE_git-supervisor"))
        .args(["--config", cfg.to_str().unwrap(), "status", "--host", "nope-*"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!out.status.success(), "expected non-zero exit for zero matches");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("no hosts matched"), "stderr: {stderr}");
}
