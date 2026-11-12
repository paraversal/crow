//! Search for Crow: directory walking (`ignore`), fuzzy matching (`nucleo`),
//! content search (`grep-searcher`) and exclusion rule sets (`globset` plus
//! sentinel files).

mod engine;
mod rules;

pub use engine::{FuzzyMatcher, Hit, HitKind, Query, Summary, search};
pub use rules::RuleSet;
