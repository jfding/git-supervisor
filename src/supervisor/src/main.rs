use clap::Parser;
use std::path::PathBuf;
use supervisor::{run_push, run_validate, run_watch, CentralConfig};

#[derive(Parser)]
#[command(name = "supervisor")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Validate config and print what would be done (no SSH)
    Validate(ConfigArg),
    /// Push to remotes: create dirs and ensure repos
    Push(PushArgs),
    /// Run check-push on each host in a loop (interval then timeout)
    Watch(WatchArgs),
}

#[derive(clap::Args)]
struct ConfigArg {
    /// Config file path
    #[arg(default_value = "deployments.yaml")]
    config: PathBuf,
}

#[derive(clap::Args)]
struct PushArgs {
    #[command(flatten)]
    config: ConfigArg,
    /// After preparing repos, run check-push script on each remote (one-shot with sandbox env)
    #[arg(long)]
    checkout: bool,
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

    let (config_path, checkout) = match &cli.command {
        Command::Validate(args) => (&args.config, false),
        Command::Push(args) => (&args.config.config, args.checkout),
        Command::Watch(args) => (&args.config.config, false),
    };
    let config = load_config_or_exit(config_path);

    let result = match &cli.command {
        Command::Validate(_) => run_validate(&config),
        Command::Push(_) => run_push(&config, checkout),
        Command::Watch(args) => run_watch(&config, args.interval, args.timeout),
    };
    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
