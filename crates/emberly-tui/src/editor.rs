//! A first-party, grapheme-aware line editor (Tech Spec §9). The input box
//! needs correct cursor motion and editing over Thai clusters (a base plus
//! stacked combining marks is one indivisible unit) and wide CJK glyphs, plus
//! multi-line entry and history — none of which a naive `String` + byte cursor
//! gets right. All motion and measurement go through [`crate::text`].
//!
//! The cursor is a byte offset into the buffer, always on a cluster boundary.
//! `\n` separates logical lines (entered with Shift+Enter); submitting returns
//! the whole buffer.

use crate::text;

/// An editable, possibly multi-line input buffer with history.
#[derive(Debug, Default)]
pub struct LineEditor {
    buf: String,
    /// Byte offset of the cursor; always on a grapheme-cluster boundary.
    cursor: usize,
    history: Vec<String>,
    /// Index into `history` while browsing it; `None` when editing live text.
    browsing: Option<usize>,
    /// The live buffer saved when history browsing began, restored on exit.
    stash: String,
}

impl LineEditor {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.buf
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The logical lines (split on `\n`) for rendering.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.buf.split('\n')
    }

    /// The number of logical lines (at least 1).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.buf.bytes().filter(|&b| b == b'\n').count() + 1
    }

    /// The cursor's position as (logical row, display column).
    #[must_use]
    pub fn cursor_row_col(&self) -> (usize, usize) {
        let before = &self.buf[..self.cursor];
        let row = before.bytes().filter(|&b| b == b'\n').count();
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = text::width(&self.buf[line_start..self.cursor]);
        (row, col)
    }

    // ---- editing ----------------------------------------------------------

    pub fn insert_char(&mut self, c: char) {
        self.leave_history();
        let mut b = [0u8; 4];
        self.buf.insert_str(self.cursor, c.encode_utf8(&mut b));
        self.cursor += c.len_utf8();
    }

    /// Insert a string (e.g. a bracketed paste) at the cursor.
    pub fn insert_str(&mut self, s: &str) {
        self.leave_history();
        self.buf.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    /// Insert a newline (Shift+Enter): begins a new logical line.
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// Delete the cluster before the cursor (Backspace).
    pub fn backspace(&mut self) {
        self.leave_history();
        if self.cursor == 0 {
            return;
        }
        let start = text::prev_boundary(&self.buf, self.cursor);
        self.buf.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Delete the cluster at the cursor (Delete).
    pub fn delete(&mut self) {
        self.leave_history();
        if self.cursor >= self.buf.len() {
            return;
        }
        let end = text::next_boundary(&self.buf, self.cursor);
        self.buf.replace_range(self.cursor..end, "");
    }

    /// Delete the word before the cursor (Ctrl+W).
    pub fn delete_word_back(&mut self) {
        self.leave_history();
        let target = self.word_left_target();
        self.buf.replace_range(target..self.cursor, "");
        self.cursor = target;
    }

    /// Delete from the cursor to the end of the current logical line (Ctrl+K).
    pub fn kill_to_end(&mut self) {
        self.leave_history();
        let end = self.line_end();
        self.buf.replace_range(self.cursor..end, "");
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = 0;
        self.leave_history();
    }

    // ---- motion -----------------------------------------------------------

    pub fn left(&mut self) {
        self.cursor = text::prev_boundary(&self.buf, self.cursor);
    }

    pub fn right(&mut self) {
        self.cursor = text::next_boundary(&self.buf, self.cursor);
    }

    /// Move to the start of the current logical line.
    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    /// Move to the end of the current logical line.
    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_left_target();
    }

    pub fn word_right(&mut self) {
        self.cursor = self.word_right_target();
    }

    /// Move up one logical line, keeping the display column. Returns `false`
    /// (without moving) when already on the first line — the caller may then
    /// step into history.
    pub fn up(&mut self) -> bool {
        let start = self.line_start();
        if start == 0 {
            return false;
        }
        let col = text::width(&self.buf[start..self.cursor]);
        let prev_end = start - 1; // the '\n'
        let prev_start = self.buf[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = &self.buf[prev_start..prev_end];
        self.cursor = prev_start + text::byte_at_col(line, col);
        true
    }

    /// Move down one logical line, keeping the display column. Returns `false`
    /// when already on the last line.
    pub fn down(&mut self) -> bool {
        let end = self.line_end();
        if end >= self.buf.len() {
            return false;
        }
        let start = self.line_start();
        let col = text::width(&self.buf[start..self.cursor]);
        let next_start = end + 1; // past the '\n'
        let next_end = self.buf[next_start..]
            .find('\n')
            .map(|i| next_start + i)
            .unwrap_or(self.buf.len());
        let line = &self.buf[next_start..next_end];
        self.cursor = next_start + text::byte_at_col(line, col);
        true
    }

    // ---- submit & history -------------------------------------------------

    /// Submit the buffer: returns its text (if non-empty) and resets to empty,
    /// pushing the entry onto history.
    pub fn submit(&mut self) -> Option<String> {
        self.leave_history();
        let text = self.buf.trim().to_string();
        self.buf.clear();
        self.cursor = 0;
        if text.is_empty() {
            return None;
        }
        if self.history.last().map(String::as_str) != Some(text.as_str()) {
            self.history.push(text.clone());
        }
        Some(text)
    }

    /// Recall the previous history entry (Up on the first line).
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.browsing {
            None => {
                self.stash = std::mem::take(&mut self.buf);
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.browsing = Some(idx);
        self.buf = self.history[idx].clone();
        self.cursor = self.buf.len();
    }

    /// Advance toward the live buffer (Down); restores the stashed draft past
    /// the newest entry.
    pub fn history_next(&mut self) {
        let Some(idx) = self.browsing else {
            return;
        };
        if idx + 1 < self.history.len() {
            self.browsing = Some(idx + 1);
            self.buf = self.history[idx + 1].clone();
        } else {
            self.browsing = None;
            self.buf = std::mem::take(&mut self.stash);
        }
        self.cursor = self.buf.len();
    }

    // ---- internals --------------------------------------------------------

    fn leave_history(&mut self) {
        // An edit adopts the recalled text as the live buffer.
        self.browsing = None;
    }

    fn line_start(&self) -> usize {
        self.buf[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn line_end(&self) -> usize {
        self.buf[self.cursor..]
            .find('\n')
            .map(|i| self.cursor + i)
            .unwrap_or(self.buf.len())
    }

    /// Target of a word-left motion: skip spaces, then a run of non-spaces.
    fn word_left_target(&self) -> usize {
        let mut pos = self.cursor;
        while pos > 0 {
            let prev = text::prev_boundary(&self.buf, pos);
            if text::is_space(&self.buf[prev..pos]) {
                pos = prev;
            } else {
                break;
            }
        }
        while pos > 0 {
            let prev = text::prev_boundary(&self.buf, pos);
            if text::is_space(&self.buf[prev..pos]) {
                break;
            }
            pos = prev;
        }
        pos
    }

    /// Target of a word-right motion: skip a run of non-spaces, then spaces.
    fn word_right_target(&self) -> usize {
        let len = self.buf.len();
        let mut pos = self.cursor;
        while pos < len {
            let next = text::next_boundary(&self.buf, pos);
            if text::is_space(&self.buf[pos..next]) {
                break;
            }
            pos = next;
        }
        while pos < len {
            let next = text::next_boundary(&self.buf, pos);
            if text::is_space(&self.buf[pos..next]) {
                pos = next;
            } else {
                break;
            }
        }
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THAI_STACKED: &str = "ที่"; // one cluster, three scalar values

    #[test]
    fn backspace_removes_a_whole_stacked_cluster() {
        let mut e = LineEditor::new();
        e.insert_str(THAI_STACKED);
        assert_eq!(e.text(), THAI_STACKED);
        e.backspace();
        assert_eq!(e.text(), "", "backspace deletes base + all combining marks");
    }

    #[test]
    fn motion_steps_over_clusters_not_bytes() {
        let mut e = LineEditor::new();
        e.insert_str(THAI_STACKED);
        // cursor at end; one left goes to start (whole cluster), not mid-cluster.
        e.left();
        assert_eq!(e.cursor_row_col(), (0, 0));
        e.right();
        assert_eq!(e.cursor_row_col(), (0, 1));
    }

    #[test]
    fn insert_and_edit_ascii() {
        let mut e = LineEditor::new();
        for c in "hello".chars() {
            e.insert_char(c);
        }
        e.home();
        assert_eq!(e.cursor_row_col(), (0, 0));
        e.end();
        assert_eq!(e.cursor_row_col(), (0, 5));
        e.backspace();
        assert_eq!(e.text(), "hell");
    }

    #[test]
    fn word_motion_and_delete() {
        let mut e = LineEditor::new();
        e.insert_str("foo bar baz");
        e.word_left(); // to start of "baz"
        assert_eq!(e.cursor_row_col(), (0, 8));
        e.delete_word_back(); // removes "bar " before "baz"
        assert_eq!(e.text(), "foo baz");
    }

    #[test]
    fn multiline_up_down_keeps_column() {
        let mut e = LineEditor::new();
        e.insert_str("abcd");
        e.newline();
        e.insert_str("ef");
        // cursor at end of line 1 (col 2)
        assert_eq!(e.cursor_row_col(), (1, 2));
        assert!(e.up());
        assert_eq!(e.cursor_row_col(), (0, 2));
        assert!(e.down());
        assert_eq!(e.cursor_row_col(), (1, 2));
        assert!(!e.down(), "no line below");
    }

    #[test]
    fn submit_returns_text_and_records_history() {
        let mut e = LineEditor::new();
        e.insert_str("  hi  ");
        assert_eq!(e.submit(), Some("hi".to_string()));
        assert!(e.is_empty());
        assert_eq!(e.submit(), None, "empty submit yields nothing");
        e.history_prev();
        assert_eq!(e.text(), "hi");
    }

    #[test]
    fn history_prev_next_restores_draft() {
        let mut e = LineEditor::new();
        e.insert_str("first");
        e.submit();
        e.insert_str("draft");
        e.history_prev();
        assert_eq!(e.text(), "first");
        e.history_next();
        assert_eq!(
            e.text(),
            "draft",
            "advancing past newest restores the draft"
        );
    }
}
