use clap::Parser;
use git_supervisor::console;
use git_supervisor::{run_check, run_hook, run_watch, CentralConfig};
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
    /// Prepare remotes (create dirs, ensure repos) then run check-push on each host in a loop
    Watch(WatchArgs),
    /// Start a GitHub webhook server that triggers check-push on push events
    Hook(HookArgs),
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
}

#[derive(clap::Args)]
struct HookArgs {
    /// Port to listen on
    #[arg(long, default_value = "9870")]
    port: u16,
    /// GitHub webhook secret (also reads GITHUB_WEBHOOK_SECRET env var)
    #[arg(long, env = "GITHUB_WEBHOOK_SECRET")]
    secret: String,
    /// External script to run on push events instead of supervisor watch-once
    #[arg(long)]
    script: Option<String>,
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

fn main() {
    let cli = Cli::parse();
    let config = load_config_or_exit(&cli.config);

    let result = match &cli.command {
        Command::Check => run_check(&config),
        Command::Watch(args) => run_watch(
            &config,
            args.interval,
            args.timeout,
            args.ignore_missing,
            args.skip_prepare,
        ),
        Command::Hook(args) => {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            rt.block_on(run_hook(
                config,
                args.port,
                args.secret.clone(),
                args.script.clone(),
                APP_VERSION.to_string(),
            ))
        }
    };
    if let Err(e) = result {
        eprintln!("{}", console::error(format!("Error: {}", e)));
        std::process::exit(1);
    }
}
