use anyhow::{bail, Context};
use std::path::PathBuf;

/// Search directories for a key name, in priority order.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".config/git-supervisor/keys"));
        dirs.push(home.join(".ssh"));
    }
    dirs
}

/// Resolve a bare key filename to an absolute path.
///
/// Searches `~/.config/git-supervisor/keys/` first, then `~/.ssh/`.
/// Returns an error if the key is not found or has overly permissive permissions.
pub fn resolve(name: &str) -> anyhow::Result<PathBuf> {
    resolve_in_dirs(name, &search_dirs())
}

fn resolve_in_dirs(name: &str, dirs: &[PathBuf]) -> anyhow::Result<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            check_permissions(&candidate)?;
            return Ok(candidate);
        }
    }
    let searched: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    bail!(
        "ssh key '{}' not found in: {}",
        name,
        searched.join(", ")
    )
}

/// Reject key files with permissions more permissive than 0600.
#[cfg(unix)]
fn check_permissions(path: &PathBuf) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("cannot stat key file '{}'", path.display()))?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o177 != 0 {
        bail!(
            "ssh key '{}' has permissions {:04o}, expected 0600 or 0400",
            path.display(),
            mode
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_path: &PathBuf) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn make_key(dir: &std::path::Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "fake-key-content").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn resolve_finds_key_in_first_search_dir() {
        let managed = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        let key_path = make_key(managed.path(), "deploy", 0o600);
        make_key(fallback.path(), "deploy", 0o600);

        let result = resolve_in_dirs("deploy", &[managed.path().into(), fallback.path().into()]);
        assert_eq!(result.unwrap(), key_path);
    }

    #[test]
    fn resolve_falls_back_to_second_dir() {
        let managed = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();
        let key_path = make_key(fallback.path(), "deploy", 0o600);

        let result = resolve_in_dirs("deploy", &[managed.path().into(), fallback.path().into()]);
        assert_eq!(result.unwrap(), key_path);
    }

    #[test]
    fn resolve_errors_when_not_found() {
        let managed = tempfile::tempdir().unwrap();
        let fallback = tempfile::tempdir().unwrap();

        let result = resolve_in_dirs("missing", &[managed.path().into(), fallback.path().into()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_permissive_key() {
        let managed = tempfile::tempdir().unwrap();
        make_key(managed.path(), "loose", 0o644);

        let result = resolve_in_dirs("loose", &[managed.path().into()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("permissions"));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_accepts_0400_key() {
        let managed = tempfile::tempdir().unwrap();
        let key_path = make_key(managed.path(), "readonly", 0o400);

        let result = resolve_in_dirs("readonly", &[managed.path().into()]);
        assert_eq!(result.unwrap(), key_path);
    }
}
