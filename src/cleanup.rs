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

/// Filter options passed to [`run_cleanup`].
pub struct CleanupOpts {
    pub host_patterns: Vec<String>,
    /// When false (default), dry-run: list what would be removed, delete nothing.
    pub apply: bool,
}

// ─── Pure helpers ────────────────────────────────────────────────────────────

/// Group reports by repo name, each repo's rows sorted by dir name ascending.
pub fn group_by_repo(reports: &[CleanupReport]) -> BTreeMap<String, Vec<CleanupReport>> {
    let mut grouped: BTreeMap<String, Vec<CleanupReport>> = BTreeMap::new();
    for r in reports {
        grouped.entry(r.repo.clone()).or_default().push(r.clone());
    }
    for rows in grouped.values_mut() {
        rows.sort_by(|a, b| a.name.cmp(&b.name));
    }
    grouped
}

/// Build the trailing summary line. `host_count` is the number of hosts that
/// contributed at least one report.
pub fn summarize(reports: &[CleanupReport], apply: bool, host_count: usize) -> String {
    if apply {
        let removed = reports.iter().filter(|r| r.outcome == Outcome::Removed).count();
        let failed = reports.iter().filter(|r| r.outcome == Outcome::Failed).count();
        format!("cleanup: removed {}, failed {} across {} host(s)", removed, failed, host_count)
    } else {
        let n = reports.iter().filter(|r| r.outcome == Outcome::WouldRemove).count();
        format!(
            "cleanup: would remove {} stale copies across {} host(s) (run with --apply to delete)",
            n, host_count
        )
    }
}

fn host_filter_matches(patterns: &[String], host_id: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| glob_match(p, host_id))
}

// ─── Host-level probe ────────────────────────────────────────────────────────

enum HostOutcome {
    Ok(Vec<CleanupReport>),
    Empty,          // probe succeeded, no stale dirs (or $DIR_COPIES missing)
    Failed(String), // SSH/probe failed; message includes stderr
}

fn collect_host(host_id: &str, host: &Host, dir_base: &Path, apply: bool) -> HostOutcome {
    let dir_esc = dir_base.to_string_lossy().replace('\'', "'\\''");
    let host_esc = host_id.replace('\'', "'\\''");
    let command = format!(
        "DIR_BASE='{}' HOST_ID='{}' APPLY={} bash -s",
        dir_esc,
        host_esc,
        if apply { 1 } else { 0 },
    );
    match ssh::ssh_run_capture(host, &command, CLEANUP_PROBE_SCRIPT.as_bytes()) {
        Err(e) => HostOutcome::Failed(e.to_string()),
        Ok(out) => {
            let reports: Vec<CleanupReport> =
                out.lines().filter_map(CleanupReport::parse_line).collect();
            if reports.is_empty() {
                HostOutcome::Empty
            } else {
                HostOutcome::Ok(reports)
            }
        }
    }
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render_host(host_id: &str, reports: &[CleanupReport], now: u64) {
    println!("host: {}", host_id);
    for (repo, rows) in group_by_repo(reports) {
        let repo_label = if repo == "-" { "(unmatched)" } else { repo.as_str() };
        println!("  {}", repo_label);
        for r in rows {
            let suffix = match r.outcome {
                Outcome::WouldRemove => {
                    paint(format_relative_time(r.mtime_unix, now), Color::Grey)
                }
                Outcome::Removed => paint("removed", Color::Green),
                Outcome::Failed => paint(format!("failed: {}", r.reason), Color::Red),
            };
            println!("    {:<40}  {}", r.name, suffix);
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Reap stale `*.to-be-removed` copies on each configured host.
/// Dry-run by default; deletes only when `opts.apply` is true. Best-effort:
/// renders everything it can, then returns Err if any host or any deletion failed.
pub fn run_cleanup(config: &CentralConfig, opts: CleanupOpts) -> anyhow::Result<()> {
    // Filter & skip empty-repos hosts (matches run_status's behavior).
    let mut targets: Vec<(String, &Host)> = Vec::new();
    for (host_id, host) in &config.hosts {
        if !host.is_wildcard() && config.repos_for_host(host_id).is_empty() {
            console::log_info(format!(
                "cleanup host {{ {} }} --> skipped (repos: [] is empty)",
                host_id
            ));
            continue;
        }
        if !host_filter_matches(&opts.host_patterns, host_id) {
            continue;
        }
        targets.push((host_id.clone(), host));
    }

    if !opts.host_patterns.is_empty() && targets.is_empty() {
        anyhow::bail!("no hosts matched: {:?}", opts.host_patterns);
    }

    // Parallel fanout across hosts.
    let apply = opts.apply;
    let outcomes: Vec<(String, HostOutcome)> = std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|(host_id, host)| {
                let host_id = host_id.clone();
                let dir_base = config.dir_base_for_host(&host_id);
                let host_ref: &Host = host;
                s.spawn(move || {
                    let outcome = collect_host(&host_id, host_ref, &dir_base, apply);
                    (host_id, outcome)
                })
            })
            .collect();
        let mut out: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().expect("thread panicked"))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    });

    let now = chrono::Local::now().timestamp().max(0) as u64;
    let mut any_failed = false;
    let mut all_reports: Vec<CleanupReport> = Vec::new();
    let mut host_count = 0usize;

    for (host_id, outcome) in &outcomes {
        match outcome {
            HostOutcome::Ok(reports) => {
                host_count += 1;
                render_host(host_id, reports, now);
                if reports.iter().any(|r| r.outcome == Outcome::Failed) {
                    any_failed = true;
                }
                all_reports.extend(reports.iter().cloned());
            }
            HostOutcome::Empty => {
                println!("host: {}  {}", host_id, paint("(nothing to clean)", Color::Grey));
            }
            HostOutcome::Failed(msg) => {
                any_failed = true;
                println!("{}", paint(format!("host: {}  ERROR", host_id), Color::Red));
                let first_line = msg.lines().next().unwrap_or("");
                if !first_line.is_empty() {
                    println!("  {}", paint(first_line, Color::Red));
                }
            }
        }
    }

    println!("{}", summarize(&all_reports, opts.apply, host_count));

    if any_failed {
        anyhow::bail!("cleanup failed on one or more hosts/dirs");
    }
    Ok(())
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

#[cfg(test)]
mod render_tests {
    use super::*;

    fn rep(repo: &str, name: &str, outcome: Outcome) -> CleanupReport {
        CleanupReport {
            host: "h".into(),
            repo: repo.into(),
            name: name.into(),
            mtime_unix: 0,
            outcome,
            reason: String::new(),
        }
    }

    #[test]
    fn group_by_repo_sorts_names() {
        let reports = vec![
            rep("webapp", "webapp.prod.v2.to-be-removed", Outcome::WouldRemove),
            rep("webapp", "webapp.prod.v1.to-be-removed", Outcome::WouldRemove),
            rep("api", "api.dev.to-be-removed", Outcome::WouldRemove),
        ];
        let grouped = group_by_repo(&reports);
        let webapp = grouped.get("webapp").unwrap();
        let names: Vec<&str> = webapp.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["webapp.prod.v1.to-be-removed", "webapp.prod.v2.to-be-removed"]);
        assert!(grouped.contains_key("api"));
    }

    #[test]
    fn summary_dry_run_counts_would_remove() {
        let reports = vec![
            rep("a", "a.x.to-be-removed", Outcome::WouldRemove),
            rep("b", "b.y.to-be-removed", Outcome::WouldRemove),
        ];
        let s = summarize(&reports, false, 1);
        assert!(s.contains("would remove 2"), "got: {s}");
        assert!(s.contains("--apply"), "dry-run summary should hint --apply; got: {s}");
    }

    #[test]
    fn summary_apply_counts_removed_and_failed() {
        let reports = vec![
            rep("a", "a.x.to-be-removed", Outcome::Removed),
            rep("b", "b.y.to-be-removed", Outcome::Removed),
            rep("c", "c.z.to-be-removed", Outcome::Failed),
        ];
        let s = summarize(&reports, true, 1);
        assert!(s.contains("removed 2"), "got: {s}");
        assert!(s.contains("failed 1"), "got: {s}");
    }
}
