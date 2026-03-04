use crate::error::{Error, Result};
use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Lock {
    file: File,
    path: std::path::PathBuf,
}

impl Lock {
    /// Try to acquire a lock file with timeout
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self> {
        let start = Instant::now();

        loop {
            // Try to open/create the lock file
            let file = match OpenOptions::new()
                .create(true)
                .write(true)
                .open(path)
            {
                Ok(f) => f,
                Err(e) => {
                    if start.elapsed() < timeout {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    return Err(Error::lock(format!("Failed to create lock file: {}", e)));
                }
            };

            // Try to acquire exclusive lock
            match file.try_lock_exclusive() {
                Ok(_) => {
                    // Write PID to lock file for stale lock detection
                    let pid = std::process::id();
                    if let Err(e) = writeln!(&file, "{}", pid) {
                        drop(file);
                        return Err(Error::lock(format!("Failed to write PID to lock file: {}", e)));
                    }

                    return Ok(Lock {
                        file,
                        path: path.to_path_buf(),
                    });
                }
                Err(_) => {
                    // Lock is held by another process
                    drop(file);

                    if start.elapsed() >= timeout {
                        return Err(Error::LockTimeout);
                    }

                    // Check if lock is stale (process doesn't exist)
                    if let Ok(pid_str) = std::fs::read_to_string(path) {
                        if let Ok(pid) = pid_str.trim().parse::<u32>() {
                            // Check if process exists (Unix-specific)
                            #[cfg(unix)]
                            {
                                use std::process::Command;
                                let output = Command::new("kill")
                                    .arg("-0")
                                    .arg(&pid.to_string())
                                    .output();

                                if let Ok(output) = output {
                                    if !output.status.success() {
                                        // Process doesn't exist, remove stale lock
                                        let _ = std::fs::remove_file(path);
                                        continue;
                                    }
                                }
                            }
                        }
                    }

                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Release lock and remove file
        let _ = self.file.unlock();
        let _ = std::fs::remove_file(&self.path);
    }
}
