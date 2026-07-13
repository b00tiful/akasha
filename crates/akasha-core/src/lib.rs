//! Trusted, interface-independent behavior for Akasha.

pub mod resolution;
pub mod validation;

pub use resolution::{
    ProjectSource, ResolutionEnvironment, ResolveError, ResolveRequest, ResolvedProject,
    RootSource, resolve_project,
};
pub use validation::{
    ParsedNote, ProjectRegistry, ProjectRegistryEntry, ValidationError, parse_leading_frontmatter,
    parse_leading_frontmatter_bytes, parse_project_registry,
};
