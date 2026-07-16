//! Trusted, interface-independent behavior for Akasha.

pub mod agent_wiring;
pub mod context;
pub mod event;
mod evidence;
pub mod init;
pub mod library;
pub mod link;
pub mod note_creation;
pub mod note_edit;
pub mod note_template;
pub mod onboarding;
pub mod project_validation;
pub mod resolution;
mod state;
pub mod validation;
mod wikilink;
pub mod writes;

pub use agent_wiring::{
    AgentClient, AgentWiringAction, AgentWiringError, AgentWiringPatch, AgentWiringPlan,
    prepare_agent_wiring,
};
pub use context::{
    ContextBundle, ContextEntry, ContextError, ContextSection, DEFAULT_CONTEXT_MAX_CHARS,
    assemble_context, render_context_markdown,
};
pub use event::{EventCreationError, EventCreationResult, create_event};
pub use init::{InitError, InitRecovery, InitRequest, InitResult, initialize_project};
pub use library::{
    LibraryBook, LibraryCategory, LibraryCollection, LibraryDocument, LibraryProjection,
    LibraryScope, LibraryShelf, build_library_projection, load_library_document,
    render_library_markdown,
};
pub use link::{LinkError, LinkRequest, LinkResult, link_project};
pub use note_creation::{MutableNoteCreationError, MutableNoteCreationResult, create_mutable_note};
pub use note_edit::{
    EntityUpdateResult, NOTE_EDIT_JOURNAL_FILE, NoteEditError, NoteEditRecovery, NoteEditResult,
    RecordUpdateResult, recover_pending_note_edit, replace_library_document, update_entity,
    update_record,
};
pub use note_template::{
    NoteTemplateError, NoteTemplateScope, ResolvedNoteTemplate, resolve_note_template,
};
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
