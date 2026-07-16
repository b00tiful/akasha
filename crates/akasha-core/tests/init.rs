use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use akasha_core::{
    InitRecovery, InitRequest, ResolutionEnvironment, ResolveRequest, initialize_project,
    parse_project_registry, resolve_project, validate_project,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn creates_configured_scaffold_exact_templates_registry_and_pointer() {
    let fixture = Fixture::new("complete");
    fs::create_dir_all(fixture.root.join("defaults/nested/empty"))
        .expect("create nested templates");
    fs::write(
        fixture.root.join("defaults/session.md"),
        b"session template\r\n",
    )
    .expect("write template");
    fs::write(
        fixture.root.join("defaults/nested/raw.bin"),
        [0_u8, 1, 2, 255],
    )
    .expect("write binary template");

    let result = initialize_project(&fixture.request()).expect("initialize project");

    assert_eq!(result.project, "example");
    assert_eq!(result.template_files, 2);
    assert_eq!(result.recovery, InitRecovery::None);
    assert_eq!(
        result.project_dir,
        fixture.root.join("ProjectMemory/example")
    );
    assert_eq!(
        fs::read(result.project_dir.join("overrides/session.md")).expect("read copied template"),
        b"session template\r\n"
    );
    assert_eq!(
        fs::read(result.project_dir.join("overrides/nested/raw.bin"))
            .expect("read binary template"),
        [0_u8, 1, 2, 255]
    );
    assert!(result.project_dir.join("overrides/nested/empty").is_dir());
    for directory in [
        "history/sessions",
        "history/handoffs",
        "work/tasks",
        "work/problems",
        "knowledge",
    ] {
        assert!(
            result.project_dir.join(directory).is_dir(),
            "missing {directory}"
        );
    }
    assert_eq!(
        fs::read(result.project_dir.join("catalog.md")).expect("read index"),
        b""
    );
    assert_eq!(
        fs::read(result.project_dir.join("plan.md")).expect("read roadmap"),
        b""
    );
    assert_eq!(result.state, result.project_dir.join(".akasha-state.toml"));
    assert!(result.state.is_file());
    assert!(result.project_dir.join(".akasha-write.lock").is_file());
    assert_eq!(
        fs::read(&result.pointer).expect("read pointer"),
        b"schema_version = 1\nproject = \"example\"\n"
    );

    let registry_source = fs::read_to_string(&result.registry).expect("read initialized registry");
    let registry = parse_project_registry(&registry_source).expect("parse initialized registry");
    assert_eq!(registry.projects.len(), 1);
    assert_eq!(registry.projects["example"].path, result.repository_dir);
    assert_eq!(registry.projects["example"].status, "active");

    let nested = fixture.repository.join("nested");
    fs::create_dir(&nested).expect("create nested repository directory");
    let resolve_request = ResolveRequest {
        root_override: Some(fixture.root.clone()),
        project_override: None,
        cwd: nested,
        environment: ResolutionEnvironment::default(),
    };
    let resolved = resolve_project(&resolve_request).expect("resolve initialized pointer");
    assert_eq!(resolved.project, "example");
    let report = validate_project(&resolve_request).expect("validate empty initialized project");
    assert_eq!(report.canonical_notes, 0);
    assert_eq!(report.immutable_events, 0);
    assert_eq!(report.projections["index"].sources, 0);
    assert_eq!(report.projections["roadmap"].sources, 0);
}

#[test]
fn existing_pointer_conflict_preserves_all_existing_state() {
    let fixture = Fixture::new("pointer-conflict");
    let pointer = fixture.repository.join(".akasha.toml");
    fs::write(&pointer, b"human-owned\n").expect("seed pointer");
    let registry = fixture.root.join("Meta/projects.yaml");
    let before = fs::read(&registry).expect("read registry before init");

    let error = initialize_project(&fixture.request()).expect_err("pointer must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(fs::read(pointer).expect("read pointer"), b"human-owned\n");
    assert_eq!(fs::read(registry).expect("read registry"), before);
    assert!(!fixture.root.join("ProjectMemory/example").exists());
}

#[test]
fn concurrent_initializers_have_one_winner_without_losing_registry_state() {
    let fixture = Fixture::new("concurrent");
    let request = Arc::new(fixture.request());
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let request = Arc::clone(&request);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                initialize_project(&request)
            })
        })
        .collect::<Vec<_>>();

    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("join initializer"))
        .collect::<Vec<_>>();

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("one initializer must fail");
    assert_eq!(error.exit_code(), 5);
    let registry = parse_project_registry(
        &fs::read_to_string(fixture.root.join("Meta/projects.yaml")).expect("read registry"),
    )
    .expect("parse registry");
    assert_eq!(registry.projects.len(), 1);
    assert!(registry.projects.contains_key("example"));
}

#[cfg(unix)]
#[test]
fn rejects_template_symlinks_before_writing() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new("template-symlink");
    fs::write(fixture.root.join("outside.md"), "outside\n").expect("write outside file");
    symlink(
        fixture.root.join("outside.md"),
        fixture.root.join("defaults/link.md"),
    )
    .expect("create template symlink");

    let error = initialize_project(&fixture.request()).expect_err("symlink must fail");

    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("must not contain symlinks"));
    assert!(!fixture.root.join("ProjectMemory/example").exists());
    assert!(!fixture.repository.join(".akasha.toml").exists());
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    repository: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let temp = TempDir::new(label);
        let root = temp.path().join("root");
        let repository = temp.path().join("repository");
        for directory in [
            root.join("Meta"),
            root.join("defaults"),
            root.join("Shared"),
            root.join("ProjectMemory"),
            root.join("Capture"),
            repository.clone(),
        ] {
            fs::create_dir_all(directory).expect("create fixture directory");
        }
        fs::write(root.join("akasha.toml"), root_config()).expect("write root config");
        fs::write(
            root.join("Meta/projects.yaml"),
            "# Product-managed registry.\n{}\n",
        )
        .expect("write empty registry");
        Self {
            _temp: temp,
            root,
            repository,
        }
    }

    fn request(&self) -> InitRequest {
        InitRequest {
            root_override: Some(self.root.clone()),
            project: "example".to_owned(),
            cwd: self.repository.clone(),
            environment: ResolutionEnvironment::default(),
        }
    }
}

fn root_config() -> &'static str {
    "schema_version = 1\n\
     \n\
     [files]\n\
     registry = \"Meta/projects.yaml\"\n\
     \n\
     [folders]\n\
     templates = \"defaults\"\n\
     global = \"Shared\"\n\
     projects = \"ProjectMemory\"\n\
     inbox = \"Capture\"\n\
     \n\
     [context]\n\
     tasks = \"task\"\n\
     problems = \"problem\"\n\
     handoffs = \"handoff\"\n\
     recent_events = [\"session\"]\n\
     open_statuses = [\"open\", \"active\"]\n\
     \n\
     [project]\n\
     templates = \"overrides\"\n\
     index = \"catalog.md\"\n\
     roadmap = \"plan.md\"\n\
     \n\
     [project.note_types.session]\n\
     class = \"event\"\n\
     folder = \"history/sessions\"\n\
     required_fields = [\"project\", \"type\", \"date\"]\n\
     \n\
     [project.note_types.handoff]\n\
     class = \"event\"\n\
     folder = \"history/handoffs\"\n\
     required_fields = [\"project\", \"type\", \"date\"]\n\
     \n\
     [project.note_types.task]\n\
     class = \"record\"\n\
     folder = \"work/tasks\"\n\
     required_fields = [\"project\", \"type\", \"status\"]\n\
     \n\
     [project.note_types.problem]\n\
     class = \"record\"\n\
     folder = \"work/problems\"\n\
     required_fields = [\"project\", \"type\", \"status\"]\n\
     \n\
     [project.note_types.entity]\n\
     class = \"entity\"\n\
     folder = \"knowledge\"\n\
     required_fields = [\"entity\", \"kind\", \"status\", \"reviewed\"]\n"
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("akasha-init-{label}-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("create temporary directory");
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
