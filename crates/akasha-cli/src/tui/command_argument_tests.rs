use super::*;

const ARGUMENT_FREE: [&str; 24] = [
    "save",
    "discard",
    "edit",
    "read",
    "previous",
    "next",
    "quit",
    "exit",
    "q",
    "motion",
    "back",
    "home",
    "projects",
    "global",
    "ls",
    "help",
    "recovery",
    "refresh",
    "onboard",
    "recover-init",
    "lifecycle",
    "resolve",
    "breadcrumb",
    "validate",
];

fn state(app: &App) -> Vec<String> {
    let mut values = vec![
        format!("{:?}", app.request),
        format!("{:?}", app.scope),
        app.title.clone(),
        app.body_title.clone(),
        app.body.clone(),
        format!("{:?}", app.document),
        format!(
            "{:?}",
            app.editor
                .as_ref()
                .map(|e| (&e.original, e.source(), e.area.cursor()))
        ),
        format!("{:?}", app.prompt.lines()),
        format!("{:?}", app.prompt.cursor()),
        format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            app.editing,
            app.busy,
            app.checking,
            app.external_change_pending,
            app.recovery_visible,
            app.recovery_blocked,
            app.quit,
            app.no_motion,
            app.tick,
            app.scroll,
            app.max_scroll,
            app.focus as u8
        ),
        format!("{:?}", app.list.selected()),
        format!("{}", app.navigation.len()),
        format!("{:?}", app.pending_open),
        format!("{:?}", app.recovery.as_ref().map(RecoveryNotice::text)),
        format!(
            "{:?}",
            app.recovery_return.as_ref().map(|(focus, scroll, prompt)| (
                *focus as u8,
                *scroll,
                prompt.lines(),
                prompt.cursor()
            ))
        ),
    ];
    values.extend(
        app.rows
            .iter()
            .map(|row| format!("{}:{}", row.label, row.detail)),
    );
    values.extend(
        [
            AdministrationKind::Integration,
            AdministrationKind::Setup,
            AdministrationKind::Init,
            AdministrationKind::InitRecovery,
            AdministrationKind::Link,
        ]
        .map(|kind| app.administration_id(kind).unwrap_or_default().to_owned()),
    );
    if let Some(workflow) = &app.workflow {
        values.extend(workflow_sources(app));
        values.push(match workflow {
            Workflow::Event(form) => format!("event:{:?}:{:?}", app.event_stage(), form.preview),
            Workflow::Creation(form) => format!(
                "creation:{:?}:{:?}:{:?}",
                app.creation_review(),
                match form.stage {
                    CreationStage::Path => 0,
                    CreationStage::Field(index) => index + 1,
                    CreationStage::Projection => usize::MAX,
                    CreationStage::Review(_) => usize::MAX - 1,
                },
                form.preview
            ),
            Workflow::Lifecycle(form) => format!(
                "lifecycle:{:?}:{}:{:?}",
                form.pane, form.reviewing, form.preview
            ),
        });
    }
    values
}

fn unchanged(f: &mut Fixture, input: &str, expected: &str) {
    let before = state(&f.app);
    let projection = f.app.projection.clone();
    let report = f.app.link_report.clone();
    let parent = f.app.link_return.clone();
    let files = link_snapshot(&f.temp);
    f.app.command(input);
    assert_eq!(state(&f.app), before, "{input}");
    assert_eq!(f.app.projection, projection, "{input}");
    assert!(
        f.app.link_report == report && f.app.link_return == parent,
        "{input}"
    );
    assert_eq!(link_snapshot(&f.temp), files, "{input}");
    assert_eq!(f.app.messages.back().unwrap(), expected, "{input}");
    assert!(f.jobs.try_recv().is_err(), "{input} must not submit work");
}

fn reject_all(f: &mut Fixture) {
    for command in ARGUMENT_FREE {
        unchanged(
            f,
            &format!("/{command} unexpected"),
            &format!("Usage: /{command}"),
        );
    }
}

#[test]
fn malformed_discard_retains_dirty_source_and_undo_before_any_state_change() {
    let mut f = Fixture::new();
    f.open_editor();
    let original = f.app.document.as_ref().unwrap().source.clone();
    f.app.paste("\nKeep this Привет 世界\r\n");
    let draft = f.app.editor.as_ref().unwrap().source();
    let files = link_snapshot(&f.temp);
    f.app.command("/discard unexpected");
    assert_eq!(
        f.app.editor.as_ref().map(Editor::source),
        Some(draft.clone())
    );
    assert_eq!(f.app.messages.back().unwrap(), "Usage: /discard");
    assert!(f.app.editing && f.app.dirty() && !f.app.busy);
    assert!(f.jobs.try_recv().is_err());
    assert_eq!(link_snapshot(&f.temp), files);
    f.app.editor.as_mut().unwrap().area.undo();
    assert_eq!(f.app.editor.as_ref().unwrap().source(), original);
    f.app.editor.as_mut().unwrap().area.redo();
    assert_eq!(f.app.editor.as_ref().unwrap().source(), draft);
}

#[test]
fn every_argument_free_spelling_rejects_nonempty_tails_before_clean_navigation() {
    let mut f = Fixture::new();
    f.open_editor();
    f.app.scroll = 3;
    f.app.tick = 19;
    f.app.prompt = prompt_area("unsubmitted 世界".into());
    for command in ARGUMENT_FREE {
        for suffix in ["unexpected", "--force", "世界 keep", "\t\n extra\u{2003}"] {
            unchanged(
                &mut f,
                &format!(" /{command} {suffix} "),
                &format!("Usage: /{command}"),
            );
        }
    }
}

#[test]
fn malformed_save_cannot_write_and_whitespace_save_discard_still_work() {
    let mut f = Fixture::new();
    f.open_editor();
    let original = f.app.document.as_ref().unwrap().source.clone();
    f.app.paste("\nExact unsaved 世界  \n");
    let draft = f.app.editor.as_ref().unwrap().source();
    reject_all(&mut f);
    assert_eq!(fs::read_to_string(f.path()).unwrap(), original);
    f.app.command(" \t/save\t \u{2003} ");
    f.finish();
    assert_eq!(fs::read_to_string(f.path()).unwrap(), draft);
    assert!(!f.app.dirty());
    f.app.paste("More unsaved");
    unchanged(&mut f, "discard keep", "Usage: /discard");
    f.app.command(" /discard \t ");
    assert!(f.app.editor.is_none() && !f.app.dirty());
    assert_eq!(f.app.document.as_ref().unwrap().source, draft);
    assert_eq!(fs::read_to_string(f.path()).unwrap(), draft);
}

#[test]
fn malformed_commands_retain_all_workflow_drafts_previews_and_stages() {
    for kind in 0..3 {
        let mut f = Fixture::new();
        match kind {
            0 => {
                guided_event(&mut f, false, true);
                f.app.command("previous");
            }
            1 => author_mutable(&mut f, "task", true),
            2 => {
                f.app.open(ID);
                f.finish();
                f.app.command("lifecycle");
                f.finish();
            }
            _ => unreachable!(),
        }
        let editor = f.app.active_editor_mut().unwrap();
        let before = editor.source();
        editor.area.move_cursor(CursorMove::Bottom);
        editor.area.move_cursor(CursorMove::End);
        editor.area.insert_str("\nRetained field 世界\n");
        let draft = editor.source();
        reject_all(&mut f);
        f.app.active_editor_mut().unwrap().area.undo();
        assert_eq!(f.app.active_editor().unwrap().source(), before);
        f.app.active_editor_mut().unwrap().area.redo();
        assert_eq!(f.app.active_editor().unwrap().source(), draft);
        match kind {
            0 => {
                f.app.command("next");
                f.finish();
            }
            1 => review_creation(&mut f),
            2 => review_lifecycle(&mut f),
            _ => unreachable!(),
        }
        reject_all(&mut f);
        let files = link_snapshot(&f.temp);
        f.app.command("discard\t");
        assert!(f.app.workflow.is_none());
        assert_eq!(link_snapshot(&f.temp), files);
    }
}

#[test]
fn malformed_commands_retain_every_administration_review_and_exact_confirmation() {
    for kind in 0..5 {
        let mut f = Fixture::new();
        let repository = f.temp.join("repository 世界 with spaces");
        fs::create_dir_all(&repository).unwrap();
        let command = match kind {
            0 => {
                let home = f.temp.join("client home 世界");
                fs::create_dir(&home).unwrap();
                format!("integration apply hook codex {}", home.display())
            }
            1 => {
                f.app.projection = None;
                format!("setup {}", f.temp.join("new root 世界").display())
            }
            2 => format!("init another {}", repository.display()),
            3 => {
                init_recovery_fixture::stage(&f.root(), &repository, "another", false);
                "recover-init".into()
            }
            4 => format!("link example {}", f.temp.join("repository").display()),
            _ => unreachable!(),
        };
        f.app.command(&command);
        f.finish();
        assert!(
            f.app.reviewing_administration(),
            "{command}: {:?}",
            f.app.messages
        );
        reject_all(&mut f);
        let family = [
            AdministrationKind::Integration,
            AdministrationKind::Setup,
            AdministrationKind::Init,
            AdministrationKind::InitRecovery,
            AdministrationKind::Link,
        ][kind];
        let id = f.app.administration_id(family).unwrap().to_owned();
        for invalid in [
            format!("confirm {id} extra"),
            format!("confirm {}", &id[..id.len() - 1]),
        ] {
            unchanged(
                &mut f,
                &invalid,
                "Confirmation must match the complete displayed plan ID.",
            );
        }
        f.app.command("discard \t");
        assert!(!f.app.reviewing_administration());
    }
}

#[test]
fn malformed_commands_cannot_hide_recovery_or_consume_retained_drafts() {
    for blocked in [false, true] {
        let mut f = Fixture::new();
        f.open_editor();
        f.app.paste("\nRecovery draft 世界\n");
        f.app.prompt = prompt_area("retained prompt".into());
        f.app.recovery = Some(RecoveryNotice {
            inspection: Err("synthetic metadata failure".into()),
            failure: Some("original refused operation".into()),
        });
        f.app.recovery_blocked = blocked;
        f.app.show_recovery();
        reject_all(&mut f);
        f.app.command("back\t ");
        assert!(!f.app.recovery_visible && f.app.recovery_blocked == blocked);
        assert_eq!(f.app.prompt.lines(), &["retained prompt"]);
        assert!(
            f.app
                .editor
                .as_ref()
                .unwrap()
                .source()
                .contains("Recovery draft 世界")
        );
    }
}

#[test]
fn malformed_navigation_retains_graph_report_and_exact_parent_return() {
    let mut f = Fixture::new();
    f.app
        .command("links Projects/example/events/sessions/2026-07-13.md");
    f.finish();
    assert!(f.app.link_report.as_ref().unwrap().result.total_matches > 0);
    f.app.scroll = 2;
    reject_all(&mut f);
    f.app.command("follow 1");
    f.finish();
    assert!(f.app.document.is_some() && f.app.link_return.is_some());
    reject_all(&mut f);
    f.app.command("back \t");
    assert!(f.app.link_report.is_some() && f.app.link_return.is_none());
    assert_eq!(f.app.scroll, 2);
}

#[test]
fn malformed_commands_leave_the_submitted_busy_write_and_completion_intact() {
    let mut f = Fixture::new();
    f.open_editor();
    f.app.paste("\nSubmitted 世界\n");
    let draft = f.app.editor.as_ref().unwrap().source();
    f.app.command("save");
    let job = f.jobs.try_recv().unwrap();
    assert!(f.app.busy);
    reject_all(&mut f);
    f.app.receive(execute_job(job));
    assert_eq!(fs::read_to_string(f.path()).unwrap(), draft);
    assert!(!f.app.busy && !f.app.dirty());
}

#[test]
fn real_prompt_enter_rejects_malformed_save_discard_and_exit_and_keeps_history() {
    let mut f = Fixture::new();
    f.open_editor();
    f.app.paste("\nPrompt draft 世界\n");
    let draft = f.app.editor.as_ref().unwrap().source();
    let files = link_snapshot(&f.temp);
    for command in ["save", "discard", "read", "quit", "exit", "q"] {
        f.app.focus = Focus::Prompt;
        f.app.paste(&format!("/{command} extra"));
        f.app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            f.app.messages.back().unwrap(),
            &format!("Usage: /{command}")
        );
        assert_eq!(f.app.history.last().unwrap(), &format!("/{command} extra"));
        assert!(f.app.prompt.lines()[0].is_empty());
        assert_eq!(f.app.editor.as_ref().unwrap().source(), draft);
        assert!(f.app.editing && f.app.dirty() && !f.app.quit && !f.app.busy);
        assert!(f.jobs.try_recv().is_err());
        assert_eq!(link_snapshot(&f.temp), files);
    }
}

#[test]
fn whitespace_only_motion_read_and_exit_aliases_keep_their_normal_behavior() {
    for alias in ["quit", "exit", "q"] {
        let mut f = Fixture::new();
        f.open_editor();
        let previous = f.app.no_motion;
        f.app.command(" /motion \t\u{2003}");
        assert_eq!(f.app.no_motion, !previous);
        f.app.command(" /read \t");
        assert!(!f.app.editing && f.app.focus == Focus::Reader);
        let files = link_snapshot(&f.temp);
        f.app.command(&format!(" /{alias}\t \u{2003}"));
        assert!(f.app.quit);
        assert_eq!(link_snapshot(&f.temp), files);
    }
}
