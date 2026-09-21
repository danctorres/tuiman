//! Package resolvers: each maps catalog entries onto package names in one
//! ecosystem. Matching is always anchored on the repository URL recorded in
//! the package's own metadata, never on the name alone.

pub mod brew;
pub mod bulk;
pub mod registries;
pub mod repology;

use tuiman_index::Ecosystem;

/// A resolver's verdict: entry index, ecosystem, package name.
pub type Found = (usize, Ecosystem, String);
