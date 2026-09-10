use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use super::editor::{Editor, safe_text};
use akasha_core::{
    LibraryBook, LibraryDocument, LibraryProjection, LibraryScope, LibrarySearchResult, NoteClass,
    ResolveRequest, assemble_context, build_library_projection, load_library_document,
    recover_pending_note_edit, render_context_markdown, replace_library_document, search_library,
    validate_project,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use ratatui_textarea::TextArea;

pub(super) const HELP: &str = "AKASHA · TERMINAL\n\nTab / Shift-Tab  switch prompt, library and reader\nEnter           open selected item / run command\nEscape          return to prompt (keeps your draft)\nCtrl-S          save the current source\nCtrl-Q / Ctrl-C quit; unsaved changes prevent exit\nF2              edit selected note\nF5              refresh library\nF1              this help\n\nCOMMANDS\nprojects        browse registered projects\nproject SLUG    select a project\nglobal          browse shared knowledge\nls              categories in current scope\ntype NAME       open a configured note category\nopen NUMBER     open a numbered item\nopen PATH       open an exact note identity\nback            return to previous list\nsearch TEXT     literal text search in current scope\nsearch-all TEXT search every project and global notes\nedit / read     source editor / reading mode\nsave            save through checked core transaction\ndiscard         discard editor changes\ncontext         bounded project orientation\nvalidate        validate selected project\nrefresh         reload data (or press F5)\nmotion          toggle ambient animation\nhelp / quit     help / exit\n\nEDITOR\nArrows, Home/End, PageUp/Down; Shift selects text.\nCtrl-Z undo; Ctrl-Y redo; Ctrl-X cut; Ctrl-V internal paste.\nUse the terminal's paste shortcut for system clipboard text.\nEsc returns to the prompt without discarding your draft.\n\nReading: arrows/PageUp/PageDown scroll.\nCommand prompt: Up/Down recall session history.\n\nWave one edits project records/entities. Events and global\nknowledge are readable. Creation and administration forms\nwill arrive in subsequent waves; existing CLI commands remain available.\n\nOpen from a linked repository or pass --root PATH --project SLUG.\nSSH: run Akasha on the remote host in an allocated terminal.";

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
}

pub(super) enum Job {
    Load(ResolveRequest),
    Open(ResolveRequest, String),
    Search(ResolveRequest, String, Option<LibraryScope>),
    Context(ResolveRequest),
    Validate(ResolveRequest),
    Save(ResolveRequest, String, String, String),
}
pub(super) enum Response {
    Loaded(ResolveRequest, Box<LibraryProjection>),
    Opened(LibraryDocument),
    Found(LibrarySearchResult),
    Text(String, String),
    Saved(String),
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
        Job::Load(mut request) => {
            recover_pending_note_edit(&request).map_err(|e| err(&e))?;
            let projection = build_library_projection(&request).map_err(|e| err(&e))?;
            request.root_override = Some(projection.root.clone());
            request.project_override = Some(projection.selected_project.clone());
            Ok(Response::Loaded(request, Box::new(projection)))
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
    }
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
    pub quit: bool,
    pub no_motion: bool,
    pub ascii: bool,
    pub color: bool,
    pub tick: u64,
    history: Vec<String>,
    history_index: usize,
    history_draft: String,
    navigation: Vec<Navigation>,
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
        let scope = LibraryScope::Project {
            project: request.project_override.clone().unwrap_or_default(),
        };
        let mut prompt = TextArea::default();
        prompt.set_placeholder_text("help · projects · search <text>");
        Self {
            request,
            projection: None,
            scope,
            rows: vec![],
            list: ListState::default(),
            title: "LIBRARY".into(),
            focus: Focus::Prompt,
            prompt,
            document: None,
            editor: None,
            editing: false,
            body_title: "WELCOME TO AKASHA".into(),
            body: HELP.into(),
            scroll: 0,
            max_scroll: 0,
            messages: VecDeque::new(),
            busy: false,
            quit: false,
            no_motion,
            ascii,
            color,
            tick: 0,
            history: vec![],
            history_index: 0,
            history_draft: String::new(),
            navigation: vec![],
            jobs,
        }
    }

    pub fn load(&mut self) {
        self.submit(Job::Load(self.request.clone()));
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
        self.editor.as_ref().is_some_and(Editor::dirty)
    }
    fn can_leave(&mut self) -> bool {
        if self.busy {
            self.message("An operation is running; please wait.");
            false
        } else if self.dirty() {
            self.message("Unsaved changes: use save or discard before leaving this note.");
            false
        } else {
            true
        }
    }
    fn remember(&mut self) {
        self.navigation.push(Navigation {
            title: self.title.clone(),
            rows: self.rows.clone(),
            selection: self.list.selected(),
            scope: self.scope.clone(),
        });
        if self.navigation.len() > 32 {
            self.navigation.remove(0);
        }
    }
    fn set_rows(&mut self, title: String, rows: Vec<Row>) {
        self.title = title;
        self.rows = rows;
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
        self.set_rows(scope_name(&scope), rows);
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
        self.set_rows("PROJECTS + GLOBAL".into(), rows);
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
        self.set_rows(format!("{} / {note_type}", scope_name(&self.scope)), rows);
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
    fn edit(&mut self) {
        if self.busy {
            return;
        }
        let editable = self.book().is_some_and(|book| book.class != NoteClass::Event && matches!(&book.scope, LibraryScope::Project { project } if Some(project) == self.request.project_override.as_ref()));
        if !editable {
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
        if self.busy {
            self.message("An operation is running; please wait.");
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
    pub fn receive(&mut self, response: WorkResult) {
        self.busy = false;
        match response {
            Err(error) => self.message(&format!("Operation failed: {error}")),
            Ok(Response::Loaded(request, projection)) => {
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
                self.message("Library loaded. Tab to browse, or type a command. F1 opens help.");
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
        }
    }

    pub fn command(&mut self, input: &str) {
        let trimmed = input.trim().trim_start_matches('/');
        let (command, argument) = trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, ""));
        let argument = argument.trim();
        match command {
            "" => {}
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
                self.editor = None;
                self.editing = false;
                self.message("Editor changes discarded; loaded source retained. Refresh to load external changes.");
            }
            _ if !self.can_leave() => {}
            "projects" => self.projects(),
            "project" => self.categories(LibraryScope::Project {
                project: argument.to_owned(),
            }),
            "global" => self.categories(LibraryScope::Global),
            "ls" => self.categories(self.scope.clone()),
            "type" => self.notes(self.scope.clone(), argument),
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
                if let Some(previous) = self.navigation.pop() {
                    self.title = previous.title;
                    self.rows = previous.rows;
                    self.scope = previous.scope;
                    if let LibraryScope::Project { project } = &self.scope {
                        self.request.project_override = Some(project.clone());
                    }
                    self.list.select(previous.selection);
                    self.focus = Focus::List;
                }
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
            "context" => self.submit(Job::Context(self.request.clone())),
            "validate" => self.submit(Job::Validate(self.request.clone())),
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
            _ => self.message("Unknown command. Type help for available operations."),
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
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
            KeyCode::F(5) => {
                self.command("refresh");
                return;
            }
            KeyCode::Esc => {
                self.focus = Focus::Prompt;
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
            Focus::Prompt => match key.code {
                KeyCode::Enter => {
                    let input = self.prompt.lines().join(" ");
                    self.prompt = TextArea::default();
                    if !input.trim().is_empty() {
                        self.history.push(input.clone());
                        if self.history.len() > 100 {
                            self.history.remove(0);
                        }
                        self.history_index = self.history.len();
                        self.history_draft.clear();
                        self.message(&format!("> {input}"));
                        self.command(&input);
                    }
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
                    self.prompt = TextArea::new(vec![line]);
                    self.prompt.move_cursor(ratatui_textarea::CursorMove::End);
                }
                _ => {
                    // Commands are one line and bounded; input_without_shortcuts prevents Ctrl-M/J
                    // from injecting command lines or editor-specific multiline operations.
                    if (self.prompt.lines()[0].len() < 4096
                        || !matches!(key.code, KeyCode::Char(_)))
                        && (!ctrl || matches!(key.code, KeyCode::Char('a' | 'e' | 'b' | 'f' | 'h')))
                    {
                        self.prompt.input_without_shortcuts(key);
                    }
                }
            },
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
                KeyCode::Left | KeyCode::Backspace => self.command("back"),
                _ => {}
            },
            Focus::Reader if self.editing => {
                if !self.busy
                    && let Some(editor) = &mut self.editor
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
            if let Some(editor) = &mut self.editor {
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
        fn finish(&mut self) {
            self.app.receive(execute_job(self.jobs.try_recv().unwrap()));
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
}
