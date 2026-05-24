use crate::config::CentralConfig;

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

pub fn run_status(_config: &CentralConfig, _opts: StatusOpts) -> anyhow::Result<()> {
    Ok(())
}

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
