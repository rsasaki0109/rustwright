use std::path::PathBuf;

use thiserror::Error;

/// Errors produced while discovering, launching or driving a browser process.
#[derive(Debug, Error)]
pub enum BrowserError {
    /// No installed Chrome/Chromium could be found.
    #[error("no installed Chrome or Chromium found; searched: {search_paths:?}")]
    NotFound {
        /// The locations that were searched.
        search_paths: Vec<PathBuf>,
    },

    /// An explicitly configured executable does not exist.
    #[error("browser executable not found at `{0}`")]
    ExecutableNotFound(PathBuf),

    /// The browser process could not be spawned.
    #[error("failed to launch browser `{executable}`: {source}")]
    Launch {
        /// The executable that failed to launch.
        executable: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The DevTools endpoint did not become ready in time.
    #[error("browser did not expose a DevTools endpoint within {timeout:?} (log: {log_path})")]
    StartupTimeout {
        /// The configured startup timeout.
        timeout: std::time::Duration,
        /// Path to the captured browser log.
        log_path: PathBuf,
    },

    /// The browser exited before the DevTools endpoint became ready.
    #[error("browser exited during startup with {status} (log: {log_path})")]
    EarlyExit {
        /// A description of the exit status.
        status: String,
        /// Path to the captured browser log.
        log_path: PathBuf,
    },

    /// The profile directory could not be prepared.
    #[error("failed to prepare profile directory `{path}`: {source}")]
    ProfileDir {
        /// The profile directory.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The `DevToolsActivePort` file was malformed.
    #[error("malformed DevToolsActivePort file at `{path}`: `{contents}`")]
    InvalidActivePort {
        /// The file path.
        path: PathBuf,
        /// The raw contents.
        contents: String,
    },

    /// An I/O error occurred.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// A CDP-level error occurred.
    #[error(transparent)]
    Cdp(#[from] rustwright_cdp::CdpError),
}

/// Convenience result alias for the browser crate.
pub type BrowserResult<T> = Result<T, BrowserError>;
