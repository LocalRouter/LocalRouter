//! Configuration migration system
//!
//! Handles migrating configuration files between versions.

use super::{AppConfig, CONFIG_VERSION};
use crate::utils::errors::AppResult;
use tracing::info;

/// Migrate configuration from an older version to the current version
pub fn migrate_config(mut config: AppConfig) -> AppResult<AppConfig> {
    let original_version = config.version;

    if original_version >= CONFIG_VERSION {
        // No migration needed
        return Ok(config);
    }

    info!(
        "Migrating configuration from version {} to {}",
        original_version, CONFIG_VERSION
    );

    // Apply migrations sequentially
    if config.version < 1 {
        config = migrate_to_v1(config)?;
    }

    // Future migrations will go here
    // if config.version < 2 {
    //     config = migrate_to_v2(config)?;
    // }

    // Update version to current
    config.version = CONFIG_VERSION;

    info!(
        "Successfully migrated configuration from version {} to {}",
        original_version, CONFIG_VERSION
    );

    Ok(config)
}

/// Migrate to version 1 (initial version)
///
/// This is a placeholder for the initial version. In practice, version 1
/// is the first version, so there's nothing to migrate from.
fn migrate_to_v1(config: AppConfig) -> AppResult<AppConfig> {
    // Version 1 is the initial version, so no actual migration is needed
    // This function exists as a template for future migrations
    Ok(config)
}

// Future migration functions will follow this pattern:
//
// fn migrate_to_v2(mut config: AppConfig) -> AppResult<AppConfig> {
//     info!("Migrating to version 2");
//
//     // Example: Add new field with default value
//     // config.new_field = default_value();
//
//     // Example: Rename a field
//     // config.new_name = config.old_name.clone();
//
//     // Example: Transform data structure
//     // config.items = config.old_items.iter().map(|item| transform(item)).collect();
//
//     config.version = 2;
//     Ok(config)
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_current_version() {
        let config = AppConfig::default();
        let original_version = config.version;

        let migrated = migrate_config(config).unwrap();

        assert_eq!(migrated.version, original_version);
        assert_eq!(migrated.version, CONFIG_VERSION);
    }

    #[test]
    fn test_migrate_from_future_version() {
        let mut config = AppConfig::default();
        config.version = CONFIG_VERSION + 1;

        let result = migrate_config(config);

        // Should succeed (no migration needed)
        assert!(result.is_ok());
        assert_eq!(result.unwrap().version, CONFIG_VERSION + 1);
    }

    #[test]
    fn test_migrate_preserves_data() {
        let mut config = AppConfig::default();
        let original_host = "test.example.com".to_string();
        config.server.host = original_host.clone();
        config.version = 0; // Old version

        let migrated = migrate_config(config).unwrap();

        assert_eq!(migrated.version, CONFIG_VERSION);
        assert_eq!(migrated.server.host, original_host);
    }
}
