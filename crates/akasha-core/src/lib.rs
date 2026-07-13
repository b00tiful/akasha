//! Trusted, interface-independent behavior for Akasha.

pub mod project_validation;
pub mod resolution;
pub mod validation;

pub use project_validation::{
    NoteTypeValidation, ProjectValidationError, ProjectValidationReport, validate_project,
};
pub use resolution::{
    NoteClass, ProjectSource, ResolutionEnvironment, ResolveError, ResolveRequest, ResolvedProject,
    RootSource, resolve_project,
};
pub use validation::{
    ParsedNote, ProjectRegistry, ProjectRegistryEntry, ValidationError, parse_leading_frontmatter,
    parse_leading_frontmatter_bytes, parse_project_registry,
};
