//! Conversion state handling (candidates, commit). The live-conversion
//! chunking lives in the sibling `chunk` module.

use std::collections::HashSet;
use std::time::Instant;

use tracing::debug;

use super::*;
use crate::core::engine::long_conversion::{
    MAX_FINAL_CANDIDATES, MAX_SEARCH_STATES, MAX_SEGMENT_CANDIDATES, RankedText,
    combine_segment_options, split_conversion_reading,
};

/// Maximum number of learning candidates to show
const MAX_LEARNING_CANDIDATES: usize = 3;

/// Find one candidate per reading segment whose concatenation exactly matches
/// the whole-reading surface. A failed alignment must never be replaced by a
/// character-count heuristic: doing so can attach an unrelated surface such
/// as `客` to the reading `の`.
fn align_surface_to_candidates(
    surface: &str,
    candidate_lists: &[CandidateList],
) -> Option<Vec<usize>> {
    fn visit(
        surface: &str,
        candidate_lists: &[CandidateList],
        segment_index: usize,
        byte_offset: usize,
        failed: &mut HashSet<(usize, usize)>,
        path: &mut Vec<usize>,
    ) -> bool {
        if segment_index == candidate_lists.len() {
            return byte_offset == surface.len();
        }
        if !failed.insert((segment_index, byte_offset)) {
            return false;
        }

        let Some(remaining) = surface.get(byte_offset..) else {
            return false;
        };
        for (candidate_index, candidate) in candidate_lists[segment_index]
            .candidates()
            .iter()
            .enumerate()
        {
            if candidate.text.is_empty() || !remaining.starts_with(&candidate.text) {
                continue;
            }
            path.push(candidate_index);
            if visit(
                surface,
                candidate_lists,
                segment_index + 1,
                byte_offset + candidate.text.len(),
                failed,
                path,
            ) {
                return true;
            }
            path.pop();
        }
        false
    }

    let mut failed = HashSet::new();
    let mut path = Vec::with_capacity(candidate_lists.len());
    visit(surface, candidate_lists, 0, 0, &mut failed, &mut path).then_some(path)
}

/// Mozc-style width/script annotation for a pure-kana candidate, or `None`
/// if the text mixes scripts or contains kanji/punctuation. Used to label
/// `あ` / `ア` / `ｱ` candidates in the conversion list.
fn width_annotation(text: &str) -> Option<&'static str> {
    if karukan_engine::is_pure_hiragana(text) {
        Some("[全]ひらがな")
    } else if karukan_engine::is_pure_full_katakana(text) {
        Some("[全]カタカナ")
    } else {
        None
    }
}

/// Helper for building a deduplicated list of conversion candidates.
///
/// Two push paths exist: [`push`] dedups by text (skips duplicates), and
/// [`push_force`] always inserts (used for learning candidates that should
/// appear at the top even if a later source re-emits the same text).
struct CandidateBuilder {
    candidates: Vec<ConversionCandidate>,
    seen: HashSet<String>,
}

impl CandidateBuilder {
    fn new() -> Self {
        Self {
            candidates: Vec::new(),
            seen: HashSet::new(),
        }
    }

    /// Push a candidate if its text hasn't been seen yet.
    fn push(&mut self, candidate: ConversionCandidate) {
        if self.seen.insert(candidate.text.clone()) {
            self.candidates.push(candidate);
        }
    }

    /// Push a candidate unconditionally, marking its text as seen so later
    /// dedup'd inserts skip it. Use only for sources that should win over
    /// duplicates from later steps (e.g. learning cache).
    fn push_force(&mut self, candidate: ConversionCandidate) {
        self.seen.insert(candidate.text.clone());
        self.candidates.push(candidate);
    }

    fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Move `text` to the front while preserving its existing source metadata.
    /// Insert `fallback` when no source emitted the text.
    fn promote(&mut self, text: &str, fallback: ConversionCandidate) {
        let candidate = self
            .candidates
            .iter()
            .position(|candidate| candidate.text == text)
            .map(|index| self.candidates.remove(index))
            .unwrap_or_else(|| {
                self.seen.insert(text.to_string());
                fallback
            });
        self.candidates.insert(0, candidate);
    }

    fn into_candidates(mut self) -> Vec<ConversionCandidate> {
        let count = self.candidates.len();
        for (index, candidate) in self.candidates.iter_mut().enumerate() {
            candidate.rank_score = Some((count - index) as f32);
        }
        self.candidates
    }
}

impl InputMethodEngine {
    /// Run kana-kanji conversion for a reading via llama.cpp model.
    ///
    /// Uses the configured model for both live and explicit conversion, measures
    /// latency, and records which model was used.
    ///
    /// Skips the model entirely when the reading has no hiragana/katakana — the
    /// model is trained on kana → kanji and hallucinates garbage (e.g. `「` → `w`)
    /// for symbol- or alphabet-only inputs. Rule-based variants from
    /// `SymbolRewriter` cover those cases instead.
    ///
    /// `api_context` is the left context (lctx) fed to the model. Callers pass
    /// `truncate_context_for_api()` for a whole-buffer conversion, or — for
    /// chunked live conversion — the converted text of the preceding chunks.
    pub(super) fn run_kana_kanji_conversion(
        &mut self,
        reading: &str,
        api_context: &str,
        num_candidates: usize,
    ) -> Vec<karukan_engine::ModelCandidate> {
        if !karukan_engine::contains_kana(reading) {
            return vec![];
        }
        let Some(converter) = self.converters.kanji.as_ref() else {
            return vec![];
        };
        let katakana = karukan_engine::hiragana_to_katakana(reading);
        let model_name = converter.model_display_name().to_string();
        let candidate_count = num_candidates.max(1);
        debug!(
            "convert: reading=\"{}\" api_context=\"{}\" candidates={}",
            reading, api_context, candidate_count
        );

        let start = Instant::now();
        let candidates = converter
            .convert_scored(&katakana, api_context, candidate_count)
            .unwrap_or_default();

        self.metrics.conversion_ms += start.elapsed().as_millis() as u64;
        self.metrics.model_name = model_name;

        candidates
    }

    /// Start kanji conversion for the current input buffer.
    ///
    /// Called when Space is pressed: flushes any pending romaji,
    /// resolves the reading, runs `build_conversion_candidates`, and
    /// transitions into the Conversion state. The previous live-conversion
    /// result is preserved as the first model candidate so the user sees
    /// the same text they had been looking at during input.
    ///
    /// `skip_learning` remains available for internal/tests that need to inspect
    /// the learning-free branch; the normal key path keeps learning included.
    pub(super) fn start_conversion(&mut self, skip_learning: bool) -> EngineResult {
        self.clear_composing_candidates();
        // Flush any remaining romaji into composed_hiragana
        self.flush_romaji_to_composed();

        let reading = self.input_buf.text.clone();

        // Save auto-suggest/live conversion result before clearing state.
        // This ensures the candidate that was displayed during input is preserved
        // in the conversion candidate list even if re-inference produces another result.
        let prev_suggest_text = std::mem::take(&mut self.live.text);

        self.converters.romaji.reset();
        self.input_buf.cursor_pos = 0;

        if reading.is_empty() {
            return EngineResult::consumed();
        }

        // Whole-reading conversion intentionally exposes only three choices.
        // Rejecting all three transitions to segmented correction instead of
        // walking an unbounded whole-sentence beam.
        let mut candidates =
            self.build_conversion_candidates(&reading, WHOLE_CANDIDATE_LIMIT, skip_learning);

        // Candidate 1 must be exactly what live conversion was displaying.
        // Preserve source metadata when fresh inference emitted the same text.
        if !prev_suggest_text.is_empty() {
            let candidate = candidates
                .iter()
                .position(|candidate| candidate.text == prev_suggest_text)
                .map(|index| candidates.remove(index))
                .unwrap_or_else(|| {
                    ConversionCandidate::new(&prev_suggest_text, CandidateSource::Model)
                });
            candidates.insert(0, candidate);
        }
        candidates.truncate(WHOLE_CANDIDATE_LIMIT);

        if candidates.is_empty() {
            // No candidates, stay in hiragana mode
            let preedit = Preedit::with_text_underlined(&reading);
            self.state = InputState::Composing {
                preedit: preedit.clone(),
                romaji_buffer: String::new(),
            };
            return EngineResult::consumed().with_action(EngineAction::UpdatePreedit(preedit));
        }

        // Map ConversionCandidate → public Candidate. The two annotation
        // slots are kept disjoint so descriptions never duplicate between the
        // aux text and the candidate's right-side comment:
        //   - `source_label` ← source.label() only (e.g. `🤖 AI`, `📚 辞書`)
        //   - `description`  ← the per-candidate description only
        //                      (e.g. `三点リーダ`, `[全]英大文字`)
        let candidate_list = CandidateList::new(
            candidates
                .into_iter()
                .map(|candidate| candidate.into_ui_candidate(&reading))
                .collect(),
        );
        let session =
            crate::core::state::ConversionSession::single(reading.clone(), candidate_list);
        self.enter_conversion_state(session)
    }

    /// Exhaust the composing-time whole-reading candidates and enter segmented
    /// correction directly.
    pub(super) fn start_segmented_conversion_from_composing(&mut self) -> EngineResult {
        self.flush_romaji_to_composed();
        let reading = self.input_buf.text.clone();
        if reading.is_empty() {
            return EngineResult::consumed();
        }
        let fallback_candidates = self
            .composing_candidates
            .clone()
            .unwrap_or_else(|| CandidateList::from_strings_with_reading([&reading], &reading));

        self.converters.romaji.reset();
        self.live.text.clear();
        self.clear_composing_candidates();
        self.input_buf.cursor_pos = 0;

        let mut session = self.build_conversion_session(&reading, fallback_candidates, false);
        session.finish_whole_candidate_phase();
        self.enter_conversion_state(session)
    }

    #[cfg(test)]
    pub(super) fn build_initial_conversion_session(
        &mut self,
        reading: &str,
        full_candidates: CandidateList,
    ) -> crate::core::state::ConversionSession {
        self.build_conversion_session(reading, full_candidates, true)
    }

    fn build_conversion_session(
        &mut self,
        reading: &str,
        full_candidates: CandidateList,
        preserve_whole_surface: bool,
    ) -> crate::core::state::ConversionSession {
        #[derive(Debug)]
        struct InitialSegment {
            start: usize,
            end: usize,
            reading: String,
            surface: String,
            from_dictionary: bool,
        }

        let preserved_candidate = preserve_whole_surface
            .then(|| full_candidates.selected().cloned())
            .flatten();
        let Some(path) = self.dictionary_lattice_paths(reading, 1).into_iter().next() else {
            return self.single_segment_session_with_learning(reading, full_candidates);
        };
        if !path.segments.iter().any(|segment| segment.source.is_some()) {
            return self.single_segment_session_with_learning(reading, full_candidates);
        }

        // Unknown fallback edges are one character wide in the lattice. Merge
        // adjacent unknowns so an uncovered phrase remains one usable segment.
        let mut initial = Vec::<InitialSegment>::new();
        for segment in path.segments {
            if segment.source.is_none()
                && let Some(previous) = initial.last_mut()
                && !previous.from_dictionary
                && previous.end == segment.char_start
            {
                previous.end = segment.char_end;
                previous.reading.push_str(&segment.reading);
                previous.surface.push_str(&segment.surface);
                continue;
            }
            initial.push(InitialSegment {
                start: segment.char_start,
                end: segment.char_end,
                reading: segment.reading,
                surface: segment.surface,
                from_dictionary: segment.source.is_some(),
            });
        }
        if initial.len() <= 1 {
            return self.single_segment_session_with_learning(reading, full_candidates);
        }

        let mut prepared_candidates = Vec::with_capacity(initial.len());
        let mut has_segment_learning = Vec::with_capacity(initial.len());
        for (index, initial_segment) in initial.iter().enumerate() {
            let left_hint = index
                .checked_sub(1)
                .and_then(|previous| initial[previous].surface.chars().last())
                .map(|ch| ch.to_string())
                .or_else(|| self.editor_left_hint());
            let right_hint = initial
                .get(index + 1)
                .and_then(|next| next.surface.chars().next())
                .map(|ch| ch.to_string())
                .or_else(|| self.editor_right_hint());
            let base_candidates = CandidateList::new(
                self.build_conversion_candidates(
                    &initial_segment.reading,
                    MAX_SEGMENT_CANDIDATES,
                    false,
                )
                .into_iter()
                .map(|candidate| candidate.into_ui_candidate(&initial_segment.reading))
                .collect(),
            );
            let (candidates, has_learning) = self.prepend_segment_learning(
                &initial_segment.reading,
                base_candidates,
                left_hint.as_deref(),
                right_hint.as_deref(),
            );

            let candidates = if candidates
                .candidates()
                .iter()
                .any(|candidate| candidate.text == initial_segment.surface)
            {
                candidates
            } else {
                let mut values = candidates.candidates().to_vec();
                values.push(Candidate {
                    text: initial_segment.surface.clone(),
                    reading: Some(initial_segment.reading.clone()),
                    source_label: initial_segment
                        .from_dictionary
                        .then(|| CandidateSource::Dictionary.label().to_string()),
                    description: None,
                });
                CandidateList::new(values)
            };
            prepared_candidates.push(candidates);
            has_segment_learning.push(has_learning);
        }

        let aligned_indices = preserved_candidate.as_ref().and_then(|candidate| {
            align_surface_to_candidates(&candidate.text, &prepared_candidates)
        });
        if preserved_candidate.is_some() && aligned_indices.is_none() {
            return self.single_segment_session_with_learning(reading, full_candidates);
        }

        let mut segments = Vec::with_capacity(initial.len());
        for (index, (initial_segment, mut candidates)) in
            initial.iter().zip(prepared_candidates).enumerate()
        {
            if let Some(candidate_index) = aligned_indices
                .as_ref()
                .and_then(|indices| indices.get(index))
                .copied()
            {
                candidates.select(candidate_index);
            } else if !has_segment_learning[index]
                && let Some(candidate_index) = candidates
                    .candidates()
                    .iter()
                    .position(|candidate| candidate.text == initial_segment.surface)
            {
                candidates.select(candidate_index);
            }
            segments.push(crate::core::state::ConversionSegment::new(
                initial_segment.start..initial_segment.end,
                initial_segment.reading.clone(),
                candidates,
            ));
        }
        crate::core::state::ConversionSession::segmented(reading.to_string(), segments)
    }

    fn single_segment_session_with_learning(
        &self,
        reading: &str,
        candidates: CandidateList,
    ) -> crate::core::state::ConversionSession {
        let left_hint = self.editor_left_hint();
        let right_hint = self.editor_right_hint();
        let (candidates, _) = self.prepend_segment_learning(
            reading,
            candidates,
            left_hint.as_deref(),
            right_hint.as_deref(),
        );
        crate::core::state::ConversionSession::single(reading.to_string(), candidates)
    }

    fn editor_left_hint(&self) -> Option<String> {
        self.surrounding_context
            .as_ref()
            .and_then(|context| context.left.as_deref())
            .and_then(|left| left.chars().last())
            .map(|ch| ch.to_string())
    }

    fn editor_right_hint(&self) -> Option<String> {
        self.surrounding_context
            .as_ref()
            .and_then(|context| context.right.as_deref())
            .and_then(|right| right.chars().next())
            .map(|ch| ch.to_string())
    }

    fn prepend_segment_learning(
        &self,
        reading: &str,
        candidates: CandidateList,
        left_hint: Option<&str>,
        right_hint: Option<&str>,
    ) -> (CandidateList, bool) {
        let Some(cache) = &self.segment_learning else {
            return (candidates, false);
        };
        let learned = cache.lookup(reading, left_hint, right_hint);
        if learned.is_empty() {
            return (candidates, false);
        }

        let mut seen = HashSet::new();
        let mut values = Vec::new();
        for (entry, _) in learned.into_iter().take(MAX_LEARNING_CANDIDATES) {
            if seen.insert(entry.surface.clone()) {
                values.push(Candidate {
                    text: entry.surface,
                    reading: Some(reading.to_string()),
                    source_label: Some(CandidateSource::Learning.label().to_string()),
                    description: Some("文節修正".to_string()),
                });
            }
        }
        let has_segment_learning = !values.is_empty();
        values.extend(
            candidates
                .candidates()
                .iter()
                .filter(|candidate| seen.insert(candidate.text.clone()))
                .cloned(),
        );
        (CandidateList::new(values), has_segment_learning)
    }

    /// Transition to Conversion state with the given session.
    ///
    /// Sets up the preedit (highlighted selected text), updates the state, and
    /// returns an EngineResult with preedit, candidates, and aux text actions.
    fn enter_conversion_state(
        &mut self,
        session: crate::core::state::ConversionSession,
    ) -> EngineResult {
        let reading = session.reading.clone();
        let preedit = session.preedit().clone();
        let candidates = session.candidates().cloned().unwrap_or_default();
        self.state = InputState::Conversion { session };

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates.clone()))
            .with_action(EngineAction::UpdateAuxText(
                self.format_aux_conversion_with_page(&reading, Some(&candidates)),
            ))
    }

    /// Search user and system dictionaries for candidates matching a reading.
    ///
    /// User dictionary results come first (higher priority), then system dictionary
    /// results sorted by score. Duplicates are removed via HashSet.
    fn search_dictionaries(&self, reading: &str, limit: usize) -> Vec<ConversionCandidate> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();

        // User dictionary (higher priority)
        if let Some(dict) = &self.dicts.user
            && let Some(result) = dict.exact_match_search(reading)
        {
            for cand in result.candidates {
                if candidates.len() >= limit {
                    break;
                }
                if seen.insert(cand.surface.clone()) {
                    candidates.push(
                        ConversionCandidate::new(
                            cand.surface.clone(),
                            CandidateSource::UserDictionary,
                        )
                        .with_raw_score(Some(cand.score))
                        .with_description(cand.description.clone()),
                    );
                }
            }
        }

        // System dictionary (sorted by score)
        if let Some(dict) = &self.dicts.system
            && let Some(result) = dict.exact_match_search(reading)
        {
            let mut dict_candidates: Vec<_> = result.candidates.to_vec();
            dict_candidates.sort_by(|a, b| a.score.total_cmp(&b.score));
            for cand in dict_candidates {
                if candidates.len() >= limit {
                    break;
                }
                if seen.insert(cand.surface.clone()) {
                    candidates.push(
                        ConversionCandidate::new(cand.surface, CandidateSource::Dictionary)
                            .with_raw_score(Some(cand.score))
                            .with_description(cand.description),
                    );
                }
            }
        }

        candidates
    }

    fn dictionary_lattice_paths(
        &self,
        reading: &str,
        max_paths: usize,
    ) -> Vec<karukan_engine::LatticePath> {
        let mut dictionaries = Vec::new();
        if let Some(dictionary) = self.dicts.user.as_ref() {
            dictionaries.push(karukan_engine::LatticeDictionary {
                dictionary,
                kind: karukan_engine::LatticeDictionaryKind::User,
                score_bias: -100.0,
            });
        }
        if let Some(dictionary) = self.dicts.system.as_ref() {
            dictionaries.push(karukan_engine::LatticeDictionary {
                dictionary,
                kind: karukan_engine::LatticeDictionaryKind::System,
                score_bias: 0.0,
            });
        }
        karukan_engine::search_dictionary_lattice(
            reading,
            &dictionaries,
            karukan_engine::LatticeLimits {
                segment_candidates: MAX_SEGMENT_CANDIDATES,
                beam_width: MAX_SEARCH_STATES,
                max_paths,
                ..karukan_engine::LatticeLimits::default()
            },
        )
    }

    /// Convert complete dictionary-lattice paths into regular IME candidates.
    pub(super) fn dictionary_lattice_candidates(
        &self,
        reading: &str,
        limit: usize,
    ) -> Vec<ConversionCandidate> {
        self.dictionary_lattice_paths(reading, limit)
            .into_iter()
            // A fully unknown path is the hiragana fallback, not a dictionary result.
            .filter(|path| path.segments.iter().any(|segment| segment.source.is_some()))
            .map(|path| {
                let mut descriptions = Vec::new();
                for description in path
                    .segments
                    .iter()
                    .filter_map(|segment| segment.description.as_deref())
                {
                    if !descriptions.contains(&description) {
                        descriptions.push(description);
                    }
                }
                ConversionCandidate::new(path.surface, CandidateSource::Dictionary)
                    .with_raw_score(Some(path.score))
                    .with_description((!descriptions.is_empty()).then(|| descriptions.join(" / ")))
            })
            .collect()
    }

    /// Generate long-input candidates whose individual spans may independently
    /// come from the model or the dictionary lattice.
    fn build_hybrid_candidates(
        &mut self,
        reading: &str,
        api_context: &str,
    ) -> Vec<ConversionCandidate> {
        if self.input_mode == InputMode::Emoji || !karukan_engine::contains_kana(reading) {
            return Vec::new();
        }
        let lattice = self.dictionary_lattice_paths(reading, 1);
        let boundaries = lattice
            .first()
            .map(|path| {
                path.segments
                    .iter()
                    .map(|segment| segment.char_end)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let spans = split_conversion_reading(reading, self.config.composing_chunk_len, &boundaries);
        if spans.len() <= 1 {
            return Vec::new();
        }

        let mut segment_options = Vec::new();
        let mut context = api_context.to_string();
        for span in spans {
            if span.passthrough {
                segment_options.push(vec![RankedText {
                    text: span.text.clone(),
                    cost: 0.0,
                }]);
                context.push_str(&span.text);
                continue;
            }

            // Materialize dictionary paths before inference so no immutable
            // dictionary borrow overlaps the mutable model call.
            let dictionary = self.dictionary_lattice_candidates(&span.text, MAX_SEGMENT_CANDIDATES);
            let model =
                self.run_kana_kanji_conversion(&span.text, &context, MAX_SEGMENT_CANDIDATES);
            let mut options = Vec::new();
            for (index, candidate) in model.into_iter().enumerate() {
                options.push(RankedText {
                    text: candidate.text,
                    cost: index as f32 * 2.0,
                });
            }
            for (index, candidate) in dictionary.into_iter().enumerate() {
                options.push(RankedText {
                    text: candidate.text,
                    cost: index as f32 * 2.0 + 0.75,
                });
            }
            options.push(RankedText {
                text: span.text.clone(),
                cost: 8.0,
            });
            let katakana = karukan_engine::hiragana_to_katakana(&span.text);
            if katakana != span.text {
                options.push(RankedText {
                    text: katakana,
                    cost: 8.5,
                });
            }
            options.sort_by(|left, right| left.cost.total_cmp(&right.cost));
            let mut seen = HashSet::new();
            options.retain(|candidate| seen.insert(candidate.text.clone()));
            options.truncate(MAX_SEGMENT_CANDIDATES);
            context.push_str(&options[0].text);
            segment_options.push(options);
        }

        combine_segment_options(&segment_options, MAX_SEARCH_STATES, MAX_FINAL_CANDIDATES)
            .into_iter()
            .map(|candidate| {
                ConversionCandidate::new(candidate.text, CandidateSource::Hybrid)
                    .with_raw_score(Some(-candidate.cost))
            })
            .collect()
    }

    /// Build conversion candidates for a reading from multiple sources.
    ///
    /// Combines learning cache, dictionaries, and model inference results
    /// with deduplication. Uses dynamic candidate count based on input token
    /// count for performance.
    ///
    /// Priority: Learning → User Dictionary → Model → System Dictionary → Fallback
    ///
    /// `skip_learning` suppresses the learning-cache step (1). Normal key input
    /// keeps this false; tests can still exercise the learning-free branch.
    pub(super) fn build_conversion_candidates(
        &mut self,
        reading: &str,
        num_candidates: usize,
        skip_learning: bool,
    ) -> Vec<ConversionCandidate> {
        // Try to initialize the kanji converter, but don't bail out if it
        // fails — symbol-only inputs (e.g. `。。。`) don't need the model and
        // we still want to produce dictionary, rewriter, and fallback candidates.
        // run_kana_kanji_conversion handles the converter-missing case.
        #[cfg(not(test))]
        {
            if self.converters.kanji.is_none()
                && let Err(e) = self.init_kanji_converter()
            {
                debug!("Failed to initialize kanji converter: {}", e);
            }
        }

        let api_context = self.truncate_context_for_api();
        let model_candidates =
            self.run_kana_kanji_conversion(reading, &api_context, num_candidates);
        let lattice_candidates = self.dictionary_lattice_candidates(reading, MAX_FINAL_CANDIDATES);
        let hybrid_candidates = self.build_hybrid_candidates(reading, &api_context);

        let hiragana = reading.to_string();
        let katakana = karukan_engine::hiragana_to_katakana(reading);

        // Priority: Learning → User Dictionary → Model → System Dictionary → Fallback
        let mut builder = CandidateBuilder::new();

        // 1. Learning cache candidates (highest priority).
        //    Force-inserted so they win against duplicate text from later sources.
        //    Skipped when the caller asks for a learning-free conversion (Tab key).
        if !skip_learning {
            for c in self.lookup_learning_candidates(reading) {
                // Exact matches have reading == input reading; use None to avoid redundancy
                let cand_reading = c.reading.filter(|r| r != reading);
                builder.push_force(
                    ConversionCandidate::new(c.text, CandidateSource::Learning)
                        .with_reading(cand_reading),
                );
            }
        }

        // 2. Dictionary candidates (user dict first, then system dict)
        let dict_results = self.search_dictionaries(reading, usize::MAX);
        // Insert user dictionary entries at the top (after learning)
        for ac in &dict_results {
            if ac.source == CandidateSource::UserDictionary {
                builder.push(ac.clone());
            }
        }

        // 3. Bounded rank fusion of the full Main-model beam, dictionary
        //    lattice K-best, and long-input hybrid candidates.
        let mut generated = Vec::new();
        for (index, candidate) in model_candidates.into_iter().enumerate() {
            generated.push((
                index as f32 * 3.0,
                ConversionCandidate::new(candidate.text, CandidateSource::Model)
                    .with_raw_score(candidate.score),
            ));
        }
        for (index, candidate) in lattice_candidates.into_iter().enumerate() {
            generated.push((index as f32 * 3.0 + 1.0, candidate));
        }
        for (index, candidate) in hybrid_candidates.into_iter().enumerate() {
            generated.push((index as f32 * 3.0 + 2.0, candidate));
        }
        generated.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut generated_seen = HashSet::new();
        generated.retain(|(_, candidate)| generated_seen.insert(candidate.text.clone()));
        generated.truncate(num_candidates.min(MAX_FINAL_CANDIDATES));

        if generated.is_empty() {
            // In emoji mode, defer the literal-fallback decision until
            // after rewriters have run — otherwise `:smile` would be
            // pinned to the top of the candidate list as a Fallback
            // and outrank the 😄 we surface in step 5/6.
            if builder.is_empty() && self.input_mode != InputMode::Emoji {
                builder.push(ConversionCandidate::new(
                    hiragana.clone(),
                    CandidateSource::Fallback,
                ));
            }
        } else {
            for (_, candidate) in generated {
                builder.push(candidate);
            }
        }

        // 4. System dictionary candidates (from search_dictionaries result)
        for ac in dict_results {
            if ac.source == CandidateSource::Dictionary {
                builder.push(ac);
            }
        }

        // 5/6. Hiragana/katakana fallback + rewriter variants.
        //
        // In emoji mode we surface ONLY the rewriter (i.e. EmojiRewriter)
        // candidates — Slack's emoji picker shows emojis and nothing
        // else, and that's the mental model the user wants here.
        // No literal `:smile` / `:xyz` fallback in the candidate list:
        // if nothing matches, the picker is just empty. (Enter on a
        // no-match query in Composing still commits the buffer
        // literal via `commit_composing`; that's the escape hatch.)
        // Non-emoji modes keep the original order so existing IME
        // behavior is untouched.
        let rewriter_variants = self
            .converters
            .rewriters
            .rewrite_all(&[reading.to_string()]);
        if self.input_mode == InputMode::Emoji {
            for (variant, description) in rewriter_variants {
                builder.push(
                    ConversionCandidate::new(variant, CandidateSource::Rewriter)
                        .with_description(description),
                );
            }
        } else {
            builder.push(ConversionCandidate::new(
                hiragana,
                CandidateSource::Fallback,
            ));
            builder.push(ConversionCandidate::new(
                katakana,
                CandidateSource::Fallback,
            ));
            // Rewriters operate on the user's typed input (the reading
            // itself). Running them on dictionary/model/fallback
            // candidates produces unrelated noise (e.g. a dictionary
            // entry of `,` for some reading would generate `、`/`，`
            // variants the user never asked for; a learning entry `アト`
            // pulled by prefix lookup on `あ` would emit `ｱﾄ`).
            for (variant, description) in rewriter_variants {
                builder.push(
                    ConversionCandidate::new(variant, CandidateSource::Rewriter)
                        .with_description(description),
                );
            }
        }

        // A one-character hiragana reading is safer and more useful as the
        // default than an aggressive learned/model/dictionary conversion.
        // Keep every alternative, but pin the raw reading to candidate 1.
        if should_prioritize_single_hiragana(self.input_mode, reading) {
            builder.promote(
                reading,
                ConversionCandidate::new(reading, CandidateSource::Fallback),
            );
        }

        // 7. Enrich Fallback candidates whose text is a known symbol with
        //    its description (mirrors the relevant slice of mozc's
        //    `AddDescForCurrentCandidates`). Restricted to Fallback so the
        //    AI/Dict/Learning paths don't pick up unwanted labels — e.g.
        //    the model returning `金` for `きん` should NOT inherit mozc's
        //    "部首" annotation. Typed-symbol input still gets annotated:
        //    pressing `「` produces a Fallback candidate `「`, which here
        //    picks up "始めかぎ括弧".
        for c in &mut builder.candidates {
            if c.source == CandidateSource::Fallback
                && c.description.is_none()
                && let Some(desc) = karukan_engine::symbol_description(&c.text)
            {
                c.description = Some(desc.to_string());
            }
        }

        // 8. Attach mozc-style width annotations (`[全]ひらがな`,
        //    `[全]カタカナ`, `[半]カタカナ`) to any pure-kana candidate that
        //    still has no description. This catches `あ`/`ア` candidates that
        //    arrived via the Model or Fallback paths and were deduped against
        //    the rewriter's already-labelled variants.
        for c in &mut builder.candidates {
            if c.description.is_none()
                && let Some(desc) = width_annotation(&c.text)
            {
                c.description = Some(desc.to_string());
            }
        }

        builder.into_candidates()
    }

    /// Look up learning cache candidates for a reading (exact + prefix match, max 3).
    ///
    /// Returns candidates from the learning cache suitable for auto-suggest display.
    pub(super) fn lookup_learning_candidates(&self, reading: &str) -> Vec<Candidate> {
        let Some(cache) = &self.learning else {
            return vec![];
        };
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut seen = HashSet::new();
        let label = CandidateSource::Learning.label().to_string();

        // Exact match
        for (surface, _score) in cache.lookup(reading) {
            if candidates.len() >= MAX_LEARNING_CANDIDATES {
                break;
            }
            if seen.insert(surface.clone()) {
                candidates.push(Candidate {
                    text: surface,
                    reading: Some(reading.to_string()),
                    source_label: Some(label.clone()),
                    description: None,
                });
            }
        }

        // Prefix match (predictive)
        for (full_reading, surface, _score) in cache.prefix_lookup(reading) {
            if candidates.len() >= MAX_LEARNING_CANDIDATES {
                break;
            }
            if full_reading == reading {
                continue;
            }
            if seen.insert(surface.clone()) {
                candidates.push(Candidate {
                    text: surface,
                    reading: Some(full_reading),
                    source_label: Some(label.clone()),
                    description: None,
                });
            }
        }

        candidates
    }

    /// Look up dictionary candidates for a reading (1 page, for live conversion display)
    ///
    /// Searches user dictionary first, then system dictionary.
    pub(super) fn lookup_dict_candidates(&self, reading: &str) -> Vec<Candidate> {
        self.search_dictionaries(reading, CandidateList::DEFAULT_PAGE_SIZE)
            .into_iter()
            .map(|ac| Candidate {
                text: ac.text,
                reading: Some(reading.to_string()),
                source_label: Some(ac.source.label().to_string()),
                description: ac.description,
            })
            .collect()
    }

    /// Build rule-based rewriter variants for the reading itself (e.g. for
    /// symbol input `「` → `『`, `【`, `（`, ...). Used in the auto-suggest path
    /// so users see mozc-style symbol variants without pressing Space first.
    pub(super) fn lookup_rewriter_variants(&self, reading: &str) -> Vec<Candidate> {
        let source_label = CandidateSource::Rewriter.label().to_string();
        self.converters
            .rewriters
            .rewrite_all(&[reading.to_string()])
            .into_iter()
            .map(|(text, description)| Candidate {
                text,
                reading: Some(reading.to_string()),
                source_label: Some(source_label.clone()),
                description,
            })
            .collect()
    }

    /// Process key in conversion state
    pub(super) fn process_key_conversion(&mut self, key: &KeyEvent) -> EngineResult {
        match key.keysym {
            Keysym::RETURN => self.commit_conversion(),
            Keysym::ESCAPE => self.cancel_conversion(),
            Keysym::LEFT if key.modifiers.shift_key => self.resize_conversion_segment(false),
            Keysym::RIGHT if key.modifiers.shift_key => self.resize_conversion_segment(true),
            Keysym::LEFT if !key.modifiers.shift_key => self.move_conversion_segment(false),
            Keysym::RIGHT if !key.modifiers.shift_key => self.move_conversion_segment(true),
            Keysym::SPACE | Keysym::DOWN | Keysym::TAB => self.next_candidate(),
            Keysym::UP => self.prev_candidate(),
            Keysym::PAGE_DOWN => self.next_candidate_page(),
            Keysym::PAGE_UP => self.prev_candidate_page(),
            Keysym::BACKSPACE => self.backspace_conversion(),
            _ => {
                // Ctrl+N / Ctrl+P: emacs-style candidate navigation
                if key.modifiers.control_key && !key.modifiers.alt_key {
                    match key.keysym {
                        Keysym::KEY_N | Keysym::KEY_N_UPPER => return self.next_candidate(),
                        Keysym::KEY_P | Keysym::KEY_P_UPPER => return self.prev_candidate(),
                        _ => {}
                    }
                }

                // Check for digit selection (1-9)
                if let Some(digit) = key.keysym.digit_value() {
                    return self.select_candidate_by_digit(digit);
                }

                // Any printable character: commit current conversion and start new input
                if let Some(ch) = key.to_char()
                    && !key.modifiers.control_key
                    && !key.modifiers.alt_key
                {
                    return self.commit_conversion_and_continue(ch);
                }

                EngineResult::not_consumed()
            }
        }
    }

    fn move_conversion_segment(&mut self, right: bool) -> EngineResult {
        let candidates = {
            let InputState::Conversion { session } = &mut self.state else {
                return EngineResult::not_consumed();
            };
            if right {
                session.move_active_right();
            } else {
                session.move_active_left();
            }
            session.candidates().cloned().unwrap_or_default()
        };
        self.update_conversion_preedit(&candidates)
    }

    fn segment_candidate_list(
        &mut self,
        reading: &str,
        left_hint: Option<&str>,
        right_hint: Option<&str>,
    ) -> CandidateList {
        let candidates = CandidateList::new(
            self.build_conversion_candidates(reading, MAX_SEGMENT_CANDIDATES, false)
                .into_iter()
                .map(|candidate| candidate.into_ui_candidate(reading))
                .collect(),
        );
        self.prepend_segment_learning(reading, candidates, left_hint, right_hint)
            .0
    }

    /// Build boundary-adjustment candidates without invoking the neural model.
    /// Exact dictionary and lattice surfaces are enough to preserve the current
    /// display when it can be aligned safely; the reading itself is the
    /// correctness-preserving fallback.
    fn boundary_candidate_list(
        &self,
        reading: &str,
        left_hint: Option<&str>,
        right_hint: Option<&str>,
    ) -> CandidateList {
        let mut seen = HashSet::new();
        let mut values = Vec::new();
        for candidate in self
            .search_dictionaries(reading, MAX_SEGMENT_CANDIDATES)
            .into_iter()
            .chain(self.dictionary_lattice_candidates(reading, MAX_SEGMENT_CANDIDATES))
        {
            if seen.insert(candidate.text.clone()) {
                values.push(candidate.into_ui_candidate(reading));
            }
        }
        if seen.insert(reading.to_string()) {
            values.push(Candidate::with_reading(reading, reading));
        }
        for candidate in self.lookup_rewriter_variants(reading) {
            if seen.insert(candidate.text.clone()) {
                values.push(candidate);
            }
        }
        self.prepend_segment_learning(reading, CandidateList::new(values), left_hint, right_hint)
            .0
    }

    fn refresh_active_segment_candidates_if_dirty(&mut self) {
        let Some((reading, preserved_surface, left_hint, right_hint)) = (|| {
            let InputState::Conversion { session } = &self.state else {
                return None;
            };
            let index = session.active_segment;
            let segment = session.segments.get(index)?;
            if !segment.candidates_dirty {
                return None;
            }
            let left_hint = index
                .checked_sub(1)
                .and_then(|previous| session.segments[previous].selected_text().chars().last())
                .map(|ch| ch.to_string())
                .or_else(|| self.editor_left_hint());
            let right_hint = session
                .segments
                .get(index + 1)
                .and_then(|next| next.selected_text().chars().next())
                .map(|ch| ch.to_string())
                .or_else(|| self.editor_right_hint());
            Some((
                segment.reading.clone(),
                segment.selected_text().to_string(),
                left_hint,
                right_hint,
            ))
        })() else {
            return;
        };

        let generated =
            self.segment_candidate_list(&reading, left_hint.as_deref(), right_hint.as_deref());
        let mut seen = HashSet::from([preserved_surface.clone()]);
        let mut values = vec![Candidate {
            text: preserved_surface,
            reading: Some(reading),
            source_label: None,
            description: Some("現在の表示".to_string()),
        }];
        values.extend(
            generated
                .candidates()
                .iter()
                .filter(|candidate| seen.insert(candidate.text.clone()))
                .cloned(),
        );

        if let InputState::Conversion { session } = &mut self.state
            && let Some(segment) = session.active_mut()
        {
            segment.candidates = CandidateList::new(values);
            segment.candidates_dirty = false;
        }
    }

    /// Move the active segment's right boundary. Expanding steals the first
    /// character from the next segment; shrinking gives its last character to
    /// the next segment. Shrinking the final (or only) segment creates a new
    /// trailing segment. Expanding through a one-character next segment merges
    /// it into the active segment. The visible surface is preserved and model
    /// alternatives are rebuilt only when the user next navigates candidates.
    fn resize_conversion_segment(&mut self, expand: bool) -> EngineResult {
        let proposal = {
            let InputState::Conversion { session } = &self.state else {
                return EngineResult::not_consumed();
            };
            let index = session.active_segment;
            let left = &session.segments[index];
            let next = session.segments.get(index + 1);
            let (left_end, right_range, insert_right) = if expand {
                let Some(right) = next else {
                    return EngineResult::consumed();
                };
                if right.reading_range.len() == 1 {
                    (right.reading_range.end, None, false)
                } else {
                    (
                        left.reading_range.end + 1,
                        Some(left.reading_range.end + 1..right.reading_range.end),
                        false,
                    )
                }
            } else {
                if left.reading_range.len() <= 1 {
                    return EngineResult::consumed();
                }
                let boundary = left.reading_range.end - 1;
                let right_end = next
                    .map(|right| right.reading_range.end)
                    .unwrap_or(left.reading_range.end);
                (boundary, Some(boundary..right_end), next.is_none())
            };
            let preserved_surface = format!(
                "{}{}",
                left.selected_text(),
                next.map(|segment| segment.selected_text()).unwrap_or("")
            );
            (
                index,
                left.reading_range.start..left_end,
                right_range,
                insert_right,
                preserved_surface,
                session.reading.chars().collect::<Vec<_>>(),
            )
        };

        let (index, left_range, right_range, insert_right, preserved_surface, chars) = proposal;
        let left_reading: String = chars[left_range.clone()].iter().collect();
        let right_reading = right_range
            .as_ref()
            .map(|range| chars[range.clone()].iter().collect::<String>());
        let (left_outer_hint, right_outer_hint) = {
            let InputState::Conversion { session } = &self.state else {
                return EngineResult::not_consumed();
            };
            let left = index
                .checked_sub(1)
                .and_then(|previous| session.segments[previous].selected_text().chars().last())
                .map(|ch| ch.to_string())
                .or_else(|| self.editor_left_hint());
            let right = session
                .segments
                .get(index + 2)
                .and_then(|next| next.selected_text().chars().next())
                .map(|ch| ch.to_string())
                .or_else(|| self.editor_right_hint());
            (left, right)
        };

        let mut candidate_lists =
            vec![self.boundary_candidate_list(&left_reading, left_outer_hint.as_deref(), None)];
        if let Some(reading) = right_reading.as_deref() {
            candidate_lists.push(self.boundary_candidate_list(
                reading,
                None,
                right_outer_hint.as_deref(),
            ));
        }
        if let Some(aligned_indices) =
            align_surface_to_candidates(&preserved_surface, &candidate_lists)
        {
            for (candidates, selected_index) in candidate_lists.iter_mut().zip(aligned_indices) {
                candidates.select(selected_index);
            }
        }
        let mut candidate_lists = candidate_lists.into_iter();
        let left_candidates = candidate_lists.next().unwrap_or_default();
        let right_candidates = candidate_lists.next();

        let candidates = {
            let InputState::Conversion { session } = &mut self.state else {
                return EngineResult::not_consumed();
            };
            let mut left_segment = crate::core::state::ConversionSegment::new(
                left_range.clone(),
                left_reading,
                left_candidates,
            );
            left_segment.candidates_dirty = true;
            session.segments[index] = left_segment;
            if let (Some(right_range), Some(right_reading), Some(right_candidates)) =
                (right_range, right_reading, right_candidates)
            {
                let mut right_segment = crate::core::state::ConversionSegment::new(
                    right_range,
                    right_reading,
                    right_candidates,
                );
                right_segment.candidates_dirty = true;
                if insert_right {
                    session.segments.insert(index + 1, right_segment);
                } else {
                    session.segments[index + 1] = right_segment;
                }
            } else {
                session.segments.remove(index + 1);
            }
            debug_assert!(session.ranges_are_valid());
            session.rebuild_preedit();
            session.candidates().cloned().unwrap_or_default()
        };
        self.update_conversion_preedit(&candidates)
    }

    /// Get selected text and reading from conversion state, or None if not in conversion
    fn selected_conversion_info(&self) -> Option<(String, Option<String>)> {
        match &self.state {
            InputState::Conversion { session } => {
                let text = session.selected_text();
                let reading = session
                    .active()
                    .and_then(|segment| segment.candidates.selected())
                    .and_then(|candidate| candidate.reading.clone())
                    .or_else(|| Some(session.reading.clone()));
                Some((text, reading))
            }
            _ => None,
        }
    }

    /// Record a conversion selection in the learning cache.
    pub(super) fn record_learning(&mut self, reading: &str, surface: &str) {
        if let Some(cache) = &mut self.learning {
            cache.record(reading, surface);
        }
    }

    /// Record only segments the user explicitly corrected. Merely accepting
    /// the initial top candidate or live conversion does not enter this cache.
    pub(super) fn record_modified_segments(&mut self) {
        let records = {
            let InputState::Conversion { session } = &self.state else {
                return;
            };
            session
                .segments
                .iter()
                .enumerate()
                .filter(|(_, segment)| segment.explicitly_modified)
                .map(|(index, segment)| {
                    let left_hint = index
                        .checked_sub(1)
                        .and_then(|previous| {
                            session.segments[previous].selected_text().chars().last()
                        })
                        .map(|ch| ch.to_string())
                        .or_else(|| self.editor_left_hint());
                    let right_hint = session
                        .segments
                        .get(index + 1)
                        .and_then(|next| next.selected_text().chars().next())
                        .map(|ch| ch.to_string())
                        .or_else(|| self.editor_right_hint());
                    (
                        segment.reading.clone(),
                        segment.selected_text().to_string(),
                        left_hint,
                        right_hint,
                    )
                })
                .collect::<Vec<_>>()
        };

        let Some(cache) = &mut self.segment_learning else {
            return;
        };
        for (reading, surface, left_hint, right_hint) in records {
            cache.record(
                &reading,
                &surface,
                left_hint.as_deref(),
                right_hint.as_deref(),
            );
        }
    }

    /// Commit the current conversion
    fn commit_conversion(&mut self) -> EngineResult {
        let Some((text, reading)) = self.selected_conversion_info() else {
            return EngineResult::not_consumed();
        };

        if text.is_empty() {
            return EngineResult::consumed();
        }

        self.record_modified_segments();

        // Skip learning when the buffer is a `:shortcode` query — the
        // reading would be e.g. `:smile`, which isn't a hiragana key
        // and would corrupt the kana-keyed learning cache.
        if self.input_mode != InputMode::Emoji
            && let Some(reading) = &reading
        {
            self.record_learning(reading, &text);
        }

        self.state = InputState::Empty;
        self.input_buf.text.clear();
        self.clear_composing_candidates();
        self.exit_emoji_mode();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(Preedit::new()))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::HideAuxText)
            .with_action(EngineAction::Commit(text))
    }

    /// Commit current conversion and then process a new character as fresh input
    fn commit_conversion_and_continue(&mut self, ch: char) -> EngineResult {
        let Some((text, reading)) = self.selected_conversion_info() else {
            return EngineResult::not_consumed();
        };

        self.record_modified_segments();

        if self.input_mode != InputMode::Emoji
            && let Some(reading) = &reading
        {
            self.record_learning(reading, &text);
        }

        self.state = InputState::Empty;
        self.input_buf.text.clear();
        self.clear_composing_candidates();
        self.exit_emoji_mode();

        // Start new input with the character
        let new_input_result = self.start_input(ch);

        // Combine: commit first, then new input actions
        let mut result = EngineResult::consumed()
            .with_action(EngineAction::Commit(text))
            .with_action(EngineAction::HideCandidates);
        result.actions.extend(new_input_result.actions);
        result
    }

    /// Cancel conversion and return to hiragana
    pub(super) fn cancel_conversion(&mut self) -> EngineResult {
        if !matches!(self.state, InputState::Conversion { .. }) {
            return EngineResult::not_consumed();
        }
        let reading = self.input_buf.text.clone();

        if reading.is_empty() {
            self.state = InputState::Empty;
            self.input_buf.clear();
            self.clear_composing_candidates();
            return EngineResult::consumed()
                .with_action(EngineAction::UpdatePreedit(Preedit::new()))
                .with_action(EngineAction::HideCandidates)
                .with_action(EngineAction::HideAuxText);
        }

        // Set up composed_hiragana with the reading
        self.input_buf.text = reading.clone();
        self.input_buf.cursor_pos = self.input_buf.text.chars().count();

        // Reset romaji converter and set output to reading
        self.converters.romaji.reset();
        // We need to push each character to rebuild the state
        for ch in reading.chars() {
            self.converters.romaji.push(ch);
        }

        let preedit = self.set_composing_state();

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::HideCandidates)
            .with_action(EngineAction::UpdateAuxText(self.format_aux_composing()))
    }

    /// Navigate candidates with the given operation, then update preedit
    fn navigate_candidate(&mut self, op: impl FnOnce(&mut CandidateList) -> bool) -> EngineResult {
        self.refresh_active_segment_candidates_if_dirty();
        let candidates = {
            let InputState::Conversion { session } = &mut self.state else {
                return EngineResult::not_consumed();
            };
            let Some(segment) = session.active_mut() else {
                return EngineResult::not_consumed();
            };
            let previous_surface = segment.selected_text().to_string();
            if op(&mut segment.candidates) {
                segment.sync_selected_surface();
                if segment.selected_text() != previous_surface {
                    segment.explicitly_modified = true;
                }
            }
            segment.candidates.clone()
        };
        self.update_conversion_preedit(&candidates)
    }

    /// Select next candidate
    fn next_candidate(&mut self) -> EngineResult {
        let exhausted_whole_candidates = matches!(
            &self.state,
            InputState::Conversion { session }
                if session.is_whole_candidate_phase()
                    && session
                        .candidates()
                        .is_some_and(|candidates| candidates.cursor() + 1 >= candidates.len())
        );
        if exhausted_whole_candidates {
            return self.activate_segmented_conversion();
        }
        self.navigate_candidate(CandidateList::move_next)
    }

    fn activate_segmented_conversion(&mut self) -> EngineResult {
        let Some((reading, fallback_candidates)) = (match &self.state {
            InputState::Conversion { session } => session
                .candidates()
                .cloned()
                .map(|candidates| (session.reading.clone(), candidates)),
            _ => None,
        }) else {
            return EngineResult::not_consumed();
        };

        let mut session = self.build_conversion_session(&reading, fallback_candidates, false);
        session.finish_whole_candidate_phase();
        self.enter_conversion_state(session)
    }

    /// Select previous candidate
    fn prev_candidate(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::move_prev)
    }

    /// Go to next candidate page
    fn next_candidate_page(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::next_page)
    }

    /// Go to previous candidate page
    fn prev_candidate_page(&mut self) -> EngineResult {
        self.navigate_candidate(CandidateList::prev_page)
    }

    /// Select and commit the candidate at `page_index` (0-based) within the
    /// current page, like pressing the digit key `page_index + 1`. Not
    /// consumed unless a candidate list is active (Conversion state).
    pub fn select_candidate_on_page(&mut self, page_index: usize) -> EngineResult {
        let start = std::time::Instant::now();
        self.metrics.conversion_ms = 0;
        let result = self.select_candidate_by_digit(page_index + 1);
        self.metrics.process_key_ms = start.elapsed().as_millis() as u64;
        result
    }

    /// Select candidate by digit (1-9)
    fn select_candidate_by_digit(&mut self, digit: usize) -> EngineResult {
        let candidates = {
            let InputState::Conversion { session } = &mut self.state else {
                return EngineResult::not_consumed();
            };
            let Some(segment) = session.active_mut() else {
                return EngineResult::not_consumed();
            };

            let previous_surface = segment.selected_text().to_string();
            if segment.candidates.select_on_page(digit).is_none() {
                return EngineResult::consumed();
            }
            segment.sync_selected_surface();
            if segment.selected_text() != previous_surface {
                segment.explicitly_modified = true;
            }
            segment.candidates.clone()
        };
        // A digit/click applies the candidate to the active segment only.
        // Enter or the explicit `commit` API commits the complete session.
        self.update_conversion_preedit(&candidates)
    }

    /// Update preedit after candidate selection change
    fn update_conversion_preedit(&mut self, candidates: &CandidateList) -> EngineResult {
        let (preedit, reading) = match &mut self.state {
            InputState::Conversion { session } => {
                session.rebuild_preedit();
                let reading = session
                    .active()
                    .map(|segment| segment.reading.clone())
                    .unwrap_or_default();
                (session.preedit().clone(), reading)
            }
            _ => return EngineResult::not_consumed(),
        };

        EngineResult::consumed()
            .with_action(EngineAction::UpdatePreedit(preedit))
            .with_action(EngineAction::ShowCandidates(candidates.clone()))
            .with_action(EngineAction::UpdateAuxText(
                self.format_aux_conversion_with_page(&reading, Some(candidates)),
            ))
    }

    /// Handle backspace in conversion mode
    fn backspace_conversion(&mut self) -> EngineResult {
        // Return to hiragana mode with the reading
        self.cancel_conversion()
    }
}
