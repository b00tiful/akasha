use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use akasha_core::{
    ContextSection, DEFAULT_CONTEXT_MAX_CHARS, ResolutionEnvironment, ResolveRequest,
    assemble_context, render_context_markdown,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution")
}

fn request(root: PathBuf, cwd: PathBuf) -> ResolveRequest {
    ResolveRequest {
        root_override: Some(root),
        project_override: None,
        cwd,
        environment: ResolutionEnvironment::default(),
    }
}

#[test]
fn assembles_context_in_priority_order() {
    let fixture = fixtures();
    let bundle = assemble_context(&request(
        fixture.join("valid-root"),
        fixture.join("repository/nested"),
    ))
    .expect("assemble context");

    let sections = bundle
        .entries
        .iter()
        .map(|entry| entry.section)
        .collect::<Vec<_>>();
    assert_eq!(
        sections,
        [
            ContextSection::OpenTask,
            ContextSection::OpenProblem,
            ContextSection::Roadmap,
            ContextSection::EntityIndex,
            ContextSection::LatestHandoff,
            ContextSection::RecentEvent,
        ]
    );
    assert!(
        bundle.entries[4]
            .content
            .contains("Latest synthetic handoff")
    );
    assert!(
        !bundle.entries[4]
            .content
            .contains("Earlier synthetic handoff")
    );
    assert!(!bundle.truncated);
    assert_eq!(bundle.omitted_entries, 0);

    let markdown = render_context_markdown(&bundle);
    assert_eq!(markdown.chars().count(), bundle.rendered_chars);
    assert!(bundle.rendered_chars <= DEFAULT_CONTEXT_MAX_CHARS);
}

#[test]
fn truncates_only_between_entries_and_reports_the_omission() {
    let temp = TempDir::copy_of(&fixtures());
    let session = temp
        .path()
        .join("valid-root/Projects/example/events/sessions/2026-07-13.md");
    fs::write(
        session,
        format!(
            "---\nschema_version: 1\nproject: example\ntype: session\ndate: 2026-07-13\n---\n\n# Oversized event\n\n{}\n",
            "x".repeat(DEFAULT_CONTEXT_MAX_CHARS)
        ),
    )
    .expect("write oversized recent event");

    let bundle = assemble_context(&request(
        temp.path().join("valid-root"),
        temp.path().join("repository/nested"),
    ))
    .expect("assemble truncated context");
    let markdown = render_context_markdown(&bundle);

    assert!(bundle.truncated);
    assert_eq!(bundle.omitted_entries, 1);
    assert!(!markdown.contains("# Oversized event"));
    assert!(markdown.contains("Omitted 1 lower-priority context entries"));
    assert!(markdown.chars().count() <= DEFAULT_CONTEXT_MAX_CHARS);
}

struct TempDir(PathBuf);

impl TempDir {
    fn copy_of(source: &Path) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("akasha-context-{}-{id}", std::process::id()));
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
