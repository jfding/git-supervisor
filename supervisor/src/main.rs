use clap::Parser;
use git_supervisor::console;
use git_supervisor::{run_check, run_watch, CentralConfig, WatchOpts};
use std::path::PathBuf;

/// Version from repo VERSION file (set in build.rs).
const APP_VERSION: &str = env!("APP_VERSION");

#[derive(Parser)]
#[command(name = "supervisor", version = APP_VERSION)]
struct Cli {
    /// Config file path
    #[arg(global = true, default_value = "deployments.yaml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Check config, SSH/git connectivity, and repo existence on remotes
    Check,
    /// Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop.
    /// Optionally start a GitHub webhook server alongside the timer.
    Watch(WatchArgs),
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
            eprintln!("{}", console::error(format!("Error loading config: {}", e)));
            std::process::exit(1);
        }
    }
}

/// Resolve config file path: if the given path doesn't exist and is the default
/// filename, look for it in ~/.config/git-supervisor/.
fn resolve_config_path(path: &std::path::Path) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    if let Some(name) = path.file_name() {
        if path == *name {
            if let Some(home) = dirs::home_dir() {
                let candidate = home.join(".config/git-supervisor").join(name);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    path.to_path_buf()
}

/// Validate that webhook_port requires a webhook_secret.
fn validate_webhook_args(args: &WatchArgs) -> Result<(), String> {
    if args.webhook_port.is_some() && args.webhook_secret.is_none() {
        return Err(
            "--webhook-port requires --webhook-secret or GITHUB_WEBHOOK_SECRET env var".into(),
        );
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let config_path = resolve_config_path(&cli.config);
    let config = load_config_or_exit(&config_path);

    let result = match &cli.command {
        Command::Check => run_check(&config),
        Command::Watch(args) => {
            if let Err(msg) = validate_webhook_args(args) {
                eprintln!("{}", console::error(format!("Error: {}", msg)));
                std::process::exit(1);
            }
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
                    version: APP_VERSION.to_string(),
                },
            ))
        }
    };
    if let Err(e) = result {
        eprintln!("{}", console::error(format!("Error: {}", e)));
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
    fn validate_webhook_port_without_secret_fails() {
        let args = WatchArgs {
            interval: 120,
            timeout: None,
            ignore_missing: false,
            skip_prepare: false,
            webhook_port: Some(9870),
            webhook_secret: None,
        };
        assert!(validate_webhook_args(&args).is_err());
    }

    #[test]
    fn validate_webhook_port_with_secret_ok() {
        let args = WatchArgs {
            interval: 120,
            timeout: None,
            ignore_missing: false,
            skip_prepare: false,
            webhook_port: Some(9870),
            webhook_secret: Some("secret".into()),
        };
        assert!(validate_webhook_args(&args).is_ok());
    }

    #[test]
    fn validate_no_webhook_flags_ok() {
        let args = WatchArgs {
            interval: 120,
            timeout: None,
            ignore_missing: false,
            skip_prepare: false,
            webhook_port: None,
            webhook_secret: None,
        };
        assert!(validate_webhook_args(&args).is_ok());
    }

    #[test]
    fn cli_gh_webhook_subcommand_removed() {
        let result = Cli::try_parse_from(["supervisor", "gh-webhook", "--secret", "s"]);
        assert!(result.is_err());
    }
}
