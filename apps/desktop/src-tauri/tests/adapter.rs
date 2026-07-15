use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_desktop::{library_document, library_projection, save_library_document};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/resolution/valid-root")
}

#[test]
fn adapter_returns_the_core_projection_and_exact_fallback() {
    let library = library_projection(Some(fixture_root()), Some("example".to_owned()))
        .expect("load fixture library");

    assert_eq!(library.projection.selected_project, "example");
    assert!(library.projection.total_books > 0);
    assert!(library.fallback_markdown.contains("# Akasha Library"));
    assert!(
        library
            .fallback_markdown
            .contains("`Global/entities/rust-pattern.md`")
    );
}

#[test]
fn adapter_loads_only_a_projected_exact_document() {
    let document = library_document(
        Some(fixture_root()),
        Some("example".to_owned()),
        "Projects/example/entities/core.md",
    )
    .expect("load projected fixture document");

    assert!(document.source.contains("# Synthetic entity"));

    let error = library_document(
        Some(fixture_root()),
        Some("example".to_owned()),
        "Projects/example/index.md",
    )
    .expect_err("reject non-projected document");
    assert_eq!(error.code, 4);
}

#[test]
fn adapter_saves_through_the_checked_core_boundary() {
    let temp = TempDir::new("save");
    let root = temp.path().join("valid-root");
    copy_tree(&fixture_root(), &root);
    fs::create_dir_all(temp.path().join("repository")).expect("create registered repository");
    let id = "Projects/example/entities/core.md";
    let document = library_document(Some(root.clone()), Some("example".to_owned()), id)
        .expect("load editable document");
    let replacement = format!("{}\nSaved through Tauri.\n", document.source);

    let result = save_library_document(
        Some(root.clone()),
        Some("example".to_owned()),
        id,
        &document.source,
        &replacement,
    )
    .expect("save through desktop adapter");

    assert!(result.changed);
    assert_eq!(
        fs::read_to_string(root.join(id)).expect("read saved document"),
        replacement
    );
    let stale = save_library_document(
        Some(root),
        Some("example".to_owned()),
        id,
        &document.source,
        &replacement,
    )
    .expect_err("stale desktop baseline must conflict");
    assert_eq!(stale.code, 5);
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let path = entry.path();
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("read fixture entry type").is_dir() {
            copy_tree(&path, &target);
        } else {
            fs::copy(path, target).expect("copy fixture file");
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akasha-desktop-{label}-{}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary directory");
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
