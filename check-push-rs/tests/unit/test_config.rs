#[cfg(test)]
mod tests {
    use check_push_rs::config::Config;
    use std::env;

    #[test]
    fn test_config_from_env() {
        // Set test environment variables
        env::set_var("VERB", "2");
        env::set_var("TIMEOUT", "300");
        env::set_var("DIR_REPOS", "/test/repos");
        env::set_var("DIR_COPIES", "/test/copies");
        env::set_var("BR_WHITELIST", "main dev");

        // Note: This test may fail if config file exists
        // In a real scenario, we'd use a temp directory
        let config = Config::load_from_env().unwrap();

        assert_eq!(config.verbosity, 2);
        assert_eq!(config.timeout, 300);
        assert_eq!(config.dir_repos.to_string_lossy(), "/test/repos");
        assert_eq!(config.dir_copies.to_string_lossy(), "/test/copies");
        assert!(config.is_branch_whitelisted("main"));
        assert!(config.is_branch_whitelisted("dev"));
        assert!(!config.is_branch_whitelisted("test"));

        // Cleanup
        env::remove_var("VERB");
        env::remove_var("TIMEOUT");
        env::remove_var("DIR_REPOS");
        env::remove_var("DIR_COPIES");
        env::remove_var("BR_WHITELIST");
    }

    #[test]
    fn test_branch_whitelist() {
        let mut config = Config::load_from_env().unwrap();
        config.branch_whitelist = vec!["main".to_string(), "dev".to_string()];

        assert!(config.is_branch_whitelisted("main"));
        assert!(config.is_branch_whitelisted("dev"));
        assert!(!config.is_branch_whitelisted("test"));
    }
}
