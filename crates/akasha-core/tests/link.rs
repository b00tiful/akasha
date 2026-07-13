use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    LinkRequest, ResolutionEnvironment, ResolveRequest, link_project, resolve_project,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/resolution/valid-root/akasha.toml")
}

#[test]
fn creates_canonical_pointer_that_the_resolver_accepts() {
    let temp = TempDir::new("resolve-compatible");
    let (root, repository) = setup_registered_project(temp.path());

    let result = link_project(&request(&root, &repository, None)).expect("link project");

    assert_eq!(result.project, "example");
    assert_eq!(result.pointer, repository.join(".akasha.toml"));
    assert_eq!(
        fs::read(&result.pointer).expect("read pointer"),
        b"schema_version = 1\nproject = \"example\"\n"
    );
    assert!(
        fs::read_dir(&repository)
            .expect("read repository")
            .all(|entry| !entry
                .expect("read repository entry")
                .file_name()
                .to_string_lossy()
                .contains(".akasha-"))
    );

    let resolved = resolve_project(&ResolveRequest {
        root_override: Some(root),
        project_override: None,
        cwd: repository.join("nested"),
        environment: ResolutionEnvironment::default(),
    })
    .expect("resolve created pointer");
    assert_eq!(resolved.project, "example");
    assert_eq!(resolved.pointer, Some(result.pointer));
}

#[test]
fn resolves_an_explicit_relative_repository_from_the_working_directory() {
    let temp = TempDir::new("relative-repository");
    let (root, repository) = setup_registered_project(temp.path());
    let request = LinkRequest {
        root_override: Some(root),
        project: "example".to_owned(),
        repository: Some(PathBuf::from("repository")),
        cwd: temp.path().to_path_buf(),
        environment: ResolutionEnvironment::default(),
    };

    let result = link_project(&request).expect("link relative repository");

    assert_eq!(result.repository_dir, repository);
    assert!(result.pointer.is_file());
}

#[test]
fn rejects_a_repository_that_does_not_match_the_registry() {
    let temp = TempDir::new("repository-mismatch");
    let (root, repository) = setup_registered_project(temp.path());
    let other = temp.path().join("other");
    fs::create_dir(&other).expect("create other repository");

    let error = link_project(&request(&root, &repository, Some(other.clone())))
        .expect_err("repository mismatch must fail");

    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("does not match registry entry"));
    assert!(!other.join(".akasha.toml").exists());
    assert!(!repository.join(".akasha.toml").exists());
}

#[test]
fn preserves_an_existing_pointer_as_a_conflict() {
    let temp = TempDir::new("existing-pointer");
    let (root, repository) = setup_registered_project(temp.path());
    let pointer = repository.join(".akasha.toml");
    fs::write(&pointer, b"human-owned content\n").expect("seed pointer");

    let error = link_project(&request(&root, &repository, None))
        .expect_err("existing pointer must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(
        fs::read(&pointer).expect("read preserved pointer"),
        b"human-owned content\n"
    );
}

#[test]
fn preserves_registry_validation_exit_class() {
    let temp = TempDir::new("invalid-registry");
    let (root, repository) = setup_registered_project(temp.path());
    fs::write(
        root.join("Meta/projects.yaml"),
        "example:\n  path: ../../repository\n  status: active\nexample:\n  path: ../../other\n  status: active\n",
    )
    .expect("write duplicate registry");

    let error =
        link_project(&request(&root, &repository, None)).expect_err("invalid registry must fail");

    assert_eq!(error.exit_code(), 4);
    assert!(!repository.join(".akasha.toml").exists());
}

fn request(root: &Path, repository: &Path, selected: Option<PathBuf>) -> LinkRequest {
    LinkRequest {
        root_override: Some(root.to_path_buf()),
        project: "example".to_owned(),
        repository: selected,
        cwd: repository.to_path_buf(),
        environment: ResolutionEnvironment::default(),
    }
}

fn setup_registered_project(base: &Path) -> (PathBuf, PathBuf) {
    let root = base.join("valid-root");
    let repository = base.join("repository");
    fs::create_dir_all(root.join("Meta")).expect("create registry directory");
    fs::create_dir_all(root.join("Projects/example")).expect("create project directory");
    fs::create_dir_all(repository.join("nested")).expect("create repository");
    fs::copy(fixture_root_config(), root.join("akasha.toml")).expect("copy root config");
    fs::write(
        root.join("Meta/projects.yaml"),
        "example:\n  path: ../../repository\n  status: active\n",
    )
    .expect("write registry");
    (root, repository)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("akasha-link-{label}-{}-{id}", std::process::id()));
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
