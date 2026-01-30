# check-push-rs

Rust impl for auto-reloader shell scripts, in order to have a more robust base to add more enhancements,
in a more controllable way, like parallel runinng, deployment central controlling, etc.


## Features - or why Rust

The basic benetifs from Rust rewriting can have:

- **Type Safety**: Compile-time checks prevent runtime errors
- **Error Handling**: Proper Result types with detailed error messages
- **Performance**: Faster execution and lower memory usage compared to bash
- **Security**: Path sanitization and input validation
- **Concurrency**: Safe parallel processing support (future enhancement)
- **Testing**: Unit and integration tests
- **Backward Compatible**: Maintains same interface and behavior as bash script

## Building

```bash
cd src/check-push-rs
cargo build --release
```

The binary will be at `target/release/check-push-rs`.

## Usage

### Command Line

```bash
# Run once and exit
check-push-rs --once

# Run in daemon mode (uses SLEEP_TIME environment variable)
check-push-rs

# Specify configuration file
check-push-rs --config /path/to/config.toml

# Dry run mode (preview changes without making them)
check-push-rs --dry-run
```

### Environment Variables

All environment variables from the original bash script are supported:

- `VERB`: Verbosity level (0=silent, 1=normal, 2=verbose), default: 1
- `TIMEOUT`: Timeout for operations in seconds, default: 600
- `SLEEP_TIME`: Sleep time between checks in seconds, default: 360 (if not set, runs once)
- `DIR_REPOS`: Directory containing git repositories, default: `/work/git_repos`
- `DIR_COPIES`: Directory for code copies, default: `/work/copies`
- `DIR_SCRIPTS`: Directory for scripts, default: `/work/scripts`
- `CI_LOCK`: Lock file path, default: `/tmp/.ci-lock`
- `BR_WHITELIST`: Space-separated list of whitelisted branches, default: "main master dev test alpha"

### Configuration File

You can also use a TOML configuration file:

```toml
dir_repos = "/work/git_repos"
dir_copies = "/work/copies"
dir_scripts = "/work/scripts"
ci_lock = "/tmp/.ci-lock"
verbosity = 1
timeout = 600
sleep_time = 360
branch_whitelist = ["main", "master", "dev", "test", "alpha"]
```

The configuration file is searched in the following order:
1. `/work/.check-push.conf`
2. `~/.check-push.conf`
3. `.check-push.conf` (current directory)

Environment variables take precedence over configuration file values.

## Migration from Bash Script

### Phase 1: Parallel Deployment

The Rust binary is built alongside the bash script. Both are available in the Docker image.

### Phase 2: Enable Rust Version

Set the `USE_RUST=1` environment variable to use the Rust binary:

```bash
docker run -e USE_RUST=1 ... rushiai/auto-reloader:latest
```

Or in docker-compose.yml:

```yaml
environment:
  - USE_RUST=1
```

## Testing

### Unit Tests

```bash
cargo test --lib
```

### Integration Tests

```bash
cargo test --test '*'
```

Note: Integration tests require git repositories to be set up. Some tests are marked with `#[ignore]` and need manual setup.

## Troubleshooting

### Lock File Issues

If you see lock timeout errors, check:
- Another instance might be running
- Stale lock file (check PID in lock file)
- File system permissions

### Git Operations Fail

- Ensure git repositories are properly cloned
- Check SSH keys are set up correctly
- Verify network connectivity to git remotes

### Permission Errors

- Ensure all directories are writable
- Check user permissions for docker operations
- Verify script execution permissions

## Contributing

When adding new features:
1. Maintain backward compatibility with bash script behavior
2. Add unit tests for new functionality
3. Update documentation
4. Follow Rust best practices and clippy suggestions

## License

Same as the parent project.
