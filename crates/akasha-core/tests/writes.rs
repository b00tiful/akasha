use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use akasha_core::create_file_atomically;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn publishes_exact_bytes_without_staging_residue() {
    let temp = TempDir::new("exact-bytes");
    let destination = temp.path().join("note.md");
    let contents = b"---\nname: exact\n---\n\0\xff\n";

    create_file_atomically(&destination, contents).expect("create file atomically");

    assert_eq!(fs::read(&destination).expect("read destination"), contents);
    assert_eq!(entries(temp.path()), vec![PathBuf::from("note.md")]);
}

#[test]
fn refuses_to_overwrite_an_existing_file() {
    let temp = TempDir::new("existing-file");
    let destination = temp.path().join("note.md");
    fs::write(&destination, b"human content").expect("seed destination");

    let error = create_file_atomically(&destination, b"replacement")
        .expect_err("existing destination must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(
        fs::read(&destination).expect("read unchanged destination"),
        b"human content"
    );
    assert!(error.to_string().contains("refusing to overwrite"));
}

#[test]
fn concurrent_creators_have_exactly_one_winner() {
    let temp = TempDir::new("concurrent");
    let destination = Arc::new(temp.path().join("note.md"));
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();

    for contents in [b"first".as_slice(), b"second".as_slice()] {
        let destination = Arc::clone(&destination);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            create_file_atomically(destination.as_ref(), contents)
        }));
    }

    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("creator thread must not panic"))
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let conflict = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("one creator must conflict");
    assert_eq!(conflict.exit_code(), 5);

    let contents = fs::read(destination.as_ref()).expect("read winning content");
    assert!(contents == b"first" || contents == b"second");
    assert_eq!(entries(temp.path()), vec![PathBuf::from("note.md")]);
}

#[test]
fn missing_parent_is_a_filesystem_failure() {
    let temp = TempDir::new("missing-parent");
    let destination = temp.path().join("missing/note.md");

    let error =
        create_file_atomically(&destination, b"content").expect_err("missing parent must fail");

    assert_eq!(error.exit_code(), 6);
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn refuses_to_replace_a_symlink() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new("existing-symlink");
    let target = temp.path().join("human.md");
    let destination = temp.path().join("note.md");
    fs::write(&target, b"human content").expect("seed symlink target");
    symlink(&target, &destination).expect("create destination symlink");

    let error = create_file_atomically(&destination, b"replacement")
        .expect_err("existing symlink must conflict");

    assert_eq!(error.exit_code(), 5);
    assert_eq!(
        fs::read(&target).expect("read unchanged symlink target"),
        b"human content"
    );
    assert!(
        fs::symlink_metadata(&destination)
            .expect("inspect destination symlink")
            .file_type()
            .is_symlink()
    );
}

fn entries(directory: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(directory)
        .expect("read temporary directory")
        .map(|entry| PathBuf::from(entry.expect("read directory entry").file_name()))
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("akasha-writes-{label}-{}-{id}", std::process::id()));
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
