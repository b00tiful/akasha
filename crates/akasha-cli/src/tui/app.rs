use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use super::editor::{Editor, safe_text};
use super::integration::{Operation as IntegrationOperation, Review as IntegrationReview};
use super::state::{NavigationLocation, NavigationState};
use crate::discover_agent_home;
use crate::render::{agent_wiring_action_name, session_hook_action_name};
use akasha_core::{
    AgentClient, AgentWiringPlan, LibraryBook, LibraryDocument, LibraryProjection, LibraryScope,
    LibrarySearchResult, MutableNoteCreationForm, MutableNoteCreationResult, NoteClass,
    RecordUpdateResult, ResolveRequest, SessionHookWiringPlan, TaskLifecycleForm, assemble_context,
    build_library_projection, create_mutable_note, load_library_document, prepare_agent_wiring,
    prepare_mutable_note_creation, prepare_session_hook_wiring, prepare_task_lifecycle,
    recover_pending_note_edit, render_context_markdown, replace_library_document, search_library,
    update_record, validate_project,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use ratatui_textarea::{CursorMove, TextArea};

const PROMPT_PLACEHOLDER: &str = "Type / for commands; /search words to find notes";

pub(super) struct Completion {
    pub command: &'static str,
    pub argument: &'static str,
    pub description: &'static str,
}

#[derive(Clone)]
pub(super) struct Suggestion {
    pub command: String,
    pub argument: String,
    pub description: String,
}

const COMMANDS: &[Completion] = &[
    Completion {
        command: "integration",
        argument: "apply|remove instructions|hook codex|claude [HOME]",
        description: "Review one client integration change before confirmation",
    },
    Completion {
        command: "confirm",
        argument: "PLAN_ID",
        description: "Authorize the exact displayed integration plan",
    },
    Completion {
        command: "home",
        argument: "",
        description: "Return to the welcome dashboard",
    },
    Completion {
        command: "projects",
        argument: "",
        description: "Browse projects and shared knowledge",
    },
    Completion {
        command: "project",
        argument: "SLUG",
        description: "Switch to a registered project",
    },
    Completion {
        command: "global",
        argument: "",
        description: "Browse shared knowledge",
    },
    Completion {
        command: "ls",
        argument: "",
        description: "Browse categories in the current scope",
    },
    Completion {
        command: "type",
        argument: "NAME",
        description: "Browse a configured note category",
    },
    Completion {
        command: "open",
        argument: "NUMBER/PATH",
        description: "Open a listed item or exact note path",
    },
    Completion {
        command: "search",
        argument: "TEXT",
        description: "Search the current project or global scope",
    },
    Completion {
        command: "search-all",
        argument: "TEXT",
        description: "Search every project and shared knowledge",
    },
    Completion {
        command: "context",
        argument: "",
        description: "Read bounded project orientation",
    },
    Completion {
        command: "create",
        argument: "TYPE",
        description: "Create a configured record or entity",
    },
    Completion {
        command: "lifecycle",
        argument: "",
        description: "Update the open task and roadmap together",
    },
    Completion {
        command: "validate",
        argument: "",
        description: "Check the selected project's memory",
    },
    Completion {
        command: "integrations",
        argument: "CLIENT [HOME]",
        description: "Inspect read-only Codex or Claude wiring plans",
    },
    Completion {
        command: "edit",
        argument: "",
        description: "Edit the current note's Markdown source",
    },
    Completion {
        command: "read",
        argument: "",
        description: "Return to reading mode",
    },
    Completion {
        command: "save",
        argument: "",
        description: "Save through a checked core transaction",
    },
    Completion {
        command: "discard",
        argument: "",
        description: "Discard unsaved editor changes",
    },
    Completion {
        command: "back",
        argument: "",
        description: "Return to the previous library list",
    },
    Completion {
        command: "refresh",
        argument: "",
        description: "Reload the library snapshot",
    },
    Completion {
        command: "motion",
        argument: "",
        description: "Pause or resume ambient animation",
    },
    Completion {
        command: "help",
        argument: "",
        description: "Show commands and keyboard shortcuts",
    },
    Completion {
        command: "quit",
        argument: "",
        description: "Exit after saving or discarding changes",
    },
];

fn prompt_area_with_placeholder(text: String, placeholder: &str) -> TextArea<'static> {
    let mut prompt = TextArea::new(vec![text]);
    prompt.set_placeholder_text(placeholder);
    prompt.move_cursor(CursorMove::End);
    prompt
}

fn prompt_area(text: String) -> TextArea<'static> {
    prompt_area_with_placeholder(text, PROMPT_PLACEHOLDER)
}

pub(super) const HELP: &str = "AKASHA · TERMINAL\n\nTab             complete nonempty prompt; otherwise switch panes\nShift-Tab       switch panes even with a command draft\nEnter           open selected item / run command\nEscape          back to list / parent level\nBackspace       focus prompt outside text editing\nCtrl-S          save or apply the current checked form\nCtrl-N / Ctrl-P next / previous document in a form\nCtrl-Q          quit; unsaved changes prevent exit\nCtrl-C          clear command first, otherwise safe quit\nF2              edit selected note\nF5              refresh library\nF1              this help\nF3              search (type words, then Enter)\nF4 / F6         projects / global knowledge\nF7              back to list / parent level\n\nCOMMANDS\nhome            return to the memory dashboard\nprojects        browse registered projects\nproject SLUG    select a project\nglobal          browse shared knowledge\nls              categories in current scope\ntype NAME       open a configured note category\nopen NUMBER     open a numbered item\nopen PATH       open an exact note identity\nback            return to previous list\nsearch TEXT     literal text search in current scope\nsearch-all TEXT search every project and global notes\ncreate TYPE     guided configured record/entity creation\nlifecycle       update the open task and roadmap together\nedit / read     source editor / reading mode\nsave            save through checked core transaction\ndiscard         discard editor changes or cancel a form\ncontext         bounded project orientation\nvalidate        validate selected project\nintegrations CLIENT [HOME]\n                inspect read-only client wiring plans\nintegration apply|remove instructions|hook CLIENT [HOME]\n                review one exact client-home change\nconfirm PLAN_ID authorize the displayed integration plan\nrefresh         reload data (or press F5)\nmotion          toggle ambient animation\nhelp / quit     help / exit\n\nEDITOR\nArrows, Home/End, PageUp/Down; Shift selects text.\nCtrl-Z undo; Ctrl-Y redo; Ctrl-X cut; Ctrl-V internal paste.\nUse the terminal's paste shortcut for system clipboard text.\nEsc goes back; unsaved changes prevent leaving. Click the prompt to enter commands.\n\nMouse: click a row or action; wheel scrolls lists/readers.\nHold Shift with the mouse for terminal-native text selection.\nReading: arrows/PageUp/PageDown scroll; Left/Esc returns to list; Backspace focuses the prompt.\nCommand prompt: / opens commands; Up/Down select; Tab completes.\n/open then Tab lists notes; filter by title or path; Enter opens.\n/create then Tab lists configured record/entity types.\n/search memory finds titles or contents containing memory in the current scope.\n/search-all memory searches all projects and global notes.\nEnter runs commands or fills an argument prefix; Esc closes the menu.\nWithout the menu, Up/Down recall session history.\nCtrl-A/E move to start/end; Ctrl-U/K clear before/after cursor.\nCtrl-W deletes the previous word.\n\nCreation and task lifecycle forms show exact configured templates and\nmaintained projections. Ctrl-S applies both through checked core writes;\nDiscard cancels without writing. Integration inspection never writes.\n/integration prepares one exact patch; /confirm PLAN_ID authorizes it.\n/discard cancels the review; stale plans require a fresh review.\n\nOpen from a linked repository or pass --root PATH --project SLUG.\nSSH: run Akasha on the remote host in an allocated terminal.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Prompt,
    List,
    Reader,
}
#[derive(Clone)]
pub(super) enum Target {
    Scope(LibraryScope),
    Category(LibraryScope, String),
    Note(String),
}
#[derive(Clone)]
pub(super) struct Row {
    pub label: String,
    pub detail: String,
    pub target: Target,
}
#[derive(Clone)]
struct Navigation {
    title: String,
    rows: Vec<Row>,
    selection: Option<usize>,
    scope: LibraryScope,
    list_context: ListContext,
}

#[derive(Clone)]
enum ListContext {
    Projects,
    Categories,
    Notes { note_type: String },
    Search,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CreationStage {
    Inputs,
    Projection,
}

struct CreationForm {
    prepared: MutableNoteCreationForm,
    path: String,
    fields: Vec<(String, String)>,
    input_index: usize,
    projection: Editor,
    stage: CreationStage,
}

impl CreationForm {
    fn input_count(&self) -> usize {
        self.fields.len() + 1
    }

    fn input_name(&self) -> &str {
        if self.input_index == 0 {
            "relative .md path"
        } else {
            &self.fields[self.input_index - 1].0
        }
    }

    fn input_value(&self) -> &str {
        if self.input_index == 0 {
            &self.path
        } else {
            &self.fields[self.input_index - 1].1
        }
    }

    fn set_input_value(&mut self, value: String) {
        if self.input_index == 0 {
            self.path = value;
        } else {
            self.fields[self.input_index - 1].1 = value;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LifecyclePane {
    Task,
    Roadmap,
}

struct LifecycleForm {
    prepared: TaskLifecycleForm,
    task: Editor,
    roadmap: Editor,
    pane: LifecyclePane,
}

enum Workflow {
    Creation(Box<CreationForm>),
    Lifecycle(Box<LifecycleForm>),
}

pub(super) struct WatchedSource {
    path: PathBuf,
    expected: String,
}

pub(super) struct RefreshCheck {
    request: ResolveRequest,
    projection: Box<LibraryProjection>,
    changed: bool,
}

pub(super) enum Job {
    Load(ResolveRequest),
    Check(ResolveRequest, Box<LibraryProjection>, Vec<WatchedSource>),
    Open(ResolveRequest, String),
    Search(ResolveRequest, String, Option<LibraryScope>),
    Context(ResolveRequest),
    Validate(ResolveRequest),
    Save(ResolveRequest, String, String, String),
    PrepareCreate(ResolveRequest, String),
    Create(
        ResolveRequest,
        String,
        PathBuf,
        BTreeMap<String, String>,
        String,
    ),
    PrepareTask(ResolveRequest, String),
    UpdateTask(ResolveRequest, String, String, String, String),
    InspectIntegrations(ResolveRequest, AgentClient, PathBuf),
    PrepareIntegration(IntegrationOperation),
    CommitIntegration(IntegrationOperation, String),
}
pub(super) enum Response {
    Loaded(ResolveRequest, Box<LibraryProjection>),
    Checked(Result<RefreshCheck, String>),
    Opened(LibraryDocument),
    Found(LibrarySearchResult),
    Text(String, String),
    Saved(String),
    CreatePrepared(MutableNoteCreationForm),
    Created(MutableNoteCreationResult),
    TaskPrepared(TaskLifecycleForm),
    TaskUpdated(RecordUpdateResult),
    IntegrationPrepared(Box<IntegrationReview>),
    IntegrationCommitted(Result<String, String>),
}
pub(super) type WorkResult = Result<Response, String>;

pub(super) fn worker() -> (Sender<Job>, Receiver<WorkResult>) {
    let (send, jobs) = mpsc::channel();
    let (results, receive) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(job) = jobs.recv() {
            if results.send(execute_job(job)).is_err() {
                break;
            }
        }
    });
    (send, receive)
}

fn execute_job(job: Job) -> WorkResult {
    let err = |error: &dyn std::fmt::Display| error.to_string();
    match job {
        Job::PrepareIntegration(operation) => operation.prepare()
            .map(|review| Response::IntegrationPrepared(Box::new(review))),
        Job::CommitIntegration(operation, plan_id) =>
            Ok(Response::IntegrationCommitted(operation.commit(&plan_id))),
        Job::Load(mut request) => {
            recover_pending_note_edit(&request).map_err(|e| err(&e))?;
            let projection = build_library_projection(&request).map_err(|e| err(&e))?;
            request.root_override = Some(projection.root.clone());
            request.project_override = Some(projection.selected_project.clone());
            Ok(Response::Loaded(request, Box::new(projection)))
        }
        Job::Check(mut request, previous, watched) => {
            let checked = (|| {
                let projection = build_library_projection(&request).map_err(|e| err(&e))?;
                request.root_override = Some(projection.root.clone());
                request.project_override = Some(projection.selected_project.clone());
                let watched_changed = watched.into_iter().try_fold(false, |changed, source| {
                    fs::read_to_string(&source.path)
                        .map(|current| changed || current != source.expected)
                        .map_err(|error| {
                            format!(
                                "could not check {} for external changes: {error}",
                                source.path.display()
                            )
                        })
                })?;
                let changed = *previous != projection || watched_changed;
                Ok(RefreshCheck {
                    request,
                    projection: Box::new(projection),
                    changed,
                })
            })();
            Ok(Response::Checked(checked))
        }
        Job::Open(request, id) => {
            recover_pending_note_edit(&request).map_err(|e| err(&e))?;
            load_library_document(&request, &id).map(Response::Opened).map_err(|e| err(&e))
        }
        Job::Search(request, query, scope) => search_library(&request, &query, scope.as_ref(), 100)
            .map(Response::Found).map_err(|e| err(&e)),
        Job::Context(request) => assemble_context(&request)
            .map(|bundle| Response::Text("PROJECT CONTEXT".into(), render_context_markdown(&bundle)))
            .map_err(|e| err(&e)),
        Job::Validate(request) => validate_project(&request)
            .map(|report| Response::Text("VALIDATION".into(), format!("Validation passed\n\nProject: {}\nRoot: {}\n\nCanonical metadata, configured layout, links and project state were checked.", report.project, report.root.display())))
            .map_err(|e| err(&e)),
        Job::Save(request, id, expected, replacement) => {
            replace_library_document(&request, &id, &expected, &replacement).map_err(|e| err(&e))?;
            Ok(Response::Saved(replacement))
        }
        Job::PrepareCreate(request, note_type) => {
            prepare_mutable_note_creation(&request, &note_type)
                .map(Response::CreatePrepared)
                .map_err(|e| err(&e))
        }
        Job::Create(request, note_type, path, fields, projection) => create_mutable_note(
            &request,
            &note_type,
            &path,
            &fields,
            &projection,
        )
        .map(Response::Created)
        .map_err(|e| err(&e)),
        Job::PrepareTask(request, id) => prepare_task_lifecycle(&request, &id)
            .map(Response::TaskPrepared)
            .map_err(|e| err(&e)),
        Job::UpdateTask(request, id, expected, replacement, roadmap) => {
            update_record(&request, &id, &expected, &replacement, &roadmap)
                .map(Response::TaskUpdated)
                .map_err(|e| err(&e))
        }
        Job::InspectIntegrations(mut request, client, home) => {
            request.project_override = None;
            let instructions = prepare_agent_wiring(&request, client, &home);
            let hook = prepare_session_hook_wiring(&request, client, &home);
            Ok(Response::Text(
                format!("INTEGRATIONS · {}", client.as_str().to_uppercase()),
                render_integration_inspection(client, &home, instructions, hook),
            ))
        }
    }
}

fn render_integration_inspection(
    client: AgentClient,
    home: &std::path::Path,
    instructions: Result<AgentWiringPlan, akasha_core::AgentWiringError>,
    hook: Result<SessionHookWiringPlan, akasha_core::SessionHookWiringError>,
) -> String {
    let mut body = format!(
        "READ-ONLY INSPECTION\nNo files were changed. Use /integration to review one apply/remove operation.\n\nClient: {}\nHome: {}\n\nINSTRUCTION POINTER\n",
        client.as_str(),
        home.display()
    );
    match instructions {
        Ok(plan) => body.push_str(&format!(
            "Status: prepared\nTarget: {}\nAction: {}\nCurrent SHA-256: {}\nResult SHA-256: {}\nPlan ID: {}\n",
            plan.target.display(),
            agent_wiring_action_name(plan.action),
            plan.current_sha256.as_deref().unwrap_or("absent"),
            plan.result_sha256.as_deref().unwrap_or("absent"),
            plan.plan_id
        )),
        Err(error) => body.push_str(&format!("Status: unavailable\nReason: {error}\n")),
    }
    body.push_str("\nSESSIONSTART HOOK\n");
    match hook {
        Ok(plan) => body.push_str(&format!(
            "Status: prepared\nTarget: {}\nAction: {}\nCurrent SHA-256: {}\nResult SHA-256: {}\nPlan ID: {}\n",
            plan.target.display(),
            session_hook_action_name(plan.action),
            plan.current_sha256.as_deref().unwrap_or("absent"),
            plan.result_sha256.as_deref().unwrap_or("absent"),
            plan.plan_id
        )),
        Err(error) => body.push_str(&format!("Status: unavailable\nReason: {error}\n")),
    }
    body
}

fn integration_arguments(argument: &str) -> Result<(AgentClient, PathBuf), String> {
    let (client, explicit_home) = argument
        .split_once(char::is_whitespace)
        .map_or((argument, ""), |(client, home)| (client, home.trim()));
    let client = match client {
        "codex" => AgentClient::Codex,
        "claude" => AgentClient::Claude,
        _ => {
            return Err(
                "Usage: /integrations <codex|claude> [HOME]. Inspection is read-only.".into(),
            );
        }
    };
    let explicit_home = (!explicit_home.is_empty()).then(|| PathBuf::from(explicit_home));
    discover_agent_home(client, explicit_home)
        .map(|home| (client, home))
        .map_err(|error| format!("{error}; pass an explicit HOME path."))
}

#[derive(Clone, Copy)]
pub(super) enum Action {
    Command(&'static str),
    Focus(Focus),
    Open(usize),
    Search,
    Completion(usize),
}

pub(super) struct App {
    pub request: ResolveRequest,
    pub projection: Option<LibraryProjection>,
    pub scope: LibraryScope,
    pub rows: Vec<Row>,
    pub list: ListState,
    pub title: String,
    pub focus: Focus,
    pub prompt: TextArea<'static>,
    pub document: Option<LibraryDocument>,
    pub editor: Option<Editor>,
    pub editing: bool,
    pub body_title: String,
    pub body: String,
    pub scroll: u16,
    pub max_scroll: u16,
    pub messages: VecDeque<String>,
    pub busy: bool,
    pub checking: bool,
    pub external_change_pending: bool,
    pub quit: bool,
    pub no_motion: bool,
    pub ascii: bool,
    pub color: bool,
    pub tick: u64,
    pub sigil_tick: u64,
    pub hits: Vec<(Rect, Action)>,
    history: Vec<String>,
    history_index: usize,
    history_draft: String,
    completion_index: usize,
    completion_dismissed: bool,
    navigation: Vec<Navigation>,
    list_context: ListContext,
    restored_navigation: Option<NavigationState>,
    pinned_project: Option<String>,
    workflow: Option<Workflow>,
    integration_review: Option<IntegrationReview>,
    pending_open: Option<String>,
    jobs: Sender<Job>,
}

impl App {
    pub fn new(
        request: ResolveRequest,
        jobs: Sender<Job>,
        no_motion: bool,
        ascii: bool,
        color: bool,
    ) -> Self {
        let pinned_project = request.project_override.clone();
        let scope = LibraryScope::Project {
            project: request.project_override.clone().unwrap_or_default(),
        };
        Self {
            request,
            projection: None,
            scope,
            rows: vec![],
            list: ListState::default(),
            title: "LIBRARY".into(),
            focus: Focus::Prompt,
            prompt: prompt_area(String::new()),
            document: None,
            editor: None,
            editing: false,
            body_title: "WELCOME TO AKASHA".into(),
            body: HELP.into(),
            scroll: 0,
            max_scroll: 0,
            messages: VecDeque::new(),
            busy: false,
            checking: false,
            external_change_pending: false,
            quit: false,
            no_motion,
            ascii,
            color,
            tick: 0,
            sigil_tick: 0,
            hits: vec![],
            history: vec![],
            history_index: 0,
            history_draft: String::new(),
            completion_index: 0,
            completion_dismissed: false,
            navigation: vec![],
            list_context: ListContext::Categories,
            restored_navigation: None,
            pinned_project,
            workflow: None,
            integration_review: None,
            pending_open: None,
            jobs,
        }
    }

    pub fn load(&mut self) {
        self.submit(Job::Load(self.request.clone()));
    }
    pub fn restore_navigation(&mut self, state: NavigationState) {
        self.restored_navigation = Some(state);
    }
    pub fn navigation_state(&self) -> Option<NavigationState> {
        if self.dirty() || self.busy {
            return None;
        }
        let projection = self.projection.as_ref()?;
        let selected = self
            .list
            .selected()
            .and_then(|index| self.rows.get(index))
            .map(|row| &row.target);
        let location = if let Some(document) = &self.document {
            let book = self
                .books()
                .into_iter()
                .find(|book| book.id == document.id)?;
            NavigationLocation::Note {
                scope: book.scope.clone(),
                note_type: book.note_type.clone(),
                id: book.id.clone(),
            }
        } else {
            match &self.list_context {
                ListContext::Projects => NavigationLocation::Projects {
                    selected_scope: selected.and_then(|target| match target {
                        Target::Scope(scope) => Some(scope.clone()),
                        _ => None,
                    }),
                },
                ListContext::Categories => NavigationLocation::Categories {
                    scope: self.scope.clone(),
                    selected_note_type: selected.and_then(|target| match target {
                        Target::Category(_, note_type) => Some(note_type.clone()),
                        _ => None,
                    }),
                },
                ListContext::Notes { note_type } => NavigationLocation::Notes {
                    scope: self.scope.clone(),
                    note_type: note_type.clone(),
                    selected_note: selected.and_then(|target| match target {
                        Target::Note(id) => Some(id.clone()),
                        _ => None,
                    }),
                },
                ListContext::Search => {
                    let id = selected.and_then(|target| match target {
                        Target::Note(id) => Some(id),
                        _ => None,
                    })?;
                    let book = self.books().into_iter().find(|book| &book.id == id)?;
                    NavigationLocation::Notes {
                        scope: book.scope.clone(),
                        note_type: book.note_type.clone(),
                        selected_note: Some(book.id.clone()),
                    }
                }
            }
        };
        Some(NavigationState::new(&projection.root, location))
    }
    pub fn check_external_changes(&mut self) -> bool {
        if self.busy || self.checking || self.external_change_pending {
            return false;
        }
        let Some(projection) = self.projection.clone() else {
            return false;
        };
        let watched = self.watched_sources();
        match self.jobs.send(Job::Check(
            self.request.clone(),
            Box::new(projection),
            watched,
        )) {
            Ok(()) => {
                self.checking = true;
                true
            }
            Err(_) => {
                self.message("Memory worker unavailable; exit and restart Akasha.");
                false
            }
        }
    }
    fn watched_sources(&self) -> Vec<WatchedSource> {
        match &self.workflow {
            Some(Workflow::Creation(form)) => vec![WatchedSource {
                path: form.prepared.projection.clone(),
                expected: form.prepared.projection_source.clone(),
            }],
            Some(Workflow::Lifecycle(form)) => vec![
                WatchedSource {
                    path: form.prepared.path.clone(),
                    expected: form.prepared.source.clone(),
                },
                WatchedSource {
                    path: form.prepared.roadmap.clone(),
                    expected: form.prepared.roadmap_source.clone(),
                },
            ],
            None => self
                .document
                .as_ref()
                .zip(self.request.root_override.as_ref())
                .map(|(document, root)| {
                    vec![WatchedSource {
                        path: root.join(&document.id),
                        expected: document.source.clone(),
                    }]
                })
                .unwrap_or_default(),
        }
    }
    fn submit(&mut self, job: Job) {
        if self.busy {
            self.message("An operation is running; please wait.");
            return;
        }
        match self.jobs.send(job) {
            Ok(()) => self.busy = true,
            Err(_) => self.message("Memory worker unavailable; exit and restart Akasha."),
        }
    }
    pub fn message(&mut self, message: &str) {
        self.messages.push_back(
            safe_text(message)
                .replace('\n', " ")
                .chars()
                .take(700)
                .collect(),
        );
        while self.messages.len() > 3 {
            self.messages.pop_front();
        }
    }
    pub fn dirty(&self) -> bool {
        self.integration_review.is_some()
            || self.workflow.is_some()
            || self.editor.as_ref().is_some_and(Editor::dirty)
    }
    pub fn in_workflow(&self) -> bool {
        self.workflow.is_some()
    }
    pub fn reviewing_integration(&self) -> bool {
        self.integration_review.is_some()
    }
    pub fn creation_input_active(&self) -> bool {
        matches!(
            self.workflow,
            Some(Workflow::Creation(ref form)) if form.stage == CreationStage::Inputs
        )
    }
    pub fn lifecycle_pane(&self) -> Option<LifecyclePane> {
        match &self.workflow {
            Some(Workflow::Lifecycle(form)) => Some(form.pane),
            _ => None,
        }
    }
    pub fn active_editor(&self) -> Option<&Editor> {
        match &self.workflow {
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Projection => {
                Some(&form.projection)
            }
            Some(Workflow::Lifecycle(form)) if form.pane == LifecyclePane::Task => Some(&form.task),
            Some(Workflow::Lifecycle(form)) => Some(&form.roadmap),
            _ => self.editor.as_ref(),
        }
    }
    pub fn active_editor_mut(&mut self) -> Option<&mut Editor> {
        match &mut self.workflow {
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Projection => {
                Some(&mut form.projection)
            }
            Some(Workflow::Lifecycle(form)) => match form.pane {
                LifecyclePane::Task => Some(&mut form.task),
                LifecyclePane::Roadmap => Some(&mut form.roadmap),
            },
            _ => self.editor.as_mut(),
        }
    }
    pub fn workflow_title(&self) -> Option<String> {
        match &self.workflow {
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Inputs => Some(format!(
                "CREATE {} · INPUT {}/{}",
                form.prepared.note_type,
                form.input_index + 1,
                form.input_count()
            )),
            Some(Workflow::Creation(form)) => Some(format!(
                "CREATE {} · REVIEW {}",
                form.prepared.note_type,
                match form.prepared.class {
                    NoteClass::Record => "ROADMAP",
                    NoteClass::Entity => "INDEX",
                    NoteClass::Event => "PROJECTION",
                }
            )),
            Some(Workflow::Lifecycle(form)) if form.pane == LifecyclePane::Task => {
                Some("TASK LIFECYCLE · TASK SOURCE".into())
            }
            Some(Workflow::Lifecycle(_)) => Some("TASK LIFECYCLE · ROADMAP".into()),
            None => None,
        }
    }
    pub fn workflow_path(&self) -> Option<String> {
        match &self.workflow {
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Projection => {
                Some(form.prepared.projection.display().to_string())
            }
            Some(Workflow::Lifecycle(form)) if form.pane == LifecyclePane::Task => {
                Some(form.prepared.id.clone())
            }
            Some(Workflow::Lifecycle(form)) => Some(form.prepared.roadmap.display().to_string()),
            _ => None,
        }
    }
    fn can_leave(&mut self) -> bool {
        if self.busy {
            self.message("An operation is running; please wait.");
            false
        } else if self.integration_review.is_some() {
            self.message(
                "Integration review pending: /confirm PLAN_ID or /discard before leaving.",
            );
            false
        } else if self.workflow.is_some() {
            self.message("Unfinished form: apply it with Ctrl-S or use discard to cancel it.");
            false
        } else if self.dirty() {
            self.message("Unsaved changes: use save or discard before leaving this note.");
            false
        } else {
            true
        }
    }

    fn creation_body(form: &CreationForm) -> String {
        let mut body = format!(
            "# Create {}\n\nProject: `{}`\nTemplate: `{}`\nMaintained projection: `{}`\n\n## Inputs\n\n",
            form.prepared.note_type,
            form.prepared.project,
            form.prepared.template.display(),
            form.prepared.projection.display()
        );
        body.push_str(&format!(
            "- {} relative .md path: {}\n",
            if form.input_index == 0 { "›" } else { "✓" },
            if form.path.is_empty() {
                "…"
            } else {
                &form.path
            }
        ));
        for (index, (name, value)) in form.fields.iter().enumerate() {
            let position = index + 1;
            body.push_str(&format!(
                "- {} {name}: {}\n",
                if position == form.input_index {
                    "›"
                } else if position < form.input_index {
                    "✓"
                } else {
                    "·"
                },
                if value.is_empty() { "…" } else { value }
            ));
        }
        body.push_str(
            "\nEnter the highlighted value. Shift-Tab returns to the previous input.\nAfter the last value, review the complete maintained projection before Ctrl-S applies one checked transaction.\n",
        );
        body
    }

    fn show_creation_input(&mut self) {
        let Some(Workflow::Creation(form)) = &self.workflow else {
            return;
        };
        let value = form.input_value().to_owned();
        let placeholder = format!(
            "{} ({}/{})",
            form.input_name(),
            form.input_index + 1,
            form.input_count()
        );
        let body = Self::creation_body(form);
        let note_type = form.prepared.note_type.clone();
        self.prompt = prompt_area_with_placeholder(value, &placeholder);
        self.prompt_changed();
        self.body_title = format!("CREATE {note_type}");
        self.body = body;
        self.scroll = 0;
        self.editing = false;
        self.focus = Focus::Prompt;
    }

    fn accept_creation_input(&mut self, input: String) {
        let value = input.trim().to_owned();
        if value.is_empty() {
            self.message("This form value is required.");
            return;
        }
        let mut projection_ready = false;
        if let Some(Workflow::Creation(form)) = &mut self.workflow {
            form.set_input_value(value);
            if form.input_index + 1 < form.input_count() {
                form.input_index += 1;
            } else {
                form.stage = CreationStage::Projection;
                projection_ready = true;
            }
        }
        if projection_ready {
            self.reset_prompt();
            self.editing = true;
            self.focus = Focus::Reader;
            self.message(
                "Review the complete maintained projection. Ctrl-S applies the note and displayed projection atomically; Ctrl-P returns to inputs.",
            );
        } else {
            self.show_creation_input();
        }
    }

    fn previous_workflow_step(&mut self) {
        match &mut self.workflow {
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Projection => {
                form.stage = CreationStage::Inputs;
                form.input_index = form.input_count().saturating_sub(1);
                self.show_creation_input();
            }
            Some(Workflow::Creation(form)) if form.input_index > 0 => {
                form.set_input_value(self.prompt.lines().join(" ").trim().to_owned());
                form.input_index -= 1;
                self.show_creation_input();
            }
            Some(Workflow::Lifecycle(form)) => {
                form.pane = LifecyclePane::Task;
                self.editing = true;
                self.focus = Focus::Reader;
            }
            _ => self.message("Already at the first form input."),
        }
    }

    fn next_workflow_step(&mut self) {
        match &mut self.workflow {
            Some(Workflow::Lifecycle(form)) => {
                form.pane = LifecyclePane::Roadmap;
                self.editing = true;
                self.focus = Focus::Reader;
            }
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Inputs => {
                let input = self.prompt.lines().join(" ");
                self.accept_creation_input(input);
            }
            Some(Workflow::Creation(_)) => {
                self.message("Creation has one projection document; Ctrl-P returns to inputs.");
            }
            None => {}
        }
    }

    fn cancel_workflow(&mut self) {
        let Some(workflow) = self.workflow.take() else {
            return;
        };
        self.editor = None;
        self.editing = false;
        self.reset_prompt();
        match workflow {
            Workflow::Creation(_) => {
                self.close_reader();
                self.focus = Focus::List;
            }
            Workflow::Lifecycle(form) => {
                self.body_title = form.prepared.id.clone();
                self.body = form.prepared.source.clone();
                self.document = Some(LibraryDocument {
                    id: form.prepared.id,
                    source: form.prepared.source,
                });
                self.scroll = 0;
                self.focus = Focus::Reader;
            }
        }
        self.message("Form discarded; no files changed.");
    }
    fn remember(&mut self) {
        self.navigation.push(Navigation {
            title: self.title.clone(),
            rows: self.rows.clone(),
            selection: self.list.selected(),
            scope: self.scope.clone(),
            list_context: self.list_context.clone(),
        });
        if self.navigation.len() > 32 {
            self.navigation.remove(0);
        }
    }
    fn set_rows(&mut self, title: String, rows: Vec<Row>, list_context: ListContext) {
        self.close_reader();
        self.title = title;
        self.rows = rows;
        self.list_context = list_context;
        self.list
            .select(if self.rows.is_empty() { None } else { Some(0) });
        self.focus = Focus::List;
    }
    fn categories(&mut self, scope: LibraryScope) {
        if !self.can_leave() {
            return;
        }
        let Some(projection) = &self.projection else {
            return;
        };
        let categories = match &scope {
            LibraryScope::Global => Some(&projection.global.categories),
            LibraryScope::Project { project } => projection
                .projects
                .iter()
                .find(|s| &s.project == project)
                .map(|s| &s.categories),
        };
        let Some(categories) = categories else {
            self.message("Unknown project. Use projects to browse registered projects.");
            return;
        };
        let rows = categories
            .iter()
            .map(|category| Row {
                label: category.note_type.clone(),
                detail: format!("{} notes", category.books.len()),
                target: Target::Category(scope.clone(), category.note_type.clone()),
            })
            .collect();
        self.remember();
        self.scope = scope.clone();
        if let LibraryScope::Project { project } = &scope {
            self.request.project_override = Some(project.clone());
        }
        self.set_rows(scope_name(&scope), rows, ListContext::Categories);
    }
    fn projects(&mut self) {
        if !self.can_leave() {
            return;
        }
        let Some(projection) = &self.projection else {
            return;
        };
        let mut rows = vec![Row {
            label: "GLOBAL KNOWLEDGE".into(),
            detail: format!("{} notes", projection.dashboard.global_notes),
            target: Target::Scope(LibraryScope::Global),
        }];
        rows.extend(projection.projects.iter().map(|s| Row {
            label: s.project.clone(),
            detail: s.status.clone(),
            target: Target::Scope(LibraryScope::Project {
                project: s.project.clone(),
            }),
        }));
        self.remember();
        self.set_rows("PROJECTS + GLOBAL".into(), rows, ListContext::Projects);
    }
    fn notes(&mut self, scope: LibraryScope, note_type: &str) {
        if !self.can_leave() {
            return;
        }
        let rows: Vec<_> = self
            .books()
            .into_iter()
            .filter(|book| book.scope == scope && book.note_type == note_type)
            .map(|book| Row {
                label: book.label.clone(),
                detail: book
                    .status
                    .clone()
                    .unwrap_or_else(|| book.date.clone().unwrap_or_default()),
                target: Target::Note(book.id.clone()),
            })
            .collect();
        self.remember();
        self.scope = scope;
        self.set_rows(
            format!("{} / {note_type}", scope_name(&self.scope)),
            rows,
            ListContext::Notes {
                note_type: note_type.to_owned(),
            },
        );
    }
    fn scope_exists(&self, scope: &LibraryScope) -> bool {
        match scope {
            LibraryScope::Global => self.projection.is_some(),
            LibraryScope::Project { project } => {
                self.projection.as_ref().is_some_and(|projection| {
                    projection
                        .projects
                        .iter()
                        .any(|shelf| &shelf.project == project)
                })
            }
        }
    }
    fn category_exists(&self, scope: &LibraryScope, note_type: &str) -> bool {
        let Some(projection) = &self.projection else {
            return false;
        };
        let categories = match scope {
            LibraryScope::Global => Some(&projection.global.categories),
            LibraryScope::Project { project } => projection
                .projects
                .iter()
                .find(|shelf| &shelf.project == project)
                .map(|shelf| &shelf.categories),
        };
        categories.is_some_and(|categories| {
            categories
                .iter()
                .any(|category| category.note_type == note_type)
        })
    }
    fn scope_allowed_by_request(&self, scope: &LibraryScope) -> bool {
        self.pinned_project.as_ref().is_none_or(
            |pinned| matches!(scope, LibraryScope::Project { project } if project == pinned),
        )
    }
    fn select_target(&mut self, matches: impl Fn(&Target) -> bool) -> bool {
        let Some(index) = self.rows.iter().position(|row| matches(&row.target)) else {
            return false;
        };
        self.list.select(Some(index));
        true
    }
    fn restore_loaded_navigation(&mut self) -> bool {
        let Some(state) = self.restored_navigation.take() else {
            return false;
        };
        let Some(projection) = &self.projection else {
            return false;
        };
        if !state.belongs_to(&projection.root) {
            return false;
        }
        match state.location {
            NavigationLocation::Projects { selected_scope } => {
                if self.pinned_project.is_some() {
                    return false;
                }
                self.projects();
                if let Some(selected_scope) = selected_scope {
                    self.select_target(
                        |target| matches!(target, Target::Scope(scope) if scope == &selected_scope),
                    );
                }
            }
            NavigationLocation::Categories {
                scope,
                selected_note_type,
            } => {
                if !self.scope_allowed_by_request(&scope) || !self.scope_exists(&scope) {
                    return false;
                }
                self.categories(scope);
                self.navigation.clear();
                if let Some(note_type) = selected_note_type {
                    self.select_target(|target| {
                        matches!(target, Target::Category(_, candidate) if candidate == &note_type)
                    });
                }
            }
            NavigationLocation::Notes {
                scope,
                note_type,
                selected_note,
            } => {
                if !self.scope_allowed_by_request(&scope) || !self.scope_exists(&scope) {
                    return false;
                }
                self.categories(scope.clone());
                self.navigation.clear();
                if !self.category_exists(&scope, &note_type) {
                    self.message(
                        "Previous navigation target no longer exists; restored its nearest valid location.",
                    );
                    return true;
                }
                self.notes(scope, &note_type);
                if let Some(id) = selected_note {
                    self.select_target(
                        |target| matches!(target, Target::Note(candidate) if candidate == &id),
                    );
                }
            }
            NavigationLocation::Note {
                scope,
                note_type,
                id,
            } => {
                if !self.scope_allowed_by_request(&scope) || !self.scope_exists(&scope) {
                    return false;
                }
                self.categories(scope.clone());
                self.navigation.clear();
                if !self.category_exists(&scope, &note_type) {
                    self.message(
                        "Previous navigation target no longer exists; restored its nearest valid location.",
                    );
                    return true;
                }
                self.notes(scope.clone(), &note_type);
                let valid_note = self.books().into_iter().any(|book| {
                    book.id == id && book.scope == scope && book.note_type == note_type
                });
                if !valid_note {
                    self.message(
                        "Previous note no longer exists; restored its validated category.",
                    );
                    return true;
                }
                self.select_target(
                    |target| matches!(target, Target::Note(candidate) if candidate == &id),
                );
                self.open(&id);
            }
        }
        true
    }
    pub fn books(&self) -> Vec<&LibraryBook> {
        self.projection
            .as_ref()
            .map(|p| {
                p.global
                    .categories
                    .iter()
                    .chain(p.projects.iter().flat_map(|s| &s.categories))
                    .flat_map(|c| &c.books)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn book(&self) -> Option<&LibraryBook> {
        let id = &self.document.as_ref()?.id;
        self.books().into_iter().find(|book| &book.id == id)
    }
    fn activate(&mut self, index: usize) {
        let Some(row) = self.rows.get(index).cloned() else {
            self.message("No item at that number.");
            return;
        };
        match row.target {
            Target::Scope(scope) => self.categories(scope),
            Target::Category(scope, note_type) => self.notes(scope, &note_type),
            Target::Note(id) => self.open(&id),
        }
    }
    fn open(&mut self, id: &str) {
        if !self.can_leave() {
            return;
        }
        self.submit(Job::Open(self.request.clone(), id.to_owned()));
    }
    pub fn editable(&self) -> bool {
        self.book().is_some_and(|book| book.class != NoteClass::Event && matches!(&book.scope, LibraryScope::Project { project } if Some(project) == self.request.project_override.as_ref()))
    }

    fn edit(&mut self) {
        if self.busy {
            return;
        }
        if self.workflow.is_some() {
            if self.active_editor().is_some() {
                self.editing = true;
                self.focus = Focus::Reader;
            } else {
                self.focus = Focus::Prompt;
            }
            return;
        }
        if !self.editable() {
            self.message("Open a record or entity in its project to edit. Events and global notes are read-only.");
            return;
        }
        if self.editor.is_none() {
            match Editor::new(&self.document.as_ref().expect("book has document").source) {
                Ok(editor) => self.editor = Some(editor),
                Err(error) => {
                    self.message(&error);
                    return;
                }
            }
        }
        self.editing = true;
        self.focus = Focus::Reader;
    }
    fn save(&mut self) {
        if self.integration_review.is_some() {
            self.message(
                "Review the integration patch, then use /confirm with its complete plan ID.",
            );
            return;
        }
        if self.busy {
            self.message("An operation is running; please wait.");
            return;
        }
        let workflow_job = match &self.workflow {
            Some(Workflow::Creation(form)) if form.stage == CreationStage::Projection => {
                Some(Job::Create(
                    self.request.clone(),
                    form.prepared.note_type.clone(),
                    PathBuf::from(&form.path),
                    form.fields.iter().cloned().collect(),
                    form.projection.source(),
                ))
            }
            Some(Workflow::Creation(_)) => {
                self.message("Complete every form input before applying creation.");
                return;
            }
            Some(Workflow::Lifecycle(form)) => Some(Job::UpdateTask(
                self.request.clone(),
                form.prepared.id.clone(),
                form.prepared.source.clone(),
                form.task.source(),
                form.roadmap.source(),
            )),
            None => None,
        };
        if let Some(job) = workflow_job {
            self.submit(job);
            return;
        }
        let (Some(document), Some(editor)) = (&self.document, &self.editor) else {
            self.message("No source editor is open.");
            return;
        };
        if !editor.dirty() {
            self.message("No unsaved changes.");
            return;
        }
        self.submit(Job::Save(
            self.request.clone(),
            document.id.clone(),
            editor.original.clone(),
            editor.source(),
        ));
    }
    pub fn receive(&mut self, response: WorkResult) -> bool {
        let redraw = !matches!(
            &response,
            Ok(Response::Checked(Ok(RefreshCheck { changed: false, .. })))
        );
        if matches!(&response, Ok(Response::Checked(_))) {
            self.checking = false;
        } else {
            self.busy = false;
        }
        match response {
            Ok(Response::IntegrationPrepared(review)) => {
                self.body_title = "INTEGRATION REVIEW".into();
                self.body = review.body.clone();
                self.integration_review = Some(*review);
                self.document = None;
                self.editor = None;
                self.editing = false;
                self.scroll = 0;
                self.focus = Focus::Reader;
            }
            Ok(Response::IntegrationCommitted(result)) => {
                self.integration_review = None;
                self.body_title = "INTEGRATION RESULT".into();
                self.body = match result {
                    Ok(result) => result,
                    Err(error) => format!(
                        "Operation failed: {error}\n\nPrepare a fresh /integration review before retrying."
                    ),
                };
                self.scroll = 0;
                self.focus = Focus::Reader;
            }
            Err(error) => self.message(&format!("Operation failed: {error}")),
            Ok(Response::Loaded(request, projection)) => {
                self.external_change_pending = false;
                self.request = request;
                self.scope = LibraryScope::Project {
                    project: projection.selected_project.clone(),
                };
                self.projection = Some(*projection);
                self.document = None;
                self.editor = None;
                self.editing = false;
                self.body_title = "WELCOME TO AKASHA".into();
                self.body = HELP.into();
                self.scroll = 0;
                self.categories(self.scope.clone());
                self.navigation.clear();
                self.focus = Focus::Prompt;
                let restored = self.restore_loaded_navigation();
                self.message(if restored {
                    "Library loaded; previous navigation restored."
                } else {
                    "Library loaded."
                });
                if let Some(id) = self.pending_open.take() {
                    self.submit(Job::Open(self.request.clone(), id));
                }
            }
            Ok(Response::Checked(Err(error))) => {
                self.external_change_pending = true;
                self.message(&format!(
                    "External library change could not be refreshed: {error}. Drafts were retained; fix the source, then press F5."
                ));
            }
            Ok(Response::Checked(Ok(checked))) if !checked.changed => {}
            Ok(Response::Checked(Ok(_))) if self.dirty() => {
                self.external_change_pending = true;
                self.message(
                    "External changes detected. Drafts were retained; save may conflict. Save or discard, then press F5.",
                );
            }
            Ok(Response::Checked(Ok(checked))) => {
                self.request = checked.request;
                self.scope = LibraryScope::Project {
                    project: checked.projection.selected_project.clone(),
                };
                self.projection = Some(*checked.projection);
                self.document = None;
                self.editor = None;
                self.editing = false;
                self.body_title = "WELCOME TO AKASHA".into();
                self.body = HELP.into();
                self.scroll = 0;
                self.categories(self.scope.clone());
                self.navigation.clear();
                self.focus = Focus::Prompt;
                self.message("Library refreshed after external changes.");
            }
            Ok(Response::Opened(document)) => {
                self.body_title = document.id.clone();
                self.body = document.source.clone();
                self.document = Some(document);
                self.editor = None;
                self.editing = false;
                self.scroll = 0;
                self.focus = Focus::Reader;
            }
            Ok(Response::Found(result)) => {
                self.remember();
                self.set_rows(
                    format!("SEARCH · {}", result.query),
                    result
                        .hits
                        .into_iter()
                        .map(|hit| Row {
                            label: hit.label,
                            detail: format!(
                                "{}{} · {}",
                                hit.id,
                                hit.line.map(|line| format!(":{line}")).unwrap_or_default(),
                                hit.snippet
                            ),
                            target: Target::Note(hit.id),
                        })
                        .collect(),
                    ListContext::Search,
                );
                self.message(&format!(
                    "{} matches{}",
                    result.total_matches,
                    if result.truncated {
                        "; showing first 100. Narrow the query."
                    } else {
                        ""
                    }
                ));
            }
            Ok(Response::Text(title, text)) => {
                self.body_title = title;
                self.body = text;
                self.scroll = 0;
                self.document = None;
                self.editor = None;
                self.editing = false;
                self.focus = Focus::Reader;
            }
            Ok(Response::Saved(source)) => {
                self.body = source.clone();
                if let Some(document) = &mut self.document {
                    document.source = source.clone();
                }
                if let Some(editor) = &mut self.editor {
                    editor.original = source;
                }
                self.message(
                    "Saved through the core. Refresh to update library metrics and lists.",
                );
            }
            Ok(Response::CreatePrepared(prepared)) => {
                let projection = match Editor::new(&prepared.projection_source) {
                    Ok(editor) => editor,
                    Err(error) => {
                        self.message(&format!("Operation failed: {error}"));
                        return true;
                    }
                };
                let fields = prepared
                    .fields
                    .iter()
                    .cloned()
                    .map(|name| (name, String::new()))
                    .collect();
                self.document = None;
                self.editor = None;
                self.workflow = Some(Workflow::Creation(Box::new(CreationForm {
                    prepared,
                    path: String::new(),
                    fields,
                    input_index: 0,
                    projection,
                    stage: CreationStage::Inputs,
                })));
                self.show_creation_input();
                self.message(
                    "Creation form loaded from the configured template; values are not written until Ctrl-S.",
                );
            }
            Ok(Response::Created(result)) => {
                self.workflow = None;
                self.editor = None;
                self.editing = false;
                self.pending_open = Some(result.id.clone());
                self.message(&format!(
                    "Created {} and {} the maintained projection through the checked core transaction.",
                    result.id,
                    if result.projection_changed {
                        "updated"
                    } else {
                        "retained"
                    }
                ));
                self.load();
            }
            Ok(Response::TaskPrepared(prepared)) => {
                let task = match Editor::new(&prepared.source) {
                    Ok(editor) => editor,
                    Err(error) => {
                        self.message(&format!("Operation failed: {error}"));
                        return true;
                    }
                };
                let roadmap = match Editor::new(&prepared.roadmap_source) {
                    Ok(editor) => editor,
                    Err(error) => {
                        self.message(&format!("Operation failed: {error}"));
                        return true;
                    }
                };
                self.workflow = Some(Workflow::Lifecycle(Box::new(LifecycleForm {
                    prepared,
                    task,
                    roadmap,
                    pane: LifecyclePane::Task,
                })));
                self.editor = None;
                self.editing = true;
                self.focus = Focus::Reader;
                self.message(
                    "Task lifecycle form loaded. Edit exact task and roadmap sources; Ctrl-N/Ctrl-P switches documents and Ctrl-S applies both.",
                );
            }
            Ok(Response::TaskUpdated(result)) => {
                self.workflow = None;
                self.editor = None;
                self.editing = false;
                self.pending_open = Some(result.id.clone());
                self.message(&format!(
                    "Task lifecycle applied: task {}, roadmap {}.",
                    if result.changed {
                        "updated"
                    } else {
                        "unchanged"
                    },
                    if result.roadmap_changed {
                        "updated"
                    } else {
                        "unchanged"
                    }
                ));
                self.load();
            }
        }
        redraw
    }

    pub fn command(&mut self, input: &str) {
        let trimmed = input.trim().trim_start_matches('/');
        let (command, argument) = trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        let argument = argument.trim();
        match command {
            "" => {}
            "confirm" => {
                if self.busy {
                    self.message("An operation is running; please wait.");
                } else if let Some(review) = &self.integration_review {
                    if argument == review.plan_id {
                        self.submit(Job::CommitIntegration(
                            review.operation.clone(),
                            review.plan_id.clone(),
                        ));
                    } else {
                        self.message("Confirmation must match the complete displayed plan ID.");
                    }
                } else {
                    self.message("No integration plan is awaiting confirmation.");
                }
            }
            "save" => self.save(),
            "quit" | "exit" | "q" => {
                if self.can_leave() {
                    self.quit = true;
                }
            }
            "motion" => {
                self.no_motion = !self.no_motion;
                if self.no_motion {
                    self.tick = 0;
                }
            }
            "edit" => self.edit(),
            "read" => {
                self.editing = false;
                self.focus = Focus::Reader;
            }
            "discard" => {
                if self.busy {
                    self.message("Wait for the operation to finish before discarding.");
                    return;
                }
                if self.integration_review.take().is_some() {
                    self.close_reader();
                    self.message("Integration review discarded; no files changed.");
                    return;
                }
                if self.workflow.is_some() {
                    self.cancel_workflow();
                    return;
                }
                self.editor = None;
                self.editing = false;
                self.message("Editor changes discarded; loaded source retained. Refresh to load external changes.");
            }
            "previous" => self.previous_workflow_step(),
            "next" => self.next_workflow_step(),
            _ if !self.can_leave() => {}
            "home" => {
                self.document = None;
                self.editor = None;
                self.editing = false;
                self.body_title = "WELCOME TO AKASHA".into();
                self.body = HELP.into();
                self.scroll = 0;
                self.focus = Focus::Prompt;
            }
            "projects" => self.projects(),
            "project" => self.categories(LibraryScope::Project {
                project: argument.to_owned(),
            }),
            "global" => self.categories(LibraryScope::Global),
            "ls" => self.categories(self.scope.clone()),
            "type" => self.notes(self.scope.clone(), argument),
            "open" if argument.is_empty() => {
                self.prompt = prompt_area("/open ".into());
                self.prompt_changed();
                self.focus = Focus::Prompt;
                self.message(
                    "Choose a note with arrows and Enter, or type a list item number: /open 1.",
                );
            }
            "open" => {
                if let Ok(index) = argument.parse::<usize>() {
                    if let Some(index) = index.checked_sub(1) {
                        self.activate(index);
                    } else {
                        self.message("Item numbers start at 1.");
                    }
                } else {
                    self.open(argument);
                }
            }
            "back" => {
                if self.document.is_some() || self.body_title != "WELCOME TO AKASHA" {
                    self.close_reader();
                    self.focus = Focus::List;
                    return;
                }
                if let Some(previous) = self.navigation.pop() {
                    self.title = previous.title;
                    self.rows = previous.rows;
                    self.scope = previous.scope;
                    self.list_context = previous.list_context;
                    if let LibraryScope::Project { project } = &self.scope {
                        self.request.project_override = Some(project.clone());
                    }
                    self.list.select(previous.selection);
                    self.focus = Focus::List;
                } else {
                    self.projects();
                }
            }
            "search" | "search-all" if argument.is_empty() => {
                self.prompt = prompt_area(format!("/{command} "));
                self.prompt_changed();
                self.focus = Focus::Prompt;
                self.message("Type words to find in note titles or contents, then Enter. Example: /search memory");
            }
            "search" | "search-all" => self.submit(Job::Search(
                self.request.clone(),
                argument.to_owned(),
                if command == "search-all" {
                    None
                } else {
                    Some(self.scope.clone())
                },
            )),
            "create" if argument.is_empty() => {
                self.prompt = prompt_area("/create ".into());
                self.prompt_changed();
                self.focus = Focus::Prompt;
                self.message("Choose a configured record or entity type with arrows and Enter.");
            }
            "create" if argument.split_whitespace().count() != 1 => {
                self.message("Usage: /create TYPE. The guided form collects path and fields.");
            }
            "create" => {
                if !matches!(self.scope, LibraryScope::Project { .. }) {
                    self.message("Select a project before creating a project-owned note.");
                    return;
                }
                self.submit(Job::PrepareCreate(
                    self.request.clone(),
                    argument.to_owned(),
                ));
            }
            "lifecycle" if !argument.is_empty() => {
                self.message("Usage: /lifecycle while a task note is open.");
            }
            "lifecycle" => {
                let Some(document) = &self.document else {
                    self.message("Open a configured task note before starting its lifecycle form.");
                    return;
                };
                self.submit(Job::PrepareTask(self.request.clone(), document.id.clone()));
            }
            "context" => self.submit(Job::Context(self.request.clone())),
            "validate" => self.submit(Job::Validate(self.request.clone())),
            "integrations" => match integration_arguments(argument) {
                Ok((client, home)) => {
                    self.submit(Job::InspectIntegrations(self.request.clone(), client, home))
                }
                Err(error) => self.message(&error),
            },
            "integration" => {
                let (operation, rest) = argument
                    .split_once(char::is_whitespace)
                    .unwrap_or((argument, ""));
                let (target, client_home) = rest
                    .trim_start()
                    .split_once(char::is_whitespace)
                    .unwrap_or((rest.trim_start(), ""));
                let client_home = client_home.trim();
                if !matches!(operation, "apply" | "remove")
                    || !matches!(target, "instructions" | "hook")
                {
                    self.message("Usage: /integration <apply|remove> <instructions|hook> <codex|claude> [HOME]");
                    return;
                }
                match integration_arguments(client_home) {
                    Ok((client, home)) => {
                        self.submit(Job::PrepareIntegration(IntegrationOperation {
                            request: self.request.clone(),
                            client,
                            home,
                            remove: operation == "remove",
                            instructions: target == "instructions",
                        }))
                    }
                    Err(error) => self.message(&error),
                }
            }
            "refresh" => self.load(),
            "help" => {
                self.body_title = "HELP".into();
                self.body = HELP.into();
                self.document = None;
                self.editor = None;
                self.editing = false;
                self.scroll = 0;
                self.focus = Focus::Reader;
            }
            _ => self.message("Unknown command. Type / for available operations."),
        }
    }

    fn close_reader(&mut self) {
        self.document = None;
        self.editor = None;
        self.editing = false;
        self.body_title = "WELCOME TO AKASHA".into();
        self.body = HELP.into();
        self.scroll = 0;
    }

    pub fn action(&mut self, action: Action) {
        match action {
            Action::Command(command) => self.command(command),
            Action::Focus(focus) => self.focus = focus,
            Action::Open(index) => {
                if self.can_leave() {
                    self.list.select(Some(index));
                    self.activate(index);
                }
            }
            Action::Search => {
                self.focus = Focus::Prompt;
                if self.prompt.lines()[0].trim().is_empty() {
                    self.prompt = prompt_area("/search ".into());
                    self.prompt_changed();
                } else {
                    self.message("Command draft kept. Ctrl-C clears it; F3 starts a search.");
                }
            }
            Action::Completion(index) => {
                self.completion_index = index;
                self.complete(true);
            }
        }
    }

    pub fn mouse(&mut self, mouse: MouseEvent) {
        let point = (mouse.column, mouse.row).into();
        let action = self
            .hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(point))
            .map(|(_, a)| *a);
        match (mouse.kind, action) {
            (MouseEventKind::Down(MouseButton::Left), Some(action)) => self.action(action),
            (MouseEventKind::ScrollUp | MouseEventKind::ScrollDown, Some(action)) => {
                let down = mouse.kind == MouseEventKind::ScrollDown;
                match action {
                    Action::Open(_) | Action::Focus(Focus::List) => {
                        self.focus = Focus::List;
                        self.move_selection(if down { 3 } else { -3 });
                    }
                    Action::Focus(Focus::Reader) if !self.editing => {
                        self.focus = Focus::Reader;
                        self.scroll = if down {
                            self.scroll.saturating_add(3).min(self.max_scroll)
                        } else {
                            self.scroll.saturating_sub(3)
                        };
                    }
                    Action::Completion(_) => {
                        self.prompt_key(KeyEvent::new(
                            if down { KeyCode::Down } else { KeyCode::Up },
                            KeyModifiers::NONE,
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn completions(&self) -> Vec<Suggestion> {
        if self.focus != Focus::Prompt || self.completion_dismissed || self.creation_input_active()
        {
            return vec![];
        }
        let Some(prefix) = self.prompt.lines()[0].strip_prefix('/') else {
            return vec![];
        };
        if let Some(query) = prefix.strip_prefix("open ") {
            let query = query.trim().to_lowercase();
            if query.parse::<usize>().is_ok() {
                return vec![];
            }
            return self
                .books()
                .into_iter()
                .filter(|book| {
                    book.id.to_lowercase().contains(&query)
                        || book.label.to_lowercase().contains(&query)
                })
                .take(100)
                .map(|book| Suggestion {
                    command: format!("open {}", book.id),
                    argument: String::new(),
                    description: book.label.clone(),
                })
                .collect();
        }
        if let Some(query) = prefix.strip_prefix("create ") {
            let query = query.trim().to_lowercase();
            let Some(project) = self.request.project_override.as_ref() else {
                return vec![];
            };
            return self
                .projection
                .as_ref()
                .and_then(|projection| {
                    projection
                        .projects
                        .iter()
                        .find(|shelf| &shelf.project == project)
                })
                .into_iter()
                .flat_map(|shelf| &shelf.categories)
                .filter(|category| {
                    category.class != NoteClass::Event
                        && category.note_type.to_lowercase().contains(&query)
                })
                .map(|category| Suggestion {
                    command: format!("create {}", category.note_type),
                    argument: String::new(),
                    description: format!(
                        "configured {} template",
                        match category.class {
                            NoteClass::Record => "record",
                            NoteClass::Entity => "entity",
                            NoteClass::Event => "event",
                        }
                    ),
                })
                .collect();
        }
        if prefix.chars().any(char::is_whitespace) {
            return vec![];
        }
        let mut matches: Vec<_> = COMMANDS
            .iter()
            .filter(|completion| completion.command.starts_with(prefix))
            .map(|c| Suggestion {
                command: c.command.into(),
                argument: c.argument.into(),
                description: c.description.into(),
            })
            .collect();
        matches.sort_by_key(|completion| completion.command != prefix);
        matches
    }

    pub fn completion_selection(&self) -> usize {
        self.completion_index
            .min(self.completions().len().saturating_sub(1))
    }

    fn prompt_changed(&mut self) {
        self.completion_index = 0;
        self.completion_dismissed = false;
    }

    fn reset_prompt(&mut self) {
        self.prompt = prompt_area(String::new());
        self.history_index = self.history.len();
        self.history_draft.clear();
        self.prompt_changed();
    }

    fn complete(&mut self, execute: bool) {
        let Some(completion) = self.completions().get(self.completion_selection()).cloned() else {
            return;
        };
        self.prompt = prompt_area(format!(
            "/{}{}",
            completion.command,
            if completion.argument.is_empty() {
                ""
            } else {
                " "
            }
        ));
        self.completion_index = 0;
        self.completion_dismissed =
            !completion.command.starts_with("open") && !completion.command.starts_with("create");
        if completion.command == "open" && self.completions().is_empty() {
            self.message(
                "No notes loaded. /open also accepts a list item number, for example /open 1.",
            );
        }
        if execute && completion.argument.is_empty() {
            self.run_prompt();
        }
    }

    fn run_prompt(&mut self) {
        let input = self.prompt.lines().join(" ");
        if self.creation_input_active() {
            self.accept_creation_input(input);
            return;
        }
        self.reset_prompt();
        if !input.trim().is_empty() {
            if self.history.last() != Some(&input) {
                self.history.push(input.clone());
                if self.history.len() > 100 {
                    self.history.remove(0);
                }
            }
            self.history_index = self.history.len();
            self.message(&format!("> {input}"));
            self.command(&input);
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let palette_len = self.completions().len();
        match key.code {
            KeyCode::Enter if palette_len > 0 => self.complete(true),
            KeyCode::Enter if self.prompt.lines()[0].trim().is_empty() => {
                self.focus = Focus::List;
                if let Some(index) = self.list.selected() {
                    self.activate(index);
                }
            }
            KeyCode::Enter => self.run_prompt(),
            KeyCode::Up | KeyCode::Down if palette_len > 0 => {
                let selected = self.completion_selection();
                self.completion_index = if key.code == KeyCode::Up {
                    selected.checked_sub(1).unwrap_or(palette_len - 1)
                } else {
                    (selected + 1) % palette_len
                };
            }
            KeyCode::Up | KeyCode::Down => {
                if self.history_index == self.history.len() {
                    self.history_draft = self.prompt.lines().join(" ");
                }
                if key.code == KeyCode::Up {
                    self.history_index = self.history_index.saturating_sub(1);
                } else {
                    self.history_index = (self.history_index + 1).min(self.history.len());
                }
                let line = self
                    .history
                    .get(self.history_index)
                    .unwrap_or(&self.history_draft)
                    .clone();
                self.prompt = prompt_area(line);
                // History uses Up/Down until the user edits the recalled command.
                self.completion_dismissed = true;
            }
            _ => {
                let before = self.prompt.lines()[0].clone();
                match key.code {
                    KeyCode::Char('a') if ctrl => self.prompt.move_cursor(CursorMove::Head),
                    KeyCode::Char('e') if ctrl => self.prompt.move_cursor(CursorMove::End),
                    KeyCode::Char('b') if ctrl => self.prompt.move_cursor(CursorMove::Back),
                    KeyCode::Char('f') if ctrl => self.prompt.move_cursor(CursorMove::Forward),
                    KeyCode::Char('h') if ctrl => {
                        self.prompt.delete_char();
                    }
                    KeyCode::Char('u') if ctrl => {
                        self.prompt.delete_line_by_head();
                    }
                    KeyCode::Char('k') if ctrl => {
                        self.prompt.delete_line_by_end();
                    }
                    KeyCode::Char('w') if ctrl => {
                        let before_cursor: Vec<_> = self.prompt.lines()[0]
                            .chars()
                            .take(self.prompt.cursor().1)
                            .collect();
                        let mut chars = before_cursor.iter().rev().peekable();
                        let mut count = 0;
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                            count += 1;
                        }
                        while chars.peek().is_some_and(|c| !c.is_whitespace()) {
                            chars.next();
                            count += 1;
                        }
                        for _ in 0..count {
                            self.prompt.delete_char();
                        }
                    }
                    // The prompt is one bounded line. Control input cannot inject a newline
                    // or invoke textarea's multiline/editor shortcuts.
                    _ if !ctrl
                        && (self.prompt.lines()[0].chars().count() < 4096
                            || !matches!(key.code, KeyCode::Char(_))) =>
                    {
                        self.prompt.input_without_shortcuts(key);
                    }
                    _ => {}
                }
                if self.prompt.lines()[0] != before {
                    self.prompt_changed();
                }
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('n') if ctrl && self.workflow.is_some() => {
                self.next_workflow_step();
                return;
            }
            KeyCode::Char('p') if ctrl && self.workflow.is_some() => {
                self.previous_workflow_step();
                return;
            }
            KeyCode::Char('c') if ctrl && !self.prompt.lines()[0].is_empty() => {
                self.reset_prompt();
                self.focus = Focus::Prompt;
                return;
            }
            KeyCode::Char('q' | 'c') if ctrl => {
                self.command("quit");
                return;
            }
            KeyCode::Char('s') if ctrl => {
                self.save();
                return;
            }
            KeyCode::F(1) => {
                self.command("help");
                return;
            }
            KeyCode::F(2) => {
                self.edit();
                return;
            }
            KeyCode::F(3) => {
                self.action(Action::Search);
                return;
            }
            KeyCode::F(4) => {
                self.command("projects");
                return;
            }
            KeyCode::F(6) => {
                self.command("global");
                return;
            }
            KeyCode::F(7) => {
                self.command("back");
                return;
            }
            KeyCode::F(5) => {
                self.command("refresh");
                return;
            }
            KeyCode::Esc => {
                if !self.completions().is_empty() {
                    self.completion_dismissed = true;
                } else {
                    self.command("back");
                }
                return;
            }
            KeyCode::Backspace
                if self.focus != Focus::Prompt
                    && !(self.focus == Focus::Reader && self.editing) =>
            {
                self.focus = Focus::Prompt;
                return;
            }
            KeyCode::Tab if self.creation_input_active() => {
                self.message(
                    "Press Enter for the next value; Shift-Tab returns to the previous input.",
                );
                return;
            }
            KeyCode::BackTab if self.creation_input_active() => {
                self.previous_workflow_step();
                return;
            }
            KeyCode::Tab if self.focus == Focus::Prompt && !self.prompt.lines()[0].is_empty() => {
                self.completion_dismissed = false;
                self.complete(false);
                return;
            }
            KeyCode::Tab if !(self.focus == Focus::Reader && self.editing) => {
                self.focus = match self.focus {
                    Focus::Prompt => Focus::List,
                    Focus::List => Focus::Reader,
                    Focus::Reader => Focus::Prompt,
                };
                return;
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Prompt => Focus::Reader,
                    Focus::Reader => Focus::List,
                    Focus::List => Focus::Prompt,
                };
                return;
            }
            _ => {}
        }
        match self.focus {
            Focus::Prompt => self.prompt_key(key),
            Focus::List => match key.code {
                KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
                KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
                KeyCode::PageDown => self.move_selection(10),
                KeyCode::PageUp => self.move_selection(-10),
                KeyCode::Home => self.list.select((!self.rows.is_empty()).then_some(0)),
                KeyCode::End => self.list.select(self.rows.len().checked_sub(1)),
                KeyCode::Enter | KeyCode::Right => {
                    if let Some(index) = self.list.selected() {
                        self.activate(index);
                    }
                }
                KeyCode::Left => self.command("back"),
                _ => {}
            },
            Focus::Reader if self.editing => {
                if !self.busy
                    && let Some(editor) = self.active_editor_mut()
                {
                    match key.code {
                        KeyCode::End if ctrl => {
                            editor
                                .area
                                .move_cursor(ratatui_textarea::CursorMove::Bottom);
                            editor.area.move_cursor(ratatui_textarea::CursorMove::End);
                        }
                        KeyCode::Home if ctrl => {
                            editor.area.move_cursor(ratatui_textarea::CursorMove::Top);
                            editor.area.move_cursor(ratatui_textarea::CursorMove::Head);
                        }
                        KeyCode::Char('z') if ctrl => {
                            editor.area.undo();
                        }
                        KeyCode::Char('y') if ctrl => {
                            editor.area.redo();
                        }
                        KeyCode::Char('v') if ctrl => {
                            editor.area.paste();
                        }
                        _ => {
                            editor.area.input(key);
                        }
                    }
                }
            }
            Focus::Reader => {
                let max = self.max_scroll;
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.scroll = self.scroll.saturating_add(1).min(max)
                    }
                    KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        self.scroll = self.scroll.saturating_add(12).min(max)
                    }
                    KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(12),
                    KeyCode::Home => self.scroll = 0,
                    KeyCode::End => self.scroll = max,
                    KeyCode::Char('e') => self.edit(),
                    KeyCode::Left => self.command("back"),
                    _ => {}
                }
            }
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.list.select(None);
            return;
        }
        let index = self
            .list
            .selected()
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(self.rows.len() - 1);
        self.list.select(Some(index));
    }
    pub fn paste(&mut self, text: &str) {
        if self.focus == Focus::Reader && self.editing && !self.busy {
            if text.len() > 1_048_576 {
                self.message("Paste exceeds the 1 MiB input limit.");
                return;
            }
            if let Some(editor) = self.active_editor_mut() {
                let text = text.replace("\r\n", "\n");
                if text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
                {
                    self.message("Paste contains unsupported control characters.");
                    return;
                }
                editor.area.insert_str(text);
            }
        } else if self.focus == Focus::Prompt {
            let remaining = 4096usize.saturating_sub(self.prompt.lines()[0].chars().count());
            let text: String = text
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(remaining)
                .collect();
            self.prompt.insert_str(text);
            self.prompt_changed();
        }
    }
}

pub(super) fn scope_name(scope: &LibraryScope) -> String {
    match scope {
        LibraryScope::Global => "GLOBAL".into(),
        LibraryScope::Project { project } => project.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use akasha_core::ResolutionEnvironment;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    const ID: &str = "Projects/example/entities/core.md";
    const GLOBAL_ID: &str = "Global/entities/rust-pattern.md";
    const TASK_ID: &str = "Projects/example/records/tasks/active.md";
    const CREATED_TASK_ID: &str = "Projects/example/records/tasks/tui-created.md";
    const TASK_TEMPLATE: &str = "---\nschema_version: 1\nproject: {{project}}\ntype: {{type}}\nstatus: {{status}}\ncreated: {{created}}\nupdated: {{updated}}\n---\n\n# {{title}}\n\n{{body}}\n";
    struct Fixture {
        temp: PathBuf,
        app: App,
        jobs: Receiver<Job>,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = std::env::temp_dir().join(format!(
                "akasha-tui-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            let root = temp.join("root");
            fs::create_dir_all(temp.join("repository")).unwrap();
            copy_tree(
                &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/resolution/valid-root"),
                &root,
            );
            fs::write(
                root.join("Projects/example/templates/task.md"),
                TASK_TEMPLATE,
            )
            .unwrap();
            let request = ResolveRequest {
                root_override: Some(root),
                project_override: Some("example".into()),
                cwd: temp.clone(),
                environment: ResolutionEnvironment::default(),
            };
            let (sender, jobs) = mpsc::channel();
            let app = App::new(request, sender, true, false, false);
            let mut fixture = Self { temp, app, jobs };
            fixture.app.load();
            fixture.finish();
            assert!(
                fixture.app.projection.is_some(),
                "{:?}",
                fixture.app.messages
            );
            fixture
        }
        fn finish(&mut self) -> bool {
            self.app.receive(execute_job(self.jobs.try_recv().unwrap()))
        }
        fn check_external_changes(&mut self) -> bool {
            assert!(self.app.check_external_changes());
            self.finish()
        }
        fn open_editor(&mut self) {
            self.app.open(ID);
            self.finish();
            self.app.edit();
            assert!(self.app.editing);
            let editor = self.app.editor.as_mut().unwrap();
            editor
                .area
                .move_cursor(ratatui_textarea::CursorMove::Bottom);
            editor.area.move_cursor(ratatui_textarea::CursorMove::End);
        }
        fn path(&self) -> PathBuf {
            self.temp.join("root").join(ID)
        }
        fn root(&self) -> PathBuf {
            self.temp.join("root")
        }
        fn restart(&self, state: NavigationState) -> App {
            let (sender, jobs) = mpsc::channel();
            let mut app = App::new(self.app.request.clone(), sender, true, false, false);
            app.restore_navigation(state);
            app.load();
            while app.busy {
                app.receive(execute_job(jobs.try_recv().unwrap()));
            }
            app
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.temp);
        }
    }
    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let to = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &to);
            } else {
                fs::copy(entry.path(), to).unwrap();
            }
        }
    }

    #[test]
    fn dirty_guards_undo_and_checked_save_cover_real_core_bytes() {
        let mut fixture = Fixture::new();
        fixture.open_editor();
        let before = fs::read_to_string(fixture.path()).unwrap();
        fixture.app.paste("\n\nПривет 世界\n");
        assert!(fixture.app.dirty());
        for command in [
            "quit",
            "global",
            "refresh",
            "help",
            "context",
            "search core",
        ] {
            fixture.app.command(command);
            assert!(fixture.app.dirty());
            assert!(!fixture.app.quit);
            assert!(!fixture.app.busy);
        }
        fixture
            .app
            .key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert!(!fixture.app.dirty());
        fixture
            .app
            .key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert!(fixture.app.dirty());
        fixture.app.command("save");
        fixture.app.command("quit");
        assert!(!fixture.app.quit);
        fixture.finish();
        assert!(!fixture.app.dirty());
        assert_eq!(
            fs::read_to_string(fixture.path()).unwrap(),
            format!("{before}\n\nПривет 世界\n")
        );
        validate_project(&fixture.app.request).unwrap();
        fixture.app.command("quit");
        assert!(fixture.app.quit);
    }

    #[test]
    fn navigation_state_restores_an_exact_note_from_fresh_core_bytes() {
        let mut fixture = Fixture::new();
        fixture.app.open(ID);
        fixture.finish();
        let state = fixture.app.navigation_state().expect("clean navigation");
        let before = fs::read_to_string(fixture.path()).unwrap();
        let external = format!("{before}\nFresh cross-launch source.\n");
        replace_library_document(&fixture.app.request, ID, &before, &external).unwrap();

        let restored = fixture.restart(state);

        assert_eq!(restored.document.as_ref().unwrap().id, ID);
        assert_eq!(restored.document.as_ref().unwrap().source, external);
        assert!(matches!(
            restored.list_context,
            ListContext::Notes { ref note_type } if note_type == "entity"
        ));
        assert!(matches!(
            restored.rows[restored.list.selected().unwrap()].target,
            Target::Note(ref id) if id == ID
        ));
    }

    #[test]
    fn stale_navigation_falls_back_without_restoring_drafts_or_paths() {
        let mut fixture = Fixture::new();
        let root = fixture.app.projection.as_ref().unwrap().root.clone();
        let stale = NavigationState::new(
            &root,
            NavigationLocation::Note {
                scope: LibraryScope::Project {
                    project: "example".into(),
                },
                note_type: "entity".into(),
                id: "Projects/example/entities/missing.md".into(),
            },
        );

        let restored = fixture.restart(stale);

        assert!(restored.document.is_none());
        assert!(matches!(
            restored.list_context,
            ListContext::Notes { ref note_type } if note_type == "entity"
        ));
        assert!(
            restored
                .messages
                .iter()
                .any(|message| message.contains("Previous note no longer exists"))
        );

        fixture.open_editor();
        fixture.app.paste("unsaved");
        assert!(fixture.app.navigation_state().is_none());
    }

    #[test]
    fn explicit_project_and_exact_root_override_unrelated_saved_navigation() {
        let fixture = Fixture::new();
        let root = fixture.app.projection.as_ref().unwrap().root.clone();
        let global = NavigationState::new(
            &root,
            NavigationLocation::Categories {
                scope: LibraryScope::Global,
                selected_note_type: None,
            },
        );
        let restored = fixture.restart(global);
        assert_eq!(
            restored.scope,
            LibraryScope::Project {
                project: "example".into()
            }
        );

        let unrelated = NavigationState::new(
            Path::new("/different/root"),
            NavigationLocation::Projects {
                selected_scope: None,
            },
        );
        let restored = fixture.restart(unrelated);
        assert!(matches!(restored.list_context, ListContext::Categories));
        assert_eq!(restored.title, "example");
    }

    #[test]
    fn stale_save_preserves_external_write_and_the_unsaved_buffer() {
        let mut fixture = Fixture::new();
        fixture.open_editor();
        let before = fs::read_to_string(fixture.path()).unwrap();
        fixture.app.paste("\nmy draft\n");
        let external = format!("{before}\nexternal edit\n");
        replace_library_document(&fixture.app.request, ID, &before, &external).unwrap();
        fixture.app.command("save");
        fixture.finish();
        assert!(fixture.app.dirty());
        assert!(
            fixture
                .app
                .editor
                .as_ref()
                .unwrap()
                .source()
                .contains("my draft")
        );
        assert_eq!(fs::read_to_string(fixture.path()).unwrap(), external);
        fixture.app.command("discard");
        assert!(!fixture.app.dirty());
        fixture.app.open(ID);
        fixture.finish();
        assert_eq!(fixture.app.document.as_ref().unwrap().source, external);
    }

    #[test]
    fn automatic_check_is_silent_until_a_clean_external_change_refreshes_the_library() {
        let mut fixture = Fixture::new();
        fixture.app.open(GLOBAL_ID);
        fixture.finish();
        let before = fixture
            .app
            .document
            .as_ref()
            .expect("global document")
            .source
            .clone();
        let message_count = fixture.app.messages.len();

        let redraw = fixture.check_external_changes();

        assert!(fixture.app.document.is_some());
        assert_eq!(fixture.app.messages.len(), message_count);
        assert!(!redraw, "an unchanged automatic check must not repaint");
        let external = format!("{before}\nExternal global detail.\n");
        fs::write(fixture.root().join(GLOBAL_ID), &external).unwrap();

        fixture.check_external_changes();

        assert!(!fixture.app.external_change_pending);
        assert!(fixture.app.document.is_none());
        assert!(fixture.app.editor.is_none());
        assert_eq!(
            fs::read_to_string(fixture.root().join(GLOBAL_ID)).unwrap(),
            external
        );
        assert_eq!(
            fixture.app.messages.back().map(String::as_str),
            Some("Library refreshed after external changes.")
        );
    }

    #[test]
    fn automatic_check_signals_external_edit_without_replacing_dirty_editor() {
        let mut fixture = Fixture::new();
        fixture.open_editor();
        let before = fs::read_to_string(fixture.path()).unwrap();
        fixture.app.paste("\nlocal draft\n");
        let draft = fixture.app.editor.as_ref().unwrap().source();
        let external = format!("{before}\nexternal edit\n");
        replace_library_document(&fixture.app.request, ID, &before, &external).unwrap();

        fixture.check_external_changes();

        assert!(fixture.app.external_change_pending);
        assert!(fixture.app.dirty());
        assert_eq!(fixture.app.editor.as_ref().unwrap().source(), draft);
        assert_eq!(fs::read_to_string(fixture.path()).unwrap(), external);
        assert!(
            fixture
                .app
                .messages
                .back()
                .unwrap()
                .contains("Drafts were retained")
        );
    }

    #[test]
    fn automatic_check_keeps_the_current_view_when_external_source_is_invalid() {
        let mut fixture = Fixture::new();
        fixture.app.open(GLOBAL_ID);
        fixture.finish();
        let loaded = fixture.app.document.as_ref().unwrap().source.clone();
        fs::write(
            fixture.root().join(GLOBAL_ID),
            "---\ntype: entity\n---\n\ninvalid external source\n",
        )
        .unwrap();

        fixture.check_external_changes();

        assert!(fixture.app.external_change_pending);
        assert_eq!(fixture.app.document.as_ref().unwrap().source, loaded);
        assert!(fixture.app.editor.is_none());
        assert!(
            fixture
                .app
                .messages
                .back()
                .unwrap()
                .contains("could not be refreshed")
        );
    }

    #[test]
    fn automatic_check_preserves_creation_inputs_after_external_change() {
        let mut fixture = Fixture::new();
        fixture.app.command("create task");
        fixture.finish();
        fixture.app.paste("draft-task.md");
        let global_path = fixture.root().join(GLOBAL_ID);
        let before = fs::read_to_string(&global_path).unwrap();
        let external = before.replace("status: stable", "status: reviewed");
        fs::write(&global_path, &external).unwrap();

        fixture.check_external_changes();

        assert!(fixture.app.external_change_pending);
        assert!(fixture.app.in_workflow());
        assert!(fixture.app.creation_input_active());
        assert_eq!(fixture.app.prompt.lines(), ["draft-task.md"]);
        assert_eq!(fs::read_to_string(global_path).unwrap(), external);
    }

    #[test]
    fn automatic_check_preserves_both_lifecycle_drafts_after_external_change() {
        let mut fixture = Fixture::new();
        fixture.app.open(TASK_ID);
        fixture.finish();
        fixture.app.command("lifecycle");
        fixture.finish();
        let (before, task_draft, roadmap_draft) = {
            let Some(Workflow::Lifecycle(form)) = &mut fixture.app.workflow else {
                panic!("task lifecycle form was not prepared");
            };
            let before = form.prepared.source.clone();
            let task_draft = format!("{}\nlocal task draft\n", before.trim_end());
            let roadmap_draft = format!(
                "{}\nlocal roadmap draft\n",
                form.prepared.roadmap_source.trim_end()
            );
            form.task = Editor::new(&task_draft).unwrap();
            form.roadmap = Editor::new(&roadmap_draft).unwrap();
            (before, task_draft, roadmap_draft)
        };
        let external = format!("{}\nexternal task edit\n", before.trim_end());
        replace_library_document(&fixture.app.request, TASK_ID, &before, &external).unwrap();

        fixture.check_external_changes();

        assert!(fixture.app.external_change_pending);
        let Some(Workflow::Lifecycle(form)) = &fixture.app.workflow else {
            panic!("external refresh discarded the lifecycle form");
        };
        assert_eq!(form.task.source(), task_draft);
        assert_eq!(form.roadmap.source(), roadmap_draft);
        assert_eq!(
            fs::read_to_string(fixture.root().join(TASK_ID)).unwrap(),
            external
        );
    }

    #[test]
    fn global_event_protection_search_and_prompt_paste() {
        let mut fixture = Fixture::new();
        for id in [
            "Global/entities/rust-pattern.md",
            "Projects/example/events/sessions/2026-07-13.md",
        ] {
            fixture.app.open(id);
            fixture.finish();
            fixture.app.edit();
            assert!(fixture.app.editor.is_none());
        }
        fixture.app.command("search core");
        fixture.finish();
        assert!(!fixture.app.rows.is_empty());
        fixture.app.focus = Focus::Prompt;
        fixture.app.paste("quit\nopen 1\x1b[2J");
        assert!(!fixture.app.quit);
        assert_eq!(fixture.app.prompt.lines().len(), 1);
        assert!(!fixture.app.prompt.lines()[0].contains('\x1b'));
    }

    #[test]
    fn integration_inspection_reports_both_exact_plans_without_writes() {
        let mut fixture = Fixture::new();
        let home = fixture.temp.join("client home");
        fs::create_dir_all(&home).unwrap();
        let instructions = b"human instructions\n";
        let hooks = b"{}\n";
        fs::write(home.join("AGENTS.md"), instructions).unwrap();
        fs::write(home.join("hooks.json"), hooks).unwrap();

        fixture
            .app
            .command(&format!("integrations codex {}", home.display()));
        fixture.finish();

        assert_eq!(fixture.app.body_title, "INTEGRATIONS · CODEX");
        assert!(fixture.app.body.contains("READ-ONLY INSPECTION"));
        assert!(fixture.app.body.contains("INSTRUCTION POINTER"));
        assert!(fixture.app.body.contains("Action: append"));
        assert!(fixture.app.body.contains("SESSIONSTART HOOK"));
        assert!(fixture.app.body.contains("Action: add-hooks"));
        assert_eq!(fs::read(home.join("AGENTS.md")).unwrap(), instructions);
        assert_eq!(fs::read(home.join("hooks.json")).unwrap(), hooks);
        assert_eq!(fs::read_dir(&home).unwrap().count(), 2);
    }

    #[test]
    fn integration_confirmation_applies_and_removes_each_client_target_independently() {
        for client in ["codex", "claude"] {
            for kind in ["instructions", "hook"] {
                let mut fixture = Fixture::new();
                let home = fixture.temp.join("client home");
                fs::create_dir(&home).unwrap();
                let instruction_file = if client == "codex" {
                    "AGENTS.md"
                } else {
                    "CLAUDE.md"
                };
                let hook_file = if client == "codex" {
                    "hooks.json"
                } else {
                    "settings.json"
                };
                let instructions = b"Human instructions\r\n";
                let settings = b"{\"humanSetting\": true}\n";
                fs::write(home.join(instruction_file), instructions).unwrap();
                fs::write(home.join(hook_file), settings).unwrap();
                for (operation, changed) in [("apply", true), ("apply", false), ("remove", true)] {
                    fixture.app.command(&format!(
                        "integration {operation} {kind} {client} {}",
                        home.display()
                    ));
                    fixture.finish();
                    let review = fixture.app.integration_review.as_ref().unwrap();
                    let plan_id = review.plan_id.clone();
                    assert!(review.body.contains("Replacement (JSON-escaped UTF-8)"));
                    assert!(
                        review
                            .body
                            .contains(&format!("Operation: {operation} {kind}"))
                    );
                    assert!(fixture.app.navigation_state().is_none());
                    let before = (
                        fs::read(home.join(instruction_file)).unwrap(),
                        fs::read(home.join(hook_file)).unwrap(),
                    );
                    fixture.app.command("confirm wrong-id");
                    fixture.app.command("save");
                    fixture.app.command("quit");
                    fixture.app.command("refresh");
                    assert!(!fixture.app.quit);
                    assert!(!fixture.app.busy);
                    assert!(fixture.jobs.try_recv().is_err());
                    assert_eq!(fs::read(home.join(instruction_file)).unwrap(), before.0);
                    assert_eq!(fs::read(home.join(hook_file)).unwrap(), before.1);
                    fixture.app.command(&format!("confirm {plan_id}"));
                    fixture.app.command(&format!("confirm {plan_id}"));
                    fixture.app.command("discard");
                    fixture.finish();
                    assert!(fixture.jobs.try_recv().is_err());
                    assert!(fixture.app.integration_review.is_none());
                    assert!(
                        fixture.app.body.contains(&format!("Changed: {changed}")),
                        "{}",
                        fixture.app.body
                    );
                    if kind == "instructions" {
                        assert_eq!(fs::read(home.join(hook_file)).unwrap(), settings);
                    } else {
                        assert_eq!(fs::read(home.join(instruction_file)).unwrap(), instructions);
                    }
                }
                assert_eq!(fs::read(home.join(instruction_file)).unwrap(), instructions);
                assert_eq!(fs::read(home.join(hook_file)).unwrap(), settings);
            }
        }
    }

    #[test]
    fn integration_discard_and_stale_confirmation_never_replace_changed_bytes() {
        for kind in ["instructions", "hook"] {
            let mut fixture = Fixture::new();
            let home = fixture.temp.join("client home");
            fs::create_dir(&home).unwrap();
            let prepare = format!("integration apply {kind} codex {}", home.display());
            fixture.app.command(&prepare);
            fixture.finish();
            let cancelled_id = fixture
                .app
                .integration_review
                .as_ref()
                .unwrap()
                .plan_id
                .clone();
            fixture.app.command("discard");
            fixture.app.command(&format!("confirm {cancelled_id}"));
            assert!(fixture.jobs.try_recv().is_err());
            assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
            fixture.app.command(&prepare);
            fixture.finish();
            let plan_id = fixture
                .app
                .integration_review
                .as_ref()
                .unwrap()
                .plan_id
                .clone();
            let target = home.join(if kind == "instructions" {
                "AGENTS.md"
            } else {
                "hooks.json"
            });
            let changed = if kind == "instructions" {
                "External instructions\n"
            } else {
                "{\"external\": true}\n"
            };
            fs::write(&target, changed).unwrap();
            fixture.app.command(&format!("confirm {plan_id}"));
            fixture.finish();
            assert!(fixture.app.body.contains("Operation failed:"));
            assert!(fixture.app.body.contains("fresh /integration review"));
            assert!(fixture.app.integration_review.is_none());
            assert_eq!(fs::read_to_string(&target).unwrap(), changed);
            fixture.app.command(&format!("confirm {plan_id}"));
            assert!(fixture.jobs.try_recv().is_err());
        }
    }

    #[test]
    fn integration_review_survives_external_refresh_and_refuses_source_drift() {
        let mut fixture = Fixture::new();
        let home = fixture.temp.join("client home");
        fs::create_dir(&home).unwrap();
        fixture.open_editor();
        fixture
            .app
            .editor
            .as_mut()
            .unwrap()
            .area
            .insert_str("draft");
        let prepare = format!("integration apply instructions codex {}", home.display());
        fixture.app.command(&prepare);
        assert!(fixture.jobs.try_recv().is_err());
        fixture.app.command("discard");
        fixture.app.command(&prepare);
        fixture.finish();
        let body = fixture.app.body.clone();
        let plan_id = fixture
            .app
            .integration_review
            .as_ref()
            .unwrap()
            .plan_id
            .clone();
        fixture
            .app
            .receive(Ok(Response::Checked(Err("external invalid note".into()))));
        assert_eq!(fixture.app.body, body);
        assert!(fixture.app.external_change_pending);
        let plan = prepare_agent_wiring(&fixture.app.request, AgentClient::Codex, &home).unwrap();
        fs::write(plan.source, "Changed canonical instructions\n").unwrap();
        fixture.app.command(&format!("confirm {plan_id}"));
        fixture.finish();
        assert!(fixture.app.body.contains("Operation failed:"));
        assert!(!home.join("AGENTS.md").exists());
    }

    #[test]
    fn integration_inspection_keeps_partial_conflicts_visible_and_validates_client() {
        let mut fixture = Fixture::new();
        let home = fixture.temp.join("codex-home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("AGENTS.override.md"), "shadowing instructions\n").unwrap();

        fixture
            .app
            .command(&format!("integrations codex {}", home.display()));
        fixture.finish();

        assert!(fixture.app.body.contains("Status: unavailable"));
        assert!(fixture.app.body.contains("AGENTS.override.md"));
        assert!(fixture.app.body.contains("SESSIONSTART HOOK"));
        assert!(fixture.app.body.contains("Status: prepared"));
        assert!(!home.join("hooks.json").exists());

        fixture.app.command("integrations vscode");
        assert!(!fixture.app.busy);
        assert!(
            fixture
                .app
                .messages
                .back()
                .unwrap()
                .contains("codex|claude")
        );
    }

    #[test]
    fn configured_creation_form_collects_template_fields_and_applies_projection() {
        let mut fixture = Fixture::new();
        fixture.app.focus = Focus::Prompt;
        fixture.app.paste("/create ");
        let suggestions = fixture
            .app
            .completions()
            .into_iter()
            .map(|suggestion| suggestion.command)
            .collect::<Vec<_>>();
        assert!(suggestions.contains(&"create task".to_owned()));
        assert!(suggestions.contains(&"create problem".to_owned()));
        assert!(suggestions.contains(&"create entity".to_owned()));
        assert!(!suggestions.iter().any(|value| value.contains("session")));
        fixture.app.reset_prompt();
        fixture.app.command("create task");
        fixture.finish();
        assert!(fixture.app.creation_input_active());
        assert!(fixture.app.dirty());

        for value in [
            "tui-created.md",
            "open",
            "2026-09-17",
            "2026-09-17",
            "TUI created task",
            "Tracks [[Projects/example/entities/core|the core]].",
        ] {
            fixture.app.paste(value);
            press(&mut fixture.app, KeyCode::Enter);
        }
        assert!(fixture.app.active_editor().is_some());
        assert!(fixture.app.editing);
        let projection = fixture.app.active_editor_mut().unwrap();
        projection.area.move_cursor(CursorMove::Bottom);
        projection.area.move_cursor(CursorMove::End);
        projection
            .area
            .insert_str("\n- [[Projects/example/records/tasks/tui-created|TUI created task]]\n");

        fixture.app.command("save");
        fixture.finish();
        assert!(fixture.root().join(CREATED_TASK_ID).is_file());
        fixture.finish();
        fixture.finish();

        assert!(!fixture.app.in_workflow());
        assert_eq!(fixture.app.document.as_ref().unwrap().id, CREATED_TASK_ID);
        assert!(
            fs::read_to_string(fixture.root().join(CREATED_TASK_ID))
                .unwrap()
                .contains("# TUI created task")
        );
        assert!(
            fs::read_to_string(fixture.root().join("Projects/example/roadmap.md"))
                .unwrap()
                .contains("TUI created task")
        );
        validate_project(&fixture.app.request).unwrap();
    }

    #[test]
    fn unfinished_creation_is_guarded_and_discarded_without_writes() {
        let mut fixture = Fixture::new();
        let roadmap_before = fs::read(fixture.root().join("Projects/example/roadmap.md")).unwrap();
        let state_before =
            fs::read(fixture.root().join("Projects/example/.akasha-state.toml")).unwrap();

        fixture.app.command("create task");
        fixture.finish();
        fixture.app.paste("never-created.md");
        fixture.app.command("projects");
        assert!(fixture.app.in_workflow());
        assert!(
            !fixture
                .root()
                .join("Projects/example/records/tasks/never-created.md")
                .exists()
        );

        fixture.app.command("discard");
        assert!(!fixture.app.in_workflow());
        assert_eq!(
            fs::read(fixture.root().join("Projects/example/roadmap.md")).unwrap(),
            roadmap_before
        );
        assert_eq!(
            fs::read(fixture.root().join("Projects/example/.akasha-state.toml")).unwrap(),
            state_before
        );
    }

    #[test]
    fn task_lifecycle_form_updates_exact_task_and_roadmap_together() {
        let mut fixture = Fixture::new();
        fixture.app.open(TASK_ID);
        fixture.finish();
        fixture.app.command("lifecycle");
        fixture.finish();

        let Some(Workflow::Lifecycle(form)) = &mut fixture.app.workflow else {
            panic!("task lifecycle form was not prepared");
        };
        let replacement = form
            .prepared
            .source
            .replace("status: active", "status: done")
            .replace("updated: 2026-07-13", "updated: 2026-09-17");
        form.task = Editor::new(&replacement).unwrap();
        form.roadmap = Editor::new(&format!(
            "{}\nTask completed through the TUI lifecycle form.\n",
            form.prepared.roadmap_source.trim_end()
        ))
        .unwrap();

        fixture.app.command("save");
        fixture.finish();
        fixture.finish();
        fixture.finish();

        assert!(!fixture.app.in_workflow());
        assert_eq!(fixture.app.document.as_ref().unwrap().id, TASK_ID);
        assert!(
            fs::read_to_string(fixture.root().join(TASK_ID))
                .unwrap()
                .contains("status: done")
        );
        assert!(
            fs::read_to_string(fixture.root().join("Projects/example/roadmap.md"))
                .unwrap()
                .contains("Task completed through the TUI lifecycle form.")
        );
        validate_project(&fixture.app.request).unwrap();
    }

    #[test]
    fn failed_task_lifecycle_apply_retains_both_drafts() {
        let mut fixture = Fixture::new();
        fixture.app.open(TASK_ID);
        fixture.finish();
        fixture.app.command("lifecycle");
        fixture.finish();

        let (task_draft, roadmap_draft) = {
            let Some(Workflow::Lifecycle(form)) = &mut fixture.app.workflow else {
                panic!("task lifecycle form was not prepared");
            };
            let task_draft = form
                .prepared
                .source
                .replace("status: active", "status: done");
            let roadmap_draft = format!("{}\nDraft roadmap.\n", form.prepared.roadmap_source);
            form.task = Editor::new(&task_draft).unwrap();
            form.roadmap = Editor::new(&roadmap_draft).unwrap();
            (task_draft, roadmap_draft)
        };
        fs::write(
            fixture.root().join(TASK_ID),
            "external task bytes invalidate the prepared snapshot\n",
        )
        .unwrap();

        fixture.app.command("save");
        fixture.finish();

        let Some(Workflow::Lifecycle(form)) = &fixture.app.workflow else {
            panic!("failed apply discarded the lifecycle form");
        };
        assert_eq!(form.task.source(), task_draft);
        assert_eq!(form.roadmap.source(), roadmap_draft);
        assert!(
            fixture
                .app
                .messages
                .back()
                .unwrap()
                .starts_with("Operation failed")
        );
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn control(app: &mut App, key: char) {
        app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL));
    }

    #[test]
    fn slash_palette_filters_wraps_completes_and_executes_only_on_enter() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.focus = Focus::Prompt;
        app.paste("/s");
        assert_eq!(
            app.completions()
                .iter()
                .map(|c| c.command.as_str())
                .collect::<Vec<_>>(),
            ["search", "search-all", "save"]
        );
        assert!(app.completions().iter().all(|c| !c.description.is_empty()));
        press(app, KeyCode::Up);
        assert_eq!(app.completion_selection(), 2);
        press(app, KeyCode::Down);
        assert_eq!(app.completion_selection(), 0);
        press(app, KeyCode::Down);
        press(app, KeyCode::Tab);
        assert_eq!(app.prompt.lines(), ["/search-all "]);
        assert!(app.completions().is_empty());
        assert!(app.focus == Focus::Prompt);
        assert!(!app.busy);
        assert!(fixture.jobs.try_recv().is_err());

        control(app, 'c');
        app.paste("/he");
        press(app, KeyCode::Tab);
        assert_eq!(app.prompt.lines(), ["/help"]);
        assert!(app.focus == Focus::Prompt);
        press(app, KeyCode::Enter);
        assert_eq!(app.body_title, "HELP");
        assert!(app.focus == Focus::Reader);
        assert_eq!(app.prompt.placeholder_text(), PROMPT_PLACEHOLDER);
        app.command("home");
        app.paste("/he");
        press(app, KeyCode::Enter);
        assert_eq!(app.body_title, "HELP");
    }

    #[test]
    fn exact_slash_command_wins_prefix_and_arguments_are_completed() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.focus = Focus::Prompt;
        app.paste("/project");
        assert_eq!(app.completions()[0].command, "project");
        press(app, KeyCode::Enter);
        assert_eq!(app.prompt.lines(), ["/project "]);
        assert!(app.focus == Focus::Prompt);
        assert!(app.history.is_empty());
        assert!(!app.busy);
        app.paste("example");
        press(app, KeyCode::Enter);
        assert_eq!(app.title, "example");
        assert!(app.focus == Focus::List);
    }

    #[test]
    fn escape_dismisses_menu_and_tab_reopens_completion_without_changing_focus() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.focus = Focus::Prompt;
        app.paste("/sea");
        assert!(!app.completions().is_empty());
        press(app, KeyCode::Esc);
        assert_eq!(app.prompt.lines(), ["/sea"]);
        assert!(app.completions().is_empty());
        press(app, KeyCode::Tab);
        assert!(app.focus == Focus::Prompt);
        assert_eq!(app.prompt.lines(), ["/search "]);
        assert!(app.completions().is_empty());
        press(app, KeyCode::BackTab);
        assert!(app.focus == Focus::Reader);
    }

    #[test]
    fn repeated_tab_never_leaves_an_unmatched_command_draft() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        for input in ["/unknown", "search words"] {
            app.focus = Focus::Prompt;
            app.reset_prompt();
            app.paste(input);
            press(app, KeyCode::Tab);
            let expected = input;
            assert_eq!(app.prompt.lines(), [expected]);
            press(app, KeyCode::Tab);
            assert!(app.focus == Focus::Prompt);
            assert_eq!(app.prompt.lines(), [expected]);
            assert!(!app.busy);
        }
        app.reset_prompt();
        press(app, KeyCode::Tab);
        assert!(app.focus == Focus::List);
    }

    #[test]
    fn open_arguments_offer_filter_and_open_exact_loaded_notes() {
        let mut f = Fixture::new();
        f.app.focus = Focus::Prompt;
        f.app.paste("/open");
        press(&mut f.app, KeyCode::Tab);
        assert_eq!(f.app.prompt.lines(), ["/open "]);
        assert!(!f.app.completions().is_empty());
        assert!(!f.app.busy);
        f.app.paste("core.md");
        assert_eq!(f.app.completions().len(), 1);
        assert_eq!(f.app.completions()[0].command, format!("open {ID}"));
        press(&mut f.app, KeyCode::Tab);
        assert_eq!(f.app.prompt.lines(), [format!("/open {ID}")]);
        assert!(!f.app.busy);
        press(&mut f.app, KeyCode::Enter);
        f.finish();
        assert_eq!(f.app.document.as_ref().unwrap().id, ID);
        f.app.command("home");
        f.app.paste("/open missing-no-such-note");
        press(&mut f.app, KeyCode::Tab);
        assert!(f.app.focus == Focus::Prompt);
        assert!(f.app.completions().is_empty());
        assert_eq!(f.app.prompt.lines(), ["/open missing-no-such-note"]);
    }

    #[test]
    fn bare_search_requests_words_and_submits_the_completed_query() {
        let mut f = Fixture::new();
        f.app.command("search");
        assert_eq!(f.app.prompt.lines(), ["/search "]);
        assert!(!f.app.busy);
        assert!(f.app.messages.back().unwrap().contains("/search memory"));
        f.app.paste("Synthetic");
        press(&mut f.app, KeyCode::Enter);
        assert!(f.app.busy);
        assert!(
            matches!(f.jobs.try_recv().unwrap(), Job::Search(_, query, Some(_)) if query == "Synthetic")
        );
    }

    #[test]
    fn history_restores_draft_without_intercepting_arrows_for_slash_commands() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.focus = Focus::Prompt;
        app.paste("/motion");
        press(app, KeyCode::Enter);
        app.paste("/home");
        press(app, KeyCode::Enter);
        app.paste("search my unfinished draft");
        press(app, KeyCode::Up);
        assert_eq!(app.prompt.lines(), ["/home"]);
        assert!(app.completions().is_empty());
        press(app, KeyCode::Up);
        assert_eq!(app.prompt.lines(), ["/motion"]);
        press(app, KeyCode::Down);
        press(app, KeyCode::Down);
        assert_eq!(app.prompt.lines(), ["search my unfinished draft"]);
        assert_eq!(app.prompt.placeholder_text(), PROMPT_PLACEHOLDER);
        control(app, 'c');
        press(app, KeyCode::Up);
        press(app, KeyCode::Down);
        assert_eq!(app.prompt.lines(), [""]);
        assert_eq!(app.prompt.placeholder_text(), PROMPT_PLACEHOLDER);
    }

    #[test]
    fn prompt_shell_shortcuts_preserve_unicode_and_remain_single_line() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.focus = Focus::Prompt;
        app.paste("search Привет/世界 tail");
        control(app, 'a');
        assert_eq!(app.prompt.cursor().1, 0);
        control(app, 'e');
        assert_eq!(app.prompt.cursor().1, 21);
        control(app, 'w');
        assert_eq!(app.prompt.lines(), ["search Привет/世界 "]);
        control(app, 'w');
        assert_eq!(app.prompt.lines(), ["search "]);
        app.paste("one two");
        control(app, 'b');
        control(app, 'b');
        control(app, 'b');
        control(app, 'k');
        assert_eq!(app.prompt.lines(), ["search one "]);
        control(app, 'a');
        control(app, 'f');
        control(app, 'u');
        assert_eq!(app.prompt.lines(), ["earch one "]);
        control(app, 'j');
        control(app, 'm');
        assert_eq!(app.prompt.lines(), ["earch one "]);
        assert!(!app.busy);
    }

    #[test]
    fn ctrl_c_clears_prompt_before_exit_and_never_discards_dirty_editor() {
        let mut fixture = Fixture::new();
        fixture.open_editor();
        fixture.app.paste("\nkeep this draft\n");
        let draft = fixture.app.editor.as_ref().unwrap().source();
        fixture.app.action(Action::Focus(Focus::Prompt));
        fixture.app.paste("/qui");
        control(&mut fixture.app, 'c');
        assert!(!fixture.app.quit);
        assert_eq!(fixture.app.prompt.lines(), [""]);
        assert!(fixture.app.completions().is_empty());
        assert!(fixture.app.dirty());
        control(&mut fixture.app, 'c');
        assert!(!fixture.app.quit);
        assert_eq!(fixture.app.editor.as_ref().unwrap().source(), draft);
        fixture.app.command("home");
        assert!(fixture.app.dirty());
        assert_ne!(fixture.app.body_title, "WELCOME TO AKASHA");
    }

    #[test]
    fn unsafe_paste_stays_inert_and_input_limit_counts_unicode_characters() {
        let mut fixture = Fixture::new();
        let app = &mut fixture.app;
        app.focus = Focus::Prompt;
        app.paste("/quit\r\n/save\x1b[2J\t");
        assert!(!app.quit);
        assert!(!app.busy);
        assert!(app.history.is_empty());
        assert!(app.completions().is_empty());
        assert_eq!(app.prompt.lines(), ["/quit  /save [2J "]);
        control(app, 'c');
        app.paste(&"界".repeat(4095));
        press(app, KeyCode::Char('界'));
        press(app, KeyCode::Char('界'));
        app.paste("more");
        assert_eq!(app.prompt.lines()[0].chars().count(), 4096);
        assert!(!app.quit);
        assert!(fixture.jobs.try_recv().is_err());
    }
    #[test]
    fn backspace_focuses_prompt_but_deletes_in_text_fields() {
        let mut f = Fixture::new();
        press(&mut f.app, KeyCode::Enter);
        press(&mut f.app, KeyCode::Backspace);
        assert!(f.app.focus == Focus::Prompt);
        f.app.paste("ab");
        press(&mut f.app, KeyCode::Backspace);
        assert_eq!(f.app.prompt.lines(), ["a"]);
        f.open_editor();
        f.app.paste("xy");
        let before = f.app.editor.as_ref().unwrap().source();
        press(&mut f.app, KeyCode::Backspace);
        assert!(f.app.focus == Focus::Reader);
        assert_eq!(
            f.app.editor.as_ref().unwrap().source().len(),
            before.len() - 1
        );
    }

    fn draw_for_mouse(app: &mut App, width: u16, height: u16) {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| super::super::view::draw(f, app)).unwrap();
    }
    fn click(app: &mut App, rect: Rect) {
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn mouse_and_back_follow_exact_note_hierarchy_and_protect_drafts() {
        let mut f = Fixture::new();
        draw_for_mouse(&mut f.app, 120, 38);
        let hit = f
            .app
            .hits
            .iter()
            .find(|(_, a)| matches!(a, Action::Open(0)))
            .unwrap()
            .0;
        click(&mut f.app, hit);
        assert_eq!(f.app.title, "example / entity");
        draw_for_mouse(&mut f.app, 120, 38);
        let hit = f
            .app
            .hits
            .iter()
            .find(|(_, a)| matches!(a, Action::Open(0)))
            .unwrap()
            .0;
        click(&mut f.app, hit);
        f.finish();
        assert_eq!(f.app.document.as_ref().unwrap().id, ID);
        press(&mut f.app, KeyCode::Esc);
        assert!(f.app.document.is_none());
        assert_eq!(f.app.title, "example / entity");
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(f.app.title, "example");
        f.open_editor();
        f.app.paste("\nprotected draft");
        let before = f.app.editor.as_ref().unwrap().source();
        draw_for_mouse(&mut f.app, 120, 38);
        let hit = f
            .app
            .hits
            .iter()
            .find(|(_, a)| matches!(a, Action::Command("projects")))
            .unwrap()
            .0;
        click(&mut f.app, hit);
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(f.app.editor.as_ref().unwrap().source(), before);
        assert!(f.app.dirty());
    }

    #[test]
    fn visible_search_shortcut_preserves_commands_and_empty_enter_browses() {
        let mut f = Fixture::new();
        press(&mut f.app, KeyCode::Enter);
        assert_eq!(f.app.title, "example / entity");
        press(&mut f.app, KeyCode::F(3));
        assert_eq!(f.app.prompt.lines(), ["/search "]);
        f.app.paste("unfinished query");
        press(&mut f.app, KeyCode::F(3));
        assert_eq!(f.app.prompt.lines(), ["/search unfinished query"]);
        control(&mut f.app, 'c');
        press(&mut f.app, KeyCode::F(6));
        assert!(matches!(f.app.scope, LibraryScope::Global));
        press(&mut f.app, KeyCode::F(4));
        assert_eq!(f.app.title, "PROJECTS + GLOBAL");
    }

    #[test]
    fn mouse_hits_rebuild_on_resize_and_command_popup_owns_its_cells() {
        let mut f = Fixture::new();
        f.app.paste("/he");
        for (w, h) in [(120, 38), (40, 12)] {
            draw_for_mouse(&mut f.app, w, h);
            assert!(
                f.app
                    .hits
                    .iter()
                    .all(|(r, _)| r.right() <= w && r.bottom() <= h)
            );
        }
        let hit = f
            .app
            .hits
            .iter()
            .find(|(_, a)| matches!(a, Action::Completion(0)))
            .unwrap()
            .0;
        click(&mut f.app, hit);
        assert_eq!(f.app.body_title, "HELP");
        assert!(!f.app.busy);
    }
}
