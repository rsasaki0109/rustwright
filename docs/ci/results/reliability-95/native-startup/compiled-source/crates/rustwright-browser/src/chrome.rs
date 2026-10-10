//! The [`Chrome`] builder: which binary to use and how to start it.

use std::path::PathBuf;
use std::time::Duration;

use crate::discovery::{configured_path, find_installed, installed_candidates};
use crate::error::{BrowserError, BrowserResult};

/// A browser family Rustwright knows how to discover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromeVariant {
    /// Google Chrome.
    Chrome,
    /// Chromium (including distro builds).
    Chromium,
    /// Microsoft Edge (Chromium-based).
    Edge,
    /// Brave (Chromium-based).
    Brave,
}

impl ChromeVariant {
    /// Every supported variant, in discovery preference order.
    pub const ALL: [ChromeVariant; 4] = [
        ChromeVariant::Chrome,
        ChromeVariant::Chromium,
        ChromeVariant::Edge,
        ChromeVariant::Brave,
    ];

    /// A human-readable name.
    pub fn display_name(self) -> &'static str {
        match self {
            ChromeVariant::Chrome => "Chrome",
            ChromeVariant::Chromium => "Chromium",
            ChromeVariant::Edge => "Microsoft Edge",
            ChromeVariant::Brave => "Brave",
        }
    }
}

/// How the browser profile (user data directory) is handled.
#[derive(Debug, Clone)]
pub enum Profile {
    /// A temporary profile created on launch and deleted on close.
    Ephemeral,
    /// A directory that persists cookies, storage and logins across runs.
    Persistent(PathBuf),
    /// The browser's own default user data directory.
    ///
    /// Note that Chrome 136 and newer **ignore remote debugging flags** when the
    /// default profile is used. Prefer [`Profile::Persistent`]; use this only
    /// with an older browser or with a custom `--user-data-dir`.
    Default,
}

/// Describes which Chrome/Chromium to launch and with what options.
///
/// ```
/// # use rustwright_browser::Chrome;
/// let chrome = Chrome::installed().headless(true).profile("./profile");
/// ```
#[derive(Debug, Clone)]
pub struct Chrome {
    pub(crate) variant: ChromeVariant,
    pub(crate) executable: Option<PathBuf>,
    pub(crate) headless: bool,
    pub(crate) profile: Profile,
    pub(crate) args: Vec<String>,
    pub(crate) envs: Vec<(String, String)>,
    pub(crate) startup_timeout: Duration,
    pub(crate) capture_stderr: bool,
}

impl Chrome {
    /// Use the first Chrome/Chromium installed on this machine.
    ///
    /// Discovery happens when the browser is launched, so this constructor
    /// never fails; a missing browser surfaces as [`BrowserError::NotFound`]
    /// from `Browser::launch`.
    pub fn installed() -> Self {
        Self {
            variant: ChromeVariant::Chrome,
            executable: None,
            headless: false,
            profile: Profile::Ephemeral,
            args: Vec::new(),
            envs: Vec::new(),
            startup_timeout: crate::launch::DEFAULT_STARTUP_TIMEOUT,
            capture_stderr: true,
        }
    }

    /// Use a specific browser binary.
    pub fn at(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: Some(executable.into()),
            ..Self::installed()
        }
    }

    /// Restrict discovery to a particular browser family.
    pub fn variant(mut self, variant: ChromeVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Run headless (`true`) or with a visible window (`false`). Defaults to
    /// `false`.
    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Use a persistent profile directory, preserving cookies, local/session
    /// storage and logins between runs.
    pub fn profile(mut self, path: impl Into<PathBuf>) -> Self {
        self.profile = Profile::Persistent(path.into());
        self
    }

    /// Use a throwaway profile (the default).
    pub fn ephemeral(mut self) -> Self {
        self.profile = Profile::Ephemeral;
        self
    }

    /// Use the browser's default profile directory. See [`Profile::Default`].
    pub fn default_profile(mut self) -> Self {
        self.profile = Profile::Default;
        self
    }

    /// Append a single browser command-line flag.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append several browser command-line flags.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable for the browser process.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Override how long to wait for the DevTools endpoint on launch.
    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    /// Capture the browser's stderr to a log file (default: `true`).
    pub fn capture_stderr(mut self, capture: bool) -> Self {
        self.capture_stderr = capture;
        self
    }

    /// Whether headless mode is enabled.
    pub fn is_headless(&self) -> bool {
        self.headless
    }

    /// The configured profile mode.
    pub fn profile_mode(&self) -> &Profile {
        &self.profile
    }

    /// Extra command-line flags.
    pub fn extra_args(&self) -> &[String] {
        &self.args
    }

    /// Environment overrides.
    pub fn environment(&self) -> &[(String, String)] {
        &self.envs
    }

    /// The startup timeout.
    pub fn startup_timeout_value(&self) -> Duration {
        self.startup_timeout
    }

    /// Whether stderr capture is enabled.
    pub fn captures_stderr(&self) -> bool {
        self.capture_stderr
    }

    /// Resolve the browser executable to an existing path.
    pub fn executable_path(&self) -> BrowserResult<PathBuf> {
        if let Some(explicit) = &self.executable {
            return if explicit.is_file() {
                Ok(explicit.clone())
            } else {
                Err(BrowserError::ExecutableNotFound(explicit.clone()))
            };
        }
        if let Some(configured) = configured_path() {
            if configured.is_file() {
                return Ok(configured);
            }
        }
        find_installed().ok_or_else(|| BrowserError::NotFound {
            search_paths: installed_candidates(),
        })
    }

    /// Every path Rustwright would consider during discovery, for diagnostics.
    pub fn searched_paths(&self) -> Vec<PathBuf> {
        installed_candidates()
    }
}
