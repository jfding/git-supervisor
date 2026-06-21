use clap::Parser;
use git_supervisor::console;
use git_supervisor::{run_check, run_cleanup, run_local_watch, run_status, run_version_check, run_watch, CentralConfig, CleanupOpts, StatusOpts, WatchOpts, CHECK_PUSH_SCRIPT};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "git-supervisor", version)]
struct Cli {
    /// Config file path (default: ~/.config/git-supervisor/deployments.yaml or ./deployments.yaml)
    #[arg(short = 'c', long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Check config, SSH/git connectivity, and repo existence on remotes
    Check,
    /// Show what is currently deployed on each host (read-only probe)
    Status(StatusArgs),
    /// Remove stale `*.to-be-removed` copies on each host. Dry-run by default; use --apply to delete.
    Cleanup(CleanupArgs),
    /// Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop.
    /// Optionally start a GitHub webhook server alongside the timer.
    Watch(WatchArgs),
    /// Print the embedded check-push.sh script to stdout
    PrintScript,
    /// Print the current version and check GitHub for a newer release
    Version,
}

#[derive(clap::Args)]
struct StatusArgs {
    /// Limit to hosts whose ID matches this glob (`*`, `?`). Repeatable; union semantics.
    #[arg(long)]
    host: Vec<String>,
}

#[derive(clap::Args)]
struct CleanupArgs {
    /// Limit to hosts whose ID matches this glob (`*`, `?`). Repeatable; union semantics.
    #[arg(long)]
    host: Vec<String>,
    /// Actually delete the stale copies. Without this flag, only list what would be removed.
    #[arg(long)]
    apply: bool,
}

#[derive(clap::Args)]
struct WatchArgs {
    /// Seconds between each round of check-push on all hosts; 0 = run once and quit
    #[arg(long, default_value = "120")]
    interval: u64,
    /// Stop after this many seconds (default: run until interrupted)
    #[arg(long)]
    timeout: Option<u64>,
    /// Ignore missing repos: do not clone; only create dirs and run check-push on existing repos
    #[arg(short = 'I', long)]
    ignore_missing: bool,
    /// Skip host/repos preparation checking at the start
    #[arg(short = 'S', long)]
    skip_prepare: bool,
    /// Port for the GitHub webhook server (enables webhook mode)
    #[arg(long)]
    webhook_port: Option<u16>,
    /// GitHub webhook secret (also reads GITHUB_WEBHOOK_SECRET env var)
    #[arg(long, env = "GITHUB_WEBHOOK_SECRET")]
    webhook_secret: Option<String>,
}

fn load_config_or_exit(path: &std::path::Path) -> CentralConfig {
    match CentralConfig::load(path) {
        Ok(c) => c,
        Err(e) => {
            console::log_error(format!("Error loading config: {}", e));
            std::process::exit(1);
        }
    }
}

/// Resolve config file path.
/// If explicitly provided, use it as-is (caller gets the error if it doesn't exist).
/// Otherwise search: ~/.config/git-supervisor/deployments.yaml, then ./deployments.yaml.
/// Returns None if not specified and not found in either default location.
fn resolve_config_path(explicit: Option<&std::path::Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".config/git-supervisor/deployments.yaml");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let cwd = PathBuf::from("deployments.yaml");
    if cwd.exists() {
        return Some(cwd);
    }
    None
}

/// Warn if webhook_port is set without a webhook_secret (webhook listening will be skipped).
fn warn_webhook_args(args: &WatchArgs) {
    if args.webhook_port.is_some() && args.webhook_secret.is_none() {
        console::log_warning(
            "Warning: webhook port given w/o secret setting, webhook listening ignored",
        );
    }
}

fn main() {
    let cli = Cli::parse();
    let config_path = resolve_config_path(cli.config.as_deref());

    let result: Result<(), anyhow::Error> = match &cli.command {
        Command::PrintScript => {
            print!("{}", CHECK_PUSH_SCRIPT);
            Ok(())
        }
        Command::Version => run_version_check(env!("CARGO_PKG_VERSION")),
        Command::Check => {
            let path = config_path.unwrap_or_else(|| {
                console::log_error(
                    "no config file found; use --config or create ~/.config/git-supervisor/deployments.yaml"
                );
                std::process::exit(1);
            });
            let config = load_config_or_exit(&path);
            run_check(&config)
        }
        Command::Status(args) => {
            let path = config_path.unwrap_or_else(|| {
                console::log_error(
                    "no config file found; use --config or create ~/.config/git-supervisor/deployments.yaml"
                );
                std::process::exit(1);
            });
            let config = load_config_or_exit(&path);
            run_status(&config, StatusOpts {
                host_patterns: args.host.clone(),
            })
        }
        Command::Cleanup(args) => {
            let path = config_path.unwrap_or_else(|| {
                console::log_error(
                    "no config file found; use --config or create ~/.config/git-supervisor/deployments.yaml"
                );
                std::process::exit(1);
            });
            let config = load_config_or_exit(&path);
            run_cleanup(&config, CleanupOpts {
                host_patterns: args.host.clone(),
                apply: args.apply,
            })
        }
        Command::Watch(args) => {
            warn_webhook_args(args);
            match config_path {
                Some(ref path) => {
                    let config = load_config_or_exit(path);
                    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
                    rt.block_on(run_watch(
                        &config,
                        WatchOpts {
                            interval_secs: args.interval,
                            timeout_secs: args.timeout,
                            ignore_missing: args.ignore_missing,
                            skip_prepare: args.skip_prepare,
                            webhook_port: args.webhook_port,
                            webhook_secret: args.webhook_secret.clone(),
                            version: env!("CARGO_PKG_VERSION").to_string(),
                        },
                    ))
                }
                None => {
                    console::log_highlight("no config found, running in local mode");
                    run_local_watch(args.interval, args.timeout)
                }
            }
        }
    };

    if let Err(e) = result {
        console::log_error(format!("Error: {}", e));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_watch_parses_with_defaults() {
        let cli = Cli::try_parse_from(["supervisor", "watch"]).unwrap();
        match cli.command {
            Command::Watch(args) => {
                assert_eq!(args.interval, 120);
                assert!(args.timeout.is_none());
                assert!(!args.ignore_missing);
                assert!(!args.skip_prepare);
                assert!(args.webhook_port.is_none());
                assert!(args.webhook_secret.is_none());
            }
            _ => panic!("expected Watch command"),
        }
    }

    #[test]
    fn cli_watch_parses_all_flags() {
        let cli = Cli::try_parse_from([
            "supervisor",
            "watch",
            "--interval",
            "60",
            "--timeout",
            "300",
            "-I",
            "-S",
            "--webhook-port",
            "9870",
            "--webhook-secret",
            "my-secret",
        ])
        .unwrap();
        match cli.command {
            Command::Watch(args) => {
                assert_eq!(args.interval, 60);
                assert_eq!(args.timeout, Some(300));
                assert!(args.ignore_missing);
                assert!(args.skip_prepare);
                assert_eq!(args.webhook_port, Some(9870));
                assert_eq!(args.webhook_secret.as_deref(), Some("my-secret"));
            }
            _ => panic!("expected Watch command"),
        }
    }

    #[test]
    fn warn_webhook_port_without_secret_no_panic() {
        // port without secret should not panic; webhook listening is simply skipped
        let args = WatchArgs {
            interval: 120,
            timeout: None,
            ignore_missing: false,
            skip_prepare: false,
            webhook_port: Some(9870),
            webhook_secret: None,
        };
        warn_webhook_args(&args); // should not panic
    }

    #[test]
    fn cli_gh_webhook_subcommand_removed() {
        let result = Cli::try_parse_from(["supervisor", "gh-webhook", "--secret", "s"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_version_subcommand_parses() {
        let cli = Cli::try_parse_from(["supervisor", "version"]).unwrap();
        assert!(matches!(cli.command, Command::Version));
    }

    #[test]
    fn cli_status_parses_no_args() {
        let cli = Cli::try_parse_from(["supervisor", "status"]).unwrap();
        match cli.command {
            Command::Status(args) => assert!(args.host.is_empty()),
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn cli_status_parses_multiple_host_filters() {
        let cli = Cli::try_parse_from([
            "supervisor", "status", "--host", "prod-*", "--host", "bastion",
        ]).unwrap();
        match cli.command {
            Command::Status(args) => {
                assert_eq!(args.host, vec!["prod-*".to_string(), "bastion".to_string()]);
            }
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn cli_cleanup_parses_defaults() {
        let cli = Cli::try_parse_from(["supervisor", "cleanup"]).unwrap();
        match cli.command {
            Command::Cleanup(args) => {
                assert!(args.host.is_empty());
                assert!(!args.apply, "apply must default to false (dry-run)");
            }
            _ => panic!("expected Cleanup"),
        }
    }

    #[test]
    fn cli_cleanup_parses_apply_and_hosts() {
        let cli = Cli::try_parse_from([
            "supervisor", "cleanup", "--host", "prod-*", "--host", "bastion", "--apply",
        ])
        .unwrap();
        match cli.command {
            Command::Cleanup(args) => {
                assert_eq!(args.host, vec!["prod-*".to_string(), "bastion".to_string()]);
                assert!(args.apply);
            }
            _ => panic!("expected Cleanup"),
        }
    }
}
