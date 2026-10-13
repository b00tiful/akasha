use super::*;
use akasha_core::{LibraryReportKind, inspect_library_report};

#[test]
fn bounded_reports_preserve_exact_library_totals_and_configuration_semantics() {
    let fixture = fixture("report-totals");
    let projection = build_library_projection(&fixture.request).unwrap();
    for kind in [LibraryReportKind::Status, LibraryReportKind::Digest] {
        let report = inspect_library_report(&fixture.request, kind, 1).unwrap();
        let mut expected = projection.dashboard.clone();
        expected.project_metrics.truncate(1);
        assert_eq!(report.dashboard, expected);
        assert_eq!(report.dashboard.projects, 2);
        assert_eq!(report.dashboard.global_notes, 1);
        assert_eq!(report.dashboard.open_tasks, 1);
        assert!(report.truncated);
        assert_eq!(report.limit, 1);
        assert_eq!(report.root, fixture.root);
        assert_eq!(report.selected_project, "alpha");
    }
    let config = fs::read_to_string(fixture.root.join("akasha.toml")).unwrap();
    fs::write(
        fixture.root.join("akasha.toml"),
        config.replace(
            "open_statuses = [\"open\", \"active\", \"blocked\", \"in-progress\"]",
            "open_statuses = [\"waiting\"]",
        ),
    )
    .unwrap();
    let report = inspect_library_report(&fixture.request, LibraryReportKind::Status, 100).unwrap();
    assert_eq!(report.dashboard.open_tasks, 0);
    assert!(!report.truncated);
    assert_eq!(report.dashboard.project_metrics[1].status, "paused");
}

fn add_date(fixture: &Fixture, project: &str, date: &str) {
    let project_dir = fixture.root.join("Projects").join(project);
    let event = "events/sessions/report.md";
    fs::write(project_dir.join(event), format!("---\nschema_version: 1\nproject: {project}\ntype: session\ndate: {date}\n---\n\n# Report\n")).unwrap();
    let entity = if project == "alpha" {
        "entities/core.md"
    } else {
        "entities/renderer.md"
    };
    let tasks: Vec<&str> = if project == "alpha" {
        vec!["records/tasks/ship-mvp.md"]
    } else {
        vec![]
    };
    support::write_project_state(&project_dir, &[event], &[entity], &tasks);
}

#[test]
fn digest_orders_dates_descending_ties_by_slug_and_missing_dates_last() {
    let fixture = fixture("report-order");
    add_date(&fixture, "zeta", "2026-10-13");
    let report = inspect_library_report(&fixture.request, LibraryReportKind::Digest, 100).unwrap();
    assert_eq!(report.dashboard.project_metrics[0].project, "zeta");
    assert_eq!(
        report.dashboard.project_metrics[1].latest_activity_date,
        None
    );
    assert_eq!(
        report.dashboard.latest_activity_date.as_deref(),
        Some("2026-10-13")
    );
    let status = inspect_library_report(&fixture.request, LibraryReportKind::Status, 1).unwrap();
    assert_eq!(status.dashboard.project_metrics[0].project, "alpha");
    add_date(&fixture, "alpha", "2026-10-13");
    let report = inspect_library_report(&fixture.request, LibraryReportKind::Digest, 100).unwrap();
    assert_eq!(report.dashboard.project_metrics[0].project, "alpha");
    add_date(&fixture, "zeta", "2026-10-14");
    let report = inspect_library_report(&fixture.request, LibraryReportKind::Digest, 1).unwrap();
    assert_eq!(report.dashboard.project_metrics[0].project, "zeta");
    assert_eq!(report.dashboard.notes, 6);
    assert!(report.truncated);
}

#[test]
fn limits_resolution_stale_sources_and_nonselected_journals_refuse_without_writes() {
    let fixture = fixture("report-refusal");
    let before = link_fixture_snapshot(&fixture.root);
    for limit in [0, 101, usize::MAX] {
        assert_eq!(
            inspect_library_report(&fixture.request, LibraryReportKind::Status, limit)
                .unwrap_err()
                .exit_code(),
            4
        );
    }
    let mut unknown = fixture.request.clone();
    unknown.project_override = Some("missing".into());
    assert_eq!(
        inspect_library_report(&unknown, LibraryReportKind::Digest, 30)
            .unwrap_err()
            .exit_code(),
        3
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    let index = fixture.root.join("Projects/zeta/index.md");
    fs::write(&index, "# Changed index\n").unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    for kind in [LibraryReportKind::Status, LibraryReportKind::Digest] {
        assert_eq!(
            inspect_library_report(&fixture.request, kind, 30)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(fs::read_to_string(&index).unwrap(), "# Changed index\n");
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
    }
    let journal = fixture
        .root
        .join("Projects/zeta")
        .join(akasha_core::NOTE_EDIT_JOURNAL_FILE);
    fs::write(&journal, b"partial journal").unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    for kind in [LibraryReportKind::Status, LibraryReportKind::Digest] {
        assert_eq!(
            inspect_library_report(&fixture.request, kind, 30)
                .unwrap_err()
                .exit_code(),
            5
        );
        assert_eq!(fs::read(&journal).unwrap(), b"partial journal");
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
    }
}

#[test]
fn default_thirty_and_explicit_expansion_keep_counts_exact_without_writes() {
    let fixture = fixture("report-expansion");
    let registry = fixture.root.join("Meta/projects.yaml");
    let mut source = fs::read_to_string(&registry).unwrap();
    for index in 0..33 {
        let project = format!("extra-{index:02}");
        let repository = fixture.root.parent().unwrap().join(&project);
        fs::create_dir(&repository).unwrap();
        create_project(&fixture.root, &project, "core", None);
        source.push_str(&format!(
            "{project}:\n  path: {repository:?}\n  status: archived\n"
        ));
    }
    fs::write(registry, source).unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    for kind in [LibraryReportKind::Status, LibraryReportKind::Digest] {
        for (limit, count, truncated) in [
            (akasha_core::DEFAULT_LIBRARY_REPORT_LIMIT, 30, true),
            (100, 35, false),
        ] {
            let report = inspect_library_report(&fixture.request, kind, limit).unwrap();
            assert_eq!(report.dashboard.projects, 35);
            assert_eq!(report.dashboard.notes, 37);
            assert_eq!(report.dashboard.project_metrics.len(), count);
            assert_eq!(report.truncated, truncated);
        }
    }
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
}

#[test]
fn malformed_global_and_symlink_sources_refuse_without_writes() {
    let fixture = fixture("report-malformed");
    fs::write(
        fixture.root.join("Global/entities/invalid.md"),
        "---\nbad\n---\n",
    )
    .unwrap();
    let before = link_fixture_snapshot(&fixture.root);
    assert_eq!(
        inspect_library_report(&fixture.request, LibraryReportKind::Status, 30)
            .unwrap_err()
            .exit_code(),
        4
    );
    assert_eq!(link_fixture_snapshot(&fixture.root), before);
    #[cfg(unix)]
    {
        fs::remove_file(fixture.root.join("Global/entities/invalid.md")).unwrap();
        std::os::unix::fs::symlink(
            "rust-pattern.md",
            fixture.root.join("Global/entities/alias.md"),
        )
        .unwrap();
        let before = link_fixture_snapshot(&fixture.root);
        assert_eq!(
            inspect_library_report(&fixture.request, LibraryReportKind::Digest, 30)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(link_fixture_snapshot(&fixture.root), before);
    }
}
