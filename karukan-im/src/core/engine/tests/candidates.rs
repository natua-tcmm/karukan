use super::*;

// --- Candidate preservation tests ---

fn commit_text(result: &EngineResult) -> Option<&str> {
    result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.as_str()),
        _ => None,
    })
}

#[test]
fn test_live_text_preserved_in_conversion_via_space() {
    // When Space is pressed during live conversion, the AI inference result
    // (live_conversion_text) should appear in the candidate list.
    let mut engine = make_live_conversion_engine();

    // Simulate typing "あい" with live conversion active
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.live.text = "愛".to_string();

    // Press Space → start_conversion()
    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // The candidate list should contain "愛"
    let candidates = engine.state().candidates().unwrap();
    assert!(
        candidates.candidates().iter().any(|c| c.text == "愛"),
        "AI inference result '愛' should be in the candidate list"
    );
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
