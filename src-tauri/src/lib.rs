//! LocalRouter Library
//!
//! Re-exports from workspace crates for backward compatibility.
//! The actual implementations live in the lr-* crates.

// Modules that remain in src-tauri
pub mod cli;
pub mod launcher;
pub mod ui;
pub mod updater;

// Re-exports from workspace crates
pub use lr_api_keys as api_keys;
pub use lr_catalog as catalog;
pub use lr_clients as clients;
pub use lr_config as config;
pub use lr_marketplace as marketplace;
pub use lr_mcp as mcp;
pub use lr_monitoring as monitoring;
pub use lr_oauth::browser as oauth_browser;
pub use lr_oauth::clients as oauth_clients;
pub use lr_providers as providers;
pub use lr_routellm as routellm;
pub use lr_router as router;
pub use lr_server as server;
pub use lr_skills as skills;
pub use lr_types as types;
pub use lr_utils as utils;
