use crate::wrapping::word_wrap_lines;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

const MAX_VISIBLE_LINES: usize = 3;

#[derive(Default)]
pub(crate) struct DictationPreview {
    completed: String,
    partial: String,
}

impl DictationPreview {
    pub(crate) fn set(&mut self, completed: &str, partial: &str) {
        completed.clone_into(&mut self.completed);
        partial.clone_into(&mut self.partial);
    }

    pub(crate) fn clear(&mut self) {
        self.completed.clear();
        self.partial.clear();
    }

    fn lines(&self, width: u16) -> Vec<Line<'static>> {
        if width == 0 || (self.completed.is_empty() && self.partial.is_empty()) {
            return Vec::new();
        }

        let mut spans = Vec::new();
        if !self.completed.is_empty() {
            spans.push(Span::from(self.completed.clone()));
        }
        if !self.partial.is_empty() {
            if !self.completed.is_empty()
                && !self.completed.ends_with(char::is_whitespace)
                && !self.partial.starts_with(char::is_whitespace)
            {
                spans.push(" ".into());
            }
            spans.push(Span::from(self.partial.clone()).dim());
        }

        let mut lines = word_wrap_lines([spans], width as usize);
        if lines.len() > MAX_VISIBLE_LINES {
            lines.drain(..lines.len() - MAX_VISIBLE_LINES);
        }
        lines
    }

    pub(crate) fn desired_height(&self, width: u16) -> u16 {
        self.lines(width).len().try_into().unwrap_or(u16::MAX)
    }

    pub(crate) fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        Paragraph::new(self.lines(area.width)).render(area, buf);
    }
}
