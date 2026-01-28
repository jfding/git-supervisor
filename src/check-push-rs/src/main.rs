use check_push_rs::*;
use clap::Parser;
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;

#[derive(Parser, Debug)]
#[command(name = "check-push-rs")]
#[command(about = "Git repository auto-reloader")]
struct Args {
    /// Run once and exit (don't loop)
    #[arg(long)]
    once: bool,

    /// Configuration file path
    #[arg(long)]
    config: Option<String>,

    /// Dry run mode (don't make changes)
    #[arg(long)]
    dry_run: bool,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    // Load configuration
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Configuration error: {}", e);
            process::exit(1);
        }
    };

    // Initialize logging
    if config.verbosity == 0 {
        // Redirect stdout to /dev/null for silent mode
        // This is handled by the shell calling the program
    } else {
        logging::init(&config);
    }

    // Setup signal handlers
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    
    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        shutdown_clone.store(true, Ordering::SeqCst);
    });

    // Initialize directories
    if let Err(e) = initialize_directories(&config) {
        tracing::error!("Failed to initialize directories: {}", e);
        process::exit(1);
    }

    // Main loop
    loop {
        // Check for shutdown signal
        if shutdown.load(Ordering::SeqCst) {
            tracing::info!("Shutdown signal received, exiting...");
            break;
        }

        // Acquire lock
        let lock_timeout = Duration::from_secs(config.timeout);
        let _lock = match lock::Lock::acquire(&config.ci_lock, lock_timeout) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!("Failed to acquire lock: {}", e);
                if args.once {
                    process::exit(1);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        // Process repositories
        match process_repositories(&config).await {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Error processing repositories: {}", e);
            }
        }

        // Release lock (dropped automatically)

        // If once mode or no sleep time, exit
        if args.once || config.sleep_time.is_none() {
            break;
        }

        tracing::info!("waiting for next check ...");
        tokio::time::sleep(Duration::from_secs(config.sleep_time.unwrap())).await;
    }
}

fn initialize_directories(config: &Config) -> Result<()> {
    // Ensure repos directory exists
    if !config.dir_repos.exists() {
        std::fs::create_dir_all(&config.dir_repos)
            .map_err(|e| Error::config(format!("Failed to create DIR_REPOS: {}", e)))?;
    }

    // Copy scripts from /scripts to DIR_SCRIPTS if DIR_SCRIPTS doesn't exist
    if !config.dir_scripts.exists() {
        std::fs::create_dir_all(&config.dir_scripts)
            .map_err(|e| Error::config(format!("Failed to create DIR_SCRIPTS: {}", e)))?;

        // Copy scripts from /scripts if it exists
        let source_scripts = Path::new("/scripts");
        if source_scripts.exists() {
            use std::process::Command;
            let output = Command::new("rsync")
                .arg("-a")
                .arg(format!("{}/", source_scripts.to_string_lossy()))
                .arg(&config.dir_scripts)
                .output()
                .map_err(|e| Error::config(format!("Failed to copy scripts: {}", e)))?;

            if !output.status.success() {
                return Err(Error::config("Failed to copy scripts".to_string()));
            }
        }
    }

    Ok(())
}

async fn process_repositories(config: &Config) -> Result<()> {
    let processor = repo::RepoProcessor::new(config.clone());

    let entries = std::fs::read_dir(&config.dir_repos)
        .map_err(|e| Error::file(format!("Failed to read DIR_REPOS: {}", e)))?;

    for entry in entries {
        let entry = entry.map_err(|e| Error::file(format!("Failed to read directory entry: {}", e)))?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(repo_name) = path.file_name().and_then(|n| n.to_str()) {
                if path.join(".git").exists() {
                    eprintln!("checking git status for <{}>", repo_name);
                    
                    if let Err(e) = processor.process_repository(repo_name) {
                        tracing::error!("Failed to process repository {}: {}", repo_name, e);
                        // Continue with other repositories
                    }
                }
            }
        }
    }

    Ok(())
}
