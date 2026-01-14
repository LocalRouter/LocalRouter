//! Middleware modules for request processing

pub mod auth;
pub mod error;

pub use auth::auth_middleware;
pub use error::{ApiErrorResponse, ApiResult};
