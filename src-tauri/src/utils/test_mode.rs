//! Test mode utilities
//!
//! Provides helpers for detecting when the app is running in E2E test mode.

use std::env;

/// Check if the app is running in E2E test mode
///
/// Test mode is detected via the LOCALROUTER_ENV environment variable.
/// Any value starting with "test" (e.g., "test", "test-e2e") triggers test mode.
pub fn is_test_mode() -> bool {
    env::var("LOCALROUTER_ENV")
        .map(|v| v.starts_with("test"))
        .unwrap_or(false)
}

/// Get the test mode suffix (e.g., "test" returns "", "test-e2e" returns "-e2e")
pub fn test_mode_suffix() -> Option<String> {
    env::var("LOCALROUTER_ENV").ok().and_then(|v| {
        if v.starts_with("test") {
            if v == "test" {
                Some(String::new())
            } else {
                Some(v.strip_prefix("test").unwrap_or("").to_string())
            }
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_is_test_mode_false() {
        env::remove_var("LOCALROUTER_ENV");
        assert!(!is_test_mode());

        env::set_var("LOCALROUTER_ENV", "dev");
        assert!(!is_test_mode());
        env::remove_var("LOCALROUTER_ENV");
    }

    #[test]
    fn test_is_test_mode_true() {
        env::set_var("LOCALROUTER_ENV", "test");
        assert!(is_test_mode());

        env::set_var("LOCALROUTER_ENV", "test-e2e");
        assert!(is_test_mode());

        env::remove_var("LOCALROUTER_ENV");
    }
}
