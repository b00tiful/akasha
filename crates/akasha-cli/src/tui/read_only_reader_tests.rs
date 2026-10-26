use super::*;

fn commands(fixture: &Fixture) -> [String; 5] {
    [
        "validate".into(),
        "breadcrumb".into(),
        "template task".into(),
        "onboard".into(),
        format!("integrations codex {}", fixture.temp.display()),
    ]
}

fn job(fixture: &Fixture, reader: usize) -> Job {
    let request = fixture.app.request.clone();
    match reader {
        0 => Job::Validate(request),
        1 => Job::Breadcrumb(request),
        2 => Job::Template(request, "task".into()),
        3 => Job::Onboard(request),
        4 => Job::InspectIntegrations(request, AgentClient::Codex, fixture.temp.clone()),
        _ => unreachable!(),
    }
}

#[test]
fn read_only_reader_syntax_refuses_before_submission_and_keeps_graph_state() {
    let mut fixture = Fixture::new();
    fixture.app.command(&format!("backlinks {ID}"));
    fixture.finish();
    fixture.app.scroll = 2;
    let body = fixture.app.body.clone();
    let report = fixture.app.link_report.clone();
    let before = link_snapshot(&fixture.temp);
    for command in [
        "validate extra",
        "validate --json",
        "breadcrumb extra",
        "onboard extra",
        "template",
        "template task extra",
        "integrations",
        "integrations vscode",
    ] {
        fixture.app.command(command);
        assert!(!fixture.app.busy, "{command}");
        assert!(fixture.jobs.try_recv().is_err(), "{command}");
        assert_eq!(fixture.app.body, body, "{command}");
        assert_eq!(fixture.app.scroll, 2, "{command}");
        assert!(fixture.app.link_report == report, "{command}");
    }
    fixture.app.command("global");
    fixture.app.command("template task");
    assert!(fixture.jobs.try_recv().is_err());
    assert_eq!(link_snapshot(&fixture.temp), before);
}

#[test]
fn read_only_reader_fresh_results_match_core_and_busy_input_cannot_start_editing() {
    let mut fixture = Fixture::new();
    let request = fixture.app.request.clone();
    let template = resolve_note_template(&request, "task").unwrap();
    let breadcrumb = render_session_breadcrumb(&assemble_session_breadcrumb(&request).unwrap());
    let handoff = crate::render::onboarding_handoff_text(
        &akasha_core::prepare_onboarding_handoff(&request).unwrap(),
    );
    let before = link_snapshot(&fixture.temp);
    for (reader, command) in commands(&fixture).iter().enumerate() {
        fixture.app.command(&format!("links {ID}"));
        fixture.finish();
        fixture.app.command(command);
        assert!(fixture.app.busy);
        // Normal command dispatch cannot create an editor/form while the job is pending.
        fixture.app.command("edit");
        fixture.app.command("create task");
        assert!(fixture.app.editor.is_none() && fixture.app.workflow.is_none());
        let response = execute_job(fixture.jobs.try_recv().unwrap());
        assert!(fixture.jobs.try_recv().is_err());
        let Ok(Response::Text(origin, _, _)) = &response else {
            panic!("expected a successful inspection: {command}");
        };
        assert!(fixture.app.request_is_current(origin), "{command}");
        fixture.app.receive(response);
        assert!(!fixture.app.busy);
        assert!(fixture.app.document.is_none() && fixture.app.editor.is_none());
        assert!(fixture.app.link_report.is_none() && fixture.app.link_return.is_none());
        assert_eq!(fixture.app.scroll, 0);
        match reader {
            0 => {
                let report = validate_project(&request).unwrap();
                assert_eq!(fixture.app.body_title, "VALIDATION");
                assert!(fixture.app.body.contains(&format!("Project: {}", report.project)));
                assert!(fixture.app.body.contains(&format!("Root: {}", report.root.display())));
            }
            1 => assert_eq!(fixture.app.body, breadcrumb),
            2 => {
                assert_eq!(fixture.app.body_title, "TEMPLATE · task");
                assert!(fixture.app.body.ends_with(&template.source));
                assert!(fixture.app.body.contains(&template.path.display().to_string()));
            }
            3 => assert_eq!(fixture.app.body, handoff),
            4 => assert!(fixture.app.body.contains("READ-ONLY INSPECTION")),
            _ => unreachable!(),
        }
        fixture.app.command("back");
        assert_eq!(link_snapshot(&fixture.temp), before);
    }
}

#[test]
fn read_only_reader_obsolete_recovery_and_editing_replies_keep_graph_and_return() {
    for reader in 0..5 {
        for change in ["root", "project", "cwd", "environment", "config-home", "home",
            "recovery", "blocked", "editing", "return"] {
            let mut fixture = Fixture::new();
            fixture.app.command("links Projects/example/events/sessions/2026-07-13.md");
            fixture.finish();
            fixture.app.scroll = 2;
            let response = execute_job(job(&fixture, reader));
            assert!(matches!(&response, Ok(Response::Text(..))));
            if change == "return" {
                fixture.app.command("follow 1");
                fixture.finish();
                assert!(fixture.app.link_return.is_some());
            }
            match change {
                "root" => fixture.app.request.root_override = None,
                "project" => fixture.app.request.project_override = None,
                "cwd" | "return" => fixture.app.request.cwd = fixture.temp.join("other"),
                "environment" => fixture.app.request.environment.akasha_root = Some("other".into()),
                "config-home" => fixture.app.request.environment.xdg_config_home = Some("other".into()),
                "home" => fixture.app.request.environment.home = Some("other".into()),
                "recovery" => fixture.app.recovery_visible = true,
                "blocked" => fixture.app.recovery_blocked = true,
                "editing" => fixture.app.editing = true,
                _ => unreachable!(),
            }
            let body = fixture.app.body.clone();
            let title = fixture.app.body_title.clone();
            let report = fixture.app.link_report.clone();
            let parent = fixture.app.link_return.clone();
            let scroll = fixture.app.scroll;
            let focus = fixture.app.focus;
            let document = fixture.app.document.as_ref().map(|d| (d.id.clone(), d.source.clone()));
            let before = link_snapshot(&fixture.temp);
            fixture.app.receive(response);
            assert_eq!(fixture.app.body, body, "reader {reader}: {change}");
            assert_eq!(fixture.app.body_title, title);
            assert_eq!(fixture.app.scroll, scroll);
            assert!(fixture.app.focus == focus);
            assert!(fixture.app.link_report == report && fixture.app.link_return == parent);
            assert_eq!(fixture.app.document.as_ref().map(|d| (d.id.clone(), d.source.clone())), document);
            assert_eq!(link_snapshot(&fixture.temp), before);
        }
    }
}

#[test]
fn read_only_reader_commands_and_late_replies_preserve_editor_undo_and_creation() {
    for reader in 0..5 {
        let mut fixture = Fixture::new();
        fixture.open_editor();
        let original = fixture.app.editor.as_ref().unwrap().source();
        let response = execute_job(job(&fixture, reader));
        fixture.app.paste("\nRetained inspection draft Привет 世界  ");
        let draft = fixture.app.editor.as_ref().unwrap().source();
        fixture.app.command(&commands(&fixture)[reader]);
        assert!(fixture.jobs.try_recv().is_err());
        fixture.app.receive(response);
        assert_eq!(fixture.app.editor.as_ref().unwrap().source(), draft);
        assert!(fixture.app.editor.as_mut().unwrap().area.undo());
        assert_eq!(fixture.app.editor.as_ref().unwrap().source(), original);
        fixture.app.command("discard");

        let response = execute_job(job(&fixture, reader));
        author_mutable(&mut fixture, "task", false);
        let projection = fixture.app.active_editor().unwrap().source();
        fixture.app.command(&commands(&fixture)[reader]);
        assert!(fixture.jobs.try_recv().is_err());
        let before = link_snapshot(&fixture.temp);
        fixture.app.receive(response);
        assert!(fixture.app.workflow.is_some());
        assert_eq!(fixture.app.active_editor().unwrap().source(), projection);
        fixture.app.previous_workflow_step();
        assert!(fixture.app.active_editor().unwrap().source().contains("Exact body end"));
        fixture.app.command("discard");
        assert_eq!(link_snapshot(&fixture.temp), before);
    }
}

#[test]
fn read_only_reader_late_replies_preserve_both_lifecycle_drafts_and_admin_review() {
    for reader in 0..5 {
        let mut fixture = Fixture::new();
        let response = execute_job(job(&fixture, reader));
        fixture.app.open(ID);
        fixture.finish();
        fixture.app.command("lifecycle");
        fixture.finish();
        let Some(Workflow::Lifecycle(form)) = &mut fixture.app.workflow else { unreachable!() };
        let note = format!("{}\nRetained note 世界  ", form.prepared.source);
        let projection = format!("{}\nRetained index Привет  ", form.prepared.projection_source);
        form.note = Editor::new(&note).unwrap();
        form.projection = Editor::new(&projection).unwrap();
        fixture.app.command(&commands(&fixture)[reader]);
        assert!(fixture.jobs.try_recv().is_err());
        let before = link_snapshot(&fixture.temp);
        fixture.app.receive(response);
        let Some(Workflow::Lifecycle(form)) = &fixture.app.workflow else { unreachable!() };
        assert_eq!(form.note.source(), note);
        assert_eq!(form.projection.source(), projection);
        fixture.app.command("discard");

        let response = execute_job(job(&fixture, reader));
        fixture.app.command(&format!("link example {}", fixture.temp.join("repository").display()));
        fixture.finish();
        let review = fixture.app.link_review.as_ref().unwrap().plan.plan_id.clone();
        fixture.app.command(&commands(&fixture)[reader]);
        assert!(fixture.jobs.try_recv().is_err());
        fixture.app.receive(response);
        assert_eq!(fixture.app.link_review.as_ref().unwrap().plan.plan_id, review);
        fixture.app.command("discard");
        assert_eq!(link_snapshot(&fixture.temp), before);
    }
}

#[test]
fn read_only_reader_integration_keeps_original_request_and_independent_client_errors() {
    let mut fixture = Fixture::new();
    let home = fixture.temp.join("client home 世界");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("AGENTS.override.md"), "human override\r\n").unwrap();
    fs::write(home.join("settings.json"), "invalid JSON\n").unwrap();
    // Root-level inspection deliberately ignores this project only in its core input.
    fixture.app.request.project_override = Some("not-registered".into());
    let before = link_snapshot(&fixture.temp);
    for client in [AgentClient::Codex, AgentClient::Claude] {
        let response = execute_job(Job::InspectIntegrations(fixture.app.request.clone(), client, home.clone()));
        let Ok(Response::Text(origin, _, body)) = &response else { unreachable!() };
        assert!(fixture.app.request_is_current(origin));
        assert_eq!(origin.project_override.as_deref(), Some("not-registered"));
        assert!(body.contains("Status: prepared") && body.contains("Status: unavailable"));
        fixture.app.receive(response);
        assert!(fixture.app.body_title.starts_with("INTEGRATIONS · "));
        assert_eq!(link_snapshot(&fixture.temp), before);
    }
}

#[test]
fn read_only_reader_failures_keep_graph_and_existing_pending_journal_policy() {
    for reader in 0..5 {
        let mut fixture = Fixture::new();
        fixture.app.command(&format!("backlinks {ID}"));
        fixture.finish();
        fixture.app.scroll = 2;
        let body = fixture.app.body.clone();
        let report = fixture.app.link_report.clone();
        let before = link_snapshot(&fixture.temp);
        // Ordinary failures preserve reader bindings and scroll for every inspection.
        let mut invalid = fixture.app.request.clone();
        invalid.root_override = Some(fixture.temp.join("missing-root"));
        let failed = match job(&fixture, reader) {
            Job::Validate(_) => execute_job(Job::Validate(invalid)),
            Job::Breadcrumb(_) => execute_job(Job::Breadcrumb(invalid)),
            Job::Template(_, kind) => execute_job(Job::Template(invalid, kind)),
            Job::Onboard(_) => execute_job(Job::Onboard(invalid)),
            Job::InspectIntegrations(_, client, home) => {
                let response = execute_job(Job::InspectIntegrations(invalid, client, home));
                assert!(matches!(&response, Ok(Response::Text(_, _, body)) if body.matches("Status: unavailable").count() == 2));
                response // A root-bound stale reply is retained, including its two errors.
            }
            _ => unreachable!(),
        };
        fixture.app.receive(failed);
        assert_eq!(fixture.app.body, body);
        assert_eq!(fixture.app.scroll, 2);
        assert!(fixture.app.link_report == report);
        assert_eq!(link_snapshot(&fixture.temp), before);

        let journal = fixture.root().join("Projects/example").join(akasha_core::NOTE_EDIT_JOURNAL_FILE);
        fs::write(&journal, b"unfinished journal\n").unwrap();
        let interrupted = link_snapshot(&fixture.temp);
        let response = execute_job(job(&fixture, reader));
        // A healthy project read does not infer failure from journal presence alone.
        assert!(matches!(response, Ok(Response::Text(..))));
        assert_eq!(link_snapshot(&fixture.temp), interrupted);

        // The existing adapter only offers recovery guidance after an actual failure.
        fs::write(fixture.root().join(ID), b"invalid canonical note\n").unwrap();
        let invalid = link_snapshot(&fixture.temp);
        let response = execute_job(job(&fixture, reader));
        if matches!(reader, 0 | 1 | 3) {
            assert!(matches!(response, Ok(Response::RecoveryRequired(..))));
        } else {
            // Template resolution and root-level planning are not full project validation.
            assert!(matches!(response, Ok(Response::Text(..))));
        }
        assert_eq!(link_snapshot(&fixture.temp), invalid);
    }
}
