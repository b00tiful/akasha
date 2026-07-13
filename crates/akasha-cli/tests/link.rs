use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/resolution/valid-root/akasha.toml")
}

#[test]
fn link_prints_plain_text_and_enables_pointer_resolution() {
    let temp = TempDir::new("plain");
    let (root, repository) = setup_registered_project(temp.path());

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&root),
            "--project",
            "example",
            "link",
            "example",
        ])
        .current_dir(&repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha link");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("linked: example\n"));
    assert!(stdout.contains(&format!(
        "pointer: {}",
        repository.join(".akasha.toml").display()
    )));

    let resolved = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", path(&root), "resolve"])
        .current_dir(repository.join("nested"))
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("resolve linked project");
    assert!(resolved.status.success());
    assert!(resolved.stderr.is_empty());
    let stdout = String::from_utf8(resolved.stdout).expect("resolve stdout is UTF-8");
    assert!(stdout.contains("project: example"));
    assert!(stdout.contains("project source: pointer"));
}

#[test]
fn link_json_supports_an_explicit_relative_repository() {
    let temp = TempDir::new("json");
    let (root, repository) = setup_registered_project(temp.path());

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&root),
            "--json",
            "link",
            "example",
            "--repo",
            "repository",
        ])
        .current_dir(temp.path())
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha link --json");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse link JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(
        value["repository_dir"],
        fs::canonicalize(repository)
            .expect("canonical repository")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        fs::read(temp.path().join("repository/.akasha.toml")).expect("read pointer"),
        b"schema_version = 1\nproject = \"example\"\n"
    );
}

#[test]
fn link_conflicts_use_stderr_and_exit_code_five() {
    let temp = TempDir::new("conflict");
    let (root, repository) = setup_registered_project(temp.path());
    let pointer = repository.join(".akasha.toml");
    fs::write(&pointer, b"human content\n").expect("seed pointer");

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", path(&root), "link", "example"])
        .current_dir(&repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run conflicting link");

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.starts_with("akasha: refusing to overwrite existing path"));
    assert_eq!(
        fs::read(pointer).expect("read preserved pointer"),
        b"human content\n"
    );
}

#[test]
fn link_rejects_a_repository_mismatch_before_writing() {
    let temp = TempDir::new("mismatch");
    let (root, repository) = setup_registered_project(temp.path());
    let other = temp.path().join("other");
    fs::create_dir(&other).expect("create other repository");

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&root),
            "link",
            "example",
            "--repo",
            path(&other),
        ])
        .current_dir(&repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run mismatched link");

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("does not match registry entry"));
    assert!(!other.join(".akasha.toml").exists());
}

#[test]
fn link_rejects_a_conflicting_global_project_option() {
    let temp = TempDir::new("conflicting-project");
    let (root, repository) = setup_registered_project(temp.path());

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args([
            "--root",
            path(&root),
            "--project",
            "other",
            "link",
            "example",
        ])
        .current_dir(&repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run invalid link usage");

    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("link slug \"example\" does not match --project \"other\""));
    assert!(!repository.join(".akasha.toml").exists());
}

fn path(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
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
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-link-{label}-{}-{id}",
            std::process::id()
        ));
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
