use crate::config::CentralConfig;
use crate::config::Host;
use crate::console::{self, paint, Color};
use crate::ssh;
use std::collections::BTreeMap;

pub const STATUS_PROBE_SCRIPT: &str = include_str!("status_probe.sh");

/// Filter options passed to [`run_status`].
pub struct StatusOpts {
    pub host_patterns: Vec<String>,
}

/// One row of the probe's TSV output: branch, release, latest-symlink, stale, or unknown dir.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ReportKind {
    Branch,
    Release,
    Latest,
    Stale,
    Unknown,
}

/// Parsed finding from the remote probe. One per directory under `$DIR_COPIES`.
#[derive(Debug, Clone)]
pub struct Report {
    pub host: String,
    pub kind: ReportKind,
    pub repo: String,
    pub name: String,
    pub sha: Option<String>,
    pub mtime_unix: u64,
    pub flags: Vec<String>,
}

impl Report {
    pub fn parse_line(line: &str) -> Option<Self> {
        let cols: Vec<&str> = line.split('\t').collect();
        // Strict 7-column contract — extra cols indicate an upstream bug (e.g. tab in a
        // flag value). Drop the row rather than silently misinterpret it.
        if cols.len() != 7 {
            return None;
        }
        let kind = match cols[1] {
            "branch" => ReportKind::Branch,
            "release" => ReportKind::Release,
            "latest" => ReportKind::Latest,
            "stale" => ReportKind::Stale,
            "unknown" => ReportKind::Unknown,
            _ => return None,
        };
        let sha = match cols[4] {
            "" | "-" => None,
            s => Some(s.to_string()),
        };
        // Non-numeric or negative mtimes → 0 (treat as "no living file").
        let mtime_unix = cols[5].parse::<i64>().ok().filter(|n| *n >= 0).map(|n| n as u64).unwrap_or(0);
        let flags = if cols[6] == "-" || cols[6].is_empty() {
            Vec::new()
        } else {
            cols[6].split(',').map(str::to_string).collect()
        };
        Some(Self {
            host: cols[0].to_string(),
            kind,
            repo: cols[2].to_string(),
            name: cols[3].to_string(),
            sha,
            mtime_unix,
            flags,
        })
    }
}

// ─── Pure helpers ────────────────────────────────────────────────────────────

/// Format a Unix-seconds mtime as a short relative string relative to `now`.
/// 0 → "-"; <60s → "just now"; <60m → "Nm ago"; <24h → "Nh ago";
/// <7d → "Nd ago"; otherwise "YYYY-MM-DD" (in local time).
pub fn format_relative_time(mtime: u64, now: u64) -> String {
    if mtime == 0 {
        return "-".to_string();
    }
    // Clock skew (mtime > now) saturates to 0 and renders as "just now" — intentional.
    let delta = now.saturating_sub(mtime);
    if delta < 60 { return "just now".to_string(); }
    if delta < 3600 { return format!("{}m ago", delta / 60); }
    if delta < 86_400 { return format!("{}h ago", delta / 3600); }
    if delta < 7 * 86_400 { return format!("{}d ago", delta / 86_400); }
    let dt = chrono::DateTime::<chrono::Local>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime),
    );
    dt.format("%Y-%m-%d").to_string()
}

/// Minimal shell-glob matcher: `*` (any run), `?` (any one char). Anchored.
pub fn glob_match(pattern: &str, s: &str) -> bool {
    fn inner(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => inner(&p[1..], s) || (!s.is_empty() && inner(p, &s[1..])),
            (Some(b'?'), Some(_)) => inner(&p[1..], &s[1..]),
            (Some(pc), Some(sc)) if pc == sc => inner(&p[1..], &s[1..]),
            _ => false,
        }
    }
    inner(pattern.as_bytes(), s.as_bytes())
}

/// Compare two release tag names with the same semantics as check-push.sh's
/// `sort_version_tags_desc`: strip leading `v`, split on `.` and `Q`, compare
/// numerically; missing segments are 0. Returns `Greater` when `a` > `b`.
pub fn cmp_version_tags_desc(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(tag: &str) -> Vec<u64> {
        let s = tag.strip_prefix('v').unwrap_or(tag);
        s.split(['.', 'Q'])
            .map(|seg| seg.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let pa = parts(a);
    let pb = parts(b);
    for i in 0..pa.len().max(pb.len()) {
        let av = pa.get(i).copied().unwrap_or(0);
        let bv = pb.get(i).copied().unwrap_or(0);
        match bv.cmp(&av) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

#[derive(Debug, Default)]
pub struct RepoEntries {
    pub branches: Vec<Report>,
    pub releases: Vec<Report>,
    pub stale: Vec<Report>,
    pub latest: Option<Report>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnnotationColor { Ok, Unset, Broken }

/// Returns `Some((text, color))` for the repo header annotation, or `None`
/// when the repo has no releases and no latest pointer (so the header is bare).
/// Add synthetic `active` flag to release rows that match the `prod.latest` target.
pub fn enrich_active_release_flags(entries: &mut RepoEntries) {
    let Some(latest) = &entries.latest else { return };
    let target = &latest.name;
    if target == "-" || target.is_empty() {
        return;
    }
    for release in &mut entries.releases {
        if release.name == *target && !release.flags.iter().any(|f| f == "active") {
            release.flags.insert(0, "active".to_string());
        }
    }
}

pub fn repo_header_annotation(entries: &RepoEntries) -> Option<(String, AnnotationColor)> {
    if entries.releases.is_empty() && entries.latest.is_none() {
        return None;
    }
    match &entries.latest {
        None => Some(("(latest: unset)".to_string(), AnnotationColor::Unset)),
        Some(l) => {
            let target = &l.name;
            if target == "-" || target.is_empty() {
                return Some(("(latest: unset)".to_string(), AnnotationColor::Unset));
            }
            let matches = entries.releases.iter().any(|r| r.name == *target);
            if matches {
                Some((format!("(latest: {})", target), AnnotationColor::Ok))
            } else {
                Some((format!("(latest: {} [missing])", target), AnnotationColor::Broken))
            }
        }
    }
}

pub type GroupedReports = BTreeMap<String, BTreeMap<String, RepoEntries>>;

/// Group reports by host and repo; sort branches alphabetically and releases
/// version-desc. `latest` and `unknown` are routed to their slots.
pub fn group_reports(reports: Vec<Report>) -> GroupedReports {
    let mut grouped: GroupedReports = BTreeMap::new();
    for r in reports {
        let entry = grouped
            .entry(r.host.clone()).or_default()
            .entry(r.repo.clone()).or_default();
        match r.kind {
            ReportKind::Branch => entry.branches.push(r),
            ReportKind::Release => entry.releases.push(r),
            ReportKind::Stale => entry.stale.push(r),
            ReportKind::Latest => entry.latest = Some(r),
            // Unknown dirs: surface as stale rows so they appear in the table.
            ReportKind::Unknown => entry.stale.push(r),
        }
    }
    for (_host, repos) in grouped.iter_mut() {
        for entries in repos.values_mut() {
            entries.branches.sort_by(|a, b| a.name.cmp(&b.name));
            entries.releases.sort_by(|a, b| cmp_version_tags_desc(&a.name, &b.name));
            entries.stale.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }
    grouped
}

// ─── Host-level probe ────────────────────────────────────────────────────────

enum HostOutcome {
    Ok(Vec<Report>),
    Empty,           // probe succeeded, $DIR_COPIES missing — fresh host
    Failed(String),  // SSH/probe failed; message includes stderr
}

fn collect_host(host_id: &str, host: &Host, dir_base: &std::path::Path) -> HostOutcome {
    let dir_esc = dir_base.to_string_lossy().replace('\'', "'\\''");
    let host_esc = host_id.replace('\'', "'\\''");
    let command = format!(
        "DIR_BASE='{}' HOST_ID='{}' bash -s",
        dir_esc, host_esc,
    );
    match ssh::ssh_run_capture(host, &command, STATUS_PROBE_SCRIPT.as_bytes()) {
        Err(e) => HostOutcome::Failed(e.to_string()),
        Ok(out) => {
            let reports: Vec<Report> = out.lines().filter_map(Report::parse_line).collect();
            if reports.is_empty() && out.trim().is_empty() {
                HostOutcome::Empty
            } else {
                HostOutcome::Ok(reports)
            }
        }
    }
}

fn host_filter_matches(patterns: &[String], host_id: &str) -> bool {
    if patterns.is_empty() { return true; }
    patterns.iter().any(|p| glob_match(p, host_id))
}

// ─── Rendering ───────────────────────────────────────────────────────────────

fn render_rows(rows: &[Report], now: u64) {
    for r in rows {
        let sha = r.sha.as_deref().unwrap_or("-");
        let sha_short = if sha.len() > 7 { &sha[..7] } else { sha };
        let ts = format_relative_time(r.mtime_unix, now);
        let flags = if r.flags.is_empty() { "-".to_string() } else { r.flags.join(",") };
        let line = format!("    {:<12} {:<7}  {:<10}  {}", r.name, sha_short, ts, flags);
        let painted = if r.flags.iter().any(|f| f == "debugging" || f == "stopping") {
            paint(line, Color::Red)
        } else if r.flags.iter().any(|f| f == "skipping") {
            paint(line, Color::Grey)
        } else if r.flags.iter().any(|f| f == "active") {
            paint(line, Color::Green)
        } else {
            line
        };
        println!("{}", painted);
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

pub fn run_status(config: &CentralConfig, opts: StatusOpts) -> anyhow::Result<()> {
    // Filter & skip-empty-repos hosts (matches run_check's behavior).
    let mut targets: Vec<(String, &Host)> = Vec::new();
    for (host_id, host) in &config.hosts {
        if !host.is_wildcard() && config.repos_for_host(host_id).is_empty() {
            console::log_info(format!("status host {{ {} }} --> skipped (repos: [] is empty)", host_id));
            continue;
        }
        if !host_filter_matches(&opts.host_patterns, host_id) { continue; }
        targets.push((host_id.clone(), host));
    }

    if !opts.host_patterns.is_empty() && targets.is_empty() {
        anyhow::bail!("no hosts matched: {:?}", opts.host_patterns);
    }

    // Parallel fanout.
    let outcomes: Vec<(String, HostOutcome)> = std::thread::scope(|s| {
        let handles: Vec<_> = targets.iter().map(|(host_id, host)| {
            let host_id = host_id.clone();
            let dir_base = config.dir_base_for_host(&host_id);
            let host_ref: &Host = host;
            s.spawn(move || {
                let outcome = collect_host(&host_id, host_ref, &dir_base);
                (host_id, outcome)
            })
        }).collect();
        let mut out: Vec<_> = handles.into_iter().map(|h| h.join().expect("thread panicked")).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    });

    let now = chrono::Local::now().timestamp().max(0) as u64;
    let mut any_failed = false;
    let mut all_reports: Vec<Report> = Vec::new();

    for (_host_id, outcome) in &outcomes {
        match outcome {
            HostOutcome::Ok(reports) => all_reports.extend(reports.iter().cloned()),
            HostOutcome::Empty => {}
            HostOutcome::Failed(_) => { any_failed = true; }
        }
    }

    let mut grouped = group_reports(all_reports);
    for repos in grouped.values_mut() {
        for entries in repos.values_mut() {
            enrich_active_release_flags(entries);
        }
    }

    // Render in host order (matches sorted outcomes).
    for (host_id, outcome) in &outcomes {
        match outcome {
            HostOutcome::Failed(msg) => {
                println!("{}", paint(format!("host: {}  ERROR", host_id), Color::Red));
                let first_line = msg.lines().next().unwrap_or("");
                if !first_line.is_empty() {
                    println!("  {}", paint(first_line, Color::Red));
                }
            }
            HostOutcome::Empty => {
                println!("host: {}  {}", host_id, paint("(no deployments yet)", Color::Grey));
            }
            HostOutcome::Ok(_) => {
                let repos = grouped.get(host_id);
                if repos.is_none_or(|r| r.is_empty()) {
                    println!("host: {}  {}", host_id, paint("(empty)", Color::Grey));
                    continue;
                }
                println!("host: {}", host_id);
                for (repo_name, entries) in repos.unwrap() {
                    let header = match repo_header_annotation(entries) {
                        None => format!("  {}", repo_name),
                        Some((text, color)) => {
                            let c = match color {
                                AnnotationColor::Ok => Color::Green,
                                AnnotationColor::Unset => Color::Yellow,
                                AnnotationColor::Broken => Color::Red,
                            };
                            format!("  {}  {}", repo_name, paint(&text, c))
                        }
                    };
                    println!("{}", header);
                    render_rows(&entries.branches, now);
                    if !entries.branches.is_empty() && !entries.releases.is_empty() {
                        println!("    --");
                    }
                    render_rows(&entries.releases, now);
                    if !entries.stale.is_empty() {
                        println!("    {}", paint("stale:", Color::Yellow));
                        for r in &entries.stale {
                            println!(
                                "      {:<32}  {}",
                                r.name,
                                paint(format_relative_time(r.mtime_unix, now), Color::Grey),
                            );
                        }
                    }
                }
            }
        }
    }

    if any_failed {
        anyhow::bail!("status probe failed on one or more hosts");
    }
    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod parser_tests {
    use super::*;

    #[test]
    fn parses_branch_line() {
        let line = "app1\tbranch\twebapp\tmain\tabc1234567\t1747720000\t-";
        let r = Report::parse_line(line).expect("parse");
        assert_eq!(r.host, "app1");
        assert_eq!(r.kind, ReportKind::Branch);
        assert_eq!(r.repo, "webapp");
        assert_eq!(r.name, "main");
        assert_eq!(r.sha.as_deref(), Some("abc1234567"));
        assert_eq!(r.mtime_unix, 1747720000);
        assert!(r.flags.is_empty());
    }

    #[test]
    fn parses_flags_csv_and_dash_sha() {
        let line = "h\tbranch\tr\tdev\t-\t0\tdebugging,no-cleanup";
        let r = Report::parse_line(line).unwrap();
        assert_eq!(r.flags, vec!["debugging".to_string(), "no-cleanup".to_string()]);
        assert!(r.sha.is_none(), "dash sha should map to None");
        assert_eq!(r.mtime_unix, 0);
    }

    #[test]
    fn parses_unknown_kind() {
        let line = "h\tunknown\t-\tstray-dir\t-\t0\t-";
        let r = Report::parse_line(line).unwrap();
        assert_eq!(r.kind, ReportKind::Unknown);
        assert_eq!(r.name, "stray-dir");
    }

    #[test]
    fn rejects_too_few_columns() {
        assert!(Report::parse_line("only\tthree\tcols").is_none());
    }

    #[test]
    fn rejects_invalid_kind() {
        let line = "h\tbogus\tr\tn\t-\t0\t-";
        assert!(Report::parse_line(line).is_none());
    }

    #[test]
    fn negative_mtime_maps_to_zero() {
        let line = "h\tbranch\tr\tn\t-\t-5\t-";
        let r = Report::parse_line(line).unwrap();
        assert_eq!(r.mtime_unix, 0);
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    fn now_at(unix: u64) -> u64 { unix }

    #[test]
    fn time_format_zero_returns_dash() {
        assert_eq!(format_relative_time(0, now_at(1_000_000)), "-");
    }

    #[test]
    fn time_format_just_now() {
        let now = 1_000_000;
        assert_eq!(format_relative_time(now - 5, now), "just now");
        assert_eq!(format_relative_time(now - 59, now), "just now");
    }

    #[test]
    fn time_format_minutes() {
        let now = 1_000_000;
        assert_eq!(format_relative_time(now - 60, now), "1m ago");
        assert_eq!(format_relative_time(now - 60 * 59, now), "59m ago");
    }

    #[test]
    fn time_format_hours() {
        let now = 1_000_000;
        assert_eq!(format_relative_time(now - 3600, now), "1h ago");
        assert_eq!(format_relative_time(now - 23 * 3600, now), "23h ago");
    }

    #[test]
    fn time_format_days() {
        let now = 1_000_000;
        assert_eq!(format_relative_time(now - 86400, now), "1d ago");
        assert_eq!(format_relative_time(now - 6 * 86400, now), "6d ago");
    }

    #[test]
    fn time_format_falls_back_to_date_after_7d() {
        // 2026-01-16 00:00:00 UTC; render uses Local tz, so we check length and year prefix only.
        let now = 1_768_521_600u64;
        let week_ago = now - 8 * 86400;
        let out = format_relative_time(week_ago, now);
        assert_eq!(out.len(), 10, "expected YYYY-MM-DD, got {:?}", out);
        assert!(out.starts_with("2026-01-"), "{}", out);
    }

    #[test]
    fn glob_matches_star() {
        assert!(glob_match("prod-*", "prod-app1"));
        assert!(glob_match("prod-*", "prod-"));
        assert!(!glob_match("prod-*", "staging-app1"));
    }

    #[test]
    fn glob_matches_question() {
        assert!(glob_match("app?", "app1"));
        assert!(!glob_match("app?", "app12"));
    }

    #[test]
    fn glob_anchors_full_string() {
        assert!(!glob_match("prod", "prod-app1"));
        assert!(glob_match("prod", "prod"));
    }

    fn rep(host: &str, kind: ReportKind, repo: &str, name: &str) -> Report {
        Report {
            host: host.into(), kind, repo: repo.into(), name: name.into(),
            sha: None, mtime_unix: 0, flags: vec![],
        }
    }

    #[test]
    fn group_sorts_branches_alpha_releases_version_desc() {
        let reports = vec![
            rep("h", ReportKind::Branch, "webapp", "main"),
            rep("h", ReportKind::Branch, "webapp", "dev"),
            rep("h", ReportKind::Release, "webapp", "v2.1.4"),
            rep("h", ReportKind::Release, "webapp", "v2.1.5"),
            rep("h", ReportKind::Release, "webapp", "v2.1.3"),
        ];
        let grouped = group_reports(reports);
        let entries = grouped.get("h").unwrap().get("webapp").unwrap();
        let branches: Vec<&str> = entries.branches.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(branches, vec!["dev", "main"]);
        let releases: Vec<&str> = entries.releases.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(releases, vec!["v2.1.5", "v2.1.4", "v2.1.3"]);
    }

    #[test]
    fn enrich_active_release_flags_marks_latest_release() {
        let mut entries = RepoEntries {
            branches: vec![],
            releases: vec![
                rep("h", ReportKind::Release, "r", "v2.1.5"),
                rep("h", ReportKind::Release, "r", "v2.1.4"),
            ],
            stale: vec![],
            latest: Some(rep("h", ReportKind::Latest, "r", "v2.1.5")),
        };
        enrich_active_release_flags(&mut entries);
        assert!(entries.releases[0].flags.contains(&"active".to_string()));
        assert!(!entries.releases[1].flags.contains(&"active".to_string()));
    }

    #[test]
    fn enrich_active_release_flags_preserves_existing_flags() {
        let mut entries = RepoEntries {
            branches: vec![],
            releases: vec![Report {
                host: "h".into(),
                kind: ReportKind::Release,
                repo: "r".into(),
                name: "v2.1.5".into(),
                sha: None,
                mtime_unix: 0,
                flags: vec!["debugging".to_string()],
            }],
            stale: vec![],
            latest: Some(rep("h", ReportKind::Latest, "r", "v2.1.5")),
        };
        enrich_active_release_flags(&mut entries);
        assert_eq!(
            entries.releases[0].flags,
            vec!["active".to_string(), "debugging".to_string()],
        );
    }

    #[test]
    fn enrich_active_release_flags_skips_unset_latest() {
        let mut entries = RepoEntries {
            branches: vec![],
            releases: vec![rep("h", ReportKind::Release, "r", "v2.1.5")],
            stale: vec![],
            latest: Some(rep("h", ReportKind::Latest, "r", "-")),
        };
        enrich_active_release_flags(&mut entries);
        assert!(entries.releases[0].flags.is_empty());
    }

    #[test]
    fn repo_header_no_latest_omits_annotation() {
        let entries = RepoEntries { branches: vec![], releases: vec![], stale: vec![], latest: None };
        assert_eq!(repo_header_annotation(&entries), None);
    }

    #[test]
    fn repo_header_with_matching_release() {
        let entries = RepoEntries {
            branches: vec![],
            releases: vec![rep("h", ReportKind::Release, "r", "v2.1.5")],
            stale: vec![],
            latest: Some(rep("h", ReportKind::Latest, "r", "v2.1.5")),
        };
        let (text, color) = repo_header_annotation(&entries).unwrap();
        assert!(text.contains("v2.1.5"));
        assert_eq!(color, AnnotationColor::Ok);
    }

    #[test]
    fn repo_header_with_missing_target() {
        let entries = RepoEntries {
            branches: vec![],
            releases: vec![rep("h", ReportKind::Release, "r", "v2.1.4")],
            stale: vec![],
            latest: Some(rep("h", ReportKind::Latest, "r", "v2.1.5")),
        };
        let (text, color) = repo_header_annotation(&entries).unwrap();
        assert!(text.contains("v2.1.5"));
        assert!(text.contains("missing"));
        assert_eq!(color, AnnotationColor::Broken);
    }

    #[test]
    fn host_filter_matches_empty_patterns_matches_all() {
        assert!(host_filter_matches(&[], "anything"));
        assert!(host_filter_matches(&[], ""));
    }

    #[test]
    fn host_filter_matches_pattern_union() {
        let pats = vec!["prod-*".to_string(), "bastion".to_string()];
        assert!(host_filter_matches(&pats, "prod-app1"));
        assert!(host_filter_matches(&pats, "bastion"));
        assert!(!host_filter_matches(&pats, "staging"));
    }

    #[test]
    fn cmp_version_tags_desc_basic() {
        use std::cmp::Ordering;
        assert_eq!(cmp_version_tags_desc("v2.1.5", "v2.1.4"), Ordering::Less);
        assert_eq!(cmp_version_tags_desc("v2.1.4", "v2.1.5"), Ordering::Greater);
        assert_eq!(cmp_version_tags_desc("v2.1.5", "v2.1.5"), Ordering::Equal);
    }

    #[test]
    fn cmp_version_tags_desc_q_notation() {
        use std::cmp::Ordering;
        // v2025Q4.2.0 → [2025, 4, 2, 0]; v2025Q3.9.0 → [2025, 3, 9, 0]; Q4 > Q3
        assert_eq!(cmp_version_tags_desc("v2025Q4.2.0", "v2025Q3.9.0"), Ordering::Less);
    }

    #[test]
    fn cmp_version_tags_desc_missing_segments_default_zero() {
        use std::cmp::Ordering;
        // v2.1 → [2, 1]; v2.1.0 → [2, 1, 0]; missing trailing segment treated as 0.
        assert_eq!(cmp_version_tags_desc("v2.1", "v2.1.0"), Ordering::Equal);
        // v2.1 < v2.1.1 because the missing segment is 0
        assert_eq!(cmp_version_tags_desc("v2.1", "v2.1.1"), Ordering::Greater);
    }

    #[test]
    fn time_format_future_mtime_renders_just_now() {
        let now = 1_000_000;
        assert_eq!(format_relative_time(now + 100, now), "just now");
    }
}
