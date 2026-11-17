//! Search for Crow: directory walking (`ignore`), fuzzy matching (`nucleo`),
//! content search (`grep-searcher`) and exclusion rule sets (`globset` plus
//! sentinel files).

mod engine;
mod rules;

pub mod corpus;

pub use engine::{FuzzyMatcher, Hit, HitKind, Query, Skip, SkipReason, Summary, search};
pub use rules::RuleSet;
