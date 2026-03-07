use clap::Parser;
use std::path::PathBuf;
use supervisor::{run_check, run_watch, CentralConfig};

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
    /// Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop
    Watch(WatchArgs),
}

#[derive(clap::Args)]
struct WatchArgs {
    /// Seconds between each round of check-push on all hosts
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
}

fn load_config_or_exit(path: &std::path::Path) -> CentralConfig {
    match CentralConfig::load(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            std::process::exit(1);
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let config = load_config_or_exit(&cli.config);

    let result = match &cli.command {
        Command::Check => run_check(&config),
        Command::Watch(args) => run_watch(&config,
                                          args.interval,
                                          args.timeout,
                                          args.ignore_missing,
                                          args.skip_prepare),
    };
    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
