use ratatui::{
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

/// One document layout. Keep logical lines rather than a document-sized terminal buffer.
#[derive(Default)]
pub(super) struct ReaderCache {
    source: String,
    key: Option<(u16, bool, bool)>,
    text: Text<'static>,
    ends: Vec<usize>,
}

impl ReaderCache {
    pub fn matches(&self, source: &str, width: u16, literal: bool, color: bool) -> bool {
        self.key == Some((width, literal, color)) && self.source == source
    }

    pub fn matches_editor(
        &self,
        editor: &super::editor::Editor,
        width: u16,
        literal: bool,
        color: bool,
    ) -> bool {
        self.key == Some((width, literal, color)) && editor.matches_source(&self.source)
    }

    pub fn update(
        &mut self,
        source: String,
        text: Text<'static>,
        width: u16,
        literal: bool,
        color: bool,
    ) {
        let mut count = 0;
        self.ends = text
            .lines
            .iter()
            .map(|line| {
                count += Paragraph::new(borrow_line(line))
                    .wrap(Wrap { trim: false })
                    .line_count(width);
                count
            })
            .collect();
        self.source = source;
        self.text = text;
        self.key = Some((width, literal, color));
    }

    pub fn line_count(&self) -> usize {
        self.ends.last().copied().unwrap_or(0)
    }

    pub fn viewport(&self, scroll: u16, height: u16) -> Paragraph<'_> {
        let offset = usize::from(scroll);
        let start = self.ends.partition_point(|end| *end <= offset);
        let end = self
            .ends
            .partition_point(|end| *end < offset + usize::from(height))
            .saturating_add(1)
            .min(self.text.lines.len());
        let skipped = if start == 0 { 0 } else { self.ends[start - 1] };
        let lines = self.text.lines[start..end.max(start)]
            .iter()
            .map(borrow_line)
            .collect();
        Paragraph::new(Text {
            lines,
            style: self.text.style,
            alignment: self.text.alignment,
        })
        .wrap(Wrap { trim: false })
        .scroll((scroll.saturating_sub(skipped as u16), 0))
    }
}

fn borrow_line<'a>(line: &'a Line<'_>) -> Line<'a> {
    Line {
        spans: line
            .spans
            .iter()
            .map(|span| Span::styled(span.content.as_ref(), span.style))
            .collect(),
        style: line.style,
        alignment: line.alignment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        buffer::Buffer,
        layout::{Alignment, Rect},
        style::{Color, Modifier, Style},
        widgets::Widget,
    };

    #[test]
    fn cached_viewports_match_full_reference_wrapping_and_styles() {
        let style = Style::default().fg(Color::Magenta);
        let cases = [
            Text::default(),
            Text::from(""),
            Text::from("\n\n"),
            Text::from("  spaces\tand tabs   \n世界 e\u{301} 👩‍💻 Привет\n\nfinal  "),
            Text::from("A".repeat(300)),
            Text::from(vec![
                Line::from(vec![
                    Span::styled("heading ", style.add_modifier(Modifier::BOLD)),
                    Span::raw("世界 e\u{301} 👩‍💻"),
                ]),
                Line::styled("right aligned long text  ", style).alignment(Alignment::Right),
                Line::from("\u{200b}a\u{200b}b"),
                Line::from(""),
            ])
            .style(Style::default().bg(Color::Blue))
            .alignment(Alignment::Center),
        ];
        for text in cases {
            for width in [1, 2, 7, 35, 80] {
                let reference = Paragraph::new(text.clone())
                    .style(style)
                    .wrap(Wrap { trim: false });
                let count = reference.line_count(width);
                let mut cache = ReaderCache::default();
                cache.update("source".into(), text.clone(), width, false, true);
                assert_eq!(cache.line_count(), count);
                for height in [1, 7] {
                    for scroll in 0..=count.min(u16::MAX as usize) as u16 {
                        let area = Rect::new(0, 0, width, height);
                        let mut expected = Buffer::empty(area);
                        reference
                            .clone()
                            .scroll((scroll, 0))
                            .render(area, &mut expected);
                        let mut actual = Buffer::empty(area);
                        cache
                            .viewport(scroll, height)
                            .style(style)
                            .render(area, &mut actual);
                        assert_eq!(
                            actual, expected,
                            "width={width}, height={height}, scroll={scroll}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn deep_scroll_borrows_only_the_visible_logical_lines() {
        let source = "first 世界 e\u{301}\n".repeat(20_000);
        let mut cache = ReaderCache::default();
        cache.update(source.clone(), Text::from(source.clone()), 35, true, false);
        assert!(cache.matches(&source, 35, true, false));
        assert_eq!(cache.line_count(), 20_000);
        // Independent visible text check near the end, including blank space below EOF.
        let area = Rect::new(0, 0, 35, 7);
        let mut actual = Buffer::empty(area);
        cache.viewport(19_998, 7).render(area, &mut actual);
        let mut expected = Buffer::empty(area);
        Paragraph::new("first 世界 e\u{301}\nfirst 世界 e\u{301}").render(area, &mut expected);
        assert_eq!(actual, expected);
        let paragraph = cache.viewport(19_000, 7);
        assert_eq!(paragraph.line_count(35), 7);
        let start = cache.ends.partition_point(|end| *end <= 19_000);
        assert_eq!(start, 19_000);
        assert!(matches!(
            borrow_line(&cache.text.lines[start]).spans[0].content,
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn editor_layout_reuse_checks_exact_lines_after_undo_redo_and_newline_changes() {
        use super::super::editor::Editor;
        use ratatui_textarea::CursorMove;
        for source in ["", "first\n\n", "first\r\n世界\r\n"] {
            let mut editor = Editor::new(source).unwrap();
            let mut cache = ReaderCache::default();
            cache.update(
                source.into(),
                Text::from(source.to_owned()),
                35,
                false,
                true,
            );
            assert!(cache.matches_editor(&editor, 35, false, true));
            editor.area.move_cursor(CursorMove::Bottom);
            editor.area.move_cursor(CursorMove::End);
            editor.area.insert_str("changed\n");
            assert!(!cache.matches_editor(&editor, 35, false, true));
            assert!(editor.area.undo());
            assert!(cache.matches_editor(&editor, 35, false, true));
            assert!(editor.area.redo());
            assert!(!cache.matches_editor(&editor, 35, false, true));
            assert!(!cache.matches_editor(&editor, 36, false, true));
            assert!(!cache.matches_editor(&editor, 35, true, true));
            assert!(!cache.matches_editor(&editor, 35, false, false));
        }
    }

    #[test]
    #[ignore = "manual synthetic runtime measurement; no machine-specific timing gate"]
    fn synthetic_reader_runtime() {
        use std::{hint::black_box, time::Instant};
        for lines in [100, 1_000, 10_000] {
            let source =
                "# Привет 世界 e\u{301}  repeated words for measured line wrapping and scroll\n"
                    .repeat(lines);
            let text = Text::from(source.clone());
            let width = 76;
            let area = Rect::new(0, 0, width, 20);
            let mut cache = ReaderCache::default();
            let start = Instant::now();
            cache.update(source.clone(), text.clone(), width, true, false);
            let cold = start.elapsed();
            let count = cache.line_count();
            let repeats = 30;
            let start = Instant::now();
            for n in 0..repeats {
                let scroll = ((count - 20) * n / (repeats - 1)).min(u16::MAX as usize) as u16;
                assert!(cache.matches(black_box(&source), width, true, false));
                let mut buffer = Buffer::empty(area);
                cache
                    .viewport(scroll, area.height)
                    .render(area, &mut buffer);
                black_box(buffer);
            }
            let cached = start.elapsed();
            let start = Instant::now();
            for n in 0..repeats {
                let scroll = ((count - 20) * n / (repeats - 1)).min(u16::MAX as usize) as u16;
                let paragraph = Paragraph::new(text.clone()).wrap(Wrap { trim: false });
                black_box(paragraph.line_count(width));
                let mut buffer = Buffer::empty(area);
                paragraph.scroll((scroll, 0)).render(area, &mut buffer);
                black_box(buffer);
            }
            let reference = start.elapsed();
            println!(
                "{{\"logical_lines\":{lines},\"bytes\":{},\"width\":{width},\"frames\":{repeats},\"cold_us\":{},\"cached_us\":{},\"reference_us\":{}}}",
                source.len(),
                cold.as_micros(),
                cached.as_micros(),
                reference.as_micros()
            );
        }
    }
}
