use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    LibraryScope, NoteLinkDirection, NoteLinkKind, ResolutionEnvironment, ResolveRequest,
    activate_library_backlink, activate_library_document_link, build_library_projection,
    inspect_library_backlinks, inspect_note_links, load_library_document,
    load_linked_library_document, render_library_markdown,
};

#[path = "support/backlinks.rs"]
mod backlink_tests;
#[path = "support/document_links.rs"]
mod document_link_tests;
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
     agent_instructions = \"Meta/AGENTS.md\"\n\
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
    assert!(projection.dashboard.validation_passed);
    assert_eq!(projection.dashboard.projects, 2);
    assert_eq!(projection.dashboard.notes, 4);
    assert_eq!(projection.dashboard.global_notes, 1);
    assert_eq!(projection.dashboard.configured_categories, 5);
    assert_eq!(projection.dashboard.open_tasks, 1);
    assert_eq!(projection.dashboard.open_problems, 0);
    assert_eq!(projection.dashboard.validated_links, 1);
    assert_eq!(projection.dashboard.latest_activity_date, None);
    assert_eq!(
        projection
            .dashboard
            .project_metrics
            .iter()
            .map(|metric| (metric.project.as_str(), metric.notes, metric.open_tasks))
            .collect::<Vec<_>>(),
        [("alpha", 2, 1), ("zeta", 1, 0)]
    );
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

#[test]
fn search_has_explicit_scope_stable_limits_and_source_hits() {
    use akasha_core::search_library;
    let fixture = fixture("search");
    let all = search_library(&fixture.request, "schema_version", None, 1).unwrap();
    assert!(all.total_matches > 1);
    assert!(all.truncated);
    assert_eq!(all.hits.len(), 1);
    assert!(all.hits[0].snippet.contains("schema_version"));
    assert_eq!(all.hits[0].line, Some(2));
    let again = search_library(&fixture.request, "schema_version", None, 100).unwrap();
    assert_eq!(all.total_matches, again.hits.len());
    assert!(again.hits.windows(2).all(|hits| hits[0].id < hits[1].id));
    let scope = LibraryScope::Project {
        project: "alpha".into(),
    };
    let project = search_library(&fixture.request, "SCHEMA_VERSION", Some(&scope), 100).unwrap();
    assert!(!project.hits.is_empty());
    assert!(project.hits.iter().all(|hit| hit.scope == scope));
    let global = search_library(
        &fixture.request,
        "SCHEMA_VERSION",
        Some(&LibraryScope::Global),
        100,
    )
    .unwrap();
    assert_eq!(global.hits.len(), 1);
    assert_eq!(global.hits[0].scope, LibraryScope::Global);
    assert!(
        search_library(&fixture.request, "no_such_text_!", None, 100)
            .unwrap()
            .hits
            .is_empty()
    );
    for query in ["", "   ", &"a".repeat(257)] {
        assert!(search_library(&fixture.request, query, None, 1).is_err());
    }
    for limit in [0, 101] {
        assert!(search_library(&fixture.request, "x", None, limit).is_err());
    }
    assert!(
        search_library(
            &fixture.request,
            "core",
            Some(&LibraryScope::Project {
                project: "absent".into()
            }),
            10
        )
        .is_err()
    );
}

#[test]
fn search_matches_unicode_title_and_exact_path_with_bounded_snippets() {
    use akasha_core::search_library;
    let fixture = fixture("search-unicode");
    // Global entities have no immutable-event fingerprint; changing a body remains valid.
    let path = fixture.root.join("Global/entities/rust-pattern.md");
    let original = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("{original}\nПРИВЕТ 世界 {}\n", "x".repeat(1000)),
    )
    .unwrap();
    let found =
        search_library(&fixture.request, "привет", Some(&LibraryScope::Global), 10).unwrap();
    assert_eq!(found.total_matches, 1);
    assert!(found.hits[0].snippet.chars().count() <= 160);
    assert_eq!(
        search_library(
            &fixture.request,
            "Global/entities/rust-pattern.md",
            None,
            10
        )
        .unwrap()
        .total_matches,
        1
    );
    assert!(
        search_library(&fixture.request, "../../", None, 10)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn library_display_fields_neutralize_controls_without_changing_source_or_identity() {
    use akasha_core::search_library;

    let fixture = fixture("control-display");
    let registry = fixture.root.join("Meta/projects.yaml");
    let registry_source = fs::read_to_string(&registry).unwrap();
    fs::write(
        &registry,
        registry_source.replace("status: active", "status: \"active\\u001b[2J\""),
    )
    .unwrap();

    let path = fixture.root.join("Global/entities/rust-pattern.md");
    let source = concat!(
        "---\n",
        "schema_version: 1\n",
        "type: entity\n",
        "entity: rust-pattern\n",
        "title: \"RED\\u001b[31m\\u001b]8;;http://evil\\u0007LINK\"\n",
        "kind: pattern\n",
        "status: \"sta\\u001bble\"\n",
        "reviewed: \"2026-07-13\\u001b]0;X\\u0007\"\n",
        "---\n\n",
        "needle \x1b[2J\n",
    );
    fs::write(&path, source).unwrap();

    let projection = build_library_projection(&fixture.request).unwrap();
    let book = &projection.global.categories[0].books[0];
    assert_eq!(book.id, "Global/entities/rust-pattern.md");
    assert_eq!(book.label, "RED [31m ]8;;http://evil LINK");
    assert_eq!(book.status.as_deref(), Some("sta ble"));
    assert_eq!(book.reviewed.as_deref(), Some("2026-07-13 ]0;X "));
    assert!(!book.explanation.chars().any(char::is_control));
    assert_eq!(projection.projects[0].status, "active [2J");
    assert_eq!(projection.dashboard.project_metrics[0].status, "active [2J");
    assert!(
        !render_library_markdown(&projection)
            .chars()
            .any(|character| character.is_control() && character != '\n')
    );

    let result =
        search_library(&fixture.request, "needle", Some(&LibraryScope::Global), 10).unwrap();
    assert_eq!(result.hits[0].id, book.id);
    assert_eq!(result.hits[0].label, book.label);
    assert_eq!(result.hits[0].snippet, "needle  [2J");
    assert!(!result.hits[0].snippet.chars().any(char::is_control));

    let document = load_library_document(&fixture.request, &book.id).unwrap();
    assert_eq!(document.source, source);
}

#[test]
fn note_links_skip_optional_projection_headers_without_canonical_schema_and_reject_invalid_source()
{
    let fixture = fixture("note-links-projection-header");
    let project = fixture.root.join("Projects/alpha");
    fs::write(project.join("index.md"),
        "---\r\nproject: alpha\r\nexample: '[[missing-header-target]]'\r\n---\r\n[[Projects/alpha/entities/core]]\r\n").unwrap();
    support::write_project_state(
        &project,
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
    let result = inspect_note_links(
        &fixture.request,
        "Projects/alpha/entities/core.md",
        NoteLinkDirection::Incoming,
        30,
    )
    .unwrap();
    assert_eq!(result.total_matches, 2);
    assert!(
        result
            .links
            .iter()
            .any(|link| link.id == "Projects/alpha/index.md")
    );
    for invalid in [
        b"\xff".as_slice(),
        b"[[]]\n".as_slice(),
        b"---\nunterminated: header\n".as_slice(),
    ] {
        fs::write(project.join("index.md"), invalid).unwrap();
        support::write_project_state(
            &project,
            &[],
            &["entities/core.md"],
            &["records/tasks/ship-mvp.md"],
        );
        let before = link_fixture_snapshot(&fixture.root);
        assert_eq!(
            inspect_note_links(
                &fixture.request,
                "Projects/alpha/entities/core.md",
                NoteLinkDirection::Incoming,
                30
            )
            .unwrap_err()
            .exit_code(),
            4
        );
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
    }
}

fn append_link_source(root: &Path, id: &str, body: &str) {
    let path = root.join(id);
    let source = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{source}{body}")).unwrap();
}

fn link_fixture_snapshot(
    directory: &Path,
) -> Vec<(PathBuf, fs::FileType, fs::Permissions, Vec<u8>)> {
    let mut paths = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    let mut result = Vec::new();
    for path in paths {
        let metadata = fs::symlink_metadata(&path).unwrap();
        let bytes = if metadata.file_type().is_symlink() {
            fs::read_link(&path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if metadata.is_file() {
            fs::read(&path).unwrap()
        } else {
            Vec::new()
        };
        result.push((
            path.clone(),
            metadata.file_type(),
            metadata.permissions(),
            bytes,
        ));
        if metadata.is_dir() {
            result.extend(link_fixture_snapshot(&path));
        }
    }
    result
}

#[test]
fn note_links_cover_all_canonical_classes_projects_global_and_projections_without_writes() {
    let fixture = fixture("note-links-coverage");
    let target = "Projects/alpha/entities/core.md";
    for (id, body) in [
        (
            target,
            "[[#Local]] [[Projects/alpha/entities/core.md#^block]]\n",
        ),
        (
            "Projects/alpha/index.md",
            "[[Projects/alpha/entities/core|Core]] [[Projects/alpha/entities/core.md]]\n",
        ),
        (
            "Projects/alpha/records/tasks/ship-mvp.md",
            "[[Projects/alpha/entities/core]]\n",
        ),
        (
            "Projects/zeta/entities/renderer.md",
            "[[Projects/alpha/entities/core]]\n",
        ),
        (
            "Projects/zeta/roadmap.md",
            "![[Projects/alpha/entities/core#Heading]]\n",
        ),
    ] {
        append_link_source(&fixture.root, id, body);
    }
    fs::write(fixture.root.join("Projects/alpha/events/sessions/touch.md"),
        "---\nschema_version: 1\nproject: alpha\ntype: session\ndate: 2026-10-12\n---\n[[Projects/alpha/entities/core]]\n").unwrap();
    support::write_project_state(
        &fixture.root.join("Projects/alpha"),
        &["events/sessions/touch.md"],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
    support::write_project_state(
        &fixture.root.join("Projects/zeta"),
        &[],
        &["entities/renderer.md"],
        &[],
    );
    let before = link_fixture_snapshot(&fixture.root);
    let incoming =
        inspect_note_links(&fixture.request, target, NoteLinkDirection::Incoming, 100).unwrap();
    assert_eq!(incoming.scanned_canonical_notes, 5);
    assert_eq!(incoming.scanned_projections, 4);
    assert_eq!(incoming.total_matches, 7);
    assert!(!incoming.truncated);
    assert_eq!(
        incoming
            .links
            .iter()
            .map(|link| link.id.as_str())
            .collect::<Vec<_>>(),
        [
            "Global/entities/rust-pattern.md",
            target,
            "Projects/alpha/events/sessions/touch.md",
            "Projects/alpha/index.md",
            "Projects/alpha/records/tasks/ship-mvp.md",
            "Projects/zeta/entities/renderer.md",
            "Projects/zeta/roadmap.md",
        ]
    );
    assert_eq!(incoming.links[3].kind, NoteLinkKind::Projection);
    let limited =
        inspect_note_links(&fixture.request, target, NoteLinkDirection::Incoming, 1).unwrap();
    assert_eq!(limited.total_matches, 7);
    assert!(limited.truncated);
    assert_eq!(limited.links, incoming.links[..1]);
    let outgoing = inspect_note_links(
        &fixture.request,
        "Projects/alpha/index.md",
        NoteLinkDirection::Outgoing,
        30,
    )
    .unwrap();
    assert_eq!(outgoing.links.len(), 1);
    assert_eq!(outgoing.links[0].id, target);
    let self_link =
        inspect_note_links(&fixture.request, target, NoteLinkDirection::Outgoing, 30).unwrap();
    assert_eq!(self_link.links, outgoing.links);
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn note_links_ignore_nonsemantic_regions_and_preserve_exact_unicode_identities() {
    let fixture = fixture("note-links-syntax");
    let id = "Global/entities/двигатель.md";
    fs::write(fixture.root.join(id),
        "---\nschema_version: 1\nentity: двигатель\nkind: pattern\nstatus: stable\nreviewed: 2026-10-12\nexample: '[[missing-frontmatter]]'\n---\n[[Projects/alpha/entities/core|Локальный]]\n\
         `[[missing-inline]]`\n```md\n[[missing-fence]]\n```\n\
         <!--\n[[missing-html]]\n-->\n%%\n[[missing-percent]]\n%%\n\\[[missing-escaped]]\n").unwrap();
    let result = inspect_note_links(&fixture.request, id, NoteLinkDirection::Outgoing, 30).unwrap();
    assert_eq!(result.note, id);
    assert_eq!(result.links.len(), 1);
    assert_eq!(result.links[0].id, "Projects/alpha/entities/core.md");
    let zero = inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30).unwrap();
    assert_eq!(zero.total_matches, 0);
    assert!(!zero.truncated);
    assert!(zero.links.is_empty());
    // Noncanonical Markdown can be a valid outgoing target but never expands source coverage.
    fs::write(fixture.root.join("Inbox/other.md"), format!("[[{id}]]\n")).unwrap();
    append_link_source(&fixture.root, id, "![[Inbox/other]]\n");
    let result = inspect_note_links(&fixture.request, id, NoteLinkDirection::Outgoing, 30).unwrap();
    assert_eq!(result.links[0].id, "Inbox/other.md");
    assert_eq!(result.links[0].kind, NoteLinkKind::Other);
    assert_eq!(
        inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30)
            .unwrap()
            .total_matches,
        0
    );
    assert_eq!(
        inspect_note_links(
            &fixture.request,
            "Inbox/other.md",
            NoteLinkDirection::Incoming,
            30
        )
        .unwrap_err()
        .exit_code(),
        4
    );
}

#[test]
fn note_links_use_configured_folders_and_projection_filenames() {
    let fixture = fixture("note-links-configured");
    fs::write(
        fixture.root.join("akasha.toml"),
        root_config()
            .replace("projects = \"Projects\"", "projects = \"Workspaces\"")
            .replace("global = \"Global\"", "global = \"Shared\"")
            .replace("folder = \"entities\"", "folder = \"knowledge\"")
            .replace("index = \"index.md\"", "index = \"maps/catalog.md\"")
            .replace("roadmap = \"roadmap.md\"", "roadmap = \"maps/plan.md\""),
    )
    .unwrap();
    fs::rename(
        fixture.root.join("Global/entities"),
        fixture.root.join("Global/knowledge"),
    )
    .unwrap();
    fs::rename(fixture.root.join("Global"), fixture.root.join("Shared")).unwrap();
    fs::write(
        fixture.root.join("Shared/knowledge/rust-pattern.md"),
        fs::read_to_string(fixture.root.join("Shared/knowledge/rust-pattern.md"))
            .unwrap()
            .replace(
                "Projects/alpha/entities/core",
                "Workspaces/alpha/knowledge/core",
            ),
    )
    .unwrap();
    for (project, entity, records) in [
        ("alpha", "core", vec!["records/tasks/ship-mvp.md"]),
        ("zeta", "renderer", vec![]),
    ] {
        let directory = fixture.root.join("Projects").join(project);
        fs::rename(directory.join("entities"), directory.join("knowledge")).unwrap();
        fs::write(
            directory.join("index.md"),
            "[[Workspaces/alpha/knowledge/core]]\n",
        )
        .unwrap();
        support::write_project_state(
            &directory,
            &[],
            &[&format!("knowledge/{entity}.md")],
            &records,
        );
        fs::create_dir(directory.join("maps")).unwrap();
        fs::rename(
            directory.join("index.md"),
            directory.join("maps/catalog.md"),
        )
        .unwrap();
        fs::rename(directory.join("roadmap.md"), directory.join("maps/plan.md")).unwrap();
    }
    fs::rename(
        fixture.root.join("Projects"),
        fixture.root.join("Workspaces"),
    )
    .unwrap();
    let result = inspect_note_links(
        &fixture.request,
        "Workspaces/alpha/knowledge/core.md",
        NoteLinkDirection::Incoming,
        30,
    )
    .unwrap();
    assert_eq!(
        result
            .links
            .iter()
            .map(|link| link.id.as_str())
            .collect::<Vec<_>>(),
        [
            "Shared/knowledge/rust-pattern.md",
            "Workspaces/alpha/maps/catalog.md",
            "Workspaces/zeta/maps/catalog.md",
        ]
    );
    assert_eq!(result.scanned_projections, 4);
    assert!(
        inspect_note_links(
            &fixture.request,
            "Workspaces/alpha/maps/plan.md",
            NoteLinkDirection::Outgoing,
            30
        )
        .unwrap()
        .links
        .is_empty()
    );
}

#[test]
fn note_links_refuse_unknown_unsafe_noncanonical_and_invalid_limits_without_writes() {
    let fixture = fixture("note-links-invalid");
    let before = link_fixture_snapshot(&fixture.root);
    for id in [
        "",
        "core.md",
        "/Projects/alpha/entities/core.md",
        "../core.md",
        "Projects//alpha/entities/core.md",
        "Projects/alpha/./entities/core.md",
        "Projects\\alpha\\entities\\core.md",
        "Projects/alpha/entities/core",
        "Projects/alpha/entities/core.md#Heading",
        "Projects/alpha/entities/core.md|Core",
        "Projects/alpha/entities/Core.md",
        "Projects/alpha/entities/missing.md",
        "Meta/projects.yaml",
        "Projects/alpha/templates/entity.md",
    ] {
        assert_eq!(
            inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30)
                .unwrap_err()
                .exit_code(),
            4,
            "{id}"
        );
    }
    for limit in [0, 101, usize::MAX] {
        assert_eq!(
            inspect_note_links(
                &fixture.request,
                "Projects/alpha/entities/core.md",
                NoteLinkDirection::Outgoing,
                limit
            )
            .unwrap_err()
            .exit_code(),
            4
        );
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn note_links_revalidate_stale_projections_missing_targets_and_pending_journals() {
    let fixture = fixture("note-links-stale");
    let id = "Projects/alpha/entities/core.md";
    assert!(inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30).is_ok());
    append_link_source(&fixture.root, "Projects/alpha/index.md", "external draft\n");
    let before = link_fixture_snapshot(&fixture.root);
    let error =
        inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("project state"));
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    support::write_project_state(
        &fixture.root.join("Projects/alpha"),
        &[],
        &["entities/core.md"],
        &["records/tasks/ship-mvp.md"],
    );
    for journal_project in ["alpha", "zeta"] {
        let journal = fixture
            .root
            .join("Projects")
            .join(journal_project)
            .join(akasha_core::NOTE_EDIT_JOURNAL_FILE);
        fs::write(&journal, b"unfinished journal, never parsed or recovered").unwrap();
        let before = link_fixture_snapshot(&fixture.root);
        let error =
            inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30).unwrap_err();
        assert_eq!(error.exit_code(), 5);
        assert!(error.to_string().contains("pending note recovery journal"));
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
        fs::remove_file(journal).unwrap();
    }
    append_link_source(
        &fixture.root,
        "Projects/zeta/roadmap.md",
        "[[Projects/alpha/entities/missing]]\n",
    );
    support::write_project_state(
        &fixture.root.join("Projects/zeta"),
        &[],
        &["entities/renderer.md"],
        &[],
    );
    let before = link_fixture_snapshot(&fixture.root);
    let error =
        inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("invalid wikilink"));
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[cfg(unix)]
#[test]
fn note_links_refuse_symlink_sources_targets_and_journals_without_changes() {
    use std::os::unix::fs::symlink;
    let fixture = fixture("note-links-symlinks");
    let id = "Projects/alpha/entities/core.md";
    for (entry, target) in [
        ("Projects/zeta/index.md", "../alpha/index.md"),
        (
            "Global/entities/rust-pattern.md",
            "../../Projects/alpha/entities/core.md",
        ),
        (
            "Projects/zeta/.akasha-edit-journal.json",
            "../alpha/index.md",
        ),
    ] {
        let path = fixture.root.join(entry);
        let original = fs::read(&path).ok();
        if original.is_some() {
            fs::remove_file(&path).unwrap();
        }
        symlink(target, &path).unwrap();
        let before = link_fixture_snapshot(&fixture.root);
        assert_eq!(
            inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
        fs::remove_file(&path).unwrap();
        if let Some(original) = original {
            fs::write(path, original).unwrap();
        }
    }
    let external = fixture.root.join("Inbox/alias.md");
    symlink("../Projects/alpha/entities/core.md", &external).unwrap();
    append_link_source(
        &fixture.root,
        "Global/entities/rust-pattern.md",
        "[[Inbox/alias]]\n",
    );
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        inspect_note_links(&fixture.request, id, NoteLinkDirection::Incoming, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}
