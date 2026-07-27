//! Composing input handling (Empty and Composing states)

use super::*;

pub(super) enum ComposingCommitForm {
    Hiragana,
    FullKatakana,
    HalfKatakana,
}

/// Append candidates to `target`, skipping duplicates by text.
fn append_candidates_dedup(target: &mut Vec<Candidate>, source: Vec<Candidate>) {
    for c in source {
        if !target.iter().any(|existing| existing.text == c.text) {
            target.push(c);
        }
    }
}

/// Move `text` to candidate 1 while preserving metadata from an existing
/// candidate. Insert `fallback` when no source emitted the text.
fn promote_composing_candidate(candidates: &mut Vec<Candidate>, text: &str, fallback: Candidate) {
    let candidate = candidates
        .iter()
        .position(|candidate| candidate.text == text)
        .map(|index| candidates.remove(index))
        .unwrap_or(fallback);
    candidates.insert(0, candidate);
}

/// Keep the typed hiragana itself at index 0 for a one-character reading.
///
/// If another source already emitted the same text, preserve that candidate's
/// metadata while moving it to the front. Otherwise add a plain reading
/// candidate without discarding any learned/model/dictionary alternatives.
fn prioritize_single_hiragana_candidate(
    input_mode: InputMode,
    reading: &str,
    candidates: &mut Vec<Candidate>,
) {
    if !should_prioritize_single_hiragana(input_mode, reading) {
        return;
    }

    promote_composing_candidate(
        candidates,
        reading,
        Candidate::with_reading(reading, reading),
    );
}

impl InputMethodEngine {
    fn live_dictionary_source_label(&self, reading: &str) -> Option<String> {
        let matches = self.live_user_dictionary_matches(reading);
        if matches.is_empty() {
            return None;
        }
        let reading_len = reading.chars().count();
        let source = if matches.len() == 1
            && matches[0].char_start == 0
            && matches[0].char_end == reading_len
        {
            CandidateSource::UserDictionary
        } else {
            CandidateSource::Hybrid
        };
        Some(source.label().to_string())
    }

    fn live_dictionary_candidate(
        text: String,
        reading: &str,
        source_label: Option<&str>,
    ) -> Candidate {
        Candidate {
            text,
            reading: Some(reading.to_string()),
            source_label: source_label.map(str::to_string),
            description: None,
        }
    }

    /// Refresh the dedicated emoji picker without invoking live conversion,
    /// learning, dictionaries, or the general rewriter chain.
    fn refresh_emoji_state(&mut self) -> EngineResult {
        self.live.clear();
        self.chunks.clear();

        let reading = self.input_buf.text.clone();
        let preedit = self.set_composing_state();
        let source_label = CandidateSource::Rewriter.label().to_string();
        let candidates: Vec<Candidate> = EmojiRewriter::new()
            .rewrite(&reading)
            .into_iter()
            .take(EMOJI_CANDIDATE_LIMIT)
            .map(|(text, description)| Candidate {
                text,
                reading: Some(reading.clone()),
                source_label: Some(source_label.clone()),
                description,
            })
            .collect();

        if candidates.is_empty() {
            self.clear_composing_candidates();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        let candidates = self.set_composing_candidates(CandidateList::new(candidates));
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates))
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Pick the visible live-conversion surface.
    ///
    /// A single hiragana remains raw by design. Otherwise user-dictionary
    /// spans are pinned and the remaining gaps use reusable/model text.
    fn preferred_live_surface(&self, reading: &str, model_surface: Option<&str>) -> Option<String> {
        if should_prioritize_single_hiragana(self.input_mode, reading) {
            return Some(reading.to_string());
        }
        self.preview_live_user_dictionary_candidates(reading, 1)
            .and_then(|candidates| candidates.into_iter().next())
            .or_else(|| model_surface.map(str::to_string))
    }

    /// Keep the visible live surface identical to candidate 1 after candidate
    /// filtering and ranking have finished. In particular, a kana-only mixed
    /// surface such as `シて` may have been chosen from the raw model beam and
    /// then removed by `finalize_whole_candidates`; leaving it in `live.text`
    /// would make Space reject the displayed list as stale and regenerate it.
    fn sync_live_surface_to_first_candidate(
        &mut self,
        reading: &str,
        applied_reading: &str,
        candidates: &[Candidate],
    ) {
        if self.live.text.is_empty() {
            return;
        }
        let Some(surface) = candidates.first().map(|candidate| candidate.text.as_str()) else {
            return;
        };
        if self.live.text == surface {
            return;
        }

        let (applied_reading, applied_surface) = reading
            .strip_prefix(applied_reading)
            .and_then(|suffix| surface.strip_suffix(suffix))
            .map(|surface_prefix| (applied_reading, surface_prefix))
            .unwrap_or((reading, surface));
        self.live
            .set_applied_prefix(applied_reading.to_string(), applied_surface.to_string());
        self.live.rebuild_for_reading(reading);
    }

    /// Refresh the input state: rebuild preedit and run auto-suggest for candidates.
    pub(super) fn refresh_input_state(&mut self) -> EngineResult {
        if self.input_mode == InputMode::Emoji {
            return self.refresh_emoji_state();
        }

        // Alphabet mode with active live conversion but no kana left to convert:
        // preserve the existing conversion display without re-running the model.
        // (When the buffer still contains kana we fall through and reconvert below,
        // so a mixed reading like `きょうはABC` keeps live-converting.)
        if self.input_mode == InputMode::Alphabet
            && !self.live.text.is_empty()
            && !karukan_engine::contains_kana(&self.input_buf.text)
        {
            let preedit = self.set_composing_state();
            return EngineResult::consumed().with_action(EngineAction::UpdatePreedit(preedit));
        }

        // Run auto-suggest via chunked conversion. Normally skipped in alphabet
        // mode (raw latin has no hiragana to convert), but if the buffer still
        // contains kana — e.g. the user typed hiragana, switched to alphabet mode,
        // and kept typing — keep converting the mixed reading so live conversion
        // stays alive. `chunked_auto_suggest` splits long input into
        // bounded-length chunks so per-keystroke latency stays flat; for input
        // within one chunk this is identical to a whole-buffer call.
        let convert = !self.input_buf.text.is_empty()
            && (self.input_mode != InputMode::Alphabet
                || karukan_engine::contains_kana(&self.input_buf.text));
        if convert && self.converters.kanji.is_some() {
            self.submit_live_inference();
            // Keep the last converted prefix visible and append newly typed
            // reading after it until the next snapshot result arrives.
            return self.refresh_without_model();
        }
        let candidates = if convert {
            let reading = self.input_buf.text.clone();
            self.chunked_auto_suggest()
                .map(|candidates| (candidates, reading))
        } else {
            self.chunks.clear();
            None
        };

        let Some((candidates, reading)) = candidates else {
            // No useful AI suggestion — still show learning + dictionary + rule-based
            // rewriter variants. The rewriter path produces mozc-style symbol variants
            // (e.g. `「` → `『`, `【`, ...) for symbol-only inputs where the model is skipped.
            let reading = self.input_buf.text.clone();
            if self.live.enabled && self.input_mode != InputMode::Katakana {
                self.live.text = self
                    .preferred_live_surface(&reading, None)
                    .unwrap_or_default();
            } else {
                self.live.clear();
            }
            let mut all_candidates = self.lookup_learning_candidates(&reading);
            append_candidates_dedup(&mut all_candidates, self.lookup_dict_candidates(&reading));
            append_candidates_dedup(&mut all_candidates, self.lookup_rewriter_variants(&reading));
            if !self.live.text.is_empty() {
                let live_text = self.live.text.clone();
                promote_composing_candidate(
                    &mut all_candidates,
                    &live_text,
                    Candidate::with_reading(&live_text, &reading),
                );
            }
            prioritize_single_hiragana_candidate(self.input_mode, &reading, &mut all_candidates);
            finalize_whole_candidates(
                !self.live.text.is_empty(),
                self.input_mode,
                &reading,
                &mut all_candidates,
            );
            if all_candidates.is_empty() {
                let preedit = self.set_composing_state();
                self.clear_composing_candidates();
                return EngineResult::consumed()
                    .with_action(EngineAction::UpdatePreedit(preedit))
                    .with_action(EngineAction::HideCandidates)
                    .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
            }
            self.sync_live_surface_to_first_candidate(&reading, &reading, &all_candidates);
            let preedit = self.set_composing_state();
            let candidate_list = self.set_composing_candidates(CandidateList::new(all_candidates));
            self.mark_composing_candidates_model_ready();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::ShowCandidates(candidate_list))
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        };

        // Live conversion mode: show converted text in preedit
        if self.live.enabled && self.input_mode != InputMode::Katakana {
            self.live.text = self
                .preferred_live_surface(&reading, candidates.first().map(String::as_str))
                .unwrap_or_default();

            // Same candidate ordering as normal auto-suggest (learning → model →
            // dictionary). Including the model candidates guarantees the list is
            // never empty, so the candidate window — whose aux line is where
            // frontends show the raw reading once the preedit displays converted
            // text — stays on screen for the whole live conversion.
            let mut all_candidates = self.lookup_learning_candidates(&reading);
            let live_dictionary_label = self.live_dictionary_source_label(&reading);
            let model_candidates: Vec<Candidate> = candidates
                .into_iter()
                .map(|text| {
                    Self::live_dictionary_candidate(
                        text,
                        &reading,
                        live_dictionary_label.as_deref(),
                    )
                })
                .collect();
            append_candidates_dedup(&mut all_candidates, model_candidates);
            append_candidates_dedup(&mut all_candidates, self.lookup_dict_candidates(&reading));
            let live_text = self.live.text.clone();
            promote_composing_candidate(
                &mut all_candidates,
                &live_text,
                Candidate::with_reading(&live_text, &reading),
            );
            prioritize_single_hiragana_candidate(self.input_mode, &reading, &mut all_candidates);
            finalize_whole_candidates(
                !self.live.text.is_empty(),
                self.input_mode,
                &reading,
                &mut all_candidates,
            );
            self.sync_live_surface_to_first_candidate(&reading, &reading, &all_candidates);
            let preedit = self.set_composing_state();
            let aux = self.format_aux_suggest(&self.input_buf.text.clone());
            let candidate_list = self.set_composing_candidates(CandidateList::new(all_candidates));
            self.mark_composing_candidates_model_ready();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::ShowCandidates(candidate_list))
                .with_action(EngineAction::UpdateAuxText(aux));
        }

        // Normal auto-suggest: show hiragana preedit + learning/model/dict candidates
        self.live.clear();
        let preedit = self.set_composing_state();
        // Learning candidates first (highest priority)
        let mut all_candidates = self.lookup_learning_candidates(&reading);
        // Then model inference candidates
        let model_candidates: Vec<Candidate> = candidates
            .into_iter()
            .map(|s| Candidate::with_reading(s, &reading))
            .collect();
        append_candidates_dedup(&mut all_candidates, model_candidates);
        // Then dictionary candidates
        append_candidates_dedup(&mut all_candidates, self.lookup_dict_candidates(&reading));
        prioritize_single_hiragana_candidate(self.input_mode, &reading, &mut all_candidates);
        finalize_whole_candidates(
            !self.live.text.is_empty(),
            self.input_mode,
            &reading,
            &mut all_candidates,
        );
        let aux = self.format_aux_suggest(&self.input_buf.text.clone());
        let candidate_list = self.set_composing_candidates(CandidateList::new(all_candidates));
        self.mark_composing_candidates_model_ready();
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidate_list))
            .with_action(EngineAction::UpdateAuxText(aux))
    }

    /// Rebuild composing UI without waiting for model inference.
    pub(super) fn refresh_without_model(&mut self) -> EngineResult {
        let reading = self.input_buf.text.clone();
        let live_active = self.live.enabled && self.input_mode != InputMode::Katakana;
        let preferred = live_active
            .then(|| self.preferred_live_surface(&reading, None))
            .flatten();
        if let Some(surface) = preferred {
            self.live.set_applied_prefix(reading.clone(), surface);
            self.live.rebuild_for_reading(&reading);
        } else if !live_active || !self.live.rebuild_for_reading(&reading) {
            self.live.clear();
        }
        let mut candidates = self.lookup_learning_candidates(&reading);
        let live_dictionary_label = self.live_dictionary_source_label(&reading);
        if let Some(preview) = self.preview_live_user_dictionary_candidates(
            &reading,
            live_candidate_pool_limit(
                self.live.enabled,
                self.input_mode,
                &reading,
                self.config.live_num_candidates,
            ),
        ) {
            append_candidates_dedup(
                &mut candidates,
                preview
                    .into_iter()
                    .map(|text| {
                        Self::live_dictionary_candidate(
                            text,
                            &reading,
                            live_dictionary_label.as_deref(),
                        )
                    })
                    .collect(),
            );
        }
        append_candidates_dedup(&mut candidates, self.lookup_dict_candidates(&reading));
        append_candidates_dedup(&mut candidates, self.lookup_rewriter_variants(&reading));
        if !self.live.text.is_empty() {
            let live_text = self.live.text.clone();
            promote_composing_candidate(
                &mut candidates,
                &live_text,
                Candidate::with_reading(&live_text, &reading),
            );
        }
        prioritize_single_hiragana_candidate(self.input_mode, &reading, &mut candidates);
        finalize_whole_candidates(
            !self.live.text.is_empty(),
            self.input_mode,
            &reading,
            &mut candidates,
        );
        if candidates.is_empty() {
            let preedit = self.set_composing_state();
            self.clear_composing_candidates();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }
        self.sync_live_surface_to_first_candidate(&reading, &reading, &candidates);
        let preedit = self.set_composing_state();
        let candidate_list = self.set_composing_candidates(CandidateList::new(candidates));
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidate_list))
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    fn submit_live_inference(&mut self) {
        self.live_revision = self.live_revision.wrapping_add(1);
        let num_candidates = live_candidate_pool_limit(
            self.live.enabled,
            self.input_mode,
            &self.input_buf.text,
            self.config.live_num_candidates,
        );
        let request = super::async_live::LiveInferenceRequest {
            revision: self.live_revision,
            reading: self.input_buf.text.clone(),
            cursor_pos: self.input_buf.cursor_pos,
            base_context: self.truncate_context_for_api(),
            max_context_len: self.config.max_api_context_len,
            chunk_len: self.config.composing_chunk_len,
            num_candidates,
            old_chunks: self.chunks.clone(),
            user_matches: self.live_user_dictionary_matches(&self.input_buf.text),
        };
        if let Some(worker) = self.converters.kanji.as_ref() {
            worker.submit_live(request);
        }
    }

    /// Apply a completed background inference to the reading prefix it covers.
    ///
    /// The user may have typed a suffix since the request was submitted. That
    /// suffix remains raw and is appended after the converted prefix.
    pub fn poll_live_conversion(&mut self) -> Option<EngineResult> {
        let result = self
            .converters
            .kanji
            .as_ref()
            .and_then(|worker| worker.poll_latest())?;
        if result.revision <= self.live_cancel_revision
            || !matches!(self.state, InputState::Composing { .. })
            || !self.input_buf.text.starts_with(&result.reading)
        {
            return None;
        }

        self.metrics.conversion_ms = result.conversion_ms;
        self.metrics.model_name = self.model_name();
        self.chunks = result.chunks;
        let top_surface: String = self
            .chunks
            .iter()
            .map(|chunk| chunk.converted.as_str())
            .collect();
        let candidates = result
            .candidates
            .unwrap_or_else(|| vec![top_surface.clone()]);
        Some(self.apply_background_candidates(
            result.reading,
            self.input_buf.text.clone(),
            candidates,
        ))
    }

    pub(super) fn apply_background_candidates(
        &mut self,
        result_reading: String,
        current_reading: String,
        prefix_candidates: Vec<String>,
    ) -> EngineResult {
        let suffix = current_reading
            .strip_prefix(&result_reading)
            .unwrap_or_default();
        if self.live.enabled && self.input_mode != InputMode::Katakana {
            if let Some(surface) = self.preferred_live_surface(&current_reading, None) {
                self.live
                    .set_applied_prefix(current_reading.clone(), surface);
            } else {
                let applied_prefix = self
                    .preferred_live_surface(
                        &result_reading,
                        prefix_candidates.first().map(String::as_str),
                    )
                    .unwrap_or_else(|| result_reading.clone());
                self.live
                    .set_applied_prefix(result_reading.clone(), applied_prefix);
            }
            self.live.rebuild_for_reading(&current_reading);
        } else {
            self.live.clear();
        }
        let mut all_candidates = self.lookup_learning_candidates(&current_reading);
        let live_dictionary_label = self.live_dictionary_source_label(&current_reading);
        if let Some(preview) = self.preview_live_user_dictionary_candidates(
            &current_reading,
            live_candidate_pool_limit(
                self.live.enabled,
                self.input_mode,
                &current_reading,
                self.config.live_num_candidates,
            ),
        ) {
            append_candidates_dedup(
                &mut all_candidates,
                preview
                    .into_iter()
                    .map(|text| {
                        Self::live_dictionary_candidate(
                            text,
                            &current_reading,
                            live_dictionary_label.as_deref(),
                        )
                    })
                    .collect(),
            );
        }
        let whole_candidates = prefix_candidates
            .into_iter()
            .map(|prefix| Candidate::with_reading(format!("{prefix}{suffix}"), &current_reading))
            .collect();
        append_candidates_dedup(&mut all_candidates, whole_candidates);
        append_candidates_dedup(
            &mut all_candidates,
            self.lookup_dict_candidates(&current_reading),
        );
        if !self.live.text.is_empty() {
            let live_text = self.live.text.clone();
            promote_composing_candidate(
                &mut all_candidates,
                &live_text,
                Candidate::with_reading(&live_text, &current_reading),
            );
        }
        prioritize_single_hiragana_candidate(
            self.input_mode,
            &current_reading,
            &mut all_candidates,
        );
        finalize_whole_candidates(
            !self.live.text.is_empty(),
            self.input_mode,
            &current_reading,
            &mut all_candidates,
        );
        self.sync_live_surface_to_first_candidate(
            &current_reading,
            &result_reading,
            &all_candidates,
        );
        let preedit = self.set_composing_state();
        let candidates = self.set_composing_candidates(CandidateList::new(all_candidates));
        if result_reading == current_reading {
            self.mark_composing_candidates_model_ready();
        }
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates))
            .with_action(EngineAction::UpdateAuxText(
                self.format_aux_suggest(&current_reading),
            ))
    }

    /// Process key in empty state
    pub(super) fn process_key_empty(&mut self, key: &KeyEvent, shift_active: bool) -> EngineResult {
        // Ctrl+Space: start input with full-width space
        if key.modifiers.control_key && key.keysym == Keysym::SPACE {
            self.converters.romaji.reset();
            self.input_buf.clear();
            self.input_buf.insert("\u{3000}");
            let preedit = self.set_composing_state();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(preedit))
                .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()));
        }

        // Bare Space from Empty state commits a half-width ASCII space in
        // Hiragana. Full-width space is reserved for Ctrl+Space.
        if key.keysym == Keysym::SPACE && !key.modifiers.control_key && !key.modifiers.alt_key {
            return if self.input_mode == InputMode::Hiragana {
                EngineResult::consumed().with_action(EngineAction::Commit(" ".to_string()))
            } else {
                EngineResult::not_consumed()
            };
        }

        // `:` from Empty state enters emoji shortcode mode — `:pien` stays
        // as `:pien` literally (no romaji conversion) while emoji candidates
        // are surfaced by the dedicated emoji picker. The mode auto-exits
        // after commit or after the query is erased, so the user's next word
        // lands in the previous input mode without an explicit toggle.
        //
        // Two keysym shapes can produce `:` depending on how the frontend
        // resolves the layout: (a) the XKB `colon` keysym (0x003A)
        // arriving directly, or (b) the `semicolon` keysym (0x003B)
        // with shift held. Accept both so we don't depend on which
        // shape the upstream stack happens to emit.
        let typed_colon =
            key.to_char() == Some(':') || (shift_active && key.keysym == Keysym(b';' as u32));
        if typed_colon
            && !key.modifiers.control_key
            && !key.modifiers.alt_key
            && self.input_mode != InputMode::Alphabet
        {
            return self.start_emoji_mode();
        }

        // Only handle printable characters without modifiers (except shift)
        if let Some(ch) = key.to_char()
            && !key.modifiers.control_key
            && !key.modifiers.alt_key
        {
            // Detect Shift+letter: shift modifier with alphabetic, OR uppercase keysym.
            // A frontend may resolve Shift into the keysym (sending 'A'
            // instead of 'a'+shift), so handle both cases.
            let is_shift_alpha =
                ch.is_ascii_uppercase() || (shift_active && ch.is_ascii_alphabetic());

            if is_shift_alpha && self.input_mode != InputMode::Alphabet {
                self.input_mode = InputMode::Alphabet;
            }
            let ch = if self.input_mode == InputMode::Alphabet && is_shift_alpha {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            return self.start_input(ch);
        }
        EngineResult::not_consumed()
    }

    /// Start input with a character (first character of a new input session).
    /// In alphabet mode, inserts directly; otherwise goes through romaji conversion.
    pub(super) fn start_input(&mut self, ch: char) -> EngineResult {
        self.converters.romaji.reset();
        self.input_buf.clear();

        if self.input_mode == InputMode::Alphabet {
            self.input_buf.insert(&ch.to_string());
        } else {
            let prev_output_len = 0;
            let _event = self.converters.romaji.push(ch);
            let romaji_buffer = self.converters.romaji.buffer().to_string();

            // PassThrough chars (no romaji rule, e.g. `'`, `;`, `<`, `(`) used to
            // auto-commit immediately, but that prevented users from composing
            // sequences like `「」` or getting symbol variants. Treat them like
            // digits — let them enter Composing and accumulate in the preedit.

            if self.converters.romaji.output().is_empty() && romaji_buffer.is_empty() {
                return EngineResult::not_consumed();
            }

            // Consume new converter output into composed_hiragana
            let new_output_len = self.converters.romaji.output().chars().count();
            if new_output_len > prev_output_len {
                let new_chars: String = self
                    .converters
                    .romaji
                    .output()
                    .chars()
                    .skip(prev_output_len)
                    .collect();
                self.input_buf.insert(&new_chars);
            }
        }

        let preedit = self.set_composing_state();
        if !self.input_buf.text.is_empty()
            && self.converters.kanji.is_some()
            && (self.input_mode != InputMode::Alphabet
                || karukan_engine::contains_kana(&self.input_buf.text))
        {
            self.submit_live_inference();
        }
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Insert a full-width space (U+3000) at cursor position
    pub(super) fn input_fullwidth_space(&mut self) -> EngineResult {
        self.input_buf.insert("\u{3000}");
        self.refresh_input_state()
    }

    /// Process key in hiragana input state
    pub(super) fn process_key_composing(
        &mut self,
        key: &KeyEvent,
        shift_active: bool,
    ) -> EngineResult {
        // Handle Ctrl+key shortcuts
        if key.modifiers.control_key {
            match key.keysym {
                // Ctrl+Space: insert full-width space (U+3000)
                Keysym::SPACE => return self.input_fullwidth_space(),
                // Ctrl+A: move to beginning (Emacs-style Home)
                Keysym::KEY_A | Keysym::KEY_A_UPPER => return self.move_caret_home(),
                // Ctrl+B: move left (Emacs-style Left)
                Keysym::KEY_B | Keysym::KEY_B_UPPER => return self.move_caret_left(),
                // Ctrl+E: move to end (Emacs-style End)
                Keysym::KEY_E | Keysym::KEY_E_UPPER => return self.move_caret_end(),
                // Ctrl+F: move right (Emacs-style Right)
                Keysym::KEY_F | Keysym::KEY_F_UPPER => return self.move_caret_right(),
                _ => {}
            }
        }

        match key.keysym {
            Keysym::RETURN => self.commit_composing(),
            Keysym::ESCAPE if self.live.enabled && self.input_mode == InputMode::Hiragana => {
                self.cancel_live_composing()
            }
            Keysym::ESCAPE => EngineResult::not_consumed(),
            Keysym::BACKSPACE => self.backspace_composing(),
            Keysym::DELETE => self.delete_composing(),
            Keysym::F6 => self.commit_composing_as(ComposingCommitForm::Hiragana),
            Keysym::F7 => self.commit_composing_as(ComposingCommitForm::FullKatakana),
            Keysym::F8 => self.commit_composing_as(ComposingCommitForm::HalfKatakana),
            Keysym::F9 | Keysym::F10 => EngineResult::consumed(),
            Keysym::SPACE if self.input_mode == InputMode::Alphabet => self.input_char(' '),
            Keysym::SPACE if self.input_mode == InputMode::Emoji => {
                self.select_next_composing_candidate()
            }
            Keysym::TAB | Keysym::DOWN => self.select_next_composing_candidate(),
            Keysym::UP => self.select_prev_composing_candidate(),
            Keysym::SPACE if self.composing_candidate_selected => {
                self.select_next_composing_candidate()
            }
            Keysym::SPACE => self.start_conversion(false),
            Keysym::LEFT => self.move_caret_left(),
            Keysym::RIGHT => self.move_caret_right(),
            Keysym::HOME => self.move_caret_home(),
            Keysym::END => self.move_caret_end(),
            _ => {
                if let Some(ch) = key.to_char()
                    && !key.modifiers.control_key
                    && !key.modifiers.alt_key
                {
                    // Detect Shift+letter: shift modifier with alphabetic, OR uppercase keysym.
                    // A frontend may resolve Shift into the keysym.
                    let is_shift_alpha =
                        ch.is_ascii_uppercase() || (shift_active && ch.is_ascii_alphabetic());

                    if is_shift_alpha && self.input_mode != InputMode::Alphabet {
                        // Bake katakana before switching so preedit doesn't revert
                        if self.input_mode == InputMode::Katakana {
                            self.bake_katakana();
                        }
                        self.input_mode = InputMode::Alphabet;
                        self.flush_romaji_to_composed();
                        self.invalidate_live_results();
                        self.live.clear();
                    }
                    let ch = if self.input_mode == InputMode::Alphabet && is_shift_alpha {
                        ch.to_ascii_uppercase()
                    } else {
                        ch
                    };
                    return self.input_char(ch);
                }
                EngineResult::not_consumed()
            }
        }
    }

    /// Leave live/candidate selection while keeping the raw reading as an
    /// ordinary unconfirmed hiragana preedit.
    fn cancel_live_composing(&mut self) -> EngineResult {
        self.invalidate_live_results();
        self.live.clear();
        self.clear_composing_candidates();
        self.chunks.clear();
        let preedit = self.set_composing_state();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Begin a new emoji-shortcode composing session.
    ///
    /// Resets any leftover state, switches `input_mode` to
    /// [`InputMode::Emoji`], seeds the buffer with `:`, and refreshes
    /// the candidate list so the user sees emoji suggestions appear
    /// the moment they press `:`.
    pub(super) fn start_emoji_mode(&mut self) -> EngineResult {
        self.invalidate_live_results();
        self.converters.romaji.reset();
        self.input_buf.clear();
        self.live.clear();
        // Remember where the user was so commit/cancel/erase-to-empty
        // can drop them back into the same mode (e.g. Katakana stays
        // Katakana). Guard against clobbering on re-entry just in case
        // start_emoji_mode is ever called while already in Emoji mode.
        if self.input_mode != InputMode::Emoji {
            self.pre_emoji_mode = Some(self.input_mode);
        }
        self.input_mode = InputMode::Emoji;
        self.input_buf.insert(":");
        self.refresh_input_state()
    }

    /// First emoji candidate the rewriter would surface for `reading`,
    /// or `None` if none match. Used by Enter in emoji mode so committing
    /// `:smile` produces 😄 directly rather than the literal `:smile`.
    pub(super) fn first_emoji_candidate(&self, reading: &str) -> Option<String> {
        self.converters
            .rewriters
            .rewrite_all(&[reading.to_string()])
            .into_iter()
            .map(|(text, _desc)| text)
            .next()
    }

    /// Input a character during composing.
    /// In alphabet mode, inserts directly; otherwise goes through romaji conversion.
    pub(super) fn input_char(&mut self, ch: char) -> EngineResult {
        if matches!(self.input_mode, InputMode::Alphabet | InputMode::Emoji) {
            self.input_buf.insert(&ch.to_string());
            return self.refresh_input_state();
        }

        let prev_output_len = self.converters.romaji.output().chars().count();
        let _event = self.converters.romaji.push(ch);
        let curr_output_len = self.converters.romaji.output().chars().count();

        // Consume ALL new converter output into composed_hiragana at cursor position.
        // The converter may recursively pass through multiple chars (e.g., "thx" →
        // output="th", buffer="x"), so capture all of them via delta detection.
        // PassThrough chars are already included in the converter output.
        if curr_output_len > prev_output_len {
            let new_chars: String = self
                .converters
                .romaji
                .output()
                .chars()
                .skip(prev_output_len)
                .collect();
            self.input_buf.insert(&new_chars);
        }

        // PassThrough chars no longer auto-commit. They accumulate in the preedit
        // alongside hiragana, allowing users to compose `「」`, type `'word'`,
        // and access symbol variants from the candidate list.

        if let Some(result) = self.try_reset_if_empty() {
            return result;
        }

        self.refresh_input_state()
    }

    /// Move through the auto-suggest candidates shown during Composing.
    ///
    /// The first Tab/Down only opts into the already-highlighted first
    /// candidate. Subsequent presses advance through the list. This preserves
    /// Enter's traditional behavior until the user explicitly starts selecting
    /// suggestions.
    fn select_next_composing_candidate(&mut self) -> EngineResult {
        let Some(mut candidates) = self.composing_candidates.clone() else {
            return EngineResult::not_consumed();
        };
        if candidates.is_empty() {
            return EngineResult::not_consumed();
        }
        self.invalidate_live_results();
        if self.composing_candidate_selected {
            if candidates.cursor() + 1 >= candidates.len() && self.input_mode != InputMode::Emoji {
                return self.start_segmented_conversion_from_composing();
            }
            candidates.move_next();
        } else {
            self.composing_candidate_selected = true;
        }
        self.update_composing_candidate_selection(candidates)
    }

    fn select_prev_composing_candidate(&mut self) -> EngineResult {
        let Some(mut candidates) = self.composing_candidates.clone() else {
            return EngineResult::not_consumed();
        };
        if candidates.is_empty() {
            return EngineResult::not_consumed();
        }
        self.invalidate_live_results();
        if self.composing_candidate_selected {
            candidates.move_prev();
        } else {
            self.composing_candidate_selected = true;
        }
        self.update_composing_candidate_selection(candidates)
    }

    pub(super) fn update_composing_candidate_selection(
        &mut self,
        candidates: CandidateList,
    ) -> EngineResult {
        let selected_text = candidates.selected_text().unwrap_or("").to_string();
        self.composing_candidates = Some(candidates.clone());

        let mut preedit = Preedit::with_text(&selected_text);
        preedit.set_attributes(vec![PreeditAttribute::new(
            0,
            selected_text.chars().count(),
            AttributeType::Highlight,
        )]);
        if let Some(p) = self.state.preedit_mut() {
            *p = preedit.clone();
        }

        let reading = candidates
            .selected()
            .and_then(|c| c.reading.as_deref())
            .unwrap_or(&self.input_buf.text)
            .to_string();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates))
            .with_action(EngineAction::UpdateAuxText(
                self.format_aux_conversion_with_page(&reading, self.composing_candidates.as_ref()),
            ))
    }

    /// Apply a clicked candidate while remaining in the dedicated emoji mode.
    pub(super) fn select_emoji_candidate_on_page(&mut self, page_index: usize) -> EngineResult {
        if self.input_mode != InputMode::Emoji
            || !matches!(self.state, InputState::Composing { .. })
        {
            return EngineResult::not_consumed();
        }
        let Some(mut candidates) = self.composing_candidates.clone() else {
            return EngineResult::not_consumed();
        };
        if candidates.select_on_page(page_index + 1).is_none() {
            return EngineResult::consumed();
        }
        self.composing_candidate_selected = true;
        self.update_composing_candidate_selection(candidates)
    }

    fn commit_selected_composing_candidate(&mut self) -> Option<EngineResult> {
        if !self.composing_candidate_selected {
            return None;
        }
        let candidates = self.composing_candidates.as_ref()?;
        let text = candidates.selected_text()?.to_string();
        self.record_selected_composing_correction();

        self.invalidate_live_results();
        self.converters.romaji.reset();
        self.input_buf.clear();
        self.live.clear();
        self.clear_composing_candidates();
        self.chunks.clear();
        self.state = InputState::Empty;
        self.exit_emoji_mode();
        if self.input_mode == InputMode::Alphabet {
            self.input_mode = InputMode::Hiragana;
        }

        Some(
            EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(Preedit::new()))
                .with_action(EngineAction::Commit(text))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::HideAuxText),
        )
    }

    pub(super) fn commit_composing_as(&mut self, form: ComposingCommitForm) -> EngineResult {
        self.invalidate_live_results();
        self.flush_romaji_to_composed();
        let reading = self.input_buf.text.clone();
        let text = match form {
            ComposingCommitForm::Hiragana => karukan_engine::katakana_to_hiragana(&reading),
            ComposingCommitForm::FullKatakana => karukan_engine::hiragana_to_katakana(&reading),
            ComposingCommitForm::HalfKatakana => {
                karukan_engine::hiragana_to_half_katakana(&reading)
            }
        };

        self.converters.romaji.reset();
        self.input_buf.clear();
        self.live.clear();
        self.clear_composing_candidates();
        self.chunks.clear();
        self.state = InputState::Empty;
        self.exit_emoji_mode();
        if self.input_mode == InputMode::Alphabet {
            self.input_mode = InputMode::Hiragana;
        }

        EngineResult::consumed()
            .with_action(EngineAction::Commit(text))
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::HideAuxText)
    }

    /// Commit the current hiragana input (or katakana if in katakana mode)
    /// In live conversion mode, commits the converted text instead of hiragana.
    pub(super) fn commit_composing(&mut self) -> EngineResult {
        if let Some(result) = self.commit_selected_composing_candidate() {
            return result;
        }

        self.invalidate_live_results();
        // Flush any pending romaji into composed_hiragana
        self.flush_romaji_to_composed();

        let reading = self.input_buf.text.clone();
        let text = if self.input_mode == InputMode::Emoji {
            // Emoji mode: Enter should select the first emoji candidate the
            // EmojiRewriter would surface, not commit the literal `:smile`.
            // Falls back to the literal buffer when nothing matches (e.g.
            // `:xyz`) so the user still sees what they typed.
            self.first_emoji_candidate(&reading)
                .unwrap_or_else(|| reading.clone())
        } else if self.input_mode == InputMode::Katakana {
            // Katakana mode always commits katakana, ignoring live conversion
            karukan_engine::hiragana_to_katakana(&reading)
        } else if !self.live.text.is_empty() {
            // Live conversion active: commit converted text
            self.live.text.clone()
        } else {
            reading.clone()
        };

        if text.is_empty() {
            self.state = InputState::Empty;
            self.input_buf.clear();
            self.live.clear();
            self.clear_composing_candidates();
            self.chunks.clear();
            return EngineResult::consumed()
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::HideAuxText);
        }

        self.converters.romaji.reset();
        self.input_buf.clear();
        self.live.clear();
        self.clear_composing_candidates();
        self.chunks.clear();
        self.state = InputState::Empty;
        self.exit_emoji_mode();
        if self.input_mode == InputMode::Alphabet {
            self.input_mode = InputMode::Hiragana;
        }

        // HideCandidates is required here: the auto-suggest/live-conversion
        // window may be open while Composing, and the macOS frontend's
        // NSPanel only closes on an explicit hide.
        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::Commit(text))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::HideAuxText)
    }
}
