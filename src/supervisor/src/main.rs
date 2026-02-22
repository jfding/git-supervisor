use clap::Parser;
use std::path::PathBuf;
use supervisor::{run_push, CentralConfig};

#[derive(Parser)]
#[command(name = "supervisor")]
struct Cli {
    #[arg(long, default_value = "supervisor.yaml")]
    config: PathBuf,

    #[arg(long)]
    dry_run: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Push,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Push => {
            let config = match CentralConfig::load(&cli.config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error loading config: {}", e);
                    std::process::exit(1);
                }
            };
            if let Err(e) = run_push(&config, cli.dry_run) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
