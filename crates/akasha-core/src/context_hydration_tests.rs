use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    report: ProjectValidationReport,
}

impl Fixture {
    fn new() -> Self {
        let base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/resolution");
        let mut report = validate_project(&ResolveRequest {
            root_override: Some(base.join("valid-root")),
            project_override: Some("example".into()),
            cwd: base,
            environment: Default::default(),
        })
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "akasha-context-hydration-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        report.root = fs::canonicalize(&root).unwrap();
        report.project_dir = report.root.join("project");
        fs::create_dir(&report.project_dir).unwrap();
        Self { report }
    }

    fn candidate(&self, name: &str, source: &str) -> ContextCandidate {
        let path = self.report.project_dir.join(name);
        fs::write(&path, source).unwrap();
        read_project_file(
            &self.report,
            Path::new(name),
            ContextSection::Roadmap,
            usize::MAX,
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.report.root).unwrap();
    }
}

#[test]
fn context_hydration_reads_only_selected_occurrences_with_output_bounded_bytes() {
    let fixture = Fixture::new();
    let content = "界🧭é\r\n".repeat(1024);
    for budget in [1, 16_000, 100_000, usize::MAX] {
        let candidates = (0..1000)
            .map(|index| {
                // Repeated source identities model overlapping configured roles. Count every
                // occurrence: unique-source payload cannot exceed this stronger bound.
                ContextCandidate::new(
                    ContextSection::RecentEvent,
                    PathBuf::from(format!("event-{}.md", index % 7)),
                    &content,
                    0..content.len(),
                    budget,
                )
            })
            .collect();
        let mut reads = Vec::new();
        let result = fit_candidates(&fixture.report, candidates, budget, |candidate| {
            reads.push(candidate.source.clone());
            Ok(content.clone())
        });
        if budget == 1 {
            assert!(matches!(result, Err(ContextError::Budget { .. })));
            assert!(reads.is_empty());
        } else {
            let bundle = result.unwrap();
            assert_eq!(
                reads,
                bundle
                    .entries
                    .iter()
                    .map(|entry| entry.source.clone())
                    .collect::<Vec<_>>()
            );
            assert_eq!(bundle.entries.len() + bundle.omissions.len(), 1000);
            assert!(
                bundle
                    .entries
                    .iter()
                    .map(|entry| entry.content.len())
                    .sum::<usize>()
                    <= budget.saturating_mul(4)
            );
            assert_eq!(
                bundle.rendered_chars,
                render_context_markdown(&bundle).chars().count()
            );
            assert!(bundle.rendered_chars <= budget);
            if budget != usize::MAX {
                assert!(reads.len() < 1000);
            }
        }
    }
}

#[test]
fn context_hydration_keeps_exact_trimmed_note_and_projection_ranges() {
    let fixture = Fixture::new();
    for body in [" \r\n界🧭é  \r\ntext\r\n\u{2003}", " \r\n\u{2003}", ""] {
        let source = format!("---\r\nschema_version: 1\r\n---\r\n{body}");
        let path = fixture.report.project_dir.join("note.md");
        fs::write(&path, &source).unwrap();
        let note = read_note(
            &path,
            &fixture.report.project_dir,
            ContextSection::OpenTask,
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            read_selected_body(&fixture.report, &note.entry).unwrap(),
            body.trim()
        );
        let projection = read_project_file(
            &fixture.report,
            Path::new("note.md"),
            ContextSection::Roadmap,
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            read_selected_body(&fixture.report, &projection).unwrap(),
            source.trim()
        );
        assert_eq!(fs::read(&path).unwrap(), source.as_bytes());
    }
}

#[test]
fn context_hydration_refuses_full_source_drift_including_unreturned_bytes() {
    let fixture = Fixture::new();
    let original = "---\nschema_version: 1\nlabel: first\n---\n \nbody 世界\n ";
    let path = fixture.report.project_dir.join("note.md");
    for changed in [
        original.replace("first", "other").into_bytes(),
        original.replace("body", "edit").into_bytes(),
        original.trim_end().as_bytes().to_vec(),
        format!("{original} ").into_bytes(),
        {
            let mut bytes = original.as_bytes().to_vec();
            *bytes.last_mut().unwrap() = 0xff;
            bytes
        },
    ] {
        fs::write(&path, original).unwrap();
        let candidate = read_note(
            &path,
            &fixture.report.project_dir,
            ContextSection::OpenTask,
            16_000,
        )
        .unwrap()
        .entry;
        fs::write(&path, &changed).unwrap();
        let error = read_selected_body(&fixture.report, &candidate).unwrap_err();
        assert_eq!(error.exit_code(), 4);
        assert!(error.to_string().contains("retry context"));
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
}

#[test]
fn context_hydration_omitted_sources_are_not_read_again() {
    let fixture = Fixture::new();
    let candidates = vec![
        fixture.candidate("first.md", &"x".repeat(900)),
        fixture.candidate("last.md", "later 🧭"),
    ];
    // The first candidate can fit individually, but identity/framing displaces it. The later
    // small source must still fit; its content is the only source hydrated.
    let first_chars = candidates[0].rendered_chars;
    let path = fixture.report.project_dir.join("first.md");
    fs::write(&path, b"\xff").unwrap();
    let bundle = fit_candidates(&fixture.report, candidates, first_chars, |candidate| {
        read_selected_body(&fixture.report, candidate)
    })
    .unwrap();
    assert_eq!(bundle.omissions.len(), 1);
    assert_eq!(bundle.omissions[0].source, Path::new("first.md"));
    assert_eq!(bundle.entries.len(), 1);
    assert_eq!(bundle.entries[0].content, "later 🧭");
    assert_eq!(fs::read(path).unwrap(), b"\xff");
}

#[test]
fn context_hydration_returns_no_bundle_after_later_selected_drift() {
    let fixture = Fixture::new();
    let candidates = vec![
        fixture.candidate("first.md", "first"),
        fixture.candidate("last.md", "last"),
    ];
    let mut reads = 0;
    let result = fit_candidates(&fixture.report, candidates, 16_000, |candidate| {
        reads += 1;
        let body = read_selected_body(&fixture.report, candidate)?;
        fs::write(fixture.report.project_dir.join("last.md"), "edit").unwrap();
        Ok(body)
    });
    assert_eq!(reads, 2);
    assert_eq!(result.unwrap_err().exit_code(), 4);
}

#[test]
fn context_hydration_rechecks_missing_nonregular_and_oversized_sources() {
    let fixture = Fixture::new();
    let candidate = fixture.candidate("source.md", "original");
    let path = fixture.report.project_dir.join("source.md");
    fs::remove_file(&path).unwrap();
    assert_eq!(
        read_selected_body(&fixture.report, &candidate)
            .unwrap_err()
            .exit_code(),
        6
    );
    fs::create_dir(&path).unwrap();
    assert_eq!(
        read_selected_body(&fixture.report, &candidate)
            .unwrap_err()
            .exit_code(),
        4
    );
    fs::remove_dir(&path).unwrap();
    fs::write(
        &path,
        vec![b'x'; crate::markdown_source::MAX_MARKDOWN_SOURCE_BYTES + 1],
    )
    .unwrap();
    let error = read_selected_body(&fixture.report, &candidate).unwrap_err();
    assert_eq!(error.exit_code(), 4);
    assert!(error.to_string().contains("16777216-byte"));
}

#[cfg(unix)]
#[test]
fn context_hydration_refuses_file_and_parent_symlink_replacements() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let candidate = fixture.candidate("source.md", "original");
    let path = fixture.report.project_dir.join("source.md");
    let foreign = fixture.report.root.join("foreign.md");
    fs::write(&foreign, "original").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&foreign, &path).unwrap();
    assert_eq!(
        read_selected_body(&fixture.report, &candidate)
            .unwrap_err()
            .exit_code(),
        4
    );
    fs::remove_file(&path).unwrap();
    fs::write(&path, "original").unwrap();
    let moved = fixture.report.root.join("moved-project");
    fs::rename(&fixture.report.project_dir, &moved).unwrap();
    symlink(&moved, &fixture.report.project_dir).unwrap();
    assert_eq!(
        read_selected_body(&fixture.report, &candidate)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(fs::read(&foreign).unwrap(), b"original");
}
