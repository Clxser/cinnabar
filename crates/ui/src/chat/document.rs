//! Explicit multiline navigation for document fields sharing the bounded text editor.

use super::ChatEditor;

impl ChatEditor {
    /// Moves to the adjacent explicit line at the current Unicode character column.
    /// The caller selects this document-editing behavior; ordinary single-line chat remains unchanged.
    pub fn move_line(&mut self, direction: i8, selecting: bool) {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1);
        let end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |at| self.cursor + at);
        let column = self.text[start..self.cursor].chars().count();
        let (target_start, target_end) = if direction < 0 && start > 0 {
            let end = start - 1;
            (self.text[..end].rfind('\n').map_or(0, |at| at + 1), end)
        } else if direction > 0 && end < self.text.len() {
            let start = end + 1;
            (
                start,
                self.text[start..]
                    .find('\n')
                    .map_or(self.text.len(), |at| start + at),
            )
        } else {
            return;
        };
        let target = self.text[target_start..target_end]
            .char_indices()
            .nth(column)
            .map_or(target_end, |(at, _)| target_start + at);
        self.move_to(target, selecting);
    }
}
