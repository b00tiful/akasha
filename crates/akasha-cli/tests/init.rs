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
fn init_prints_plain_text_and_creates_a_valid_resolvable_project() {
    let fixture = Fixture::new("plain");
    fs::write(fixture.root.join("templates/session.md"), "template\n").expect("write template");

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", path(&fixture.root), "init", "example"])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha init");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(stdout.starts_with("initialized: example\n"));
    assert!(stdout.contains("templates copied: 1"));
    assert_eq!(
        fs::read(fixture.repository.join(".akasha.toml")).expect("read pointer"),
        b"schema_version = 1\nproject = \"example\"\n"
    );
    assert_eq!(
        fs::read(fixture.root.join("Projects/example/templates/session.md"))
            .expect("read copied template"),
        b"template\n"
    );

    let nested = fixture.repository.join("nested");
    fs::create_dir(&nested).expect("create nested directory");
    let validate = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", path(&fixture.root), "validate"])
        .current_dir(nested)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("validate initialized project");
    assert!(validate.status.success());
    assert!(validate.stderr.is_empty());
    assert!(
        String::from_utf8(validate.stdout)
            .expect("validation stdout is UTF-8")
            .starts_with("valid: example\n")
    );
}

#[test]
fn init_json_reports_the_same_created_paths() {
    let fixture = Fixture::new("json");

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", path(&fixture.root), "--json", "init", "example"])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run akasha init --json");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse init JSON");
    assert_eq!(value["project"], "example");
    assert_eq!(value["template_files"], 0);
    assert_eq!(
        value["repository_dir"],
        fs::canonicalize(&fixture.repository)
            .expect("canonical repository")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        value["project_dir"],
        fixture
            .root
            .join("Projects/example")
            .to_string_lossy()
            .as_ref()
    );
}

#[test]
fn init_conflicts_use_stderr_and_leave_no_project_directory() {
    let fixture = Fixture::new("conflict");
    fs::write(fixture.repository.join(".akasha.toml"), "human\n").expect("seed pointer");

    let output = Command::new(env!("CARGO_BIN_EXE_akasha"))
        .args(["--root", path(&fixture.root), "init", "example"])
        .current_dir(&fixture.repository)
        .env_remove("AKASHA_ROOT")
        .output()
        .expect("run conflicting init");

    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.starts_with("akasha: initialization conflict"));
    assert!(!fixture.root.join("Projects/example").exists());
    assert_eq!(
        fs::read_to_string(fixture.root.join("Meta/projects.yaml")).expect("read registry"),
        "{}\n"
    );
}

fn path(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
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
            root.join("templates"),
            root.join("Global"),
            root.join("Projects"),
            root.join("Inbox"),
            repository.clone(),
        ] {
            fs::create_dir_all(directory).expect("create fixture directory");
        }
        fs::copy(fixture_root_config(), root.join("akasha.toml")).expect("copy root config");
        fs::write(root.join("Meta/projects.yaml"), "{}\n").expect("write registry");
        Self {
            _temp: temp,
            root,
            repository,
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-cli-init-{label}-{}-{id}",
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
