use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    LinkRequest, ResolutionEnvironment, ResolveRequest, apply_project_link, link_project,
    prepare_project_link, resolve_project,
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

#[test]
fn reviewed_link_is_read_only_until_apply_and_round_trips_exact_source() {
    let temp = TempDir::new("review space-猫");
    let (root, repository) = setup_registered_project(temp.path());
    let request = request(&root, &repository, None);
    let before = snapshot(temp.path());
    let plan = prepare_project_link(&request).unwrap();
    assert_eq!(prepare_project_link(&request).unwrap(), plan);
    assert_eq!(snapshot(temp.path()), before);
    assert!(plan.plan_id.starts_with("sha256:"));
    assert_eq!(plan.plan_id.len(), 71);
    assert_eq!(plan.source, "schema_version = 1\nproject = \"example\"\n");
    let result = apply_project_link(&request, &plan).unwrap();
    assert_eq!(result, plan.destination);
    assert_eq!(fs::read_to_string(&result.pointer).unwrap(), plan.source);
    let resolve = ResolveRequest {
        root_override: Some(root),
        project_override: None,
        cwd: repository.join("nested"),
        environment: ResolutionEnvironment::default(),
    };
    assert_eq!(
        resolve_project(&resolve).unwrap().pointer,
        Some(result.pointer)
    );
    let published = snapshot(temp.path());
    assert_eq!(
        apply_project_link(&request, &plan).unwrap_err().exit_code(),
        5
    );
    assert_eq!(snapshot(temp.path()), published);
}

#[test]
fn reviewed_link_binds_every_identity_source_and_plan_id() {
    let temp = TempDir::new("review-tamper");
    let (root, repository) = setup_registered_project(temp.path());
    let request = request(&root, &repository, None);
    let plan = prepare_project_link(&request).unwrap();
    let before = snapshot(temp.path());
    for field in 0..8 {
        let mut changed = plan.clone();
        match field {
            0 => changed.destination.root.push("other"),
            1 => changed.destination.registry.push("other"),
            2 => changed.destination.repository_dir.push("other"),
            3 => changed.destination.project_dir.push("other"),
            4 => changed.destination.pointer.push("other"),
            5 => changed.destination.project.push_str("-other"),
            6 => changed.source.push_str("# injected\n"),
            _ => changed.plan_id.push('0'),
        }
        assert_eq!(
            apply_project_link(&request, &changed)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(snapshot(temp.path()), before);
    }
}

#[test]
fn reviewed_link_refuses_changed_registry_and_configured_project_identity() {
    for config_drift in [false, true] {
        let temp = TempDir::new("review-drift");
        let (root, repository) = setup_registered_project(temp.path());
        let request = request(&root, &repository, None);
        let plan = prepare_project_link(&request).unwrap();
        if config_drift {
            fs::create_dir_all(root.join("Renamed/example")).unwrap();
            let config = root.join("akasha.toml");
            fs::write(
                &config,
                fs::read_to_string(&config)
                    .unwrap()
                    .replace("projects = \"Projects\"", "projects = \"Renamed\""),
            )
            .unwrap();
        } else {
            fs::create_dir(temp.path().join("other")).unwrap();
            fs::write(
                root.join("Meta/projects.yaml"),
                "example:\n  path: ../../other\n  status: active\n",
            )
            .unwrap();
        }
        let before = snapshot(temp.path());
        assert_eq!(
            apply_project_link(&request, &plan).unwrap_err().exit_code(),
            if config_drift { 5 } else { 3 }
        );
        assert_eq!(snapshot(temp.path()), before);
    }
}

#[test]
fn reviewed_link_refuses_late_files_directories_and_dangling_symlinks() {
    for kind in 0..3 {
        let temp = TempDir::new("review-occupied");
        let (root, repository) = setup_registered_project(temp.path());
        let request = request(&root, &repository, None);
        let plan = prepare_project_link(&request).unwrap();
        let pointer = &plan.destination.pointer;
        match kind {
            0 => fs::write(pointer, b"human bytes\r\n").unwrap(),
            1 => fs::create_dir(pointer).unwrap(),
            _ => {
                #[cfg(unix)]
                std::os::unix::fs::symlink("missing-target", pointer).unwrap();
                #[cfg(not(unix))]
                continue;
            }
        }
        let before = snapshot(temp.path());
        assert_eq!(prepare_project_link(&request).unwrap_err().exit_code(), 5);
        assert_eq!(
            apply_project_link(&request, &plan).unwrap_err().exit_code(),
            5
        );
        assert_eq!(snapshot(temp.path()), before);
    }
}

#[test]
fn concurrent_reviewed_links_publish_exactly_once() {
    let temp = TempDir::new("review-race");
    let (root, repository) = setup_registered_project(temp.path());
    let request = request(&root, &repository, None);
    let plan = prepare_project_link(&request).unwrap();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let run = || {
            barrier.wait();
            apply_project_link(&request, &plan)
        };
        let first = scope.spawn(run);
        let second = scope.spawn(run);
        [first.join().unwrap(), second.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap()
            .exit_code(),
        5
    );
    assert_eq!(
        fs::read_to_string(&plan.destination.pointer).unwrap(),
        plan.source
    );
    assert_eq!(fs::read_dir(repository).unwrap().count(), 2); // nested directory + pointer
}

#[cfg(unix)]
#[test]
fn reviewed_link_refuses_non_utf8_identity_without_writing() {
    use std::os::unix::ffi::OsStringExt;
    let temp = TempDir::new("review-utf8");
    let base = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"invalid-\xff".to_vec()));
    let (root, repository) = setup_registered_project(&base);
    let before = snapshot(temp.path());
    assert_eq!(
        prepare_project_link(&request(&root, &repository, None))
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(snapshot(temp.path()), before);
}

fn snapshot(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, entries: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        let bytes = if metadata.is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if metadata.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        entries.insert(path.strip_prefix(root).unwrap().to_owned(), bytes);
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                walk(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = std::collections::BTreeMap::new();
    walk(root, root, &mut entries);
    entries
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
