use super::*;

fn transition(fixture: &Fixture, reload: bool) -> WorkResult {
    execute_job(if reload {
        Job::Load(fixture.app.view_origin())
    } else {
        Job::Open(fixture.app.view_origin(), ID.into())
    })
}

fn change_request(fixture: &mut Fixture, field: &str) {
    match field {
        "root" => fixture.app.request.root_override = None,
        "project" => fixture.app.request.project_override = None,
        "cwd" => fixture.app.request.cwd = fixture.temp.join("other"),
        "environment" => fixture.app.request.environment.akasha_root = Some("other".into()),
        "config-home" => fixture.app.request.environment.xdg_config_home = Some("other".into()),
        "home" => fixture.app.request.environment.home = Some("other".into()),
        _ => unreachable!(),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct View {
    title: String,
    body: String,
    document: Option<(String, String)>,
    editor: Option<String>,
    scroll: u16,
    focus: u8,
}

fn view(app: &App) -> View {
    View {
        title: app.body_title.clone(),
        body: app.body.clone(),
        document: app
            .document
            .as_ref()
            .map(|d| (d.id.clone(), d.source.clone())),
        editor: app.editor.as_ref().map(Editor::source),
        scroll: app.scroll,
        focus: match app.focus {
            Focus::Prompt => 0,
            Focus::List => 1,
            Focus::Reader => 2,
        },
    }
}

fn changed_check(fixture: &Fixture) -> WorkResult {
    let path = fixture.root().join(GLOBAL_ID);
    let source = fs::read_to_string(&path).unwrap();
    fs::write(path, source.replace("status: stable", "status: reviewed")).unwrap();
    execute_job(Job::Check(
        fixture.app.request.clone(),
        Box::new(fixture.app.projection.clone().unwrap()),
        fixture.app.watched_sources(),
    ))
}

#[test]
fn open_and_reload_obsolete_inputs_preserve_graph_parent_and_exact_view() {
    for reload in [false, true] {
        for field in [
            "root",
            "project",
            "cwd",
            "environment",
            "config-home",
            "home",
        ] {
            let mut fixture = Fixture::new();
            fixture
                .app
                .command("links Projects/example/events/sessions/2026-07-13.md");
            fixture.finish();
            fixture.app.scroll = 2;
            fixture.app.command("follow 1");
            fixture.finish();
            assert!(fixture.app.link_return.is_some());
            fixture.app.scroll = 3;
            let response = transition(&fixture, reload);
            assert!(response.is_ok());
            change_request(&mut fixture, field);
            let before = view(&fixture.app);
            let parent = fixture.app.link_return.clone();
            let projection = fixture.app.projection.clone();
            let files = link_snapshot(&fixture.temp);
            fixture.app.receive(response);
            assert_eq!(view(&fixture.app), before, "reload={reload}, field={field}");
            assert!(fixture.app.link_return == parent);
            assert_eq!(fixture.app.projection, projection);
            assert!(fixture.app.messages.back().unwrap().contains("obsolete"));
            assert_eq!(link_snapshot(&fixture.temp), files);
        }
    }
}

#[test]
fn open_and_reload_retain_clean_dirty_editors_and_undo() {
    for reload in [false, true] {
        for dirty in [false, true] {
            let mut fixture = Fixture::new();
            let response = transition(&fixture, reload);
            fixture.open_editor();
            let original = fixture.app.editor.as_ref().unwrap().source();
            fixture.app.paste("\nRetained Привет 世界  ");
            if !dirty {
                assert!(fixture.app.editor.as_mut().unwrap().area.undo());
            }
            let before = view(&fixture.app);
            let files = link_snapshot(&fixture.temp);
            fixture.app.receive(response);
            assert_eq!(view(&fixture.app), before);
            assert!(fixture.app.editing);
            let editor = fixture.app.editor.as_mut().unwrap();
            if dirty {
                assert!(editor.area.undo());
                assert_eq!(editor.source(), original);
            } else {
                assert!(editor.area.redo(), "clean editor must retain history");
                assert!(editor.source().ends_with("Retained Привет 世界  "));
            }
            assert_eq!(link_snapshot(&fixture.temp), files);
        }
    }
}

#[test]
fn open_and_reload_retain_forms_reviews_and_new_recovery_views() {
    for reload in [false, true] {
        for state in [
            "creation",
            "lifecycle",
            "administration",
            "recovery",
            "blocked",
        ] {
            let mut fixture = Fixture::new();
            let response = transition(&fixture, reload);
            match state {
                "creation" => {
                    fixture.app.command("create task");
                    fixture.finish();
                    fixture.app.paste("Retained path 世界.md");
                }
                "lifecycle" => {
                    fixture.app.open(ID);
                    fixture.finish();
                    fixture.app.command("lifecycle");
                    fixture.finish();
                    let Some(Workflow::Lifecycle(form)) = &mut fixture.app.workflow else {
                        unreachable!()
                    };
                    form.note.area.insert_str("Retained note 世界");
                    form.projection.area.insert_str("Retained index Привет");
                }
                "administration" => {
                    fixture.app.command(&format!(
                        "link example {}",
                        fixture.temp.join("repository").display()
                    ));
                    fixture.finish();
                }
                "recovery" => {
                    fixture.app.command("recovery");
                    fixture.finish();
                }
                "blocked" => fixture.app.recovery_blocked = true,
                _ => unreachable!(),
            }
            let before = view(&fixture.app);
            let prompt = fixture.app.prompt.lines().to_vec();
            let lifecycle = match &fixture.app.workflow {
                Some(Workflow::Lifecycle(form)) => {
                    Some((form.note.source(), form.projection.source()))
                }
                _ => None,
            };
            let review = fixture
                .app
                .link_review
                .as_ref()
                .map(|r| r.plan.plan_id.clone());
            let files = link_snapshot(&fixture.temp);
            fixture.app.receive(response);
            assert_eq!(view(&fixture.app), before, "reload={reload}, state={state}");
            assert_eq!(fixture.app.prompt.lines(), prompt);
            assert_eq!(
                fixture
                    .app
                    .link_review
                    .as_ref()
                    .map(|r| r.plan.plan_id.clone()),
                review
            );
            if let Some(expected) = lifecycle {
                let Some(Workflow::Lifecycle(form)) = &fixture.app.workflow else {
                    panic!("lost lifecycle")
                };
                assert_eq!((form.note.source(), form.projection.source()), expected);
            }
            assert_eq!(link_snapshot(&fixture.temp), files);
        }
    }
}

#[test]
fn reload_preserves_original_resolution_and_allows_reviewed_recovery_retry() {
    let mut fixture = Fixture::new();
    fixture.app.request.environment.akasha_root = Some(fixture.root().into());
    fixture.app.request.root_override = None;
    fixture.app.load();
    fixture.finish();
    assert_eq!(fixture.app.request.root_override, Some(fixture.root()));
    assert!(fixture.app.projection.is_some());

    let (journal, before, _) = stage_recovery(&fixture, false);
    // Open uses the existing recovery-running core path, then returns exact canonical source.
    fixture.app.open(ID);
    fixture.finish();
    assert!(!journal.exists());
    assert_eq!(fixture.app.document.as_ref().unwrap().source, before);
    let (journal, before, _) = stage_recovery(&fixture, false);
    fixture.app.recovery_visible = true;
    fixture.app.recovery_blocked = true;
    fixture.app.external_change_pending = true;
    fixture.app.command("refresh");
    fixture.finish();
    assert!(!journal.exists());
    assert!(!fixture.app.recovery_visible && !fixture.app.recovery_blocked);
    assert!(!fixture.app.external_change_pending);
    assert!(
        fixture
            .app
            .messages
            .back()
            .unwrap()
            .contains("Core recovery completed")
    );
    fixture.app.open(ID);
    fixture.finish();
    assert_eq!(fixture.app.document.as_ref().unwrap().source, before);
}

#[test]
fn automatic_checks_discard_obsolete_successes_and_failures_without_changing_view() {
    for field in [
        "root",
        "project",
        "cwd",
        "environment",
        "config-home",
        "home",
    ] {
        for outcome in ["changed", "unchanged", "failure"] {
            let mut fixture = Fixture::new();
            let response = match outcome {
                "changed" => changed_check(&fixture),
                "unchanged" => execute_job(Job::Check(
                    fixture.app.request.clone(),
                    Box::new(fixture.app.projection.clone().unwrap()),
                    vec![],
                )),
                "failure" => {
                    fs::write(fixture.root().join(GLOBAL_ID), "invalid note").unwrap();
                    execute_job(Job::Check(
                        fixture.app.request.clone(),
                        Box::new(fixture.app.projection.clone().unwrap()),
                        vec![],
                    ))
                }
                _ => unreachable!(),
            };
            change_request(&mut fixture, field);
            let before = view(&fixture.app);
            let projection = fixture.app.projection.clone();
            let messages = fixture.app.messages.clone();
            let files = link_snapshot(&fixture.temp);
            fixture.app.checking = true;
            assert!(!fixture.app.receive(response));
            assert!(!fixture.app.checking);
            assert!(!fixture.app.external_change_pending);
            assert_eq!(view(&fixture.app), before);
            assert_eq!(fixture.app.projection, projection);
            assert_eq!(fixture.app.messages, messages);
            assert_eq!(link_snapshot(&fixture.temp), files);
        }
    }
}

#[test]
fn automatic_changes_retain_clean_notes_editors_diagnostics_graph_and_search() {
    for state in [
        "note",
        "editor",
        "diagnostic",
        "graph",
        "followed",
        "search",
        "recovery",
    ] {
        let mut fixture = Fixture::new();
        match state {
            "note" => fixture.app.open(ID),
            "editor" => {
                fixture.open_editor();
            }
            "diagnostic" => fixture.app.command("template task"),
            "graph" | "followed" => fixture
                .app
                .command("links Projects/example/events/sessions/2026-07-13.md"),
            "search" => fixture.app.command("search schema_version --limit 1"),
            "recovery" => fixture.app.command("recovery"),
            _ => unreachable!(),
        }
        if fixture.app.busy {
            fixture.finish();
        }
        if state == "followed" {
            fixture.app.command("follow 1");
            fixture.finish();
        }
        fixture.app.scroll = 2;
        let response = changed_check(&fixture);
        let before = view(&fixture.app);
        let projection = fixture.app.projection.clone();
        let report = fixture.app.link_report.clone();
        let parent = fixture.app.link_return.clone();
        let rows: Vec<_> = fixture
            .app
            .rows
            .iter()
            .map(|r| (r.label.clone(), r.detail.clone()))
            .collect();
        let files = link_snapshot(&fixture.temp);
        fixture.app.receive(response);
        assert!(fixture.app.external_change_pending, "{state}");
        assert_eq!(view(&fixture.app), before, "{state}");
        assert_eq!(fixture.app.projection, projection);
        assert!(fixture.app.link_report == report && fixture.app.link_return == parent);
        assert_eq!(
            fixture
                .app
                .rows
                .iter()
                .map(|r| (r.label.clone(), r.detail.clone()))
                .collect::<Vec<_>>(),
            rows
        );
        assert!(
            !fixture.app.check_external_changes(),
            "signal must prevent a refresh loop"
        );
        assert_eq!(link_snapshot(&fixture.temp), files);
    }
}

#[test]
fn background_refresh_preserves_foreground_busy_job_and_navigation() {
    let mut fixture = Fixture::new();
    assert!(fixture.app.check_external_changes());
    let job = fixture.jobs.try_recv().unwrap();
    let path = fixture.root().join(GLOBAL_ID);
    let source = fs::read_to_string(&path).unwrap();
    fs::write(path, source.replace("status: stable", "status: reviewed")).unwrap();
    fixture.app.open(ID);
    let before = view(&fixture.app);
    fixture.app.receive(execute_job(job));
    assert!(fixture.app.busy && !fixture.app.checking);
    assert!(fixture.app.external_change_pending);
    assert_eq!(view(&fixture.app), before);
    fixture.finish();
    assert!(!fixture.app.busy);
    assert_eq!(fixture.app.document.as_ref().unwrap().id, ID);
}

#[test]
fn automatic_browser_refresh_preserves_current_identity_prompt_and_focus() {
    for commands in [
        vec!["projects"],
        vec!["global"],
        vec!["global", "type entity"],
        vec!["type task"],
    ] {
        let mut fixture = Fixture::new();
        // Capture the check before the user moves to a different list in the same request.
        let response = changed_check(&fixture);
        for command in commands {
            fixture.app.command(command);
        }
        fixture.app.list.select(Some(fixture.app.rows.len() - 1));
        let navigation = fixture.app.navigation_state().unwrap();
        fixture.app.prompt = prompt_area("/open Привет 世界".into());
        fixture.app.focus = Focus::Prompt;
        let files = link_snapshot(&fixture.temp);
        fixture.app.receive(response);
        assert!(!fixture.app.external_change_pending);
        assert_eq!(fixture.app.navigation_state().unwrap(), navigation);
        assert_eq!(fixture.app.prompt.lines(), ["/open Привет 世界"]);
        assert!(fixture.app.focus == Focus::Prompt);
        assert!(
            fixture
                .app
                .books()
                .iter()
                .any(|b| b.id == GLOBAL_ID && b.status.as_deref() == Some("reviewed"))
        );
        assert_eq!(link_snapshot(&fixture.temp), files);
    }
}

#[test]
fn automatic_browser_refresh_falls_back_when_selected_note_was_removed() {
    let mut fixture = Fixture::new();
    fixture.app.command("global");
    fixture.app.command("type entity");
    fs::remove_file(fixture.root().join(GLOBAL_ID)).unwrap();
    let files = link_snapshot(&fixture.temp);
    fixture.check_external_changes();
    assert!(fixture.app.projection.is_some());
    assert!(fixture.app.rows.is_empty());
    assert!(fixture.app.document.is_none());
    assert!(fixture.app.scope == LibraryScope::Global);
    assert!(matches!(
        fixture.app.list_context,
        ListContext::Notes { .. }
    ));
    assert_eq!(link_snapshot(&fixture.temp), files);
}

#[test]
fn refresh_invalid_syntax_failures_and_obsolete_recovery_keep_graph_and_signal() {
    let mut fixture = Fixture::new();
    fixture.app.command(&format!("backlinks {ID}"));
    fixture.finish();
    fixture.app.scroll = 2;
    fixture.app.external_change_pending = true;
    let before = view(&fixture.app);
    let report = fixture.app.link_report.clone();
    for command in ["refresh extra", "refresh --json"] {
        fixture.app.command(command);
        assert!(fixture.jobs.try_recv().is_err());
        assert_eq!(view(&fixture.app), before);
    }
    fs::write(fixture.root().join(GLOBAL_ID), "invalid note").unwrap();
    fixture.app.command("refresh");
    fixture.finish();
    assert_eq!(view(&fixture.app), before);
    assert!(fixture.app.link_report == report && fixture.app.external_change_pending);
    let journal = fixture
        .root()
        .join("Projects/example/.akasha-edit-journal.json");
    fs::write(journal, "invalid journal").unwrap();
    let response = transition(&fixture, false);
    assert!(matches!(response, Ok(Response::RecoveryRequired(..))));
    fixture.app.request.cwd = fixture.temp.join("other");
    let files = link_snapshot(&fixture.temp);
    fixture.app.receive(response);
    assert_eq!(view(&fixture.app), before);
    assert!(fixture.app.projection.is_some() && !fixture.app.recovery_blocked);
    assert!(fixture.app.link_report == report);
    assert_eq!(link_snapshot(&fixture.temp), files);
}

#[test]
fn explicit_navigation_and_reload_from_an_unchanged_clean_editor_succeed() {
    for reload in [false, true] {
        let mut fixture = Fixture::new();
        fixture.open_editor();
        assert!(!fixture.app.dirty() && fixture.app.editing);
        let files = link_snapshot(&fixture.temp);
        fixture.app.command(if reload {
            "refresh"
        } else {
            "open Global/entities/rust-pattern.md"
        });
        assert!(fixture.app.busy);
        fixture.finish();
        assert!(!fixture.app.busy && !fixture.app.editing);
        assert!(fixture.app.editor.is_none());
        if reload {
            assert!(fixture.app.document.is_none());
            assert_eq!(fixture.app.body_title, "WELCOME TO AKASHA");
        } else {
            assert_eq!(fixture.app.document.as_ref().unwrap().id, GLOBAL_ID);
        }
        assert_eq!(link_snapshot(&fixture.temp), files);
    }
}
