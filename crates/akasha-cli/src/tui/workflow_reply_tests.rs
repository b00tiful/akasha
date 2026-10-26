use super::*;

fn fixture() -> Fixture {
    let f = Fixture::new();
    fs::write(
        f.root().join("Projects/example/templates/session.md"),
        EVENT_TEMPLATE,
    )
    .unwrap();
    f
}

const INPUTS: [&str; 6] = [
    "root",
    "project",
    "cwd",
    "environment",
    "config-home",
    "home",
];

fn change_request(f: &mut Fixture, input: &str) {
    match input {
        "root" => f.app.request.root_override = None,
        "project" => f.app.request.project_override = None,
        "cwd" => f.app.request.cwd = f.temp.join("different cwd"),
        "environment" => f.app.request.environment.akasha_root = Some("other".into()),
        "config-home" => f.app.request.environment.xdg_config_home = Some("other".into()),
        "home" => f.app.request.environment.home = Some("other".into()),
        _ => unreachable!(),
    }
}

fn preparation(f: &Fixture, kind: usize) -> WorkResult {
    let origin = f.app.view_origin();
    let job = match kind {
        0 => Job::PrepareEvent(origin, Some("session".into())),
        1 => Job::PrepareCreate(origin, "task".into()),
        2 => Job::PrepareLifecycle(origin, ID.into()),
        3 => {
            let home = f.temp.join("client home 世界");
            fs::create_dir_all(&home).unwrap();
            Job::PrepareIntegration(
                origin,
                IntegrationOperation {
                    request: f.app.request.clone(),
                    client: AgentClient::Codex,
                    home,
                    remove: false,
                    instructions: true,
                },
            )
        }
        4 => Job::PrepareSetup(origin, f.temp.join("new root 世界")),
        5 => Job::PrepareInit(
            origin,
            init::arguments(&f.app.request, "another repository").unwrap(),
        ),
        6 => Job::PrepareInitRecovery(origin),
        7 => Job::PrepareLink(
            origin,
            link::arguments(&f.app.request, "example repository").unwrap(),
        ),
        _ => unreachable!(),
    };
    let response = execute_job(job);
    assert!(
        response.is_ok(),
        "kind={kind}: {:?}",
        response.as_ref().err()
    );
    response
}

fn graph_parent(f: &mut Fixture) {
    f.app
        .command("links Projects/example/events/sessions/2026-07-13.md");
    f.finish();
    f.app.command("follow 1");
    f.finish();
    assert_eq!(f.app.document.as_ref().unwrap().id, ID);
    assert!(f.app.link_return.is_some());
    f.app.scroll = 3;
}

#[derive(Debug, PartialEq, Eq)]
struct View {
    title: String,
    body: String,
    document: Option<(String, String)>,
    editor: Option<(String, String)>,
    workflow: Option<(String, Vec<String>)>,
    reviews: Vec<String>,
    prompt: Vec<String>,
    scroll: u16,
    focus: u8,
    editing: bool,
    recovery: (bool, bool),
}

fn view(app: &App) -> View {
    View {
        title: app.body_title.clone(),
        body: app.body.clone(),
        document: app
            .document
            .as_ref()
            .map(|d| (d.id.clone(), d.source.clone())),
        editor: app
            .editor
            .as_ref()
            .map(|e| (e.original.clone(), e.source())),
        workflow: app
            .workflow
            .as_ref()
            .map(|_| (app.workflow_title().unwrap(), workflow_sources(app))),
        reviews: [
            app.integration_review.as_ref().map(|r| r.plan_id.clone()),
            app.setup_review.as_ref().map(|r| r.plan_id.clone()),
            app.init_review.as_ref().map(|r| r.plan.plan_id.clone()),
            app.init_recovery_review
                .as_ref()
                .map(|r| r.plan.plan_id.clone()),
            app.link_review.as_ref().map(|r| r.plan.plan_id.clone()),
        ]
        .into_iter()
        .flatten()
        .collect(),
        prompt: app.prompt.lines().to_vec(),
        scroll: app.scroll,
        focus: match app.focus {
            Focus::Prompt => 0,
            Focus::List => 1,
            Focus::Reader => 2,
        },
        editing: app.editing,
        recovery: (app.recovery_visible, app.recovery_blocked),
    }
}

fn retained(f: &mut Fixture, response: WorkResult) {
    let before = view(&f.app);
    let report = f.app.link_report.clone();
    let parent = f.app.link_return.clone();
    let files = link_snapshot(&f.temp);
    f.app.receive(response);
    assert_eq!(view(&f.app), before);
    assert!(f.app.link_report == report);
    assert!(f.app.link_return == parent);
    assert_eq!(link_snapshot(&f.temp), files);
    assert!(f.app.messages.back().unwrap().contains("retained"));
}

#[test]
fn all_preparations_bind_every_original_session_input_before_graph_mutation() {
    for kind in 0..8 {
        for input in INPUTS {
            let mut f = fixture();
            graph_parent(&mut f);
            let response = preparation(&f, kind);
            change_request(&mut f, input);
            retained(&mut f, response);
            assert!(
                f.app.messages.back().unwrap().contains("obsolete"),
                "{kind}/{input}"
            );
        }
    }
}

#[test]
fn preparation_keeps_new_clean_dirty_source_history_and_redo() {
    for kind in 0..8 {
        for dirty in [false, true] {
            let mut f = fixture();
            graph_parent(&mut f);
            let response = preparation(&f, kind);
            f.app.edit();
            let original = f.app.editor.as_ref().unwrap().source();
            f.app.paste("\nRetained Привет 世界  ");
            let draft = f.app.editor.as_ref().unwrap().source();
            if !dirty {
                assert!(f.app.editor.as_mut().unwrap().area.undo());
            }
            retained(&mut f, response);
            let editor = f.app.editor.as_mut().unwrap();
            if dirty {
                assert!(editor.area.undo());
            } else {
                assert!(editor.area.redo());
            }
            assert_eq!(editor.source(), if dirty { original } else { draft });
        }
    }
}

#[test]
fn preparation_retains_multiline_forms_reviews_and_recovery_views() {
    for kind in 0..8 {
        for state in 0..5 {
            let mut f = fixture();
            graph_parent(&mut f);
            let response = preparation(&f, kind);
            match state {
                0 => {
                    author_mutable(&mut f, "task", true);
                    review_creation(&mut f);
                }
                1 => {
                    f.app.command("lifecycle");
                    f.finish();
                    review_lifecycle(&mut f);
                }
                2 => {
                    let review = preparation(&f, 3);
                    f.app.receive(review);
                }
                3 => f.app.show_recovery(),
                4 => f.app.recovery_blocked = true,
                _ => unreachable!(),
            }
            f.app.scroll = 2;
            retained(&mut f, response);
        }
    }
}

#[test]
fn preparation_accepts_explicit_clean_editor_intent_and_clears_old_graph_only_on_success() {
    for kind in 0..8 {
        let mut f = fixture();
        graph_parent(&mut f);
        f.app.edit();
        assert!(!f.app.dirty());
        let response = preparation(&f, kind);
        let before = link_snapshot(&f.temp);
        f.app.receive(response);
        assert_eq!(link_snapshot(&f.temp), before);
        assert!(f.app.link_report.is_none() && f.app.link_return.is_none());
        // Lifecycle enters its existing source editor; other preparations open a fresh reader.
        if kind != 2 {
            assert_eq!(f.app.scroll, 0);
        }
        assert!(
            f.app.in_workflow()
                || f.app.reviewing_administration()
                || f.app.body.starts_with("No pending initialization")
        );
    }
}

#[test]
fn preparation_failure_and_invalid_editor_sources_retain_graph_and_reader() {
    let mut f = fixture();
    graph_parent(&mut f);
    let before = view(&f.app);
    let parent = f.app.link_return.clone();
    f.app.receive(execute_job(Job::PrepareCreate(
        f.app.view_origin(),
        "unknown".into(),
    )));
    assert_eq!(view(&f.app), before);
    assert!(f.app.link_return == parent);
    for kind in [0, 1, 2] {
        let mut response = preparation(&f, kind);
        match &mut response {
            Ok(Response::EventPrepared(_, form)) => form.template_source.push('\0'),
            Ok(Response::CreatePrepared(_, form)) => form.projection_source.push('\0'),
            Ok(Response::LifecyclePrepared(_, form)) => form.source.push('\0'),
            _ => unreachable!(),
        }
        f.app.receive(response);
        assert_eq!(view(&f.app), before);
        assert!(f.app.link_return == parent);
        assert!(
            f.app
                .messages
                .back()
                .unwrap()
                .starts_with("Operation failed")
        );
    }
}

#[test]
fn lifecycle_preparation_retains_a_new_document_then_accepts_fresh_preparation() {
    let mut f = fixture();
    graph_parent(&mut f);
    let response = preparation(&f, 2);
    f.app.open(TASK_ID);
    f.finish();
    retained(&mut f, response);
    f.app.command("lifecycle");
    f.finish();
    let Some(Workflow::Lifecycle(form)) = &f.app.workflow else {
        panic!("fresh preparation refused")
    };
    assert_eq!(form.prepared.id, TASK_ID);
}

#[test]
fn initialization_recovery_preparation_binds_populated_empty_and_refused_results() {
    for state in 0..3 {
        let mut f = fixture();
        graph_parent(&mut f);
        if state == 1 {
            init_recovery_fixture::stage(
                &f.root(),
                &f.temp.join("recovery repo"),
                "interrupted",
                false,
            );
        } else if state == 2 {
            fs::write(
                f.root()
                    .join("Meta/.projects.yaml.akasha-init-journal.json"),
                "invalid journal\n",
            )
            .unwrap();
        }
        let response = preparation(&f, 6);
        match &response {
            Ok(Response::InitRecoveryPrepared(_, Ok(None))) => assert_eq!(state, 0),
            Ok(Response::InitRecoveryPrepared(_, Ok(Some(_)))) => assert_eq!(state, 1),
            Ok(Response::InitRecoveryPrepared(_, Err(_))) => assert_eq!(state, 2),
            _ => unreachable!(),
        }
        change_request(&mut f, "cwd");
        retained(&mut f, response);
    }
}

fn ready_preview(f: &mut Fixture, kind: usize) {
    match kind {
        0 => {
            guided_event(f, false, true);
            f.app.previous_workflow_step();
        }
        1 => author_mutable(f, "task", true),
        2 => {
            f.app.open(ID);
            f.finish();
            f.app.command("lifecycle");
            f.finish();
            f.app.next_workflow_step();
        }
        _ => unreachable!(),
    }
}

fn preview(f: &mut Fixture) -> WorkResult {
    f.app.next_workflow_step();
    assert!(f.app.busy);
    let result = execute_job(f.jobs.try_recv().unwrap());
    assert!(result.is_ok(), "{:?}", result.as_ref().err());
    // Delay delivery deterministically; production freezes draft input while busy.
    f.app.busy = false;
    result
}

#[test]
fn event_preview_rejects_revised_path_fields_baseline_and_stage() {
    for changed in 0..5 {
        let mut f = fixture();
        ready_preview(&mut f, 0);
        let response = preview(&mut f);
        let Some(Workflow::Event(form)) = &mut f.app.workflow else {
            unreachable!()
        };
        match changed {
            0 => form.path = "revised.md".into(),
            1 => {
                form.fields
                    .last_mut()
                    .unwrap()
                    .1
                    .area
                    .insert_str("\nRevised 世界  ");
            }
            2 => form
                .prepared
                .template_source
                .push_str("\nchanged baseline\n"),
            3 => form.stage = EventStage::Path,
            4 => form.path = format!("./{}", form.path),
            _ => unreachable!(),
        };
        retained(&mut f, response);
        assert_eq!(f.app.event_stage(), Some(false));
    }
}

#[test]
fn creation_preview_rejects_revised_path_fields_projection_baseline_and_stage() {
    for changed in 0..6 {
        let mut f = fixture();
        ready_preview(&mut f, 1);
        let response = preview(&mut f);
        let Some(Workflow::Creation(form)) = &mut f.app.workflow else {
            unreachable!()
        };
        match changed {
            0 => form.path = "revised.md".into(),
            1 => {
                form.fields
                    .last_mut()
                    .unwrap()
                    .1
                    .area
                    .insert_str("\nRevised 世界  ");
            }
            2 => {
                form.projection.area.insert_str("\nRevised projection\n");
            }
            3 => form
                .prepared
                .template_source
                .push_str("\nchanged baseline\n"),
            4 => form.stage = CreationStage::Field(0),
            5 => form.path = format!("./{}", form.path),
            _ => unreachable!(),
        }
        retained(&mut f, response);
        assert!(f.app.creation_review().is_none());
    }
}

#[test]
fn lifecycle_preview_rejects_revised_note_projection_baseline_and_stage() {
    for changed in 0..5 {
        let mut f = fixture();
        ready_preview(&mut f, 2);
        let response = preview(&mut f);
        let Some(Workflow::Lifecycle(form)) = &mut f.app.workflow else {
            unreachable!()
        };
        match changed {
            0 => {
                form.note.area.insert_str("\nRevised note\n");
            }
            1 => {
                form.projection.area.insert_str("\nRevised projection\n");
            }
            2 => form.prepared.source.push_str("\nchanged baseline\n"),
            3 => form.pane = LifecyclePane::Note,
            4 => form.reviewing = true,
            _ => unreachable!(),
        }
        retained(&mut f, response);
    }
}

#[test]
fn previews_bind_all_original_request_fields_and_keep_cancelled_replaced_or_hidden_forms() {
    for kind in 0..3 {
        for input in INPUTS {
            let mut f = fixture();
            ready_preview(&mut f, kind);
            let response = preview(&mut f);
            change_request(&mut f, input);
            retained(&mut f, response);
        }
        for state in 0..5 {
            let mut f = fixture();
            ready_preview(&mut f, kind);
            let response = preview(&mut f);
            match state {
                0 => f.app.command("discard"),
                1 => {
                    f.app.command("discard");
                    ready_preview(&mut f, (kind + 1) % 3);
                }
                2 => f.app.show_recovery(),
                3 => f.app.recovery_blocked = true,
                4 => {
                    f.app.command("discard");
                    let review = preparation(&f, 3);
                    f.app.receive(review);
                }
                _ => unreachable!(),
            }
            f.app.scroll = 2;
            retained(&mut f, response);
        }
    }
}

#[test]
fn fresh_preview_after_refusal_reviews_the_exact_revised_drafts_and_retains_undo() {
    for kind in 0..3 {
        let mut f = fixture();
        ready_preview(&mut f, kind);
        let response = preview(&mut f);
        let editor = f.app.active_editor_mut().unwrap();
        let original = editor.source();
        editor.area.move_cursor(CursorMove::Bottom);
        editor.area.move_cursor(CursorMove::End);
        editor.area.insert_str("\nNew Привет 世界  \n");
        let revised = f.app.active_editor().unwrap().source();
        retained(&mut f, response);
        let response = preview(&mut f);
        let before = link_snapshot(&f.temp);
        f.app.receive(response);
        assert_eq!(link_snapshot(&f.temp), before);
        assert!(
            f.app.event_stage() == Some(true)
                || f.app.creation_review().is_some()
                || f.app.lifecycle_review().is_some()
        );
        f.app.previous_workflow_step();
        let editor = f.app.active_editor_mut().unwrap();
        assert_eq!(editor.source(), revised);
        assert!(editor.area.undo());
        assert_eq!(editor.source(), original);
        assert!(editor.area.redo());
        assert_eq!(editor.source(), revised);
    }
}

#[test]
fn busy_preview_freezes_real_keyboard_paste_navigation_save_and_discard() {
    for kind in 0..3 {
        let mut f = fixture();
        ready_preview(&mut f, kind);
        f.app.next_workflow_step();
        assert!(f.app.busy);
        let sources = workflow_sources(&f.app);
        f.app.paste("must not enter draft");
        f.app
            .key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        for command in ["next", "previous", "discard", "save", "quit"] {
            f.app.command(command);
        }
        assert_eq!(workflow_sources(&f.app), sources);
        let response = execute_job(f.jobs.try_recv().unwrap());
        assert!(f.jobs.try_recv().is_err());
        f.app.receive(response);
        assert!(!f.app.busy && !f.app.quit);
        assert!(
            f.app.event_stage() == Some(true)
                || f.app.creation_review().is_some()
                || f.app.lifecycle_review().is_some()
        );
    }
}
