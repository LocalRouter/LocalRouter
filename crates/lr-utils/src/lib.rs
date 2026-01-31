//! Utility functions and helpers for LocalRouter

pub mod crypto;
pub mod paths;
pub mod test_mode;

// Re-export errors from lr-types for backward compatibility (utils::errors::AppError)
pub use lr_types::errors;
