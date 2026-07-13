use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    ProjectSource, ResolutionEnvironment, ResolveRequest, RootSource, resolve_project,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("akasha-{name}-{}-{id}", std::process::id()));
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

fn create_root(base: &Path, name: &str, project: &str) -> PathBuf {
    let root = base.join(name);
    fs::create_dir_all(root.join("Projects").join(project)).expect("create project");
    fs::write(
        root.join("akasha.toml"),
        "schema_version = 1\n\n[folders]\nprojects = \"Projects\"\n",
    )
    .expect("write root config");
    root
}

fn request(cwd: &Path) -> ResolveRequest {
    ResolveRequest {
        root_override: None,
        project_override: None,
        cwd: cwd.to_path_buf(),
        environment: ResolutionEnvironment::default(),
    }
}

#[test]
fn uses_command_line_root_before_environment_and_user_config() {
    let temp = TempDir::new("root-precedence");
    let command_line_root = create_root(temp.path(), "command", "example");
    let environment_root = create_root(temp.path(), "environment", "example");
    let config_root = create_root(temp.path(), "config", "example");
    let config_home = temp.path().join("xdg");
    fs::create_dir_all(config_home.join("akasha")).expect("create config directory");
    fs::write(
        config_home.join("akasha/config.toml"),
        format!(
            "schema_version = 1\nroot = {:?}\n",
            config_root.to_string_lossy()
        ),
    )
    .expect("write user config");

    let mut request = request(temp.path());
    request.root_override = Some(command_line_root.clone());
    request.project_override = Some("example".to_owned());
    request.environment = ResolutionEnvironment {
        akasha_root: Some(environment_root.into_os_string()),
        xdg_config_home: Some(config_home.into_os_string()),
        home: None,
    };

    let resolved = resolve_project(&request).expect("resolve command-line root");
    assert_eq!(resolved.root_source, RootSource::CommandLine);
    assert_eq!(
        resolved.root,
        fs::canonicalize(command_line_root).expect("canonical command-line root")
    );
}

#[test]
fn uses_environment_root_when_no_command_line_root_exists() {
    let temp = TempDir::new("environment-root");
    let root = create_root(temp.path(), "root", "example");
    let mut request = request(temp.path());
    request.project_override = Some("example".to_owned());
    request.environment.akasha_root = Some(root.clone().into_os_string());

    let resolved = resolve_project(&request).expect("resolve environment root");
    assert_eq!(resolved.root_source, RootSource::Environment);
    assert_eq!(
        resolved.root,
        fs::canonicalize(root).expect("canonical root")
    );
}

#[test]
fn resolves_relative_command_line_roots_from_the_requested_working_directory() {
    let temp = TempDir::new("relative-root");
    let root = create_root(temp.path(), "root", "example");
    let mut request = request(temp.path());
    request.root_override = Some(PathBuf::from("root"));
    request.project_override = Some("example".to_owned());

    let resolved = resolve_project(&request).expect("resolve relative command-line root");
    assert_eq!(
        resolved.root,
        fs::canonicalize(root).expect("canonical root")
    );
}

#[test]
fn falls_back_to_xdg_user_config_and_resolves_relative_root() {
    let temp = TempDir::new("user-config-root");
    let config_home = temp.path().join("xdg");
    let config_dir = config_home.join("akasha");
    let root = create_root(&config_dir, "data", "example");
    fs::write(
        config_dir.join("config.toml"),
        "schema_version = 1\nroot = \"data\"\n",
    )
    .expect("write user config");

    let mut request = request(temp.path());
    request.project_override = Some("example".to_owned());
    request.environment.xdg_config_home = Some(config_home.into_os_string());

    let resolved = resolve_project(&request).expect("resolve user-config root");
    assert_eq!(resolved.root_source, RootSource::UserConfig);
    assert_eq!(
        resolved.root,
        fs::canonicalize(root).expect("canonical root")
    );
}

#[test]
fn finds_the_nearest_pointer_from_a_nested_directory() {
    let temp = TempDir::new("nearest-pointer");
    let root = create_root(temp.path(), "root", "nearest");
    let repository = temp.path().join("repo");
    let nested = repository.join("a/b");
    fs::create_dir_all(&nested).expect("create nested repository");
    fs::write(
        repository.join(".akasha.toml"),
        "schema_version = 1\nproject = \"nearest\"\n",
    )
    .expect("write parent pointer");
    fs::write(
        temp.path().join(".akasha.toml"),
        "schema_version = 1\nproject = \"farther\"\n",
    )
    .expect("write farther pointer");

    let mut request = request(&nested);
    request.root_override = Some(root);
    let resolved = resolve_project(&request).expect("resolve nearest pointer");

    assert_eq!(resolved.project, "nearest");
    assert_eq!(resolved.project_source, ProjectSource::Pointer);
    assert_eq!(resolved.pointer, Some(repository.join(".akasha.toml")));
}

#[test]
fn command_line_project_takes_precedence_without_reading_a_pointer() {
    let temp = TempDir::new("project-precedence");
    let root = create_root(temp.path(), "root", "explicit");
    fs::write(
        temp.path().join(".akasha.toml"),
        "this is deliberately invalid toml",
    )
    .expect("write invalid pointer");

    let mut request = request(temp.path());
    request.root_override = Some(root);
    request.project_override = Some("explicit".to_owned());
    let resolved = resolve_project(&request).expect("resolve explicit project");

    assert_eq!(resolved.project, "explicit");
    assert_eq!(resolved.project_source, ProjectSource::CommandLine);
    assert_eq!(resolved.pointer, None);
}

#[test]
fn rejects_malformed_pointers() {
    let temp = TempDir::new("malformed-pointer");
    let root = create_root(temp.path(), "root", "example");
    fs::write(
        temp.path().join(".akasha.toml"),
        "schema_version = 1\nproject = [",
    )
    .expect("write malformed pointer");

    let mut request = request(temp.path());
    request.root_override = Some(root);
    let error = resolve_project(&request).expect_err("malformed pointer must fail");

    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("invalid project pointer"));
}

#[test]
fn rejects_parent_traversal_in_root_folder_config() {
    let temp = TempDir::new("folder-traversal");
    let root = temp.path().join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(
        root.join("akasha.toml"),
        "schema_version = 1\n\n[folders]\nprojects = \"../outside\"\n",
    )
    .expect("write malicious root config");

    let mut request = request(temp.path());
    request.root_override = Some(root);
    request.project_override = Some("example".to_owned());
    let error = resolve_project(&request).expect_err("traversal must fail");

    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("without parent traversal"));
}

#[cfg(unix)]
#[test]
fn rejects_project_symlinks_that_escape_the_data_root() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new("symlink-escape");
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("Projects")).expect("create projects folder");
    fs::create_dir_all(&outside).expect("create outside folder");
    fs::write(
        root.join("akasha.toml"),
        "schema_version = 1\n\n[folders]\nprojects = \"Projects\"\n",
    )
    .expect("write root config");
    symlink(&outside, root.join("Projects/escaped")).expect("create escaping symlink");

    let mut request = request(temp.path());
    request.root_override = Some(root);
    request.project_override = Some("escaped".to_owned());
    let error = resolve_project(&request).expect_err("symlink escape must fail");

    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("escapes data root"));
}

#[test]
fn rejects_missing_resolution_inputs_instead_of_guessing() {
    let temp = TempDir::new("missing-inputs");
    let request = ResolveRequest {
        root_override: None,
        project_override: None,
        cwd: temp.path().to_path_buf(),
        environment: ResolutionEnvironment {
            akasha_root: None,
            xdg_config_home: None,
            home: None,
        },
    };

    let error = resolve_project(&request).expect_err("missing root must fail");
    assert_eq!(error.exit_code(), 3);
    assert!(error.to_string().contains("no data root was provided"));
}

#[test]
fn fixture_contract_resolves_without_process_environment() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution");
    let mut request = request(&fixture.join("repository/nested"));
    request.root_override = Some(fixture.join("valid-root"));

    let resolved = resolve_project(&request).expect("resolve checked-in fixture");
    assert_eq!(resolved.project, "example");
    assert_eq!(resolved.project_source, ProjectSource::Pointer);
}
