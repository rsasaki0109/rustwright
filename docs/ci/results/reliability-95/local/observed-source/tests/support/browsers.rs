//! Optional local browser tests become mandatory when a browser is configured.

use std::{fmt::Display, path::PathBuf};

fn required(executable_env: &str) -> bool {
    std::env::var_os(executable_env).is_some()
        || std::env::var("RUSTWRIGHT_REQUIRE_BROWSERS").as_deref() == Ok("1")
}

pub fn available<E: Display>(executable_env: &str, discovery: Result<PathBuf, E>) -> bool {
    // installed() may fall back to discovery when an override is invalid.
    // Tests must not silently substitute another browser for an explicit one.
    if let Some(configured) = std::env::var_os(executable_env) {
        let path = PathBuf::from(configured);
        assert!(
            path.is_file(),
            "required browser {executable_env} is not an installed file: {}",
            path.display()
        );
    }
    optional_result(executable_env, "discovery", discovery).is_some()
}

pub fn optional_result<T, E: Display>(
    executable_env: &str,
    stage: &str,
    result: Result<T, E>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            assert!(
                !required(executable_env),
                "required browser {executable_env} {stage} failed: {error}"
            );
            eprintln!("skipping optional browser {executable_env}: {stage} failed: {error}");
            None
        }
    }
}
