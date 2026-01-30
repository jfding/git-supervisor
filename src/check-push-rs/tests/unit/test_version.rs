#[cfg(test)]
mod tests {
    use check_push_rs::version::{compare_versions, version_less_than};
    use std::cmp::Ordering;

    #[test]
    fn test_version_comparison_basic() {
        assert_eq!(
            compare_versions("v1.0.0", "v1.0.1").unwrap(),
            Ordering::Less
        );
        assert_eq!(
            compare_versions("v1.0.1", "v1.0.0").unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("v1.0.0", "v1.0.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_without_prefix() {
        assert_eq!(
            compare_versions("1.0.0", "v1.0.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_with_q_separator() {
        assert_eq!(
            compare_versions("v1Q0.0", "v1.0.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_less_than() {
        assert!(version_less_than("v1.0.0", "v1.0.1").unwrap());
        assert!(!version_less_than("v1.0.1", "v1.0.0").unwrap());
        assert!(!version_less_than("v1.0.0", "v1.0.0").unwrap());
    }

    #[test]
    fn test_version_padding() {
        assert_eq!(
            compare_versions("v1.0", "v1.0.0").unwrap(),
            Ordering::Equal
        );
        assert_eq!(
            compare_versions("v1.0.0", "v1.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_invalid() {
        assert!(compare_versions("", "v1.0.0").is_err());
        assert!(compare_versions("v1.0.0", "").is_err());
        assert!(compare_versions("v1.a.0", "v1.0.0").is_err());
    }
}
