use crate::error::{Error, Result};
use std::cmp::Ordering;

/// Compare two version strings, handling 'v' prefix and 'Q' separator
/// Returns Ordering::Less if v1 < v2, Ordering::Equal if v1 == v2, Ordering::Greater if v1 > v2
pub fn compare_versions(v1: &str, v2: &str) -> Result<Ordering> {
    if v1.is_empty() || v2.is_empty() {
        return Err(Error::version("Version strings cannot be empty"));
    }

    // Strip 'v' prefix if present
    let v1_clean = v1.strip_prefix('v').unwrap_or(v1);
    let v2_clean = v2.strip_prefix('v').unwrap_or(v2);

    // Parse versions: split by '.', then split each part by 'Q'
    let v1_parts: Vec<Vec<i32>> = v1_clean
        .split('.')
        .map(|part| {
            part.split('Q')
                .map(|n| {
                    n.parse::<i32>()
                        .map_err(|_| Error::version(format!("Invalid version component: {}", n)))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;

    let v2_parts: Vec<Vec<i32>> = v2_clean
        .split('.')
        .map(|part| {
            part.split('Q')
                .map(|n| {
                    n.parse::<i32>()
                        .map_err(|_| Error::version(format!("Invalid version component: {}", n)))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;

    // Flatten the nested vectors
    let mut v1_nums: Vec<i32> = v1_parts.into_iter().flatten().collect();
    let mut v2_nums: Vec<i32> = v2_parts.into_iter().flatten().collect();

    // Pad shorter version with zeros
    let max_len = v1_nums.len().max(v2_nums.len());
    v1_nums.resize(max_len, 0);
    v2_nums.resize(max_len, 0);

    // Compare component by component
    for (n1, n2) in v1_nums.iter().zip(v2_nums.iter()) {
        match n1.cmp(n2) {
            Ordering::Less => return Ok(Ordering::Less),
            Ordering::Greater => return Ok(Ordering::Greater),
            Ordering::Equal => continue,
        }
    }

    Ok(Ordering::Equal)
}

/// Check if version v1 is less than v2
pub fn version_less_than(v1: &str, v2: &str) -> Result<bool> {
    match compare_versions(v1, v2)? {
        Ordering::Less => Ok(true),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
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
        assert_eq!(
            compare_versions("1.0.0", "v1.0.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_with_q() {
        assert_eq!(
            compare_versions("v1Q0.0", "v1.0.0").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn test_version_less_than() {
        assert!(version_less_than("v1.0.0", "v1.0.1").unwrap());
        assert!(!version_less_than("v1.0.1", "v1.0.0").unwrap());
    }
}
