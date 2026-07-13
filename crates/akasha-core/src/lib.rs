//! Trusted, interface-independent behavior for Akasha.

pub mod context;
pub mod init;
pub mod link;
pub mod project_validation;
pub mod resolution;
pub mod validation;
mod wikilink;
pub mod writes;

pub use context::{
    ContextBundle, ContextEntry, ContextError, ContextSection, DEFAULT_CONTEXT_MAX_CHARS,
    assemble_context, render_context_markdown,
};
pub use init::{InitError, InitRequest, InitResult, initialize_project};
pub use link::{LinkError, LinkRequest, LinkResult, link_project};
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
pub use writes::{AtomicCreateError, create_file_atomically};
