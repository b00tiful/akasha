use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    LibraryScope, ResolutionEnvironment, ResolveRequest, build_library_projection,
    load_library_document, render_library_markdown,
};

mod support;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("akasha-library-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    request: ResolveRequest,
}

fn fixture(name: &str) -> Fixture {
    let temp = TempDir::new(name);
    let root = temp.path().join("root");
    let alpha_repository = temp.path().join("alpha-repository");
    let zeta_repository = temp.path().join("zeta-repository");
    for directory in [
        root.join("Meta"),
        root.join("templates"),
        root.join("Global/entities"),
        root.join("Inbox"),
        alpha_repository.clone(),
        zeta_repository.clone(),
    ] {
        fs::create_dir_all(directory).expect("create fixture directory");
    }

    fs::write(root.join("akasha.toml"), root_config()).expect("write root config");
    fs::write(
        root.join("Meta/projects.yaml"),
        format!(
            "zeta:\n  path: {:?}\n  status: paused\nalpha:\n  path: {:?}\n  status: active\n",
            zeta_repository, alpha_repository
        ),
    )
    .expect("write registry");

    create_project(&root, "alpha", "core", Some(("ship-mvp", "active")));
    create_project(&root, "zeta", "renderer", None);
    fs::write(
        root.join("Global/entities/rust-pattern.md"),
        "---\nschema_version: 1\ntype: entity\nentity: rust-pattern\nkind: pattern\nstatus: stable\nreviewed: 2026-07-13\n---\n\n# Rust pattern\n\nSee [[Projects/alpha/entities/core]].\n",
    )
    .expect("write global entity");

    let request = ResolveRequest {
        root_override: Some(root.clone()),
        project_override: Some("alpha".to_owned()),
        cwd: alpha_repository,
        environment: ResolutionEnvironment::default(),
    };

    Fixture {
        _temp: temp,
        root,
        request,
    }
}

fn create_project(root: &Path, project: &str, entity: &str, task: Option<(&str, &str)>) {
    let project_dir = root.join("Projects").join(project);
    for relative in [
        "templates",
        "events/sessions",
        "events/handoffs",
        "records/tasks",
        "records/problems",
        "entities",
    ] {
        fs::create_dir_all(project_dir.join(relative)).expect("create project directory");
    }
    fs::write(project_dir.join("index.md"), format!("# {project} index\n")).expect("write index");
    fs::write(
        project_dir.join("roadmap.md"),
        format!("# {project} roadmap\n"),
    )
    .expect("write roadmap");
    fs::write(
        project_dir.join(format!("entities/{entity}.md")),
        format!(
            "---\nschema_version: 1\nentity: {entity}\nkind: subsystem\nstatus: active\nreviewed: 2026-07-13\n---\n\n# {entity}\n"
        ),
    )
    .expect("write entity");

    let mut roadmap_sources = Vec::new();
    if let Some((task, status)) = task {
        fs::write(
            project_dir.join(format!("records/tasks/{task}.md")),
            format!(
                "---\nschema_version: 1\nproject: {project}\ntype: task\nstatus: {status}\n---\n\n# {task}\n"
            ),
        )
        .expect("write task");
        roadmap_sources.push(format!("records/tasks/{task}.md"));
    }
    let index_source = format!("entities/{entity}.md");
    support::write_project_state(
        &project_dir,
        &[],
        &[index_source.as_str()],
        &roadmap_sources
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
}

fn root_config() -> &'static str {
    "schema_version = 1\n\
     \n\
     [files]\n\
     registry = \"Meta/projects.yaml\"\n\
     \n\
     [folders]\n\
     templates = \"templates\"\n\
     global = \"Global\"\n\
     projects = \"Projects\"\n\
     inbox = \"Inbox\"\n\
     \n\
     [context]\n\
     tasks = \"task\"\n\
     problems = \"problem\"\n\
     handoffs = \"handoff\"\n\
     recent_events = [\"session\"]\n\
     open_statuses = [\"open\", \"active\", \"blocked\", \"in-progress\"]\n\
     \n\
     [project]\n\
     templates = \"templates\"\n\
     index = \"index.md\"\n\
     roadmap = \"roadmap.md\"\n\
     \n\
     [project.note_types.session]\n\
     class = \"event\"\n\
     folder = \"events/sessions\"\n\
     required_fields = [\"project\", \"type\", \"date\"]\n\
     \n\
     [project.note_types.handoff]\n\
     class = \"event\"\n\
     folder = \"events/handoffs\"\n\
     required_fields = [\"project\", \"type\", \"date\"]\n\
     \n\
     [project.note_types.task]\n\
     class = \"record\"\n\
     folder = \"records/tasks\"\n\
     required_fields = [\"project\", \"type\", \"status\"]\n\
     \n\
     [project.note_types.problem]\n\
     class = \"record\"\n\
     folder = \"records/problems\"\n\
     required_fields = [\"project\", \"type\", \"status\"]\n\
     \n\
     [project.note_types.entity]\n\
     class = \"entity\"\n\
     folder = \"entities\"\n\
     required_fields = [\"entity\", \"kind\", \"status\", \"reviewed\"]\n"
}

#[test]
fn projects_and_global_entities_form_a_deterministic_projection() {
    let fixture = fixture("valid");
    let projection = build_library_projection(&fixture.request).expect("build library projection");

    assert_eq!(projection.selected_project, "alpha");
    assert_eq!(projection.total_books, 4);
    assert_eq!(
        projection
            .projects
            .iter()
            .map(|shelf| shelf.project.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(projection.global.categories.len(), 1);
    let global = &projection.global.categories[0].books[0];
    assert_eq!(global.id, "Global/entities/rust-pattern.md");
    assert_eq!(global.scope, LibraryScope::Global);
    assert_eq!(global.label, "rust-pattern");
    assert_eq!(global.outgoing_links, ["Projects/alpha/entities/core.md"]);

    let alpha_entity = projection.projects[0]
        .categories
        .iter()
        .find(|category| category.note_type == "entity")
        .expect("entity category")
        .books
        .first()
        .expect("alpha entity");
    assert_eq!(alpha_entity.id, "Projects/alpha/entities/core.md");
    assert_eq!(
        alpha_entity.scope,
        LibraryScope::Project {
            project: "alpha".to_owned()
        }
    );

    let markdown = render_library_markdown(&projection);
    assert!(markdown.contains("## Global knowledge"));
    assert!(markdown.contains("### `alpha` — active"));
    assert!(markdown.contains("`Global/entities/rust-pattern.md`"));
    assert!(markdown.contains("status stable; reviewed 2026-07-13"));
    assert!(markdown.contains("`Projects/zeta/entities/renderer.md`"));
}

#[test]
fn loads_only_exact_documents_from_the_validated_projection() {
    let fixture = fixture("document");
    let id = "Projects/alpha/entities/core.md";

    let document = load_library_document(&fixture.request, id).expect("load projected document");
    assert_eq!(document.id, id);
    assert!(document.source.ends_with("# core\n"));

    let error = load_library_document(&fixture.request, "Projects/alpha/index.md")
        .expect_err("reject non-book document");
    assert_eq!(error.exit_code(), 4);
    assert!(
        error
            .to_string()
            .contains("not part of the validated library")
    );
}

#[test]
fn rejects_project_owned_metadata_in_global_knowledge() {
    let fixture = fixture("project-owned-global");
    fs::write(
        fixture.root.join("Global/entities/rust-pattern.md"),
        "---\nschema_version: 1\nproject: alpha\ntype: entity\nentity: rust-pattern\nkind: pattern\nstatus: stable\nreviewed: 2026-07-13\n---\n\n# Rust pattern\n",
    )
    .expect("replace global entity");

    let error = build_library_projection(&fixture.request).expect_err("global owner must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("must not declare a project"));
}

#[test]
fn rejects_global_notes_outside_configured_entity_folders() {
    let fixture = fixture("unconfigured-global-folder");
    fs::write(
        fixture.root.join("Global/stray.md"),
        "---\nschema_version: 1\nentity: stray\nkind: pattern\nstatus: stable\nreviewed: 2026-07-13\n---\n\n# Stray\n",
    )
    .expect("write stray global note");

    let error = build_library_projection(&fixture.request).expect_err("stray note must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(
        error
            .to_string()
            .contains("configured entity note-type folder")
    );
}

#[test]
fn validates_wikilinks_from_global_knowledge() {
    let fixture = fixture("missing-global-link");
    fs::write(
        fixture.root.join("Global/entities/rust-pattern.md"),
        "---\nschema_version: 1\ntype: entity\nentity: rust-pattern\nkind: pattern\nstatus: stable\nreviewed: 2026-07-13\n---\n\n[[Projects/alpha/entities/missing]]\n",
    )
    .expect("replace global entity");

    let error = build_library_projection(&fixture.request).expect_err("missing link must fail");
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("invalid wikilink"));
}

#[test]
fn canonical_resolution_fixture_projects_without_rewriting_it() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/resolution")
        .canonicalize()
        .expect("canonical fixture root");
    let request = ResolveRequest {
        root_override: Some(fixture.join("valid-root")),
        project_override: Some("example".to_owned()),
        cwd: fixture.join("repository/nested"),
        environment: ResolutionEnvironment::default(),
    };

    let projection = build_library_projection(&request).expect("project canonical fixture");
    assert_eq!(projection.total_books, 7);
    assert_eq!(
        projection.global.categories[0].books[0].id,
        "Global/entities/rust-pattern.md"
    );
}
