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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;
    use std::env;

    #[test]
    fn test_is_test_mode_false() {
        let _lock = env_lock();
        env::remove_var("LOCALROUTER_ENV");
        assert!(!is_test_mode());

        env::set_var("LOCALROUTER_ENV", "dev");
        assert!(!is_test_mode());
        env::remove_var("LOCALROUTER_ENV");
    }

    #[test]
    fn test_is_test_mode_true() {
        let _lock = env_lock();
        env::set_var("LOCALROUTER_ENV", "test");
        assert!(is_test_mode());

        env::set_var("LOCALROUTER_ENV", "test-e2e");
        assert!(is_test_mode());

        env::remove_var("LOCALROUTER_ENV");
    }
}
