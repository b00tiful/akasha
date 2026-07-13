use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use akasha_core::{
    MAX_ONBOARDING_EVIDENCE_CLAIMS, MAX_ONBOARDING_EVIDENCE_SOURCES,
    MAX_ONBOARDING_INVENTORY_ENTRIES, MAX_ONBOARDING_NOTE_CHARS, MAX_ONBOARDING_NOTES,
    MAX_ONBOARDING_PROJECTION_CHARS, MAX_ONBOARDING_PROPOSAL_CHARS, MAX_ONBOARDING_TEMPLATE_CHARS,
    NoteClass, OnboardingBatchError, OnboardingBatchPreview, OnboardingBatchRequest,
    OnboardingBatchResult, OnboardingNoteAction, OnboardingPreparation, ProposedNote,
    ResolveRequest, apply_approved_onboarding_batch, prepare_onboarding, preview_onboarding_batch,
};
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::CallToolResult;
use rmcp::schemars;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const WIRE_SCHEMA_VERSION: u32 = 1;
const MAX_STORED_PROPOSALS: usize = 8;

struct ToolExecutionError<T>(Box<T>);

impl<T> ToolExecutionError<T> {
    fn new(output: T) -> Self {
        Self(Box::new(output))
    }
}

impl<T: Serialize> IntoCallToolResult for ToolExecutionError<T> {
    fn into_call_tool_result(self) -> Result<CallToolResult, ErrorData> {
        let value = serde_json::to_value(*self.0).map_err(|error| {
            ErrorData::internal_error(
                format!("failed to serialize structured tool error: {error}"),
                None,
            )
        })?;
        Ok(CallToolResult::structured_error(value))
    }
}

#[derive(Clone)]
pub struct OnboardingMcpServer {
    resolution: ResolveRequest,
    proposals: Arc<Mutex<BTreeMap<String, StoredProposal>>>,
}

#[derive(Clone)]
struct StoredProposal {
    request: OnboardingBatchRequest,
    last_preview_id: Option<String>,
}

impl OnboardingMcpServer {
    #[must_use]
    pub fn new(resolution: ResolveRequest) -> Self {
        Self {
            resolution,
            proposals: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn proposal_request(&self, proposal: WireProposal) -> OnboardingBatchRequest {
        OnboardingBatchRequest {
            resolution: self.resolution.clone(),
            notes: proposal
                .notes
                .into_iter()
                .map(|note| ProposedNote {
                    note_type: note.note_type,
                    path: PathBuf::from(note.path),
                    source: note.source,
                })
                .collect(),
            index: proposal.index,
            roadmap: proposal.roadmap,
        }
    }

    fn store_validated_proposal(
        &self,
        preview: &OnboardingBatchPreview,
        request: OnboardingBatchRequest,
    ) {
        let mut proposals = self
            .proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if proposals.len() >= MAX_STORED_PROPOSALS
            && !proposals.contains_key(&preview.proposal_id)
            && let Some(oldest_key) = proposals.keys().next().cloned()
        {
            proposals.remove(&oldest_key);
        }
        proposals.insert(
            preview.proposal_id.clone(),
            StoredProposal {
                request,
                last_preview_id: None,
            },
        );
    }

    fn load_proposal(&self, proposal_id: &str) -> Option<StoredProposal> {
        self.proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(proposal_id)
            .cloned()
    }

    fn record_preview(&self, proposal_id: &str, preview_id: String) {
        if let Some(stored) = self
            .proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(proposal_id)
        {
            stored.last_preview_id = Some(preview_id);
        }
    }

    fn clear_preview(&self, proposal_id: &str) {
        if let Some(stored) = self
            .proposals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(proposal_id)
        {
            stored.last_preview_id = None;
        }
    }
}

#[tool_router]
impl OnboardingMcpServer {
    /// Return bounded templates, note schemas, coverage criteria, limits, and existing identities.
    #[tool(
        name = "akasha_onboarding_prepare",
        description = "Prepare bounded onboarding instructions for the one project selected when this on-demand server started. This tool never returns repository source or existing note bodies.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn prepare(
        &self,
        Parameters(_input): Parameters<PrepareInput>,
    ) -> Result<Json<PrepareOutput>, ToolExecutionError<PrepareOutput>> {
        match prepare_onboarding(&self.resolution) {
            Ok(preparation) => Ok(Json(PrepareOutput::success(preparation))),
            Err(error) => Err(ToolExecutionError::new(PrepareOutput::failure(error))),
        }
    }

    /// Validate and retain one bounded proposal in memory; rejected proposals write nothing.
    #[tool(
        name = "akasha_onboarding_validate",
        description = "Validate a bounded source-attributed onboarding proposal and retain it only in this server process. Call prepare first. Invalid or rejected proposals create no files.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn validate(
        &self,
        Parameters(input): Parameters<ValidateInput>,
    ) -> Result<Json<ValidateOutput>, ToolExecutionError<ValidateOutput>> {
        let request = self.proposal_request(input.proposal);
        match preview_onboarding_batch(&request) {
            Ok(preview) => {
                self.store_validated_proposal(&preview, request);
                Ok(Json(ValidateOutput::success(&preview)))
            }
            Err(error) => Err(ToolExecutionError::new(ValidateOutput::failure(error))),
        }
    }

    /// Revalidate a retained proposal and return the exact summary and preview binding to approve.
    #[tool(
        name = "akasha_onboarding_preview",
        description = "Revalidate a retained proposal against current project state and return the exact bounded write summary plus preview_id. Present the summary to the human before requesting apply.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn preview(
        &self,
        Parameters(input): Parameters<PreviewInput>,
    ) -> Result<Json<PreviewOutput>, ToolExecutionError<PreviewOutput>> {
        let Some(stored) = self.load_proposal(&input.proposal_id) else {
            return Err(ToolExecutionError::new(PreviewOutput::missing(
                &input.proposal_id,
            )));
        };
        match preview_onboarding_batch(&stored.request) {
            Ok(preview) => {
                self.record_preview(&input.proposal_id, preview.preview_id.clone());
                Ok(Json(PreviewOutput::success(&preview)))
            }
            Err(error) => Err(ToolExecutionError::new(PreviewOutput::failure(error))),
        }
    }

    /// Apply exactly the retained proposal and project snapshot named by an approved preview.
    #[tool(
        name = "akasha_onboarding_apply",
        description = "Apply a retained proposal only when approved_preview_id is the last preview emitted for it and still matches current project state. This is the only onboarding MCP tool that writes.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn apply(
        &self,
        Parameters(input): Parameters<ApplyInput>,
    ) -> Result<Json<ApplyOutput>, ToolExecutionError<ApplyOutput>> {
        let Some(stored) = self.load_proposal(&input.proposal_id) else {
            return Err(ToolExecutionError::new(ApplyOutput::missing(
                &input.proposal_id,
            )));
        };
        if stored.last_preview_id.as_deref() != Some(input.approved_preview_id.as_str()) {
            return Err(ToolExecutionError::new(ApplyOutput::failure_message(
                5,
                "approved_preview_id is not the last preview emitted for this proposal",
            )));
        }

        match apply_approved_onboarding_batch(&stored.request, &input.approved_preview_id) {
            Ok(result) => {
                self.clear_preview(&input.proposal_id);
                Ok(Json(ApplyOutput::success(&input.proposal_id, result)))
            }
            Err(error) => Err(ToolExecutionError::new(ApplyOutput::failure(error))),
        }
    }
}

#[tool_handler(
    name = "akasha-onboarding",
    version = "0.1.0",
    instructions = "On-demand, model-free onboarding only. Use prepare, inspect the repository with your own tools, validate, preview, obtain human approval for that exact summary, then apply. Ordinary Markdown operations do not belong here."
)]
impl ServerHandler for OnboardingMcpServer {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateInput {
    pub proposal: WireProposal,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewInput {
    pub proposal_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyInput {
    pub proposal_id: String,
    pub approved_preview_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WireProposal {
    pub notes: Vec<WireProposedNote>,
    pub index: String,
    pub roadmap: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WireProposedNote {
    pub note_type: String,
    pub path: String,
    pub source: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ToolFailure {
    pub exit_code: u8,
    pub message: String,
}

impl ToolFailure {
    fn from_core(error: OnboardingBatchError) -> Self {
        Self {
            exit_code: error.exit_code(),
            message: error.to_string(),
        }
    }

    fn message(exit_code: u8, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PrepareOutput {
    pub schema_version: u32,
    pub success: bool,
    pub preparation: Option<WirePreparation>,
    pub error: Option<ToolFailure>,
}

impl PrepareOutput {
    fn success(preparation: OnboardingPreparation) -> Self {
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: true,
            preparation: Some(preparation.into()),
            error: None,
        }
    }

    fn failure(error: OnboardingBatchError) -> Self {
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: false,
            preparation: None,
            error: Some(ToolFailure::from_core(error)),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WirePreparation {
    pub root: String,
    pub project: String,
    pub repository_dir: String,
    pub project_dir: String,
    pub note_types: BTreeMap<String, WireNoteType>,
    pub templates: Vec<WireTemplate>,
    pub omitted_templates: usize,
    pub template_characters: usize,
    pub existing_notes: Vec<WireInventoryEntry>,
    pub omitted_existing_notes: usize,
    pub coverage_criteria: Vec<String>,
    pub evidence_contract: String,
    pub limits: WireLimits,
}

impl From<OnboardingPreparation> for WirePreparation {
    fn from(preparation: OnboardingPreparation) -> Self {
        Self {
            root: path_string(&preparation.root),
            project: preparation.project,
            repository_dir: path_string(&preparation.repository_dir),
            project_dir: path_string(&preparation.project_dir),
            note_types: preparation
                .note_types
                .into_iter()
                .map(|(name, note_type)| {
                    (
                        name,
                        WireNoteType {
                            class: note_class_name(note_type.class).to_owned(),
                            folder: path_string(&note_type.folder),
                            required_fields: note_type.required_fields,
                        },
                    )
                })
                .collect(),
            templates: preparation
                .templates
                .into_iter()
                .map(|template| WireTemplate {
                    path: path_string(&template.path),
                    source: template.source,
                })
                .collect(),
            omitted_templates: preparation.omitted_templates,
            template_characters: preparation.template_characters,
            existing_notes: preparation
                .existing_notes
                .into_iter()
                .map(|entry| WireInventoryEntry {
                    note_type: entry.note_type,
                    path: path_string(&entry.path),
                })
                .collect(),
            omitted_existing_notes: preparation.omitted_existing_notes,
            coverage_criteria: preparation.coverage_criteria,
            evidence_contract: preparation.evidence_contract,
            limits: WireLimits::current(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WireNoteType {
    pub class: String,
    pub folder: String,
    pub required_fields: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WireTemplate {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WireInventoryEntry {
    pub note_type: String,
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WireLimits {
    pub stored_proposals: usize,
    pub notes: usize,
    pub note_characters: usize,
    pub projection_characters: usize,
    pub proposal_characters: usize,
    pub evidence_claims_per_note: usize,
    pub evidence_sources_per_claim: usize,
    pub template_characters: usize,
    pub inventory_entries: usize,
}

impl WireLimits {
    const fn current() -> Self {
        Self {
            stored_proposals: MAX_STORED_PROPOSALS,
            notes: MAX_ONBOARDING_NOTES,
            note_characters: MAX_ONBOARDING_NOTE_CHARS,
            projection_characters: MAX_ONBOARDING_PROJECTION_CHARS,
            proposal_characters: MAX_ONBOARDING_PROPOSAL_CHARS,
            evidence_claims_per_note: MAX_ONBOARDING_EVIDENCE_CLAIMS,
            evidence_sources_per_claim: MAX_ONBOARDING_EVIDENCE_SOURCES,
            template_characters: MAX_ONBOARDING_TEMPLATE_CHARS,
            inventory_entries: MAX_ONBOARDING_INVENTORY_ENTRIES,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ValidateOutput {
    pub schema_version: u32,
    pub success: bool,
    pub proposal_id: Option<String>,
    pub project: Option<String>,
    pub notes: Vec<WireNotePreview>,
    pub error: Option<ToolFailure>,
}

impl ValidateOutput {
    fn success(preview: &OnboardingBatchPreview) -> Self {
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: true,
            proposal_id: Some(preview.proposal_id.clone()),
            project: Some(preview.project.clone()),
            notes: preview_notes(preview),
            error: None,
        }
    }

    fn failure(error: OnboardingBatchError) -> Self {
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: false,
            proposal_id: None,
            project: None,
            notes: Vec::new(),
            error: Some(ToolFailure::from_core(error)),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewOutput {
    pub schema_version: u32,
    pub success: bool,
    pub proposal_id: Option<String>,
    pub preview_id: Option<String>,
    pub project: Option<String>,
    pub notes: Vec<WireNotePreview>,
    pub index_changed: Option<bool>,
    pub roadmap_changed: Option<bool>,
    pub state_changed: Option<bool>,
    pub approval_summary: Option<String>,
    pub error: Option<ToolFailure>,
}

impl PreviewOutput {
    fn success(preview: &OnboardingBatchPreview) -> Self {
        let created = preview
            .notes
            .iter()
            .filter(|note| note.action == OnboardingNoteAction::Create)
            .count();
        let unchanged = preview.notes.len() - created;
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: true,
            proposal_id: Some(preview.proposal_id.clone()),
            preview_id: Some(preview.preview_id.clone()),
            project: Some(preview.project.clone()),
            notes: preview_notes(preview),
            index_changed: Some(preview.index_changed),
            roadmap_changed: Some(preview.roadmap_changed),
            state_changed: Some(preview.state_changed),
            approval_summary: Some(format!(
                "Project {}: create {created} notes, keep {unchanged} exact notes, {} index, {} roadmap, and {} project state.",
                preview.project,
                change_word(preview.index_changed),
                change_word(preview.roadmap_changed),
                change_word(preview.state_changed)
            )),
            error: None,
        }
    }

    fn failure(error: OnboardingBatchError) -> Self {
        Self::failure_value(ToolFailure::from_core(error))
    }

    fn missing(proposal_id: &str) -> Self {
        Self::failure_value(ToolFailure::message(
            4,
            format!("proposal {proposal_id:?} is not retained by this server process"),
        ))
    }

    fn failure_value(error: ToolFailure) -> Self {
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: false,
            proposal_id: None,
            preview_id: None,
            project: None,
            notes: Vec::new(),
            index_changed: None,
            roadmap_changed: None,
            state_changed: None,
            approval_summary: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WireNotePreview {
    pub note_type: String,
    pub path: String,
    pub action: String,
    pub evidence_claims: usize,
}

fn preview_notes(preview: &OnboardingBatchPreview) -> Vec<WireNotePreview> {
    preview
        .notes
        .iter()
        .map(|note| WireNotePreview {
            note_type: note.note_type.clone(),
            path: path_string(&note.path),
            action: match note.action {
                OnboardingNoteAction::Create => "create",
                OnboardingNoteAction::Unchanged => "unchanged",
            }
            .to_owned(),
            evidence_claims: note.evidence_claims,
        })
        .collect()
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApplyOutput {
    pub schema_version: u32,
    pub success: bool,
    pub proposal_id: Option<String>,
    pub project: Option<String>,
    pub created_notes: Vec<String>,
    pub unchanged_notes: Vec<String>,
    pub updated_projections: Vec<String>,
    pub state: Option<String>,
    pub error: Option<ToolFailure>,
}

impl ApplyOutput {
    fn success(proposal_id: &str, result: OnboardingBatchResult) -> Self {
        let relative = |path: &Path| {
            path.strip_prefix(&result.project_dir)
                .map_or_else(|_| path_string(path), path_string)
        };
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: true,
            proposal_id: Some(proposal_id.to_owned()),
            project: Some(result.project),
            created_notes: result
                .created_notes
                .iter()
                .map(|path| relative(path))
                .collect(),
            unchanged_notes: result
                .unchanged_notes
                .iter()
                .map(|path| relative(path))
                .collect(),
            updated_projections: result
                .updated_projections
                .iter()
                .map(|path| relative(path))
                .collect(),
            state: Some(relative(&result.state)),
            error: None,
        }
    }

    fn failure(error: OnboardingBatchError) -> Self {
        Self::failure_value(ToolFailure::from_core(error))
    }

    fn failure_message(exit_code: u8, message: impl Into<String>) -> Self {
        Self::failure_value(ToolFailure::message(exit_code, message))
    }

    fn missing(proposal_id: &str) -> Self {
        Self::failure_message(
            4,
            format!("proposal {proposal_id:?} is not retained by this server process"),
        )
    }

    fn failure_value(error: ToolFailure) -> Self {
        Self {
            schema_version: WIRE_SCHEMA_VERSION,
            success: false,
            proposal_id: None,
            project: None,
            created_notes: Vec::new(),
            unchanged_notes: Vec::new(),
            updated_projections: Vec::new(),
            state: None,
            error: Some(error),
        }
    }
}

fn note_class_name(class: NoteClass) -> &'static str {
    match class {
        NoteClass::Event => "event",
        NoteClass::Record => "record",
        NoteClass::Entity => "entity",
    }
}

fn change_word(changed: bool) -> &'static str {
    if changed { "update" } else { "keep" }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use akasha_core::{InitRequest, ResolutionEnvironment, initialize_project};
    use rmcp::ServerHandler;
    use rmcp::handler::server::tool::IntoCallToolResult;
    use sha2::{Digest, Sha256};

    use super::*;

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn advertises_four_bounded_tools_with_exact_safety_annotations() {
        let server = OnboardingMcpServer::new(ResolveRequest {
            root_override: None,
            project_override: None,
            cwd: PathBuf::from("."),
            environment: ResolutionEnvironment::default(),
        });

        for (name, read_only, destructive) in [
            ("akasha_onboarding_prepare", true, false),
            ("akasha_onboarding_validate", true, false),
            ("akasha_onboarding_preview", true, false),
            ("akasha_onboarding_apply", false, true),
        ] {
            let tool = ServerHandler::get_tool(&server, name).expect("advertised tool");
            assert!(tool.output_schema.is_some());
            let annotations = tool.annotations.expect("tool safety annotations");
            assert_eq!(annotations.read_only_hint, Some(read_only));
            assert_eq!(annotations.destructive_hint, Some(destructive));
            assert_eq!(annotations.idempotent_hint, Some(true));
            assert_eq!(annotations.open_world_hint, Some(false));
        }
    }

    #[test]
    fn tool_workflow_retains_preview_binding_and_applies_through_core() {
        let fixture = Fixture::new();
        let server = OnboardingMcpServer::new(fixture.resolution.clone());

        let Json(prepared) = server
            .prepare(Parameters(PrepareInput {}))
            .unwrap_or_else(|_| panic!("prepare succeeds"));
        assert!(prepared.success);
        assert_eq!(
            prepared.preparation.expect("preparation").project,
            "example"
        );

        let Json(validated) = server
            .validate(Parameters(ValidateInput {
                proposal: fixture.proposal(),
            }))
            .unwrap_or_else(|_| panic!("validate succeeds"));
        assert!(validated.success);
        let proposal_id = validated.proposal_id.expect("proposal id");

        let Json(previewed) = server
            .preview(Parameters(PreviewInput {
                proposal_id: proposal_id.clone(),
            }))
            .unwrap_or_else(|_| panic!("preview succeeds"));
        assert!(previewed.success);
        assert!(previewed.approval_summary.is_some());
        let preview_id = previewed.preview_id.expect("preview id");

        let Json(applied) = server
            .apply(Parameters(ApplyInput {
                proposal_id,
                approved_preview_id: preview_id,
            }))
            .unwrap_or_else(|_| panic!("apply succeeds"));
        assert!(applied.success);
        assert_eq!(applied.created_notes, vec!["entities/core.md"]);
        assert!(fixture.project.join("entities/core.md").is_file());
    }

    #[test]
    fn classified_failures_are_mcp_tool_execution_errors() {
        let fixture = Fixture::new();
        let server = OnboardingMcpServer::new(fixture.resolution);

        let missing = server.preview(Parameters(PreviewInput {
            proposal_id: "missing".to_owned(),
        }));
        match &missing {
            Err(error) => assert!(!error.0.success),
            Ok(_) => panic!("missing proposal must be a classified failure"),
        }

        let result = missing
            .into_call_tool_result()
            .expect("serialize classified tool failure");
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_some());
    }

    struct Fixture {
        _temp: TempDir,
        project: PathBuf,
        repository: PathBuf,
        resolution: ResolveRequest,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new();
            let root = temp.0.join("root");
            let repository = temp.0.join("repository");
            for directory in [
                root.join("Meta"),
                root.join("templates"),
                root.join("Global"),
                root.join("Projects"),
                root.join("Inbox"),
                repository.clone(),
            ] {
                fs::create_dir_all(directory).expect("create fixture directory");
            }
            fs::write(
                root.join("akasha.toml"),
                include_str!("../../../tests/fixtures/resolution/valid-root/akasha.toml"),
            )
            .expect("write root config");
            fs::write(root.join("Meta/projects.yaml"), "{}\n").expect("write registry");
            fs::write(
                root.join("templates/entity.md"),
                "---\nschema_version: 1\n---\n\n# Entity\n",
            )
            .expect("write template");
            fs::write(
                repository.join("Cargo.toml"),
                "[package]\nname = \"example\"\n",
            )
            .expect("write evidence source");
            initialize_project(&InitRequest {
                root_override: Some(root.clone()),
                project: "example".to_owned(),
                cwd: repository.clone(),
                environment: ResolutionEnvironment::default(),
            })
            .expect("initialize project");
            let project = root.join("Projects/example");
            let resolution = ResolveRequest {
                root_override: Some(root),
                project_override: None,
                cwd: repository.clone(),
                environment: ResolutionEnvironment::default(),
            };
            Self {
                _temp: temp,
                project,
                repository,
                resolution,
            }
        }

        fn proposal(&self) -> WireProposal {
            let source = fs::read(self.repository.join("Cargo.toml")).expect("read source");
            let fingerprint = fingerprint(&source);
            WireProposal {
                notes: vec![WireProposedNote {
                    note_type: "entity".to_owned(),
                    path: "core.md".to_owned(),
                    source: format!(
                        "---\nschema_version: 1\nentity: core\nkind: subsystem\nstatus: active\n\
                         reviewed: 2026-07-13\nevidence:\n  - kind: fact\n    claim: The repository \
                         declares a Rust package.\n    sources:\n      - path: Cargo.toml\n        \
                         fingerprint: \"{fingerprint}\"\n        line_start: 1\n        line_end: 2\n\
                         ---\n\n# Core\n"
                    ),
                }],
                index: "# Index\n\n- [[Projects/example/entities/core]]\n".to_owned(),
                roadmap: String::new(),
            }
        }
    }

    fn fingerprint(source: &[u8]) -> String {
        let digest = Sha256::digest(source);
        let mut output = String::from("sha256:");
        for byte in digest {
            write!(output, "{byte:02x}").expect("writing to a string cannot fail");
        }
        output
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("akasha-mcp-test-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("create temporary directory");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
