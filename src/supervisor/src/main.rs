use clap::Parser;
use std::path::PathBuf;
use supervisor::{run_push, run_validate, CentralConfig};

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
    Push(ConfigArg),
}

#[derive(clap::Args)]
struct ConfigArg {
    /// Config file path
    #[arg(default_value = "deployments.yaml")]
    config: PathBuf,
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
        Command::Validate(args) | Command::Push(args) => &args.config,
    };
    let config = load_config_or_exit(config_path);

    let result = match &cli.command {
        Command::Validate(_) => run_validate(&config),
        Command::Push(_) => run_push(&config),
    };
    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
