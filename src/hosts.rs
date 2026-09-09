//! Shared plumbing for the per-host commands — `check`, `status`, `cleanup`, and
//! `watch`'s prepare step: picking the hosts to act on, running the per-host work
//! concurrently, and reporting a host-level failure the same way everywhere.

use crate::config::{CentralConfig, Host};
use crate::console::{self, paint, Color};

/// One host's result from a per-host probe.
pub enum HostOutcome<T> {
    /// Probe succeeded and produced rows.
    Ok(Vec<T>),
    /// Probe succeeded with nothing to report (e.g. a fresh host with no copies dir).
    Empty,
    /// SSH or probe failure; the message carries the remote stderr.
    Failed(String),
}

/// Glob match supporting `*` and `?`. Used for `--host` filters.
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

/// Whether `host_id` passes a `--host` filter. An empty filter matches every host.
pub fn host_filter_matches(patterns: &[String], host_id: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| glob_match(p, host_id))
}

/// The hosts a per-host command should act on, sorted by host id so output order is
/// stable (`config.hosts` is a `HashMap`).
///
/// Hosts with an explicitly empty repo list are skipped and announced as
/// `<verb> host { id } --> skipped`; `verb` is used verbatim so each command keeps its
/// own wording. Errors when `host_patterns` was given but matched nothing.
pub fn select_targets<'a>(
    config: &'a CentralConfig,
    verb: &str,
    host_patterns: &[String],
) -> anyhow::Result<Vec<(String, &'a Host)>> {
    let mut targets: Vec<(String, &Host)> = Vec::new();
    for (host_id, host) in &config.hosts {
        if !host.is_wildcard() && config.repos_for_host(host_id).is_empty() {
            console::log_info(format!(
                "{} host {{ {} }} --> skipped (repos: [] is empty)",
                verb, host_id
            ));
            continue;
        }
        if !host_filter_matches(host_patterns, host_id) {
            continue;
        }
        targets.push((host_id.clone(), host));
    }
    targets.sort_by(|a, b| a.0.cmp(&b.0));

    if !host_patterns.is_empty() && targets.is_empty() {
        anyhow::bail!("no hosts matched: {:?}", host_patterns);
    }
    Ok(targets)
}

/// Run `work` on every target concurrently — one thread per host, because the SSH
/// underneath is blocking — and return each host's result in target (sorted) order.
pub fn fanout<'a, T, F>(targets: &[(String, &'a Host)], work: F) -> Vec<(String, T)>
where
    F: Fn(&str, &'a Host) -> T + Send + Sync,
    T: Send,
{
    std::thread::scope(|s| {
        let handles: Vec<_> = targets
            .iter()
            .map(|(host_id, host)| {
                let host_id = host_id.clone();
                let host: &'a Host = host;
                let work = &work;
                s.spawn(move || {
                    let result = work(&host_id, host);
                    (host_id, result)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("host thread panicked"))
            .collect()
    })
}

/// Print the standard host-level failure block: a red `host: <id>  ERROR` header
/// followed by the first line of the message.
pub fn render_host_error(host_id: &str, msg: &str) {
    println!("{}", paint(format!("host: {}  ERROR", host_id), Color::Red));
    let first_line = msg.lines().next().unwrap_or("");
    if !first_line.is_empty() {
        println!("  {}", paint(first_line, Color::Red));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
