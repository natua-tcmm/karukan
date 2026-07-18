//! Cursor movement and character deletion

use super::*;

impl InputMethodEngine {
    /// Recover the longest trailing ASCII sequence before the cursor that is a
    /// valid unfinished romaji prefix (`s`, `sh`, etc.).
    ///
    /// Invalid romaji is passed through into `input_buf`. If later mistyped
    /// characters are deleted, a remaining valid prefix must move back into
    /// the converter buffer so the next vowel can complete it.
    fn restore_pending_romaji_before_cursor(&mut self) -> bool {
        self.converters.romaji.reset();
        if self.input_mode != InputMode::Hiragana || self.input_buf.cursor_pos == 0 {
            return false;
        }

        let chars: Vec<char> = self.input_buf.text.chars().collect();
        let cursor = self.input_buf.cursor_pos.min(chars.len());
        let ascii_start = chars[..cursor]
            .iter()
            .rposition(|ch| !ch.is_ascii_lowercase())
            .map_or(0, |index| index + 1);

        let mut restored = None;
        for start in ascii_start..cursor {
            let suffix: String = chars[start..cursor].iter().collect();
            let mut converter = RomajiConverter::new();
            for ch in suffix.chars() {
                converter.push(ch);
            }
            if converter.output().is_empty() && converter.buffer() == suffix {
                restored = Some((start, converter));
                break;
            }
        }

        let Some((start, converter)) = restored else {
            return false;
        };
        for _ in start..cursor {
            self.input_buf.remove_char_at(start);
        }
        self.input_buf.cursor_pos = start;
        self.converters.romaji = converter;
        true
    }

    /// Common helper for cursor movement: flush romaji, clear live conversion, set new position
    fn move_caret(&mut self, new_pos: usize) -> EngineResult {
        if !self.converters.romaji.buffer().is_empty() {
            self.flush_romaji_to_composed();
            self.converters.romaji.reset();
        }
        self.invalidate_live_results();
        self.live.clear();
        self.input_buf.cursor_pos = new_pos;
        self.log_chunk_state("cursor");
        let preedit = self.set_composing_state();
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Handle backspace in composing mode
    pub(super) fn backspace_composing(&mut self) -> EngineResult {
        // If romaji buffer is not empty, backspace from buffer (not from composed text)
        if !self.converters.romaji.buffer().is_empty() {
            self.converters.romaji.backspace();
            self.invalidate_live_results();
            self.live.clear();
            self.clear_composing_candidates();
            self.chunks.clear();
            if let Some(result) = self.try_reset_if_empty() {
                return result;
            }

            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        // Remove character before cursor from composed_hiragana
        if self.input_buf.cursor_pos > 0 {
            self.input_buf.remove_char_before_cursor();
        } else {
            // Nothing to delete
            return EngineResult::consumed();
        }

        let restored_romaji = self.restore_pending_romaji_before_cursor();
        if let Some(result) = self.try_reset_if_empty() {
            return result;
        }

        if restored_romaji {
            self.invalidate_live_results();
            self.live.clear();
            self.clear_composing_candidates();
            self.chunks.clear();
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        self.refresh_input_state()
    }

    /// Move caret left within hiragana input
    pub(super) fn move_caret_left(&mut self) -> EngineResult {
        let new_pos = self.input_buf.cursor_pos.saturating_sub(1);
        self.move_caret(new_pos)
    }

    /// Move caret right within hiragana input
    pub(super) fn move_caret_right(&mut self) -> EngineResult {
        let total = self.input_buf.text.chars().count();
        let new_pos = (self.input_buf.cursor_pos + 1).min(total);
        self.move_caret(new_pos)
    }

    /// Handle delete key in hiragana mode
    pub(super) fn delete_composing(&mut self) -> EngineResult {
        // If romaji buffer is not empty, don't delete from composed (buffer is at cursor)
        if !self.converters.romaji.buffer().is_empty() {
            return EngineResult::consumed();
        }

        // Delete character at cursor position
        if self.input_buf.remove_char_at_cursor().is_none() {
            return EngineResult::consumed();
        }

        if let Some(result) = self.try_reset_if_empty() {
            return result;
        }

        self.refresh_input_state()
    }

    /// Move caret to start of input
    pub(super) fn move_caret_home(&mut self) -> EngineResult {
        self.move_caret(0)
    }

    /// Move caret to end of input
    pub(super) fn move_caret_end(&mut self) -> EngineResult {
        let total = self.input_buf.text.chars().count();
        self.move_caret(total)
    }
}
