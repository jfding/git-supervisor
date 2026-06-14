use std::fs;
use std::process::Command;

fn write_config(path: &std::path::Path, dir_base: &str) {
    let yaml = format!(
        "repos:\n  webapp:\n    git_url: https://example.invalid/webapp.git\n  api:\n    git_url: https://example.invalid/api.git\nhosts:\n  local:\n    ssh_target: localhost\n    dir_base: {dir_base}\n    repos: [webapp, api]\n"
    );
    fs::write(path, yaml).unwrap();
}

fn fixture(base: &std::path::Path) {
    fs::create_dir_all(base.join("git_repos/webapp")).unwrap();
    fs::create_dir_all(base.join("git_repos/api")).unwrap();
    fs::create_dir_all(base.join("copies/webapp.main")).unwrap(); // live
    fs::create_dir_all(base.join("copies/webapp.prod.v1.0.to-be-removed")).unwrap(); // stale
    fs::create_dir_all(base.join("copies/api.dev.to-be-removed")).unwrap(); // stale
}

fn run(cfg: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let mut args = vec!["--config", cfg.to_str().unwrap(), "cleanup"];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_git-supervisor"))
        .args(&args)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn cleanup_dry_run_lists_without_deleting() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &[]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("webapp.prod.v1.0.to-be-removed"), "stdout: {stdout}");
    assert!(stdout.contains("api.dev.to-be-removed"), "stdout: {stdout}");
    assert!(stdout.contains("would remove 2"), "summary missing; stdout: {stdout}");
    assert!(stdout.contains("--apply"), "dry-run should hint --apply; stdout: {stdout}");
    // Nothing deleted.
    assert!(tmp.path().join("copies/webapp.prod.v1.0.to-be-removed").is_dir());
    assert!(tmp.path().join("copies/api.dev.to-be-removed").is_dir());
}

#[test]
fn cleanup_apply_deletes_only_stale() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &["--apply"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("removed 2"), "summary missing; stdout: {stdout}");
    // Stale dirs gone, live dir intact.
    assert!(!tmp.path().join("copies/webapp.prod.v1.0.to-be-removed").exists());
    assert!(!tmp.path().join("copies/api.dev.to-be-removed").exists());
    assert!(tmp.path().join("copies/webapp.main").is_dir(), "live copy must survive");
}

#[test]
fn cleanup_host_filter_no_match_errors() {
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &["--host", "nope-*"]);
    assert!(!out.status.success(), "expected non-zero exit for zero matches");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("no hosts matched"), "stderr: {stderr}");
}

#[test]
fn cleanup_empty_host_reports_nothing_to_clean() {
    let tmp = tempfile::tempdir().unwrap();
    // git_repos + copies exist but no *.to-be-removed dirs.
    fs::create_dir_all(tmp.path().join("git_repos/webapp")).unwrap();
    fs::create_dir_all(tmp.path().join("copies/webapp.main")).unwrap();
    fs::create_dir_all(tmp.path().join("git_repos/api")).unwrap();
    let cfg = tmp.path().join("config.yaml");
    write_config(&cfg, tmp.path().to_str().unwrap());

    let out = run(&cfg, &[]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("(nothing to clean)"), "stdout: {stdout}");
}
