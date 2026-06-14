#![allow(unused_imports)] // imports consumed in Task 4 (render/run_cleanup)
use crate::config::CentralConfig;
use crate::config::Host;
use crate::console::{self, paint, Color};
use crate::ssh;
use crate::status::{format_relative_time, glob_match};
use std::collections::BTreeMap;
use std::path::Path;

/// Embedded cleanup probe/reaper script, run on remotes with `DIR_BASE`/`HOST_ID`/`APPLY` env.
pub const CLEANUP_PROBE_SCRIPT: &str = include_str!("cleanup_probe.sh");

/// What happened (or would happen) to a stale dir.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Outcome {
    WouldRemove,
    Removed,
    Failed,
}

/// One parsed row from the cleanup probe. One per `*.to-be-removed` dir.
#[derive(Debug, Clone)]
pub struct CleanupReport {
    pub host: String,
    pub repo: String,
    pub name: String,
    pub mtime_unix: u64,
    pub outcome: Outcome,
    pub reason: String,
}

impl CleanupReport {
    /// Parse one TSV line. Strict 6-column contract; returns None on any mismatch.
    pub fn parse_line(line: &str) -> Option<Self> {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 6 {
            return None;
        }
        let outcome = match cols[4] {
            "would-remove" => Outcome::WouldRemove,
            "removed" => Outcome::Removed,
            "failed" => Outcome::Failed,
            _ => return None,
        };
        // Non-numeric or negative mtimes → 0.
        let mtime_unix = cols[3]
            .parse::<i64>()
            .ok()
            .filter(|n| *n >= 0)
            .map(|n| n as u64)
            .unwrap_or(0);
        let reason = if cols[5] == "-" { String::new() } else { cols[5].to_string() };
        Some(Self {
            host: cols[0].to_string(),
            repo: cols[1].to_string(),
            name: cols[2].to_string(),
            mtime_unix,
            outcome,
            reason,
        })
    }
}

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parses_would_remove_line() {
        let line = "app1\twebapp\twebapp.prod.v10.0.to-be-removed\t1747720000\twould-remove\t-";
        let r = CleanupReport::parse_line(line).expect("parse");
        assert_eq!(r.host, "app1");
        assert_eq!(r.repo, "webapp");
        assert_eq!(r.name, "webapp.prod.v10.0.to-be-removed");
        assert_eq!(r.mtime_unix, 1747720000);
        assert_eq!(r.outcome, Outcome::WouldRemove);
        assert!(r.reason.is_empty());
    }

    #[test]
    fn parses_removed_line() {
        let line = "h\tapi\tapi.dev.to-be-removed\t0\tremoved\t-";
        let r = CleanupReport::parse_line(line).unwrap();
        assert_eq!(r.outcome, Outcome::Removed);
        assert_eq!(r.mtime_unix, 0);
    }

    #[test]
    fn parses_failed_line_with_reason() {
        let line = "h\t-\tevil.to-be-removed\t0\tfailed\toutside copies tree";
        let r = CleanupReport::parse_line(line).unwrap();
        assert_eq!(r.outcome, Outcome::Failed);
        assert_eq!(r.repo, "-");
        assert_eq!(r.reason, "outside copies tree");
    }

    #[test]
    fn rejects_wrong_column_count() {
        assert!(CleanupReport::parse_line("a\tb\tc").is_none());
        assert!(CleanupReport::parse_line("a\tb\tc\td\te\tf\tg").is_none());
    }

    #[test]
    fn rejects_unknown_outcome() {
        assert!(CleanupReport::parse_line("h\tr\tn\t0\tbogus\t-").is_none());
    }

    #[test]
    fn negative_mtime_maps_to_zero() {
        let r = CleanupReport::parse_line("h\tr\tn\t-5\twould-remove\t-").unwrap();
        assert_eq!(r.mtime_unix, 0);
    }
}
