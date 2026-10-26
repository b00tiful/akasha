use super::*;

const INPUTS: [&str; 6] = [
    "root",
    "project",
    "cwd",
    "environment",
    "config-home",
    "home",
];

fn change_request(f: &mut Fixture, field: &str) {
    match field {
        "root" => f.app.request.root_override = None,
        "project" => f.app.request.project_override = None,
        "cwd" => f.app.request.cwd = f.temp.join("different cwd"),
        "environment" => f.app.request.environment.akasha_root = Some("other".into()),
        "config-home" => f.app.request.environment.xdg_config_home = Some("other".into()),
        "home" => f.app.request.environment.home = Some("other".into()),
        _ => unreachable!(),
    }
}

fn view(app: &App) -> Vec<String> {
    let mut result = vec![
        format!(
            "{:?}/{:?}/{:?}/{:?}/{:?}/{:?}",
            app.request.root_override,
            app.request.project_override,
            app.request.cwd,
            app.request.environment.akasha_root,
            app.request.environment.xdg_config_home,
            app.request.environment.home
        ),
        app.body_title.clone(),
        app.body.clone(),
        format!("{:?}", app.prompt.lines()),
        format!("{:?}/{}", app.scope, app.scroll),
        format!(
            "{}/{}/{}/{}",
            app.editing,
            app.recovery_visible,
            app.recovery_blocked,
            match app.focus {
                Focus::Prompt => 0,
                Focus::List => 1,
                Focus::Reader => 2,
            }
        ),
        format!("{:?}", app.document.as_ref().map(|d| (&d.id, &d.source))),
        format!(
            "{:?}",
            app.editor.as_ref().map(|e| (&e.original, e.source()))
        ),
        format!("{:?}", app.pending_open),
        app.workflow_title().unwrap_or_default(),
    ];
    if app.workflow.is_some() {
        result.extend(workflow_sources(app));
    }
    result.extend(
        [
            AdministrationKind::Integration,
            AdministrationKind::Setup,
            AdministrationKind::Init,
            AdministrationKind::InitRecovery,
            AdministrationKind::Link,
        ]
        .map(|kind| app.administration_id(kind).unwrap_or_default().to_owned()),
    );
    result
}

fn retained(f: &mut Fixture, response: WorkResult, status: &str) {
    retained_after_consumption(f, response, status, None);
}

fn retained_after_consumption(
    f: &mut Fixture,
    response: WorkResult,
    status: &str,
    consumed: Option<usize>,
) {
    let mut before = view(&f.app);
    if let Some(kind) = consumed {
        let index = before.len() - 5 + kind;
        before[index].clear();
    }
    let projection = f.app.projection.clone();
    let report = f.app.link_report.clone();
    let parent = f.app.link_return.clone();
    let files = link_snapshot(&f.temp);
    f.app.receive(response);
    assert_eq!(view(&f.app), before);
    assert_eq!(f.app.projection, projection);
    assert!(f.app.link_report == report && f.app.link_return == parent);
    assert_eq!(
        link_snapshot(&f.temp),
        files,
        "delivery must perform no write"
    );
    let message = f.app.messages.back().unwrap();
    assert!(
        message.contains("retained") && message.contains(status),
        "{message}"
    );
    assert!(
        f.jobs.try_recv().is_err(),
        "retained completion must not queue reload"
    );
}

fn finish_write(f: &mut Fixture) -> WorkResult {
    assert!(f.app.busy);
    let response = execute_job(f.jobs.try_recv().unwrap());
    assert!(matches!(response, Ok(Response::WriteFinished(..))));
    f.app.busy = false; // Deterministically delay delivery after the real core operation.
    response
}

fn form(f: &mut Fixture, kind: usize) {
    match kind {
        0 => guided_event(f, false, true),
        1 => {
            author_mutable(f, "task", true);
            review_creation(f);
        }
        2 => {
            f.app.open(ID);
            f.finish();
            f.app.command("lifecycle");
            f.finish();
            let editor = f.app.active_editor_mut().unwrap();
            editor.area.move_cursor(CursorMove::Bottom);
            editor.area.move_cursor(CursorMove::End);
            editor.area.insert_str("\nReviewed Привет 世界\n");
            review_lifecycle(f);
        }
        _ => unreachable!(),
    }
}

fn administration(f: &mut Fixture, kind: usize, newer: bool) {
    let suffix = if newer { "newer" } else { "submitted" };
    let repository = f.temp.join(format!("{suffix} repository 世界"));
    fs::create_dir_all(&repository).unwrap();
    let command = match kind {
        0 => {
            let home = f.temp.join(format!("{suffix} home 世界"));
            fs::create_dir_all(&home).unwrap();
            format!("integration apply instructions codex {}", home.display())
        }
        1 => {
            // /setup is a first-run operation; a loaded project correctly refuses it.
            f.app.projection = None;
            format!(
                "setup {}",
                f.temp.join(format!("{suffix} root 世界")).display()
            )
        }
        2 => format!("init {suffix} {}", repository.display()),
        3 => {
            init_recovery_fixture::stage(&f.root(), &repository, suffix, false);
            "recover-init".into()
        }
        4 if !newer => format!("link example {}", f.temp.join("repository").display()),
        4 => {
            // Link preparation must use the exact registered repository. Create a second
            // synthetic registration, then remove only its fixture pointer to review linking it.
            let request =
                init::arguments(&f.app.request, &format!("newer {}", repository.display()))
                    .unwrap();
            let plan = akasha_core::prepare_project_init(&request).unwrap();
            let result = akasha_core::apply_project_init(&request, &plan).unwrap();
            fs::remove_file(result.pointer).unwrap();
            format!("link newer {}", repository.display())
        }
        _ => unreachable!(),
    };
    f.app.command(&command);
    f.finish();
    assert!(
        f.app.reviewing_administration(),
        "{command}: {:?}",
        f.app.messages
    );
}

fn confirm(f: &mut Fixture, kind: usize) {
    let kind = [
        AdministrationKind::Integration,
        AdministrationKind::Setup,
        AdministrationKind::Init,
        AdministrationKind::InitRecovery,
        AdministrationKind::Link,
    ][kind];
    let id = f.app.administration_id(kind).unwrap().to_owned();
    f.app.command(&format!("confirm {id}"));
}

#[test]
fn save_completion_retains_newer_text_original_baseline_and_exact_undo() {
    let mut f = Fixture::new();
    f.open_editor();
    let original = f.app.editor.as_ref().unwrap().source();
    f.app.paste("\nSubmitted Привет 世界  \n");
    let submitted = f.app.editor.as_ref().unwrap().source();
    f.app.save();
    // Real input cannot revise a submitted write while busy.
    f.app.paste("ignored");
    f.app.command("discard");
    f.app.save();
    assert_eq!(f.app.editor.as_ref().unwrap().source(), submitted);
    let response = finish_write(&mut f);
    assert_eq!(fs::read_to_string(f.path()).unwrap(), submitted);
    f.app.paste("Newer draft λ\n");
    let draft = f.app.editor.as_ref().unwrap().source();
    retained(&mut f, response, "completed");
    let editor = f.app.editor.as_mut().unwrap();
    assert_eq!(editor.original, original, "no silent rebase");
    assert!(editor.area.undo());
    assert_eq!(editor.source(), submitted);
    assert!(editor.area.redo());
    assert_eq!(editor.source(), draft);
    assert!(f.app.external_change_pending);
    // Explicitly resolve the retained draft, then reopen the committed source.
    f.app.command("discard");
    f.app.command("refresh");
    f.finish();
    f.app.open(ID);
    f.finish();
    assert_eq!(f.app.document.as_ref().unwrap().source, submitted);
    validate_project(&f.app.request).unwrap();
}

#[test]
fn save_completion_cannot_assign_submitted_source_to_a_different_document() {
    let mut f = Fixture::new();
    f.open_editor();
    f.app.paste("\nSaved entity.\n");
    let submitted = f.app.editor.as_ref().unwrap().source();
    f.app.save();
    let response = finish_write(&mut f);
    f.app.editor = None;
    f.app.editing = false;
    f.app.open(TASK_ID);
    f.finish();
    f.app.edit();
    let original = f.app.editor.as_ref().unwrap().source();
    f.app
        .active_editor_mut()
        .unwrap()
        .area
        .insert_str("New task draft 世界\n");
    retained(&mut f, response, "completed");
    assert_eq!(fs::read_to_string(f.path()).unwrap(), submitted);
    assert!(f.app.editor.as_mut().unwrap().area.undo());
    assert_eq!(f.app.editor.as_ref().unwrap().source(), original);
}

#[test]
fn every_write_family_binds_all_original_resolution_inputs() {
    for family in 0..10 {
        for input in INPUTS {
            let mut f = Fixture::new();
            match family {
                0 => {
                    f.open_editor();
                    f.app.paste("\nSubmitted.\n");
                    f.app.save();
                }
                1..=3 => {
                    form(&mut f, family - 1);
                    f.app.save();
                }
                4 => {
                    fs::write(
                        f.root().join("Projects/example/templates/session.md"),
                        EVENT_TEMPLATE,
                    )
                    .unwrap();
                    f.app.command("event session inline.md | date=2026-10-26 | title=Inline | body=Привет 世界");
                }
                5..=9 => {
                    administration(&mut f, family - 5, false);
                    confirm(&mut f, family - 5);
                }
                _ => unreachable!(),
            }
            let response = finish_write(&mut f);
            change_request(&mut f, input);
            // Leave the submitted review intact: request identity must independently guard
            // replacement, while the attempted plan alone is consumed.
            retained_after_consumption(&mut f, response, "completed", family.checked_sub(5));
            assert!(
                !f.app.external_change_pending,
                "obsolete session must not signal a different identity"
            );
        }
    }
}

#[test]
fn form_completions_retain_revised_cancelled_replaced_and_recovery_hidden_work() {
    for kind in 0..3 {
        for change in [
            "draft",
            "baseline",
            "stage",
            "cancelled",
            "replaced",
            "recovery",
        ] {
            let mut f = Fixture::new();
            form(&mut f, kind);
            f.app.save();
            let response = finish_write(&mut f);
            validate_project(&f.app.request).unwrap();
            match change {
                "draft" => match f.app.workflow.as_mut().unwrap() {
                    Workflow::Event(form) => {
                        form.fields
                            .last_mut()
                            .unwrap()
                            .1
                            .area
                            .insert_str("Newer 世界");
                    }
                    Workflow::Creation(form) => {
                        form.projection.area.insert_str("Newer 世界");
                    }
                    Workflow::Lifecycle(form) => {
                        form.note.area.insert_str("Newer 世界");
                    }
                },
                "baseline" => match f.app.workflow.as_mut().unwrap() {
                    Workflow::Event(form) => form.prepared.template_source.push_str("Changed"),
                    Workflow::Creation(form) => form.prepared.projection_source.push_str("Changed"),
                    Workflow::Lifecycle(form) => form.prepared.source.push_str("Changed"),
                },
                "stage" => f.app.previous_workflow_step(),
                "cancelled" => {
                    f.app.command("discard");
                }
                "replaced" => {
                    f.app.command("discard");
                    f.app.command("create entity");
                    f.finish();
                }
                "recovery" => f.app.show_recovery(),
                _ => unreachable!(),
            }
            retained(&mut f, response, "completed");
            assert!(f.app.external_change_pending);
        }
    }
}

#[test]
fn inline_handoff_and_event_completions_retain_new_editor_and_graph_parent() {
    for handoff in [false, true] {
        let mut f = Fixture::new();
        let note_type = if handoff { "handoff" } else { "session" };
        fs::write(
            f.root()
                .join(format!("Projects/example/templates/{note_type}.md")),
            EVENT_TEMPLATE,
        )
        .unwrap();
        f.app.command(&format!(
            "{} inline.md | date=2026-10-26 | title=Inline | body=Привет 世界",
            if handoff { "handoff" } else { "event session" }
        ));
        let response = finish_write(&mut f);
        f.app
            .command("links Projects/example/events/sessions/2026-07-13.md");
        f.finish();
        f.app.command("follow 1");
        f.finish();
        f.app.edit();
        f.app
            .active_editor_mut()
            .unwrap()
            .area
            .insert_str("Retained editor 世界\n");
        assert!(f.app.link_return.is_some());
        retained(&mut f, response, "completed");
        validate_project(&f.app.request).unwrap();
    }
}

#[test]
fn completed_administration_retains_a_newer_review_of_each_family() {
    for kind in 0..5 {
        let mut f = Fixture::new();
        administration(&mut f, kind, false);
        confirm(&mut f, kind);
        let response = finish_write(&mut f);
        f.app.command("discard");
        administration(&mut f, kind, true);
        retained(&mut f, response, "completed");
        assert!(f.app.reviewing_administration());
    }
}

#[test]
fn obsolete_administration_attempt_consumes_only_its_matching_review() {
    for kind in 0..5 {
        let mut f = Fixture::new();
        administration(&mut f, kind, false);
        confirm(&mut f, kind);
        let response = finish_write(&mut f);
        change_request(&mut f, "cwd");
        let body = f.app.body.clone();
        let request = f.app.request.cwd.clone();
        let files = link_snapshot(&f.temp);
        f.app.receive(response);
        assert!(!f.app.reviewing_administration());
        assert_eq!(f.app.body, body);
        assert_eq!(f.app.request.cwd, request);
        assert_eq!(link_snapshot(&f.temp), files);
        f.app.command("confirm anything");
        assert!(f.jobs.try_recv().is_err());
        assert!(
            f.app
                .messages
                .back()
                .unwrap()
                .contains("No administration plan")
        );
    }
}

#[test]
fn save_failure_with_pending_recovery_retains_newer_draft_and_reports_failure() {
    let mut f = Fixture::new();
    f.open_editor();
    f.app.paste("\nSubmitted draft.\n");
    f.app.save();
    let (journal, _, _) = stage_recovery(&f, true);
    let files = link_snapshot(&f.temp);
    let response = finish_write(&mut f);
    assert_eq!(
        link_snapshot(&f.temp),
        files,
        "refused core recovery preserves exact files"
    );
    f.app.paste("Newer recovery draft 世界\n");
    retained(&mut f, response, "failed");
    assert!(journal.exists());
    assert!(f.app.external_change_pending);
    assert!(!f.app.recovery_visible && !f.app.recovery_blocked);
    f.app.command("recovery");
    f.finish();
    assert!(f.app.recovery_visible && f.app.recovery_blocked);
    assert_eq!(link_snapshot(&f.temp), files);
}

#[test]
fn metadata_recovery_binds_all_inputs_and_preserves_existing_failure_guidance() {
    for input in INPUTS {
        let mut f = Fixture::new();
        f.open_editor();
        f.app.paste("\nRetained 世界\n");
        f.app.recovery = Some(RecoveryNotice {
            inspection: inspect_pending_note_edit(&f.app.request).map_err(|e| e.to_string()),
            failure: Some("Original refused operation".into()),
        });
        f.app.command("recovery");
        let response = execute_job(f.jobs.try_recv().unwrap());
        change_request(&mut f, input);
        retained(&mut f, response, "obsolete");
        assert!(
            f.app
                .recovery
                .as_ref()
                .unwrap()
                .text()
                .contains("Original refused operation")
        );
        assert!(f.app.editor.as_mut().unwrap().area.undo());
    }
}

#[test]
fn dismissed_recovery_inspection_cannot_reopen_or_consume_failure_guidance() {
    for error in [false, true] {
        let mut f = Fixture::new();
        f.open_editor();
        f.app.paste("\nRetained 世界\n");
        f.app.recovery = Some(RecoveryNotice {
            inspection: inspect_pending_note_edit(&f.app.request).map_err(|e| e.to_string()),
            failure: Some("Original refused operation".into()),
        });
        f.app.command("recovery");
        f.finish();
        assert!(f.app.recovery_visible);
        if error {
            fs::create_dir(f.root().join("Projects/example/.akasha-edit-journal.json")).unwrap();
        }
        f.app.command("recovery");
        let response = execute_job(f.jobs.try_recv().unwrap());
        f.app.command("back"); // Returning from inspection is allowed while its next observation runs.
        assert!(!f.app.recovery_visible);
        retained(&mut f, response, "obsolete");
        assert!(
            f.app
                .recovery
                .as_ref()
                .unwrap()
                .text()
                .contains("Original refused operation")
        );
    }
}

#[test]
fn recovery_inspection_preserves_a_newer_failure_with_identical_visibility_flags() {
    let mut f = Fixture::new();
    f.open_editor();
    f.app.paste("\nRetained 世界\n");
    f.app.command("recovery");
    let response = execute_job(f.jobs.try_recv().unwrap());
    f.app.recovery = Some(RecoveryNotice {
        inspection: Err("Newer inspection failure".into()),
        failure: Some("Newer refused operation".into()),
    });
    retained(&mut f, response, "obsolete");
    let text = f.app.recovery.as_ref().unwrap().text();
    assert!(text.contains("Newer inspection failure") && text.contains("Newer refused operation"));
}

#[test]
fn form_apply_failures_report_actual_refusal_and_preserve_revised_drafts() {
    for kind in 0..3 {
        let mut f = Fixture::new();
        form(&mut f, kind);
        f.app.save();
        match f.app.workflow.as_ref().unwrap() {
            Workflow::Event(form) => {
                fs::write(&form.prepared.template, "External template bytes\n").unwrap()
            }
            Workflow::Creation(form) => {
                fs::write(&form.prepared.template, "External template bytes\n").unwrap()
            }
            Workflow::Lifecycle(_) => fs::write(f.path(), "External entity bytes\n").unwrap(),
        }
        let response = finish_write(&mut f);
        f.app.previous_workflow_step();
        retained(&mut f, response, "failed");
        assert!(f.app.in_workflow());
        assert!(f.app.external_change_pending);
    }
}

#[test]
fn administration_refusals_preserve_foreign_bytes_and_new_editor_work() {
    for kind in 0..5 {
        let mut f = Fixture::new();
        administration(&mut f, kind, false);
        confirm(&mut f, kind);
        let foreign = b"Foreign bytes must survive.\n";
        let path = match kind {
            0 => f.temp.join("submitted home 世界/AGENTS.md"),
            1 => {
                let root = f.temp.join("submitted root 世界");
                fs::create_dir(&root).unwrap();
                root.join("foreign.md")
            }
            2 => f
                .app
                .init_review
                .as_ref()
                .unwrap()
                .plan
                .destination
                .pointer
                .clone(),
            3 => {
                // The reviewed rollback must refuse a changed published artifact.
                f.temp.join("submitted repository 世界/.akasha.toml")
            }
            4 => f
                .app
                .link_review
                .as_ref()
                .unwrap()
                .plan
                .destination
                .pointer
                .clone(),
            _ => unreachable!(),
        };
        fs::write(&path, foreign).unwrap();
        let response = finish_write(&mut f);
        f.app.command("discard");
        f.app.editor = Some(Editor::new("Retained new source 世界\n").unwrap());
        f.app.editing = true;
        retained(&mut f, response, "failed");
        assert_eq!(fs::read(&path).unwrap(), foreign);
        assert!(f.app.external_change_pending);
    }
}

#[test]
fn busy_write_completion_preserves_a_new_command_draft_from_real_prompt_input() {
    for family in [4, 0, 1, 2, 3, 5, 6, 7, 8, 9] {
        let mut f = Fixture::new();
        match family {
            0 => {
                f.open_editor();
                f.app.paste("\nSubmitted.\n");
                f.app.save();
            }
            1..=3 => {
                form(&mut f, family - 1);
                f.app.save();
            }
            4 => {
                fs::write(
                    f.root().join("Projects/example/templates/session.md"),
                    EVENT_TEMPLATE,
                )
                .unwrap();
                f.app.command(
                    "event session inline.md | date=2026-10-26 | title=Inline | body=Привет 世界",
                );
            }
            5..=9 => {
                administration(&mut f, family - 5, false);
                confirm(&mut f, family - 5);
            }
            _ => unreachable!(),
        }
        assert!(f.app.busy);
        f.app.focus = Focus::Prompt;
        f.app.paste("/search Newer Привет 世界");
        f.app
            .key(KeyEvent::new(KeyCode::Char('λ'), KeyModifiers::NONE));
        assert_eq!(f.app.prompt.lines(), ["/search Newer Привет 世界λ"]);
        let response = finish_write(&mut f);
        if family == 0 {
            // Save acknowledgement changes only its matching saved baseline. It can safely
            // acknowledge the write while keeping the new command draft and prompt focus.
            let source = fs::read_to_string(f.path()).unwrap();
            f.app.receive(response);
            assert_eq!(f.app.editor.as_ref().unwrap().original, source);
            assert!(!f.app.editor.as_ref().unwrap().dirty());
            assert!(matches!(f.app.focus, Focus::Prompt));
            assert!(f.jobs.try_recv().is_err());
        } else {
            retained_after_consumption(&mut f, response, "completed", family.checked_sub(5));
            assert!(f.app.external_change_pending);
        }
        assert_eq!(f.app.prompt.lines(), ["/search Newer Привет 世界λ"]);
    }
}
