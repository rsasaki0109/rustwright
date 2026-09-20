//! Platform-specific discovery of installed Chrome/Chromium binaries.

use std::path::{Path, PathBuf};

use crate::chrome::ChromeVariant;

/// Environment variable that points at an explicit browser binary.
pub const CHROME_PATH_ENV: &str = "RUSTWRIGHT_CHROME";

/// Environment variable that points at an explicit Firefox binary.
pub const FIREFOX_PATH_ENV: &str = "RUSTWRIGHT_FIREFOX";

/// Well-known install locations for Firefox on the current platform.
pub fn firefox_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(configured) = std::env::var_os(FIREFOX_PATH_ENV) {
        candidates.push(PathBuf::from(configured));
    }

    #[cfg(target_os = "linux")]
    {
        for name in ["firefox", "firefox-esr", "firefox-bin"] {
            candidates.extend(find_on_path(name));
        }
        for fixed in [
            "/usr/bin/firefox",
            "/usr/local/bin/firefox",
            "/snap/bin/firefox",
            "/opt/firefox/firefox",
        ] {
            candidates.push(PathBuf::from(fixed));
        }
    }

    #[cfg(target_os = "macos")]
    {
        for app in [
            "/Applications/Firefox.app/Contents/MacOS/firefox",
            "/Applications/Firefox Developer Edition.app/Contents/MacOS/firefox",
        ] {
            candidates.push(PathBuf::from(app));
        }
    }

    #[cfg(target_os = "windows")]
    {
        for key in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(root) = std::env::var_os(key) {
                candidates.push(PathBuf::from(root).join("Mozilla Firefox\\firefox.exe"));
            }
        }
    }

    dedupe(candidates)
}

/// Resolve the first installed Firefox, if any.
pub(crate) fn find_firefox() -> Option<PathBuf> {
    firefox_candidates()
        .into_iter()
        .find(|path| is_browser(path))
}

/// Well-known install locations for `variant` on the current platform.
///
/// Paths are returned in preference order but are not guaranteed to exist;
/// callers should filter with [`find_existing`].
pub fn path_candidates(variant: ChromeVariant) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "linux")]
    {
        let (names, fixed): (&[&str], &[&str]) = match variant {
            ChromeVariant::Chrome => (
                &["google-chrome-stable", "google-chrome"],
                &["/opt/google/chrome/chrome"],
            ),
            ChromeVariant::Chromium => (
                &["chromium", "chromium-browser", "chromium-freeworld"],
                &["/opt/chromium/chrome", "/snap/bin/chromium"],
            ),
            ChromeVariant::Edge => (
                &["microsoft-edge-stable", "microsoft-edge"],
                &["/opt/microsoft/msedge/msedge"],
            ),
            ChromeVariant::Brave => (&["brave-browser", "brave"], &["/opt/brave.com/brave/brave"]),
        };
        for name in names {
            candidates.extend(find_on_path(name));
        }
        candidates.extend(fixed.iter().map(PathBuf::from));
    }

    #[cfg(target_os = "macos")]
    {
        let homes = home_dirs();
        let apps: &[&str] = match variant {
            ChromeVariant::Chrome => &[
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            ],
            ChromeVariant::Chromium => &["/Applications/Chromium.app/Contents/MacOS/Chromium"],
            ChromeVariant::Edge => {
                &["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"]
            }
            ChromeVariant::Brave => {
                &["/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"]
            }
        };
        candidates.extend(apps.iter().map(PathBuf::from));
        for home in homes {
            let app = home.join("Applications");
            match variant {
                ChromeVariant::Chrome => {
                    candidates.push(app.join("Google Chrome.app/Contents/MacOS/Google Chrome"))
                }
                ChromeVariant::Chromium => {
                    candidates.push(app.join("Chromium.app/Contents/MacOS/Chromium"))
                }
                ChromeVariant::Edge => {
                    candidates.push(app.join("Microsoft Edge.app/Contents/MacOS/Microsoft Edge"))
                }
                ChromeVariant::Brave => {
                    candidates.push(app.join("Brave Browser.app/Contents/MacOS/Brave Browser"))
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut roots = Vec::new();
        for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            if let Some(value) = std::env::var_os(key) {
                roots.push(PathBuf::from(value));
            }
        }
        let relative: &[&str] = match variant {
            ChromeVariant::Chrome => &["Google\\Chrome\\Application\\chrome.exe"],
            ChromeVariant::Chromium => &["Chromium\\Application\\chrome.exe"],
            ChromeVariant::Edge => &["Microsoft\\Edge\\Application\\msedge.exe"],
            ChromeVariant::Brave => &["BraveSoftware\\Brave-Browser\\Application\\brave.exe"],
        };
        for root in roots {
            for rel in relative {
                candidates.push(root.join(rel));
            }
        }
    }

    candidates
}

/// All well-known install locations across every supported variant.
pub fn installed_candidates() -> Vec<PathBuf> {
    let mut all = Vec::new();
    for variant in ChromeVariant::ALL {
        all.extend(path_candidates(variant));
    }
    if let Some(configured) = configured_path() {
        all.insert(0, configured);
    }
    dedupe(all)
}

/// Resolve the first existing browser among the known locations.
pub(crate) fn find_installed() -> Option<PathBuf> {
    installed_candidates()
        .into_iter()
        .find(|path| is_browser(path))
}

/// The explicit binary configured through the environment, if any.
pub fn configured_path() -> Option<PathBuf> {
    std::env::var_os(CHROME_PATH_ENV).map(PathBuf::from)
}

/// Search `PATH` for a program name, returning every match.
pub fn find_on_path(name: &str) -> Vec<PathBuf> {
    let Some(path_var) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(executable_name(name));
        if is_browser(&candidate) {
            matches.push(candidate);
        }
    }
    matches
}

fn is_browser(path: &Path) -> bool {
    path.is_file()
}

fn executable_name(name: &str) -> String {
    if cfg!(target_os = "windows") && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

#[cfg(target_os = "macos")]
fn home_dirs() -> Vec<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| vec![PathBuf::from(home)])
        .unwrap_or_default()
}

fn dedupe(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.clone()) {
            out.push(path);
        }
    }
    out
}
