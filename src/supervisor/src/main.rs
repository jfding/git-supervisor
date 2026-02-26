use clap::Parser;
use std::path::PathBuf;
use supervisor::{run_push, run_validate, CentralConfig};

#[derive(Parser)]
#[command(name = "supervisor")]
struct Cli {
    #[arg(long, default_value = "supervisor.yaml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Validate config and print what would be done (no SSH)
    Validate,
    /// Push to remotes: create dirs and ensure repos
    Push,
}

fn main() {
    let cli = Cli::parse();

    let config = match CentralConfig::load(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            std::process::exit(1);
        }
    };

    match cli.command {
        Command::Validate => {
            if let Err(e) = run_validate(&config) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Command::Push => {
            if let Err(e) = run_push(&config) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
