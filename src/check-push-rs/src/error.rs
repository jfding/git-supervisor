use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Lock error: {0}")]
    Lock(String),

    #[error("Script execution error: {0}")]
    Script(String),

    #[error("File operation error: {0}")]
    File(String),

    #[error("Version parsing error: {0}")]
    Version(String),

    #[error("Path error: {0}")]
    Path(String),

    #[error("Docker error: {0}")]
    Docker(String),

    #[error("Invalid branch name: {0}")]
    InvalidBranch(String),

    #[error("Invalid tag name: {0}")]
    InvalidTag(String),

    #[error("Timeout error: operation exceeded timeout")]
    Timeout,

    #[error("Lock timeout: could not acquire lock within timeout period")]
    LockTimeout,
}

impl Error {
    pub fn config(msg: impl Into<String>) -> Self {
        Error::Config(msg.into())
    }

    pub fn lock(msg: impl Into<String>) -> Self {
        Error::Lock(msg.into())
    }

    pub fn script(msg: impl Into<String>) -> Self {
        Error::Script(msg.into())
    }

    pub fn file(msg: impl Into<String>) -> Self {
        Error::File(msg.into())
    }

    pub fn version(msg: impl Into<String>) -> Self {
        Error::Version(msg.into())
    }

    pub fn path(msg: impl Into<String>) -> Self {
        Error::Path(msg.into())
    }

    pub fn docker(msg: impl Into<String>) -> Self {
        Error::Docker(msg.into())
    }

    pub fn invalid_branch(name: impl Into<String>) -> Self {
        Error::InvalidBranch(name.into())
    }

    pub fn invalid_tag(name: impl Into<String>) -> Self {
        Error::InvalidTag(name.into())
    }
}
