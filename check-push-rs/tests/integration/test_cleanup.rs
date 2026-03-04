// Integration tests for cleanup logic

#[cfg(test)]
mod tests {
    use check_push_rs::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cleanup_deprecated_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let copies_dir = temp_dir.path().join("copies");
        fs::create_dir_all(&copies_dir).unwrap();

        // Create a test directory without .living file
        let test_dir = copies_dir.join("test.repo.main");
        fs::create_dir_all(&test_dir).unwrap();

        // Run cleanup
        let result = cleanup::cleanup_deprecated_dirs("test.repo", &copies_dir);

        // Directory should be renamed to .to-be-removed
        assert!(result.is_ok());
        assert!(copies_dir.join("test.repo.main.to-be-removed").exists());
    }

    #[test]
    fn test_cleanup_with_living_file() {
        let temp_dir = TempDir::new().unwrap();
        let copies_dir = temp_dir.path().join("copies");
        fs::create_dir_all(&copies_dir).unwrap();

        // Create a test directory with .living file
        let test_dir = copies_dir.join("test.repo.main");
        fs::create_dir_all(&test_dir).unwrap();
        fs::File::create(test_dir.join(".living")).unwrap();

        // Run cleanup
        let result = cleanup::cleanup_deprecated_dirs("test.repo", &copies_dir);

        // Directory should still exist, .living file removed
        assert!(result.is_ok());
        assert!(copies_dir.join("test.repo.main").exists());
        assert!(!copies_dir.join("test.repo.main").join(".living").exists());
    }
}
