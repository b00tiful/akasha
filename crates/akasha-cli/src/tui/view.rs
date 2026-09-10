use super::{
    app::{App, Focus, scope_name},
    editor::safe_text,
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};

fn ink(app: &App) -> Style {
    if app.color {
        Style::default().fg(Color::Gray).bg(Color::Black)
    } else {
        Style::default()
    }
}
fn accent(app: &App) -> Style {
    if app.color {
        ink(app).fg(Color::Magenta)
    } else {
        ink(app).add_modifier(Modifier::BOLD)
    }
}
fn subdued(app: &App) -> Style {
    if app.color {
        ink(app).fg(Color::DarkGray)
    } else {
        ink(app).add_modifier(Modifier::DIM)
    }
}
fn frame_box(title: &str, focused: bool, app: &App) -> Block<'static> {
    let set = if app.ascii {
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
        symbols::border::DOUBLE
    };
    Block::default()
        .borders(Borders::ALL)
        .border_set(set)
        .border_style(if focused { accent(app) } else { subdued(app) })
        .title(Line::from(format!(" {} ", safe_text(title))).style(ink(app)))
}

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(ink(app)), area);
    if area.width < 40 || area.height < 12 {
        frame.render_widget(Paragraph::new("AKASHA\nTerminal too small: use at least 40 x 12.\nResize to continue; Ctrl-Q exits safely.")
            .wrap(Wrap { trim: false }), area);
        return;
    }
    let [header, main, log, prompt, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let mark = if app.ascii { "+" } else { "◇" };
    let state = if app.busy {
        "working"
    } else if app.dirty() {
        "unsaved"
    } else {
        "ready"
    };
    let title = format!(
        "{mark}  A K A S H A  {mark}    {}    /    {state}",
        scope_name(&app.scope)
    );
    frame.render_widget(
        Paragraph::new(safe_text(&title))
            .alignment(Alignment::Center)
            .block(frame_box("MEMORY LIBRARY", false, app))
            .style(ink(app)),
        header,
    );

    // Narrow layouts keep the focused pane usable; all operations remain reachable with Tab.
    if main.width >= 84 {
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length((main.width / 3).clamp(26, 38)),
                Constraint::Min(20),
            ])
            .split(main);
        draw_list(frame, app, panels[0]);
        draw_reader(frame, app, panels[1]);
    } else if app.focus == Focus::List {
        draw_list(frame, app, main);
    } else {
        draw_reader(frame, app, main);
    }

    let messages: Vec<_> = app
        .messages
        .iter()
        .rev()
        .take(2)
        .rev()
        .map(|m| Line::from(m.clone()))
        .collect();
    frame.render_widget(Paragraph::new(messages).style(subdued(app)), log);
    let block = frame_box(
        "COMMAND  /  Up Down history",
        app.focus == Focus::Prompt,
        app,
    );
    let style = ink(app);
    let cursor = if app.focus == Focus::Prompt {
        accent(app).add_modifier(Modifier::REVERSED)
    } else {
        style
    };
    app.prompt.set_block(block);
    app.prompt.set_style(style);
    app.prompt.set_placeholder_style(if app.color {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    });
    app.prompt.set_cursor_line_style(style);
    app.prompt.set_cursor_style(cursor);
    app.prompt
        .set_selection_style(style.add_modifier(Modifier::REVERSED));
    frame.render_widget(&app.prompt, prompt);
    let keys = if app.editing {
        " Esc prompt   Ctrl-S save   Ctrl-Z undo   Ctrl-Y redo   Shift select "
    } else {
        " Tab focus   Enter open   F2 edit   F5 refresh   F1 help   Ctrl-Q quit "
    };
    frame.render_widget(Paragraph::new(keys).style(subdued(app)), footer);
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let rows: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            ListItem::new(vec![
                Line::from(format!("{:>2}  {}", i + 1, safe_text(&row.label))),
                Line::from(format!("    {}", safe_text(&row.detail))).style(subdued(app)),
            ])
        })
        .collect();
    let list = List::new(rows)
        .block(frame_box(
            &format!("{} · {}", app.title, app.rows.len()),
            app.focus == Focus::List,
            app,
        ))
        .style(ink(app))
        .highlight_style(accent(app).add_modifier(Modifier::REVERSED))
        .highlight_symbol(if app.ascii { "> " } else { "▸ " });
    frame.render_stateful_widget(list, area, &mut app.list);
    if app.rows.is_empty() && area.height > 3 {
        let inner = Rect::new(
            area.x + 2,
            area.y + 2,
            area.width.saturating_sub(4),
            area.height.saturating_sub(4),
        );
        frame.render_widget(
            Paragraph::new(if app.busy {
                "Loading…"
            } else {
                "No notes here.\nUse projects or help."
            })
            .style(subdued(app)),
            inner,
        );
    }
}

fn draw_reader(frame: &mut Frame, app: &mut App, area: Rect) {
    let style = ink(app);
    if app.editing && app.editor.is_some() {
        let title = format!(
            "SOURCE{} · {}",
            if app.dirty() { " *" } else { "" },
            app.body_title
        );
        let block = frame_box(&title, app.focus == Focus::Reader, app);
        let cursor = accent(app).add_modifier(Modifier::REVERSED);
        let line_style = subdued(app);
        if let Some(editor) = &mut app.editor {
            editor.area.set_block(block);
            editor.area.set_style(style);
            editor.area.set_cursor_line_style(style);
            editor.area.set_line_number_style(line_style);
            editor
                .area
                .set_selection_style(style.add_modifier(Modifier::REVERSED));
            editor.area.set_cursor_style(if app.focus == Focus::Reader {
                cursor
            } else {
                style
            });
            editor
                .area
                .set_wrap_mode(ratatui_textarea::WrapMode::WordOrGlyph);
            frame.render_widget(&editor.area, area);
        }
        return;
    }
    let block = frame_box(&app.body_title, app.focus == Focus::Reader, app);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.document.is_none()
        && app.body_title == "WELCOME TO AKASHA"
        && inner.height >= 13
        && inner.width >= 42
    {
        let [dashboard, hint] =
            Layout::vertical([Constraint::Length(13), Constraint::Min(0)]).areas(inner);
        draw_dashboard(frame, app, dashboard);
        frame.render_widget(Paragraph::new("\nBrowse your memory\n\nTab to select a category. Enter opens a note.\nType search <text> to find knowledge.\n\nF2 opens the built-in source editor.\nF1 shows all commands and shortcuts.")
            .style(style).wrap(Wrap { trim: false }), hint);
    } else {
        let source = if let Some(editor) = &app.editor {
            editor.source()
        } else {
            app.body.clone()
        };
        let text = reading_text(&source, app);
        // Paragraph owns wrapping and scroll geometry; no note parser or filesystem access here.
        let paragraph = Paragraph::new(text).style(style).wrap(Wrap { trim: false });
        app.max_scroll = paragraph
            .line_count(inner.width)
            .saturating_sub(usize::from(inner.height))
            .min(u16::MAX as usize) as u16;
        app.scroll = app.scroll.min(app.max_scroll);
        frame.render_widget(paragraph.scroll((app.scroll, 0)), inner);
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
                        accent(app).add_modifier(Modifier::BOLD),
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
    let [art, metrics] =
        Layout::horizontal([Constraint::Length(25), Constraint::Min(1)]).areas(area);
    draw_sigil(frame, app, art);
    let Some(projection) = &app.projection else {
        return;
    };
    let d = &projection.dashboard;
    let text = vec![
        Line::styled("LIBRARY STATUS", accent(app)),
        Line::from(""),
        Line::from(format!("Projects       {}", d.projects)),
        Line::from(format!("Notes          {}", d.notes)),
        Line::from(format!("Global notes   {}", d.global_notes)),
        Line::from(format!("Open tasks     {}", d.open_tasks)),
        Line::from(format!("Open problems  {}", d.open_problems)),
        Line::from(format!("Checked links  {}", d.validated_links)),
        Line::from(""),
        Line::styled("Validated snapshot", subdued(app)),
        Line::styled("F5 refreshes memory", subdued(app)),
    ];
    frame.render_widget(Paragraph::new(text).style(ink(app)), metrics);
}

fn draw_sigil(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 24 || area.height < 12 {
        return;
    }
    let phase = app.tick as f64 * 0.065;
    let center_x = area.x + 12;
    let center_y = area.y + 6;
    let buffer = frame.buffer_mut();
    for i in 0..48 {
        let angle = (i as f64 / 48.0) * std::f64::consts::TAU;
        let x = (f64::from(center_x) + angle.cos() * 10.0).round() as u16;
        let y = (f64::from(center_y) + angle.sin() * 5.0).round() as u16;
        if let Some(cell) = buffer.cell_mut((x, y)) {
            cell.set_symbol(if app.ascii { "." } else { "·" })
                .set_style(subdued(app));
        }
    }
    for offset in [0.0, std::f64::consts::PI] {
        let x = (f64::from(center_x) + (phase + offset).cos() * 10.0).round() as u16;
        let y = (f64::from(center_y) + (phase + offset).sin() * 5.0).round() as u16;
        if let Some(cell) = buffer.cell_mut((x, y)) {
            cell.set_symbol(if app.ascii { "*" } else { "◆" })
                .set_style(accent(app));
        }
    }
    let book = if app.ascii {
        [
            "  _____ _____  ",
            " / ___ | ___ \\ ",
            "|  --- | ---  |",
            "|  --- | ---  |",
            "|______|______|",
        ]
    } else {
        [
            "  ▄▄▄▄▄ ▄▄▄▄▄  ",
            " ▐ ░░░ ▌ ░░░ ▌ ",
            " ▐ ▄▄▄ ▌ ▄▄▄ ▌ ",
            " ▐ ▄▄▄ ▌ ▄▄▄ ▌ ",
            " ▀▀▀▀▀▀▄▀▀▀▀▀▀ ",
        ]
    };
    for (i, line) in book.iter().enumerate() {
        buffer.set_string(center_x - 7, center_y - 2 + i as u16, line, ink(app));
    }
    for (dx, dy) in [(0, -5), (0, 5), (-10, 0), (10, 0)] {
        if let Some(cell) = buffer.cell_mut((
            center_x.saturating_add_signed(dx),
            center_y.saturating_add_signed(dy),
        )) {
            cell.set_symbol("+").set_style(ink(app));
        }
    }
    buffer.set_string(center_x - 4, area.y, "  *  *  ", subdued(app));
}

#[cfg(test)]
mod tests {
    use super::*;
    use akasha_core::{ResolutionEnvironment, ResolveRequest};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::mpsc;

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
        for (width, height) in [(120, 40), (80, 24), (40, 12), (20, 5), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            let first = terminal.backend().buffer().clone();
            for cell in &first.content {
                assert_eq!(cell.fg, Color::Reset);
                assert_eq!(cell.bg, Color::Reset);
            }
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            assert_eq!(&first, terminal.backend().buffer());
        }
    }
}
