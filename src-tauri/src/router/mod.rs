//! Smart routing system
//!
//! Intelligent model selection and request routing with fallback support.

pub mod rate_limit;

// TODO: Implement remaining routing system components
// - RouterConfig struct
// - Routing strategies (cost, performance, local/remote)
// - Routing engine
// - Fallback mechanism

// Re-export commonly used types
pub use rate_limit::{
    RateLimitCheckResult, RateLimitType, RateLimiter, RateLimiterKey, RateLimiterManager, UsageInfo,
};
