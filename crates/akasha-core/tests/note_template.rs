use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    NoteClass, NoteTemplateScope, ResolutionEnvironment, ResolveRequest, resolve_note_template,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    project_templates: PathBuf,
    request: ResolveRequest,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temp = TempDir::copy_of(label, &fixtures());
        let root = temp.path().join("valid-root");
        let project_templates = root.join("Projects/example/templates");
        let request = ResolveRequest {
            root_override: Some(root.clone()),
            project_override: None,
            cwd: temp.path().join("repository/nested"),
            environment: ResolutionEnvironment::default(),
        };

        Self {
            _temp: temp,
            root,
            project_templates,
            request,
        }
    }
}

#[test]
fn project_template_precedes_root_template_and_preserves_exact_source() {
    let fixture = Fixture::new("project-precedence");
    let project_source = "project\r\n{{ exact }}\r\n";
    fs::write(fixture.project_templates.join("session.md"), project_source)
        .expect("write project template");
    fs::write(fixture.root.join("templates/session.md"), "root template\n")
        .expect("write root template");

    let template = resolve_note_template(&fixture.request, "session").expect("resolve template");

    assert_eq!(template.note_type, "session");
    assert_eq!(template.class, NoteClass::Event);
    assert_eq!(template.scope, NoteTemplateScope::Project);
    assert_eq!(template.path, fixture.project_templates.join("session.md"));
    assert_eq!(template.source, project_source);
}

#[test]
fn missing_project_template_falls_back_to_root_template() {
    let fixture = Fixture::new("root-fallback");
    fs::write(fixture.root.join("templates/entity.md"), "root entity\n")
        .expect("write root template");

    let template = resolve_note_template(&fixture.request, "entity").expect("resolve template");

    assert_eq!(template.class, NoteClass::Entity);
    assert_eq!(template.scope, NoteTemplateScope::Root);
    assert_eq!(template.path, fixture.root.join("templates/entity.md"));
    assert_eq!(template.source, "root entity\n");
}

#[test]
fn rejects_unknown_and_missing_configured_templates() {
    let fixture = Fixture::new("missing");

    let unknown = resolve_note_template(&fixture.request, "unknown")
        .expect_err("unknown note type must fail");
    assert_eq!(unknown.exit_code(), 2);
    assert!(unknown.to_string().contains("unknown configured note type"));

    let missing =
        resolve_note_template(&fixture.request, "task").expect_err("missing template must fail");
    assert_eq!(missing.exit_code(), 4);
    assert!(missing.to_string().contains("no project template"));
    assert!(missing.to_string().contains("no root template"));
}

#[test]
fn existing_invalid_override_does_not_silently_fall_back() {
    let fixture = Fixture::new("invalid-override");
    fs::create_dir(fixture.project_templates.join("problem.md"))
        .expect("create invalid project template");
    fs::write(
        fixture.root.join("templates/problem.md"),
        "valid root template\n",
    )
    .expect("write root template");

    let error = resolve_note_template(&fixture.request, "problem")
        .expect_err("invalid project override must fail");

    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("not a regular file"));
    assert!(
        error
            .to_string()
            .contains("Projects/example/templates/problem.md")
    );
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_template() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("symlink");
    let target = fixture.root.join("templates/session.md");
    fs::write(&target, "root template\n").expect("write root template");
    symlink(&target, fixture.project_templates.join("session.md"))
        .expect("create project template symlink");

    let error = resolve_note_template(&fixture.request, "session")
        .expect_err("symlinked template must fail");

    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("must not be a symbolic link"));
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_template_directory() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("symlink-directory");
    fs::remove_dir_all(&fixture.project_templates).expect("remove project template directory");
    symlink(fixture.root.join("templates"), &fixture.project_templates)
        .expect("create project template directory symlink");

    let error = resolve_note_template(&fixture.request, "session")
        .expect_err("symlinked template directory must fail");

    assert_eq!(error.exit_code(), 4);
    assert!(
        error
            .to_string()
            .contains("template directory must not be a symbolic link")
    );
}

struct TempDir(PathBuf);

impl TempDir {
    fn copy_of(label: &str, source: &Path) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-note-template-{label}-{}-{id}",
            std::process::id()
        ));
        copy_directory(source, &path);
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

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture type").is_dir() {
            copy_directory(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("copy fixture file");
        }
    }
}
