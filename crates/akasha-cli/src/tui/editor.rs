use ratatui_textarea::TextArea;

/// A terminal editor buffer preserves newline style and a trailing empty line explicitly.
/// Canonical bytes never pass through the reading-mode renderer.
pub(super) struct Editor {
    pub area: TextArea<'static>,
    pub original: String,
    separator: &'static str,
}

impl Editor {
    pub fn new(source: &str) -> Result<Self, String> {
        let crlf = source.contains("\r\n");
        let normalized = source.replace("\r\n", "\n");
        if normalized
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err("This source contains control characters; it is readable but cannot be edited here.".into());
        }
        if crlf && source.replace("\r\n", "").contains('\n') {
            return Err(
                "Mixed LF/CRLF endings require exact external editing; this note remains readable."
                    .into(),
            );
        }
        let mut area = TextArea::new(normalized.split('\n').map(str::to_owned).collect());
        area.set_max_histories(100);
        area.set_hard_tab_indent(true);
        Ok(Self {
            area,
            original: source.to_owned(),
            separator: if crlf { "\r\n" } else { "\n" },
        })
    }

    pub fn source(&self) -> String {
        self.area.lines().join(self.separator)
    }
    pub fn dirty(&self) -> bool {
        self.source() != self.original
    }
}

pub(super) fn safe_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => '�',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui_textarea::CursorMove;

    #[test]
    fn exact_newlines_unicode_and_undo() {
        for source in [
            "",
            "one",
            "one\n",
            "one\n\n",
            "one\r\n",
            "one\r\ntwo",
            "Привет 世界\n\ttext  \n",
        ] {
            let mut editor = Editor::new(source).unwrap();
            assert_eq!(editor.source(), source);
            editor.area.move_cursor(CursorMove::Bottom);
            editor.area.move_cursor(CursorMove::End);
            editor.area.insert_str("λ");
            assert!(editor.dirty());
            assert!(editor.area.undo());
            assert_eq!(editor.source(), source);
            assert!(!editor.dirty());
        }
    }

    #[test]
    fn unsafe_edit_forms_are_rejected_and_display_is_sanitized() {
        assert!(Editor::new("one\r\ntwo\n").is_err());
        assert!(Editor::new("a\x1b[2J").is_err());
        assert!(!safe_text("a\x1b[2J\u{9b}").contains('\x1b'));
    }
}
