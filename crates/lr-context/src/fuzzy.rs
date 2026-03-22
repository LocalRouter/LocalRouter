//! Fuzzy correction for search queries.
//!
//! Delegates to the shared `lr_types::fuzzy` module.

pub(crate) use lr_types::fuzzy::{find_best_correction, max_edit_distance};
