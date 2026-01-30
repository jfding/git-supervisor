# Design of the new Rust version of auto-reloader

Firstly, to keep the same desing of file flags and env flags, also the basic logic
of the original shell scripts.

## Behavior Compatibility

- Same directory structure and flag files
- Same command-line interface (`once` argument)
- Same log format (for parsing tools)
- Same exit codes
- Compatible with existing post scripts and docker files

## Improvements Against Shell Version

1. **Atomic Locking**: Uses `flock` for atomic file locking, eliminating race conditions
2. **Error Handling**: All operations return Result types, no silent failures
3. **Path Sanitization**: Prevents directory traversal attacks
4. **Version Comparison**: Pure Rust implementation, no Python dependency
5. **Structured Logging**: Uses tracing crate for better log management
6. **Type Safety**: Compile-time checks catch many errors

## Code Architecture

The code is organized into modules:

- `config.rs`: Configuration management
- `error.rs`: Custom error types
- `logging.rs`: Structured logging setup
- `version.rs`: Version comparison logic
- `git.rs`: Git operations using git2
- `file_ops.rs`: File operations and path sanitization
- `lock.rs`: File locking mechanism
- `branch.rs`: Branch processing logic
- `tag.rs`: Tag/release processing logic
- `cleanup.rs`: Cleanup of deprecated directories
- `scripts.rs`: Post script execution and docker restart
- `repo.rs`: Repository orchestration
- `main.rs`: Entry point and main loop
