use clap::Parser;
use git_supervisor::console;
use git_supervisor::{run_check, run_watch, CentralConfig};
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

fn main() {
    let cli = Cli::parse();
    let config_path = resolve_config_path(&cli.config);
    let config = load_config_or_exit(&config_path);

    let result = match &cli.command {
        Command::Check => run_check(&config),
        Command::Watch(args) => {
            if args.webhook_port.is_some() && args.webhook_secret.is_none() {
                eprintln!(
                    "{}",
                    console::error(
                        "Error: --webhook-port requires --webhook-secret or GITHUB_WEBHOOK_SECRET env var"
                    )
                );
                std::process::exit(1);
            }
            run_watch(
                &config,
                args.interval,
                args.timeout,
                args.ignore_missing,
                args.skip_prepare,
                args.webhook_port,
                args.webhook_secret.clone(),
            )
        }
    };
    if let Err(e) = result {
        eprintln!("{}", console::error(format!("Error: {}", e)));
        std::process::exit(1);
    }
}
