//! Fuzzy matching for skill name resolution.
//!
//! Re-exports from the shared `lr_types::fuzzy` module.

pub(crate) use lr_types::fuzzy::{find_best_match, MatchKind};

// Re-export normalize_name as normalize_skill_name for backwards compat within this crate
#[allow(unused_imports)]
pub(crate) use lr_types::fuzzy::normalize_name;
