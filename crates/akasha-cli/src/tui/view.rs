use super::{
    app::{Action, App, Focus, LifecyclePane, Target, scope_name},
    editor::safe_text,
    starlight,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap},
};
use std::borrow::Cow;

fn ink(app: &App) -> Style {
    if app.color {
        Style::default()
            .fg(Color::Rgb(224, 222, 216))
            .bg(Color::Reset)
    } else {
        Style::default()
    }
}
fn accent(app: &App) -> Style {
    if app.color {
        ink(app).fg(Color::Rgb(179, 153, 213))
    } else {
        ink(app).add_modifier(Modifier::BOLD)
    }
}
fn subdued(app: &App) -> Style {
    if app.color {
        ink(app).fg(Color::Rgb(148, 144, 154))
    } else {
        ink(app).add_modifier(Modifier::DIM)
    }
}
fn rail(app: &App) -> Style {
    if app.color {
        ink(app).fg(Color::Rgb(76, 72, 84))
    } else {
        subdued(app)
    }
}
fn selected(app: &App, focused: bool) -> Style {
    if focused {
        accent(app).add_modifier(Modifier::BOLD)
    } else {
        ink(app).add_modifier(Modifier::BOLD)
    }
}
fn border_set(app: &App) -> symbols::border::Set<'static> {
    if app.ascii {
        symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            horizontal_top: "-",
            horizontal_bottom: "-",
            vertical_left: "|",
            vertical_right: "|",
        }
    } else {
        symbols::border::PLAIN
    }
}
fn panel(title: &str, focused: bool, app: &App) -> Block<'static> {
    Block::default()
        .borders(Borders::TOP)
        .border_set(border_set(app))
        .border_style(if focused { accent(app) } else { rail(app) })
        .title(
            Line::from(format!(" {} ", safe_text(title))).style(if focused {
                ink(app)
            } else {
                subdued(app)
            }),
        )
        .padding(Padding::new(1, 1, 1, 0))
}
fn home(app: &App) -> bool {
    !app.recovery_visible && app.document.is_none() && app.body_title == "WELCOME TO AKASHA"
}

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.hits.clear();
    frame.render_widget(Block::default().style(ink(app)), area);
    if area.width < 40 || area.height < 12 {
        frame.render_widget(Paragraph::new("AKASHA\nTerminal too small: use at least 40 x 12.\nResize to continue; Ctrl-Q exits safely.")
            .wrap(Wrap { trim: false }), area);
        return;
    }
    // Limit prose line length on a wide monitor while letting small terminals use every cell.
    let width = area
        .width
        .saturating_sub(if area.width < 60 { 2 } else { 4 })
        .min(148);
    let content = Rect::new(
        area.x + (area.width - width) / 2,
        area.y,
        width,
        area.height,
    );
    let [header, main, log, prompt, footer] = Layout::vertical([
        Constraint::Length(if area.height >= 13 { 3 } else { 2 }),
        Constraint::Min(3),
        Constraint::Length(if area.height >= 26 { 3 } else { 1 }),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .areas(content);
    draw_header(frame, app, header);
    if app.recovery_visible || app.reviewing_administration() {
        draw_reader(frame, app, main);
    } else if main.width >= 84 {
        let [list, gap, reader] = Layout::horizontal([
            Constraint::Length((main.width / 4).clamp(26, 32)),
            Constraint::Length(3),
            Constraint::Min(20),
        ])
        .areas(main);
        draw_list(frame, app, list);
        frame.render_widget(
            Block::default()
                .borders(Borders::LEFT)
                .border_set(border_set(app))
                .border_style(rail(app)),
            Rect::new(gap.x + 1, gap.y + 1, 1, gap.height.saturating_sub(1)),
        );
        draw_reader(frame, app, reader);
    } else if app.focus == Focus::List || (home(app) && main.height < 17) {
        draw_list(frame, app, main);
    } else {
        draw_reader(frame, app, main);
    }
    let messages: Vec<_> = app
        .messages
        .iter()
        .rev()
        .take(usize::from(log.height.min(2)))
        .rev()
        .map(|m| {
            let error = m.starts_with("Operation failed")
                || m.starts_with("Unsaved changes")
                || m.contains("unavailable");
            Line::styled(
                format!(" {}", safe_text(m)),
                if error {
                    ink(app).add_modifier(Modifier::BOLD)
                } else {
                    subdued(app)
                },
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(messages).style(ink(app)), log);
    draw_prompt(frame, app, prompt);
    draw_footer(frame, app, footer);
    draw_completions(frame, app, main, log, prompt);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let [name, status] =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
            .areas(Rect::new(area.x, area.y + 1, area.width, 1));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(if app.ascii { " + " } else { " ◇ " }, accent(app)),
            Span::styled("AKASHA", ink(app).add_modifier(Modifier::BOLD)),
            Span::styled(format!("  v{}", env!("CARGO_PKG_VERSION")), subdued(app)),
        ])),
        name,
    );
    let state = if app.busy {
        "working"
    } else if app.recovery_blocked {
        "recovery"
    } else if app.external_change_pending && app.in_workflow() {
        "external change · form pending"
    } else if app.external_change_pending && app.dirty() {
        "external change · unsaved"
    } else if app.external_change_pending {
        "external change · F5"
    } else if app.in_workflow() {
        "form pending"
    } else if app.dirty() {
        "unsaved"
    } else {
        "ready"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if area.width >= 60 {
                    format!("{}  /  ", safe_text(&scope_name(&app.scope)))
                } else {
                    String::new()
                },
                subdued(app),
            ),
            Span::styled(
                state,
                if app.dirty() || app.external_change_pending {
                    accent(app)
                } else {
                    ink(app)
                },
            ),
        ]))
        .alignment(Alignment::Right),
        status,
    );
}

// A tiny four-point seal rotates around a fixed center in a 6 x 4 Braille canvas.
fn prompt_sigil(tick: u64, ascii: bool, still: bool) -> String {
    if ascii {
        return if still {
            " * "
        } else {
            [" + ", " x "][(tick / 6 % 2) as usize]
        }
        .into();
    }
    let angle = if still {
        0.0
    } else {
        (tick % 80) as f64 * std::f64::consts::TAU / 80.0
    };
    let mut cells = [0u8; 3];
    let dots = [[0, 1, 2, 6], [3, 4, 5, 7]];
    for spoke in 0..4 {
        let theta = angle + spoke as f64 * std::f64::consts::FRAC_PI_2;
        for step in 0..=6 {
            let radius = step as f64 / 6.0;
            let x = (2.5 + 2.5 * radius * theta.cos()).round().clamp(0.0, 5.0) as usize;
            let y = (1.5 + 1.5 * radius * theta.sin()).round().clamp(0.0, 3.0) as usize;
            cells[x / 2] |= 1 << dots[x % 2][y];
        }
    }
    cells
        .into_iter()
        .map(|bits| char::from_u32(0x2800 + u32::from(bits)).unwrap())
        .collect()
}

fn draw_prompt(frame: &mut Frame, app: &mut App, area: Rect) {
    app.hits.push((area, Action::Focus(Focus::Prompt)));
    let style = ink(app);
    frame.render_widget(Block::default().style(style), area);
    let line = Rect::new(area.x + 6, area.y + 1, area.width.saturating_sub(8), 1);
    app.prompt.remove_block();
    app.prompt.set_style(style);
    app.prompt
        .set_placeholder_style(subdued(app).patch(style).fg(if app.color {
            Color::Rgb(148, 144, 154)
        } else {
            Color::Reset
        }));
    app.prompt.set_cursor_line_style(style);
    app.prompt.set_cursor_style(if app.focus == Focus::Prompt {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    });
    app.prompt
        .set_selection_style(style.add_modifier(Modifier::REVERSED));
    frame.render_widget(&app.prompt, line);
    let marker = if app.ascii { ">" } else { "›" };
    frame.render_widget(
        Paragraph::new(marker).style(if app.focus == Focus::Prompt {
            accent(app).patch(Style::default().bg(style.bg.unwrap_or(Color::Reset)))
        } else {
            style
        }),
        Rect::new(area.x + 4, area.y + 1, 1, 1),
    );
    frame.render_widget(
        Paragraph::new(prompt_sigil(app.sigil_tick, app.ascii, app.no_motion)).style(accent(app)),
        Rect::new(area.x, area.y + 1, 3, 1),
    );
    // The complete typing row (including spaces and cursor) is excluded from decoration.
    starlight::draw(
        frame,
        area,
        app.tick,
        app.ascii,
        app.color,
        Some(Rect::new(area.x, area.y + 1, area.width, 1)),
    );
}

fn button(frame: &mut Frame, app: &mut App, area: Rect, x: &mut u16, label: &str, action: Action) {
    let text = format!("{label}  ");
    let width = Line::from(text.clone()).width() as u16;
    if x.saturating_add(width) > area.right() {
        return;
    }
    let hit = Rect::new(*x, area.y, width - 1, 1);
    frame.render_widget(
        Paragraph::new(text).style(subdued(app)),
        Rect::new(*x, area.y, width, 1),
    );
    app.hits.push((hit, action));
    *x += width;
}

fn draw_navigation(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut x = area.x;
    if app.recovery_visible || app.recovery_blocked {
        for (label, action) in [
            ("Inspect", Action::Command("recovery")),
            ("Bksp Prompt", Action::Focus(Focus::Prompt)),
            ("Ctrl-Q quit", Action::Command("quit")),
        ] {
            button(frame, app, area, &mut x, label, action);
        }
        return;
    }
    for (label, action) in [
        ("F4 Projects", Action::Command("projects")),
        ("F3 Search", Action::Search),
        ("F1 Help", Action::Command("help")),
        ("F6 Global", Action::Command("global")),
        ("F5 Refresh", Action::Command("refresh")),
        ("Home", Action::Command("home")),
    ] {
        button(frame, app, area, &mut x, label, action);
    }
}

fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) {
    draw_navigation(frame, app, Rect::new(area.x, area.y + 1, area.width, 1));
    let area = Rect::new(area.x, area.y, area.width, 1);
    if !app.completions().is_empty() {
        frame.render_widget(
            Paragraph::new(
                if app.prompt.lines()[0].starts_with("/open ") && area.width >= 60 {
                    "Type to filter notes  Up/Down choose  Tab fill  Enter open  Esc close"
                } else if area.width < 60 {
                    "Up/Down choose  Tab fill  Enter  Esc"
                } else {
                    "Click a command  /  Up/Down choose  Tab complete  Enter run  Esc close"
                },
            )
            .style(subdued(app)),
            area,
        );
        return;
    }
    let mut x = area.x;
    if app.recovery_visible || app.recovery_blocked {
        button(
            frame,
            app,
            area,
            &mut x,
            "F5 Retry",
            Action::Command("refresh"),
        );
        button(
            frame,
            app,
            area,
            &mut x,
            "Esc Return",
            Action::Command("back"),
        );
        if app.dirty() {
            button(
                frame,
                app,
                area,
                &mut x,
                "Discard",
                Action::Command("discard"),
            );
        }
        return;
    }
    if app.reviewing_administration() {
        button(
            frame,
            app,
            area,
            &mut x,
            "Prompt: /confirm PLAN_ID",
            Action::Focus(Focus::Prompt),
        );
        button(
            frame,
            app,
            area,
            &mut x,
            "Discard review",
            Action::Command("discard"),
        );
        return;
    }
    if app.in_workflow() {
        if app.creation_input_active() {
            button(
                frame,
                app,
                area,
                &mut x,
                "Enter Next",
                Action::Command("next"),
            );
            button(
                frame,
                app,
                area,
                &mut x,
                "Shift-Tab Previous",
                Action::Command("previous"),
            );
        } else {
            if app.event_stage() == Some(false)
                || (app.lifecycle_pane().is_some()
                    && app.lifecycle_review() != Some(LifecyclePane::Projection))
                || (app.event_stage().is_none()
                    && app.lifecycle_pane().is_none()
                    && app.creation_review() != Some(LifecyclePane::Projection))
            {
                button(
                    frame,
                    app,
                    area,
                    &mut x,
                    "Ctrl-N Next",
                    Action::Command("next"),
                );
                button(
                    frame,
                    app,
                    area,
                    &mut x,
                    "Ctrl-P Previous",
                    Action::Command("previous"),
                );
                button(
                    frame,
                    app,
                    area,
                    &mut x,
                    "Discard form",
                    Action::Command("discard"),
                );
                return;
            }
            button(
                frame,
                app,
                area,
                &mut x,
                "Ctrl-S Apply",
                Action::Command("save"),
            );
            button(
                frame,
                app,
                area,
                &mut x,
                if app.creation_review().is_some() || app.lifecycle_review().is_some() {
                    "Ctrl-P Note"
                } else if app.event_stage() == Some(true) {
                    "Ctrl-P Fields"
                } else {
                    "Ctrl-P Inputs"
                },
                Action::Command("previous"),
            );
        }
        button(
            frame,
            app,
            area,
            &mut x,
            "Discard form",
            Action::Command("discard"),
        );
        return;
    }
    if app.editing {
        button(
            frame,
            app,
            area,
            &mut x,
            "Ctrl-S Save",
            Action::Command("save"),
        );
        button(
            frame,
            app,
            area,
            &mut x,
            "Esc Back",
            Action::Command("back"),
        );
    } else if app.has_link_report() {
        button(
            frame,
            app,
            area,
            &mut x,
            "Follow",
            Action::Command("follow"),
        );
        button(
            frame,
            app,
            area,
            &mut x,
            "Esc Back",
            Action::Command("back"),
        );
    } else if app.focus == Focus::Reader && app.document.is_some() {
        button(
            frame,
            app,
            area,
            &mut x,
            "Esc Back",
            Action::Command("back"),
        );
        if app.editable() {
            button(frame, app, area, &mut x, "F2 Edit", Action::Command("edit"));
            button(
                frame,
                app,
                area,
                &mut x,
                "Lifecycle",
                Action::Command("lifecycle"),
            );
        }
    } else {
        if let Some(i) = app.list.selected() {
            button(frame, app, area, &mut x, "Enter Open", Action::Open(i));
        }
        button(
            frame,
            app,
            area,
            &mut x,
            "Esc Back",
            Action::Command("back"),
        );
    }
    button(
        frame,
        app,
        area,
        &mut x,
        "Ctrl-Q quit",
        Action::Command("quit"),
    );
    if app.editing {
        button(frame, app, area, &mut x, "Read", Action::Command("read"));
        if app.dirty() {
            button(
                frame,
                app,
                area,
                &mut x,
                "Discard",
                Action::Command("discard"),
            );
        }
    }
    button(
        frame,
        app,
        area,
        &mut x,
        if app.editing { "Prompt" } else { "Bksp Prompt" },
        Action::Focus(Focus::Prompt),
    );
}

fn draw_completions(frame: &mut Frame, app: &mut App, main: Rect, log: Rect, prompt: Rect) {
    let entries = app.completions();
    if entries.is_empty() {
        return;
    }
    let capacity = usize::from((main.height + log.height).saturating_sub(2).min(7));
    if capacity == 0 {
        return;
    }
    let count = entries.len().min(capacity);
    let selection = app.completion_selection().min(entries.len() - 1);
    let offset = selection.saturating_sub(count - 1);
    let height = count as u16 + 2;
    let area = Rect::new(
        prompt.x,
        prompt.y.saturating_sub(height),
        prompt.width.min(88),
        height,
    );
    app.hits.push((area, Action::Focus(Focus::Prompt)));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .style(ink(app))
            .borders(Borders::ALL)
            .border_set(border_set(app))
            .border_style(rail(app))
            .title(Line::styled(
                format!(
                    " {}  {}/{} ",
                    if app.prompt.lines()[0].starts_with("/open ") {
                        "Open notes (up to 100)"
                    } else if app.prompt.lines()[0].starts_with("/create ") {
                        "Configured record/entity types"
                    } else {
                        "Commands"
                    },
                    selection + 1,
                    entries.len()
                ),
                subdued(app),
            )),
        area,
    );
    for (row, entry) in entries.iter().enumerate().skip(offset).take(count) {
        app.hits.push((
            Rect::new(
                area.x + 1,
                area.y + 1 + (row - offset) as u16,
                area.width - 2,
                1,
            ),
            Action::Completion(row),
        ));
        let focused = row == selection;
        let style = if focused {
            selected(app, true)
        } else {
            ink(app)
        };
        let line = Line::from(vec![
            Span::styled(
                if focused {
                    if app.ascii { "> " } else { "› " }
                } else {
                    "  "
                },
                accent(app).patch(style),
            ),
            Span::styled(
                if entry.command.starts_with("open ") {
                    format!("{}  ", safe_text(&entry.description))
                } else {
                    format!("/{:<12}", entry.command)
                },
                style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if let Some(path) = entry.command.strip_prefix("open ") {
                    safe_text(path)
                } else if area.width >= 62 {
                    format!("{:<12} {}", entry.argument, entry.description)
                } else {
                    entry.description.to_owned()
                },
                style,
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect::new(
                area.x + 1,
                area.y + 1 + (row - offset) as u16,
                area.width - 2,
                1,
            ),
        );
    }
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect) {
    app.hits.push((area, Action::Focus(Focus::List)));
    let compact = area.width < 45
        || usize::from(area.height) < app.rows.len().saturating_mul(2).saturating_add(4);
    let rows: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let label = safe_text(&row.label);
            let detail = safe_text(&row.detail);
            let lines = if matches!(row.target, Target::Category(..)) {
                let number = format!("{:>2} ", i + 1);
                let available = usize::from(area.width.saturating_sub(10));
                let label_width = Line::from(label.clone()).width();
                let count = detail.split_whitespace().next().unwrap_or("");
                let padding = available.saturating_sub(label_width + count.len());
                vec![Line::from(vec![
                    Span::styled(number, subdued(app)),
                    Span::raw(label),
                    Span::styled(
                        format!("{}{}", " ".repeat(padding.max(1)), count),
                        subdued(app),
                    ),
                ])]
            } else {
                let mut lines = vec![Line::from(vec![
                    Span::styled(format!("{:>2} ", i + 1), subdued(app)),
                    Span::raw(label),
                ])];
                if !compact && !detail.is_empty() {
                    lines.push(Line::styled(format!("   {detail}"), subdued(app)));
                }
                lines
            };
            ListItem::new(lines)
        })
        .collect();
    let title = format!("{} / {} items", app.title, app.rows.len());
    let block = panel(&title, app.focus == Focus::List, app).padding(Padding::new(
        1,
        1,
        u16::from(area.height >= 10),
        0,
    ));
    let list_area = Rect {
        height: area.height.saturating_sub(u16::from(area.height >= 12)),
        ..area
    };
    let inner = block.inner(list_area);
    let heights: Vec<u16> = app
        .rows
        .iter()
        .map(|r| {
            if !matches!(r.target, Target::Category(..)) && !compact && !r.detail.is_empty() {
                2
            } else {
                1
            }
        })
        .collect();
    let list = List::new(rows)
        .block(block)
        .style(ink(app))
        .highlight_style(selected(app, app.focus == Focus::List))
        .highlight_symbol(if app.ascii { "> " } else { "› " });
    frame.render_stateful_widget(list, list_area, &mut app.list);
    let mut y = inner.y;
    for (index, height) in heights.iter().enumerate().skip(app.list.offset()) {
        if y >= inner.bottom() {
            break;
        }
        app.hits.push((
            Rect::new(inner.x, y, inner.width, (*height).min(inner.bottom() - y)),
            Action::Open(index),
        ));
        y += height;
    }
    if app.rows.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.busy {
                "Loading memory..."
            } else {
                "No notes here."
            })
            .style(subdued(app))
            .wrap(Wrap { trim: false }),
            inner,
        );
    } else if area.height >= 12 {
        let index = app.list.selected().map_or(0, |i| i + 1);
        frame.render_widget(
            Paragraph::new(format!(" {index}/{}", app.rows.len())).style(subdued(app)),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}

fn draw_reader(frame: &mut Frame, app: &mut App, area: Rect) {
    app.hits.push((area, Action::Focus(Focus::Reader)));
    if home(app) {
        draw_dashboard(frame, app, area);
        return;
    }
    let title = if app.recovery_visible {
        "RECOVERY INSPECTION".into()
    } else if let Some(title) = app.workflow_title() {
        title
    } else if app.editing {
        format!("SOURCE{}", if app.dirty() { " *" } else { "" })
    } else if app.document.is_some() {
        if app.editable() {
            "READING".into()
        } else {
            "READ ONLY".into()
        }
    } else {
        app.body_title.clone()
    };
    let block = panel(&title, app.focus == Focus::Reader, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let workflow_path = app.workflow_path();
    let [path, body, status] = Layout::vertical([
        Constraint::Length(
            if !app.recovery_visible
                && (app.document.is_some() || workflow_path.is_some())
                && inner.height >= 5
            {
                2
            } else {
                0
            },
        ),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    if path.height > 0 {
        frame.render_widget(
            Paragraph::new(safe_text(
                workflow_path.as_deref().unwrap_or(&app.body_title),
            ))
            .style(subdued(app)),
            path,
        );
    }
    let style = ink(app);
    if !app.recovery_visible && app.editing && app.active_editor().is_some() {
        let cursor_style = style.add_modifier(Modifier::REVERSED);
        let line_style = subdued(app);
        let focused = app.focus == Focus::Reader;
        let dirty = app.dirty();
        let form_pending = app.in_workflow();
        if let Some(editor) = app.active_editor_mut() {
            editor.area.remove_block();
            editor.area.set_style(style);
            editor.area.set_cursor_line_style(style);
            editor.area.set_line_number_style(line_style);
            editor
                .area
                .set_selection_style(style.add_modifier(Modifier::REVERSED));
            editor
                .area
                .set_cursor_style(if focused { cursor_style } else { style });
            editor
                .area
                .set_wrap_mode(ratatui_textarea::WrapMode::WordOrGlyph);
            frame.render_widget(&editor.area, body);
            let cursor = editor.area.cursor();
            frame.render_widget(
                Paragraph::new(format!(
                    "Ln {}, Col {}   {}",
                    cursor.0 + 1,
                    cursor.1 + 1,
                    if dirty {
                        if form_pending {
                            "Form pending"
                        } else {
                            "Unsaved changes"
                        }
                    } else {
                        "Saved source"
                    }
                ))
                .style(line_style),
                status,
            );
        }
    } else {
        let recovery_source = app.recovery_visible.then(|| {
            app.recovery
                .as_ref()
                .map(super::app::RecoveryNotice::text)
                .unwrap_or_default()
        });
        let literal = app.recovery_visible
            || app.reviewing_administration()
            || app.event_stage() == Some(true)
            || app.creation_review().is_some()
            || app.lifecycle_review().is_some()
            || matches!(
                app.body_title.as_str(),
                "NOTE LINKS"
                    | "NOTE BACKLINKS"
                    | "CROSS-PROJECT STATUS"
                    | "CROSS-PROJECT DIGEST"
                    | "PROJECT RESOLUTION"
                    | "PROJECT CONTEXT"
                    | "VALIDATION"
                    | "PROJECT BREADCRUMB"
                    | "AGENT ONBOARDING HANDOFF"
            )
            || app.body_title.starts_with("TEMPLATE · ")
            || app.body_title.starts_with("INTEGRATIONS · ");
        let cached = if let Some(source) = &recovery_source {
            app.reader_cache
                .matches(source, body.width, literal, app.color)
        } else if let Some(editor) = app.active_editor() {
            app.reader_cache
                .matches_editor(editor, body.width, literal, app.color)
        } else {
            app.reader_cache
                .matches(&app.body, body.width, literal, app.color)
        };
        if !cached {
            let source = if let Some(source) = recovery_source {
                Cow::Owned(source)
            } else if let Some(editor) = app.active_editor() {
                Cow::Owned(editor.source())
            } else {
                Cow::Borrowed(app.body.as_str())
            };
            let text = if literal {
                Text::from(safe_text(&source))
            } else {
                reading_text(&source, app)
            };
            app.reader_cache
                .update(source.into_owned(), text, body.width, literal, app.color);
        }
        let count = app.reader_cache.line_count();
        app.max_scroll = count
            .saturating_sub(usize::from(body.height))
            .min(u16::MAX as usize) as u16;
        app.scroll = app.scroll.min(app.max_scroll);
        frame.render_widget(
            app.reader_cache
                .viewport(app.scroll, body.height)
                .style(style),
            body,
        );
        frame.render_widget(
            Paragraph::new(format!(
                "{}-{} / {} lines  {}",
                usize::from(app.scroll) + 1,
                (usize::from(app.scroll) + usize::from(body.height)).min(count),
                count,
                if app.dirty() { "draft" } else { "" }
            ))
            .style(subdued(app)),
            status,
        );
    }
}

fn reading_text(source: &str, app: &App) -> Text<'static> {
    let source = safe_text(source);
    let mut code = false;
    Text::from(
        source
            .lines()
            .map(|line| {
                if line.starts_with("```") || line.starts_with("~~~") {
                    code = !code;
                    return Line::styled(line.to_owned(), subdued(app));
                }
                if !code && line.starts_with('#') {
                    Line::styled(
                        line.trim_start_matches('#').trim_start().to_owned(),
                        ink(app).add_modifier(Modifier::BOLD),
                    )
                } else if code {
                    Line::styled(line.to_owned(), ink(app))
                } else if line.starts_with('>') || line == "---" {
                    Line::styled(line.to_owned(), subdued(app))
                } else {
                    Line::from(line.to_owned())
                }
            })
            .collect::<Vec<_>>(),
    )
}

fn draw_dashboard(frame: &mut Frame, app: &App, area: Rect) {
    let block = panel("OVERVIEW", app.focus == Focus::Reader, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = vec![
        Line::styled(
            scope_name(&app.scope),
            ink(app).add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
    ];
    if let Some(p) = &app.projection {
        lines.push(Line::styled(
            format!(
                "Notes: {}    Projects: {}",
                p.dashboard.notes, p.dashboard.projects
            ),
            subdued(app),
        ));
        lines.push(Line::styled(
            format!(
                "Open tasks: {}    Open problems: {}",
                p.dashboard.open_tasks, p.dashboard.open_problems
            ),
            subdued(app),
        ));
    } else {
        lines.push(Line::styled("Loading memory...", subdued(app)));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(ink(app))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use akasha_core::{ResolutionEnvironment, ResolveRequest};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::mpsc;

    #[test]
    fn shortcut_actions_stay_below_prompt_and_use_muted_text() {
        let mut app = test_app();
        let mut terminal = Terminal::new(TestBackend::new(120, 38)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let text = screen(&terminal);
        assert!(!text.lines().take(3).any(|line| line.contains("F4")));
        for (rect, action) in &app.hits {
            if matches!(action, Action::Command(_) | Action::Search) {
                assert!(rect.y >= 36);
                assert_eq!(
                    terminal.backend().buffer()[(rect.x, rect.y)].fg,
                    subdued(&app).fg.unwrap()
                );
            }
        }
    }

    #[test]
    fn layouts_render_at_small_large_and_tiny_sizes_without_overflow() {
        let (jobs, _) = mpsc::channel();
        let request = ResolveRequest {
            root_override: None,
            project_override: None,
            cwd: "/tmp".into(),
            environment: ResolutionEnvironment::default(),
        };
        let mut app = App::new(request, jobs, true, true, false);
        for (width, height) in [(213, 60), (120, 40), (80, 24), (40, 12), (20, 5), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let first = terminal.backend().buffer().clone();
            for cell in &first.content {
                assert_eq!(cell.fg, Color::Reset);
                assert_eq!(cell.bg, Color::Reset);
                assert!(
                    cell.symbol().is_ascii(),
                    "ASCII fallback emitted {}",
                    cell.symbol()
                );
            }
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            assert_eq!(&first, terminal.backend().buffer());
        }
    }
    fn test_app() -> App {
        let (jobs, _) = mpsc::channel();
        App::new(
            ResolveRequest {
                root_override: None,
                project_override: Some("example".into()),
                cwd: "/tmp".into(),
                environment: ResolutionEnvironment::default(),
            },
            jobs,
            true,
            false,
            true,
        )
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        let b = terminal.backend().buffer();
        b.content
            .chunks(usize::from(b.area.width))
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn reader_cache_invalidates_for_exact_source_width_presentation_and_palette() {
        let mut app = test_app();
        app.body_title = "HELP".into();
        app.body = "# First 世界\n\n```\n# code\n```\n".into();
        app.focus = Focus::Reader;
        let draw_at = |app: &mut App, width| {
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            terminal.draw(|frame| draw(frame, app)).unwrap();
            screen(&terminal)
        };
        let initial = draw_at(&mut app, 80);
        assert!(initial.contains("First"));
        assert!(!initial.contains("# First"));
        assert!(initial.contains("# code"));
        let width = (1..80)
            .find(|width| app.reader_cache.matches(&app.body, *width, false, true))
            .unwrap();
        let pointer = app.body.as_ptr();
        assert_eq!(draw_at(&mut app, 80), initial);
        assert_eq!(app.body.as_ptr(), pointer);
        app.body = app.body.replace("First", "Other"); // same byte length
        assert!(draw_at(&mut app, 80).contains("Other"));
        assert!(!app.reader_cache.matches(
            "# First 世界\n\n```\n# code\n```\n",
            width,
            false,
            true
        ));
        app.body_title = "VALIDATION".into();
        assert!(draw_at(&mut app, 80).contains("# Other"));
        assert!(app.reader_cache.matches(&app.body, width, true, true));
        app.color = false;
        draw_at(&mut app, 80);
        assert!(app.reader_cache.matches(&app.body, width, true, false));
        draw_at(&mut app, 40);
        assert!(!app.reader_cache.matches(&app.body, width, true, false));
        app.body = "# literal\u{1b}[2J\n".into();
        let literal = draw_at(&mut app, 80);
        assert!(literal.contains("# literal�[2J"));
        assert!(app.body.contains('\u{1b}'));
    }

    #[test]
    fn animation_freezes_in_readers_editors_unfocused_and_reduced_motion() {
        let mut app = test_app();
        app.no_motion = false;
        app.focus = Focus::List;
        assert!(app.animation_active(true)); // dashboard
        app.body_title = "HELP".into();
        for editing in [false, true] {
            app.editing = editing;
            for focus in [Focus::Reader, Focus::List] {
                app.focus = focus;
                assert!(!app.animation_active(true));
            }
            app.focus = Focus::Prompt;
            assert!(app.animation_active(true));
            assert!(!app.animation_active(false));
            app.no_motion = true;
            assert!(!app.animation_active(true));
            app.no_motion = false;
        }
        app.body_title = "WELCOME TO AKASHA".into();
        app.focus = Focus::Reader;
        app.recovery_visible = true;
        assert!(!app.animation_active(true));
    }

    #[test]
    fn minimum_layout_keeps_state_quit_and_last_selected_item_visible() {
        for (width, height) in [(40, 12), (80, 24), (120, 38)] {
            let mut app = test_app();
            app.focus = Focus::List;
            app.rows = (1..=30)
                .map(|i| super::super::app::Row {
                    label: format!("section-{i}"),
                    detail: "3 notes".into(),
                    target: Target::Category(app.scope.clone(), format!("section-{i}")),
                })
                .collect();
            app.list.select(Some(29));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            let text = screen(&terminal);
            assert!(
                text.contains("section-30"),
                "Selected item was hidden: {text}"
            );
            assert!(text.contains("ready"), "Status was truncated: {text}");
            assert!(
                text.contains("Ctrl-Q quit") || text.contains("ctrl+q quit"),
                "Exit hint was truncated: {text}"
            );
        }
    }

    #[test]
    fn command_palette_scrolls_and_never_overwrites_typed_unicode() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for (width, height) in [(40, 12), (80, 24), (120, 38)] {
            let mut app = test_app();
            app.paste("/");
            let last_command = app.completions().last().unwrap().command.clone();
            app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            assert!(screen(&terminal).contains(&format!("/{last_command}")));
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
            app.paste("search Привет 世界");
            let before = app.prompt.lines().to_vec();
            for tick in [0, 12, 96] {
                app.tick = tick;
                terminal.draw(|f| draw(f, &mut app)).unwrap();
                let rendered = screen(&terminal);
                assert!(rendered.contains("search Привет"), "{rendered}");
                // TestBackend stores a blank continuation cell after each wide CJK glyph.
                assert!(
                    rendered.replace(' ', "").contains("searchПривет世界"),
                    "{rendered}"
                );
                assert_eq!(app.prompt.lines(), before);
            }
        }
    }
    #[test]
    fn colored_interface_preserves_terminal_background() {
        let mut app = test_app();
        for command in ["", "/", "search something"] {
            app.prompt = ratatui_textarea::TextArea::new(vec![command.into()]);
            let mut terminal = Terminal::new(TestBackend::new(120, 38)).unwrap();
            terminal.draw(|f| draw(f, &mut app)).unwrap();
            for cell in &terminal.backend().buffer().content {
                assert_eq!(cell.bg, Color::Reset);
            }
            let text = screen(&terminal);
            assert!(!text.contains("Project memory"));
            assert!(!text.contains("A K A S H A"));
        }
    }

    #[test]
    fn recovery_inspection_is_literal_scrollable_and_has_compact_retry_controls() {
        use super::super::app::Response;
        use akasha_core::PendingNoteEditInspection;
        for (width, height) in [(40, 12), (80, 24), (120, 38)] {
            let mut app = test_app();
            app.receive(Ok(Response::RecoveryInspected(
                app.recovery_origin(),
                Ok(PendingNoteEditInspection {
                    project: "example".into(),
                    journal_path: "/tmp/Привет\n#literal\u{1b}[2J".into(),
                    pending: true,
                }),
            )));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let initial = screen(&terminal);
            assert!(initial.contains("recovery"), "{initial}");
            assert!(initial.contains("RECOVERY INSPECTION"), "{initial}");
            for action in ["refresh", "back", "quit", "recovery"] {
                assert!(
                    app.hits.iter().any(
                        |(_, hit)| matches!(hit, Action::Command(command) if *command == action)
                    ),
                    "missing {action}: {initial}"
                );
            }
            if height <= 24 {
                assert!(app.max_scroll > 0);
            }
            let mut visible = initial;
            for scroll in 1..=app.max_scroll {
                app.scroll = scroll;
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                visible.push_str(&screen(&terminal));
            }
            assert!(
                visible.contains("#literal"),
                "diagnostics must omit Markdown decoration: {visible}"
            );
            assert!(!visible.contains('\u{1b}'));
            assert!(visible.contains("private backup"));
            assert!(!visible.contains("Project memory"));
        }
    }

    #[test]
    fn cross_project_reports_render_literal_safe_bounded_fields_at_compact_sizes() {
        use super::super::app::Response;
        use akasha_core::{
            LibraryDashboard, LibraryProjectDashboard, LibraryReport, LibraryReportKind,
        };
        for (width, height) in [(40, 12), (80, 24), (120, 38)] {
            let mut app = test_app();
            app.receive(Ok(Response::LibraryReport(LibraryReport {
                kind: LibraryReportKind::Status,
                root: "/tmp".into(),
                selected_project: "example".into(),
                limit: 30,
                truncated: true,
                dashboard: LibraryDashboard {
                    validation_passed: true,
                    projects: 32,
                    notes: 32,
                    global_notes: 2,
                    configured_categories: 5,
                    open_tasks: 3,
                    open_problems: 2,
                    validated_links: 9,
                    latest_activity_date: None,
                    project_metrics: (0..30)
                        .map(|number| LibraryProjectDashboard {
                            project: format!("#literal-{number:02}"),
                            status: format!("Привет\u{1b}[2J{}", "x".repeat(150)),
                            notes: 1,
                            populated_categories: 1,
                            open_tasks: 0,
                            open_problems: 0,
                            validated_links: 0,
                            latest_activity_date: None,
                        })
                        .collect(),
                },
            })));
            assert!(app.body.contains('…'));
            assert!(!app.body.contains(&"x".repeat(121)));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let mut visible = screen(&terminal);
            assert!(visible.contains("CROSS-PROJECT STATUS"));
            assert!(app.max_scroll > 0);
            for scroll in 1..=app.max_scroll {
                app.scroll = scroll;
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                visible.push_str(&screen(&terminal));
            }
            assert!(visible.contains("#literal-29"));
            assert!(!visible.contains('\u{1b}'));
            assert!(
                app.hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Action::Command("back")))
            );
            app.command("back");
            assert_eq!(app.body_title, "WELCOME TO AKASHA");
        }
    }

    #[test]
    fn resolution_inspection_is_literal_scrollable_and_safe_at_compact_sizes() {
        use super::super::app::Response;
        use akasha_core::{ProjectSource, ResolvedProject, RootSource};
        // A real terminal consumes continuation cells when drawing a wide symbol.
        // TestBackend can retain their previous-frame bytes; do not observe those bytes.
        fn glyph_screen(terminal: &Terminal<TestBackend>) -> String {
            let buffer = terminal.backend().buffer();
            let mut text = String::new();
            for row in buffer.content.chunks(usize::from(buffer.area.width)) {
                let mut continuation = 0;
                for cell in row {
                    if continuation > 0 {
                        continuation -= 1;
                        continue;
                    }
                    text.push_str(cell.symbol());
                    continuation = ratatui::text::Span::raw(cell.symbol())
                        .width()
                        .saturating_sub(1);
                }
                text.push('\n');
            }
            text
        }
        for (width, height) in [(40, 12), (80, 24), (120, 38)] {
            let mut app = test_app();
            app.receive(Ok(Response::Resolved(
                app.request.clone(),
                Box::new(ResolvedProject {
                    root: "/tmp\n#literal\n---\u{1b}[2J".into(),
                    root_source: RootSource::Environment,
                    project: "example".into(),
                    project_source: ProjectSource::Pointer,
                    pointer: Some("/tmp/Привет 世界/.akasha.toml".into()),
                    registry: "/tmp/Meta/projects.yaml".into(),
                    repository_dir: "/tmp/Привет 世界".into(),
                    project_dir: "/tmp/Projects/example".into(),
                }),
            )));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let mut visible = glyph_screen(&terminal);
            assert!(visible.contains("PROJECT RESOLUTION"));
            for scroll in 1..=app.max_scroll {
                app.scroll = scroll;
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                visible.push_str(&glyph_screen(&terminal));
            }
            assert!(visible.contains("#literal") && visible.contains("---"));
            assert!(visible.contains("Привет 世界"), "{visible}");
            assert!(!visible.contains('\u{1b}'));
            assert!(
                app.hits
                    .iter()
                    .any(|(_, action)| matches!(action, Action::Command("back")))
            );
            app.command("back");
            assert_eq!(app.body_title, "WELCOME TO AKASHA");
        }
    }

    #[test]
    fn context_budget_reader_preserves_literal_framing_and_compact_return() {
        use super::super::app::Response;
        use akasha_core::{ContextBundle, ContextEntry, ContextSection};
        for (width, height) in [(40, 12), (80, 24)] {
            let mut app = test_app();
            app.receive(Ok(Response::Context(
                app.request.clone(),
                Box::new(ContextBundle {
                    project: "example".into(),
                    repository_dir: "/tmp/repository".into(),
                    project_dir: "/tmp/project".into(),
                    errors: vec![],
                    entries: vec![ContextEntry {
                        section: ContextSection::OpenTask,
                        source: "records/tasks/a.md".into(),
                        content: "#literal\n---\n``````\nCONTROL\u{1b}[2J\n".repeat(10),
                    }],
                    truncated: false,
                    omitted_entries: 0,
                    omissions: vec![],
                    max_chars: 32_000,
                    rendered_chars: 0,
                }),
            )));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let mut visible = screen(&terminal);
            assert!(visible.contains("PROJECT CONTEXT"));
            assert!(app.max_scroll > 0);
            for scroll in 1..=app.max_scroll {
                app.scroll = scroll;
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                visible.push_str(&screen(&terminal));
            }
            assert!(visible.contains("#literal") && visible.contains("---"));
            assert!(visible.contains("```````akasha-project-data"));
            assert!(!visible.contains('\u{1b}'));
            assert!(
                app.hits
                    .iter()
                    .any(|(_, action)| matches!(action, Action::Command("back")))
            );
            app.command("back");
            assert_eq!(app.body_title, "WELCOME TO AKASHA");
        }
    }

    #[test]
    fn read_only_reader_text_is_literal_unicode_safe_scrollable_and_returnable() {
        use super::super::app::Response;
        fn glyph_screen(terminal: &Terminal<TestBackend>) -> String {
            let buffer = terminal.backend().buffer();
            let mut text = String::new();
            for row in buffer.content.chunks(usize::from(buffer.area.width)) {
                let mut continuation = 0;
                for cell in row {
                    if continuation > 0 {
                        continuation -= 1;
                        continue;
                    }
                    text.push_str(cell.symbol());
                    continuation = ratatui::text::Span::raw(cell.symbol())
                        .width()
                        .saturating_sub(1);
                }
                text.push('\n');
            }
            text
        }
        let source = "# literal\n---\n```\n{{field}}\n```\n[[link]]\nПривет 世界\nCONTROL\u{1b}[2J\n  leading  \nend";
        for title in [
            "VALIDATION",
            "PROJECT BREADCRUMB",
            "TEMPLATE · task",
            "AGENT ONBOARDING HANDOFF",
            "INTEGRATIONS · CODEX",
        ] {
            for (width, height) in [(40, 12), (80, 24), (120, 38)] {
                let mut app = test_app();
                app.receive(Ok(Response::Text(
                    app.request.clone(),
                    title.into(),
                    source.into(),
                )));
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                let mut visible = glyph_screen(&terminal);
                for scroll in 1..=app.max_scroll {
                    app.scroll = scroll;
                    terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                    visible.push_str(&glyph_screen(&terminal));
                }
                for literal in [
                    "# literal",
                    "---",
                    "```",
                    "{{field}}",
                    "[[link]]",
                    "Привет 世界",
                    "CONTROL",
                    "  leading  ",
                    "end",
                ] {
                    assert!(
                        visible.contains(literal),
                        "{title} {width}x{height}: {literal}"
                    );
                }
                assert!(!visible.contains('\u{1b}'));
                assert_eq!(app.body, source);
                assert!(
                    app.hits
                        .iter()
                        .any(|(_, action)| matches!(action, Action::Command("back")))
                );
                app.command("back");
                assert_eq!(app.body_title, "WELCOME TO AKASHA");
            }
        }
    }

    #[test]
    fn note_link_reports_are_literal_scrollable_and_keep_compact_return_controls() {
        use super::super::app::Response;
        use akasha_core::{NoteLink, NoteLinkDirection, NoteLinkKind, NoteLinksResult};
        for (width, height) in [(40, 12), (80, 24), (120, 38)] {
            let mut app = test_app();
            app.receive(Ok(Response::NoteLinks(super::super::links::Report {
                source: None,
                limit: 30,
                result: NoteLinksResult {
                    note: "Projects/example/entities/Привет 世界.md".into(),
                    direction: NoteLinkDirection::Incoming,
                    scanned_canonical_notes: 35,
                    scanned_projections: 2,
                    total_matches: 32,
                    truncated: true,
                    links: (0..30)
                        .map(|number| NoteLink {
                            id: format!("#literal/{number:02}.md\u{1b}[2J"),
                            kind: NoteLinkKind::Canonical,
                        })
                        .collect(),
                },
            })));
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let mut visible = screen(&terminal);
            assert!(visible.contains("NOTE BACKLINKS"));
            assert!(
                app.hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Action::Command("back")))
            );
            assert!(
                app.hits
                    .iter()
                    .any(|(_, hit)| matches!(hit, Action::Command("quit")))
            );
            assert!(app.max_scroll > 0);
            for scroll in 1..=app.max_scroll {
                app.scroll = scroll;
                terminal.draw(|frame| draw(frame, &mut app)).unwrap();
                visible.push_str(&screen(&terminal));
            }
            assert!(visible.contains("#literal/29.md"), "{visible}");
            assert!(!visible.contains('\u{1b}'));
            assert!(visible.contains("32 matches; showing 30"), "{visible}");
            app.command("back");
            assert_eq!(app.body_title, "WELCOME TO AKASHA");
            assert!(app.focus == Focus::List);
        }
    }
}
