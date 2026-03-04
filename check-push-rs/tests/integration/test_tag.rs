// Integration tests for tag processing
// These tests require a test git repository to be set up

#[cfg(test)]
mod tests {
    use check_push_rs::*;

    #[test]
    #[ignore] // Requires git repository setup
    fn test_tag_processing() {
        // This test would:
        // 1. Create a temporary git repository
        // 2. Create a test tag (e.g., v1.0.0)
        // 3. Process the tag
        // 4. Verify files are copied correctly
        // 5. Verify .latest symlink is updated
    }
}
