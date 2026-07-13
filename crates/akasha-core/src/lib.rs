//! Trusted, interface-independent behavior for Akasha.

pub mod context;
pub mod init;
pub mod library;
pub mod link;
pub mod onboarding;
pub mod project_validation;
pub mod resolution;
mod state;
pub mod validation;
mod wikilink;
pub mod writes;

pub use context::{
    ContextBundle, ContextEntry, ContextError, ContextSection, DEFAULT_CONTEXT_MAX_CHARS,
    assemble_context, render_context_markdown,
};
pub use init::{InitError, InitRequest, InitResult, initialize_project};
pub use library::{
    LibraryBook, LibraryCategory, LibraryCollection, LibraryDocument, LibraryProjection,
    LibraryScope, LibraryShelf, build_library_projection, load_library_document,
    render_library_markdown,
};
pub use link::{LinkError, LinkRequest, LinkResult, link_project};
pub use onboarding::{
    MAX_ONBOARDING_EVIDENCE_CLAIMS, MAX_ONBOARDING_EVIDENCE_SOURCES,
    MAX_ONBOARDING_INVENTORY_ENTRIES, MAX_ONBOARDING_NOTE_CHARS, MAX_ONBOARDING_NOTES,
    MAX_ONBOARDING_PROJECTION_CHARS, MAX_ONBOARDING_PROPOSAL_CHARS, MAX_ONBOARDING_TEMPLATE_CHARS,
    OnboardingBatchError, OnboardingBatchPreview, OnboardingBatchRequest, OnboardingBatchResult,
    OnboardingInventoryEntry, OnboardingNoteAction, OnboardingNotePreview, OnboardingNoteType,
    OnboardingPreparation, OnboardingTemplate, ProposedNote, apply_approved_onboarding_batch,
    apply_onboarding_batch, prepare_onboarding, preview_onboarding_batch,
};
pub use project_validation::{
    NoteTypeValidation, ProjectValidationError, ProjectValidationReport, ProjectionValidation,
    validate_project,
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
