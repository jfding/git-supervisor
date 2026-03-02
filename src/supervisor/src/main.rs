use clap::Parser;
use std::path::PathBuf;
use supervisor::{run_check, run_watch, CentralConfig};

#[derive(Parser)]
#[command(name = "supervisor")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Check config, SSH/git connectivity, and repo existence on remotes
    Check(ConfigArg),
    /// Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop
    Watch(WatchArgs),
}

#[derive(clap::Args)]
struct ConfigArg {
    /// Config file path
    #[arg(default_value = "deployments.yaml")]
    config: PathBuf,
}

#[derive(clap::Args)]
struct WatchArgs {
    #[command(flatten)]
    config: ConfigArg,
    /// Seconds between each round of check-push on all hosts
    #[arg(long, default_value = "120")]
    interval: u64,
    /// Stop after this many seconds (default: run until interrupted)
    #[arg(long)]
    timeout: Option<u64>,
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

    let config_path = match &cli.command {
        Command::Check(args) => &args.config,
        Command::Watch(args) => &args.config.config,
    };
    let config = load_config_or_exit(config_path);

    let result = match &cli.command {
        Command::Check(_) => run_check(&config),
        Command::Watch(args) => run_watch(&config, args.interval, args.timeout),
    };
    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
