use super::*;
use karukan_engine::LearningCache;

// --- Candidate preservation tests ---

fn commit_text(result: &EngineResult) -> Option<&str> {
    result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.as_str()),
        _ => None,
    })
}

fn shown_candidate_texts(result: &EngineResult) -> Vec<String> {
    result
        .actions
        .iter()
        .find_map(|action| match action {
            EngineAction::ShowCandidates(candidates) => Some(
                candidates
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.text.clone())
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

fn candidate_texts(candidates: &CandidateList) -> Vec<String> {
    candidates
        .candidates()
        .iter()
        .map(|candidate| candidate.text.clone())
        .collect()
}

fn learn(engine: &mut InputMethodEngine, reading: &str, surface: &str) {
    let mut cache = LearningCache::new(100);
    cache.record(reading, surface);
    engine.learning = Some(cache);
}

#[test]
fn single_hiragana_is_first_composing_candidate() {
    let mut engine = InputMethodEngine::new();
    learn(&mut engine, "し", "詩");

    engine.process_key(&press('s'));
    let result = engine.process_key(&press('i'));
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.preedit().unwrap().text(), "し");
    assert_eq!(candidates.first().map(String::as_str), Some("し"));
    assert!(candidates.iter().any(|candidate| candidate == "詩"));
}

#[test]
fn single_hiragana_is_live_text_and_first_candidate_before_model_alternatives() {
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = "し".to_string();
    engine.input_buf.cursor_pos = 1;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("し"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        reading: "し".to_string(),
        converted: "詩".to_string(),
        candidates: vec!["詩".to_string(), "市".to_string()],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.live.text, "し");
    assert_eq!(engine.preedit().unwrap().text(), "し");
    assert_eq!(candidates.first().map(String::as_str), Some("し"));
    assert!(candidates.iter().any(|candidate| candidate == "詩"));
    assert!(candidates.iter().any(|candidate| candidate == "市"));
}

#[test]
fn multi_character_live_text_is_first_and_whole_candidates_are_limited_to_three() {
    let mut engine = make_live_conversion_engine();
    learn(&mut engine, "しよう", "私用");
    engine.input_buf.text = "しよう".to_string();
    engine.input_buf.cursor_pos = 3;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("しよう"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        reading: "しよう".to_string(),
        converted: "使用".to_string(),
        candidates: vec!["使用".to_string(), "仕様".to_string(), "しよう".to_string()],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.preedit().unwrap().text(), "使用");
    assert_eq!(candidates.first().map(String::as_str), Some("使用"));
    assert_eq!(candidates.len(), WHOLE_CANDIDATE_LIMIT);
    assert!(candidates.iter().any(|candidate| candidate == "私用"));
}

#[test]
fn single_hiragana_is_first_explicit_conversion_candidate() {
    let mut engine = InputMethodEngine::new();
    learn(&mut engine, "し", "詩");

    engine.process_key(&press('s'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::SPACE));

    let candidates = engine.state().candidates().unwrap();
    assert_eq!(candidates.selected_text(), Some("し"));
    assert!(
        candidates
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "詩")
    );
}

#[test]
fn space_skips_the_live_first_candidate_and_starts_from_the_second() {
    // When Space is pressed during live conversion, the AI inference result
    // remains candidate 1, but it was already visible before Space. Explicit
    // selection therefore starts from candidate 2.
    let mut engine = make_live_conversion_engine();

    // Simulate typing "あい" with live conversion active
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.live.text = "愛".to_string();

    // Press Space → start_conversion()
    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // The candidate list keeps "愛" first while selecting the next choice.
    let candidates = engine.state().candidates().unwrap();
    assert_eq!(
        candidates
            .candidates()
            .first()
            .map(|candidate| candidate.text.as_str()),
        Some("愛")
    );
    assert_eq!(candidates.cursor(), 1);
    assert_ne!(candidates.selected_text(), Some("愛"));
    assert_eq!(
        engine.preedit().map(Preedit::text),
        candidates.selected_text()
    );
    assert_eq!(candidates.len(), WHOLE_CANDIDATE_LIMIT);
}

#[test]
fn space_reuses_the_exact_live_candidate_list() {
    let mut engine = make_live_conversion_engine();
    learn(&mut engine, "しよう", "私用");
    engine.input_buf.text = "しよう".to_string();
    engine.input_buf.cursor_pos = 3;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("しよう"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        reading: "しよう".to_string(),
        converted: "使用".to_string(),
        candidates: vec!["使用".to_string(), "仕様".to_string(), "しよう".to_string()],
    }];

    engine.refresh_input_state();
    let before = candidate_texts(engine.composing_candidates.as_ref().unwrap());

    engine.process_key(&press_key(Keysym::SPACE));

    let after = engine.state().candidates().unwrap();
    assert_eq!(candidate_texts(after), before);
    assert_eq!(after.cursor(), 1);
    assert_eq!(after.selected_text(), before.get(1).map(String::as_str));
}

#[test]
fn space_does_not_regenerate_mixed_long_candidates() {
    let reading = "ぶんかつhennkannいこうじにへんかんちゅうもじれつがへんか";
    let candidates = [
        "分割hennkann移行時に変換中文字列が変化",
        "分割変換移行時に変換中文字列が変化",
        "分割hennkann移行時の変換中文字列が変化",
    ];
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = reading.to_string();
    engine.input_buf.cursor_pos = reading.chars().count();
    engine.live.text = candidates[0].to_string();
    engine.composing_candidates = Some(CandidateList::from_strings_with_reading(
        candidates, reading,
    ));
    engine.composing_candidates_model_ready = true;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text(candidates[0]),
        romaji_buffer: String::new(),
    };

    engine.process_key(&press_key(Keysym::SPACE));

    let after = engine.state().candidates().unwrap();
    assert_eq!(candidate_texts(after), candidates);
    assert_eq!(after.selected_text(), Some(candidates[1]));
}

#[test]
fn background_result_replaces_only_its_reading_prefix() {
    let mut engine = make_live_conversion_engine();
    let current = "ほんらいのようと";
    let completed = "ほんらいのよ";
    engine.input_buf.text = current.to_string();
    engine.input_buf.cursor_pos = current.chars().count();
    engine.state = InputState::Composing {
        preedit: Preedit::with_text(current),
        romaji_buffer: String::new(),
    };

    let result = engine.apply_background_candidates(
        completed.to_string(),
        current.to_string(),
        vec!["本来のよ".to_string(), "本来の世".to_string()],
    );

    assert_eq!(engine.live.applied_reading, completed);
    assert_eq!(engine.live.applied_text, "本来のよ");
    assert_eq!(engine.live.text, "本来のようと");
    assert_eq!(engine.preedit().map(Preedit::text), Some("本来のようと"));
    assert_eq!(
        candidate_texts(engine.composing_candidates.as_ref().unwrap()),
        ["本来のようと", "本来の世うと"]
    );
    assert!(!engine.composing_candidates_model_ready);
    assert!(result.actions.iter().any(|action| {
        matches!(
            action,
            EngineAction::UpdatePreedit(preedit) if preedit.text() == "本来のようと"
        )
    }));
}

#[test]
fn converted_prefix_stays_visible_while_input_suffix_grows() {
    let mut engine = make_live_conversion_engine();
    engine
        .live
        .set_applied_prefix("ほんらいのよ".to_string(), "本来のよ".to_string());
    engine.input_buf.text = "ほんらいのようと".to_string();
    engine.input_buf.cursor_pos = engine.input_buf.text.chars().count();

    let first = engine.refresh_without_model();
    assert_eq!(engine.live.text, "本来のようと");
    assert!(first.actions.iter().any(|action| {
        matches!(
            action,
            EngineAction::UpdatePreedit(preedit) if preedit.text() == "本来のようと"
        )
    }));

    engine.input_buf.text.push_str("いがい");
    engine.input_buf.cursor_pos = engine.input_buf.text.chars().count();
    engine.refresh_without_model();
    assert_eq!(engine.live.text, "本来のようといがい");
}

#[test]
fn tab_selects_visible_composing_candidate_then_enter_commits_it() {
    let mut engine = InputMethodEngine::new();

    // "a" has composing-time rewriter candidates: あ, ア, ｱ.
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    assert!(engine.composing_candidates.is_some());

    // First Tab opts into the visible first suggestion without opening
    // explicit Conversion state.
    let first = engine.process_key(&press_key(Keysym::TAB));
    assert!(first.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "アイ");

    // Second Tab advances to the next visible suggestion.
    let second = engine.process_key(&press_key(Keysym::TAB));
    assert!(second.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "ｱｲ");

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert_eq!(commit_text(&commit), Some("ｱｲ"));
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn down_selects_visible_composing_candidate_like_tab() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    let result = engine.process_key(&press_key(Keysym::DOWN));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "アイ");

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert_eq!(commit_text(&commit), Some("アイ"));
}

#[test]
fn up_first_selects_the_visible_composing_candidate_without_jumping_to_the_end() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    let visible = engine
        .composing_candidates
        .as_ref()
        .and_then(CandidateList::selected_text)
        .unwrap()
        .to_string();

    let result = engine.process_key(&press_key(Keysym::UP));

    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), visible);
}

#[test]
fn enter_without_tab_keeps_traditional_composing_commit() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    assert!(engine.composing_candidates.is_some());

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert_eq!(commit_text(&commit), Some("あい"));
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn test_live_text_not_duplicated_in_conversion() {
    // If the live_text matches the reading, it should not be duplicated
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    // live_conversion_text same as hiragana reading → should not be added
    engine.live.text = "あい".to_string();

    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // "あい" should not appear twice (it's same as reading, so live_text is skipped)
    let candidates = engine.state().candidates().unwrap();
    let count = candidates
        .candidates()
        .iter()
        .filter(|c| c.text == "あい")
        .count();
    assert_eq!(count, 1, "Reading should appear exactly once");
}

#[test]
fn test_suggest_result_preserved_in_start_conversion() {
    // When Space is pressed, the previous auto-suggest/live conversion result
    // should be preserved in the candidate list even if re-inference doesn't produce it.
    // (Without a kanji converter, build_conversion_candidates returns fallback only,
    // so the live_conversion_text would be lost without the preservation logic.)
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.live.text = "愛".to_string();

    // Press Space → start_conversion()
    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // "愛" should be preserved in the candidate list
    let candidates = engine.state().candidates().unwrap();
    assert!(
        candidates.candidates().iter().any(|c| c.text == "愛"),
        "Previous suggest result '愛' should be preserved in candidates"
    );
}

#[test]
fn test_empty_live_text_not_added_to_candidates() {
    // When live_conversion_text is empty, no extra candidate should be added
    let mut engine = make_live_conversion_engine();

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    // Force empty to test the "no live text" scenario
    engine.live.text.clear();

    // Space → start_conversion()
    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);

    // Should have candidates but no empty-string candidate
    if let Some(candidates) = engine.state().candidates() {
        assert!(
            !candidates.candidates().iter().any(|c| c.text.is_empty()),
            "Empty candidate should not be in the list"
        );
    }
}
