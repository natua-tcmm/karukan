//! Display and preedit construction for the IME engine

use super::*;

impl InputMethodEngine {
    /// Build display text from the input buffer and romaji buffer
    /// Format: composed[:cursor] + romaji_buffer + composed[cursor:]
    /// In katakana mode, the composed parts are converted to katakana.
    pub(super) fn build_input_display(&self) -> String {
        let before: String = self
            .input_buf
            .text
            .chars()
            .take(self.input_buf.cursor_pos)
            .collect();
        let after: String = self
            .input_buf
            .text
            .chars()
            .skip(self.input_buf.cursor_pos)
            .collect();
        let buffer = self.converters.romaji.buffer();

        let katakana = self.input_mode == InputMode::Katakana;
        let display_before = if katakana {
            karukan_engine::hiragana_to_katakana(&before)
        } else {
            before
        };
        let display_after = if katakana {
            karukan_engine::hiragana_to_katakana(&after)
        } else {
            after
        };

        format!("{}{}{}", display_before, buffer, display_after)
    }

    /// Get the caret position in the display text (in characters)
    pub(super) fn display_caret_position(&self) -> usize {
        self.input_buf.cursor_pos + self.converters.romaji.buffer().chars().count()
    }

    /// Build a preedit for composing state.
    /// If live conversion text is present, shows live_text + romaji_buffer with caret at end.
    /// Otherwise shows the input buffer display with cursor-based caret.
    pub(super) fn build_composing_preedit(&self) -> Preedit {
        let (display, caret) = if !self.live.text.is_empty() {
            let buffer = self.converters.romaji.buffer();
            let display = format!("{}{}", self.live.text, buffer);
            let caret = display.chars().count();
            (display, caret)
        } else {
            (self.build_input_display(), self.display_caret_position())
        };
        let len = display.chars().count();
        let mut preedit = Preedit::with_text(&display);
        preedit.set_caret(caret);
        if self.live.enabled && matches!(self.input_mode, InputMode::Hiragana | InputMode::Alphabet)
        {
            let completed_len = if !self.live.text.is_empty()
                && !self.live.applied_reading.is_empty()
                && self.input_buf.text.starts_with(&self.live.applied_reading)
            {
                self.live.applied_text.chars().count().min(len)
            } else {
                0
            };
            let mut attributes = Vec::with_capacity(2);
            if completed_len > 0 {
                attributes.push(PreeditAttribute::underline(0, completed_len));
            }
            if completed_len < len {
                attributes.push(PreeditAttribute::new(
                    completed_len,
                    len,
                    AttributeType::UnderlineDotted,
                ));
            }
            preedit.set_attributes(attributes);
        } else {
            preedit.set_attributes(vec![PreeditAttribute::underline(0, len)]);
        }
        preedit
    }

    /// Get the current mode indicator string
    pub(super) fn mode_indicator(&self) -> String {
        match self.input_mode {
            InputMode::Alphabet => "[A]",
            InputMode::Katakana => "[カ]",
            InputMode::Hiragana => "",
            // ☺ (U+263A, Unicode 1.1 / 1993) — the oldest smiley-face
            // codepoint in Unicode; gives emoji mode an unambiguous
            // glyph in the aux text that's distinct from `[A]` so the
            // user sees they're not in plain alphabet input.
            InputMode::Emoji => "[☺]",
        }
        .to_string()
    }

    fn format_mode_and_reading(&self, indicator: &str, reading: &str) -> String {
        match (indicator.is_empty(), reading.is_empty()) {
            (true, true) => String::new(),
            (true, false) => reading.to_string(),
            (false, true) => indicator.to_string(),
            (false, false) => format!("{} {}", indicator, reading),
        }
    }

    /// Whether the current Hiragana reading, including any pending romaji,
    /// is fully covered by the most recently applied live-conversion result.
    fn live_conversion_finished(&self, reading: &str) -> bool {
        self.input_mode == InputMode::Hiragana
            && self.live.enabled
            && !reading.is_empty()
            && self.converters.romaji.buffer().is_empty()
            && self.live.applied_reading == reading
            && !self.live.applied_text.is_empty()
    }

    fn composing_indicator(&self, reading: &str) -> String {
        if self.live_conversion_finished(reading) {
            "\u{2705}\u{FE0F}".to_string()
        } else {
            self.mode_indicator()
        }
    }

    fn reading_with_romaji_buffer(&self, reading: &str) -> String {
        let romaji_buf = self.converters.romaji.buffer();
        if romaji_buf.is_empty() {
            reading.to_string()
        } else {
            format!("{}{}", reading, romaji_buf)
        }
    }

    /// Format aux text for composing input mode
    pub(super) fn format_aux_composing(&self) -> String {
        let reading = self.reading_with_romaji_buffer(&self.input_buf.text);
        let indicator = self.composing_indicator(&self.input_buf.text);
        self.format_mode_and_reading(&indicator, &reading)
    }

    /// Format aux text for conversion mode
    pub(super) fn format_aux_conversion_with_page(
        &self,
        reading: &str,
        _candidates: Option<&CandidateList>,
    ) -> String {
        self.format_mode_and_reading("[変換]", reading)
    }

    /// Format aux text for auto-suggest mode
    pub(super) fn format_aux_suggest(&self, reading: &str) -> String {
        let indicator = self.composing_indicator(reading);
        let display_reading = self.reading_with_romaji_buffer(reading);
        self.format_mode_and_reading(&indicator, &display_reading)
    }

    /// Truncate context to safe size for API calls
    pub(super) fn truncate_context_for_api(&self) -> String {
        match self
            .surrounding_context
            .as_ref()
            .and_then(|ctx| ctx.left.as_deref())
        {
            Some(left) => self.truncate_context(left),
            None => String::new(),
        }
    }

    /// Truncate a context string to safe size for API calls
    pub(super) fn truncate_context(&self, context: &str) -> String {
        let char_count = context.chars().count();
        if char_count > self.config.max_api_context_len {
            let start = char_count - self.config.max_api_context_len;
            context.chars().skip(start).collect()
        } else {
            context.to_string()
        }
    }
}
