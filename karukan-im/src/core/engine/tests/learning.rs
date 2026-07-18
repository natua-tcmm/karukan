//! Tests for correction learning and composing-time suggestion selection.
//!
//! Only explicit candidate changes are learned. Lookup is exact by segment
//! reading, so a previously committed sentence never becomes prefix completion.

use karukan_engine::SegmentLearningCache;

use super::*;

/// Engine seeded with an explicit correction `reading → surface`, no model.
fn engine_with_learned(reading: &str, surface: &str) -> InputMethodEngine {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    let mut cache = SegmentLearningCache::new(100);
    cache.record(reading, surface, None, None);
    engine.segment_learning = Some(cache);
    engine
}

#[test]
fn build_candidates_includes_learning_when_not_skipped() {
    let mut engine = engine_with_learned("あい", "藍");

    let texts: Vec<String> = engine
        .build_conversion_candidates("あい", 9, false)
        .into_iter()
        .map(|c| c.text)
        .collect();

    assert!(
        texts.contains(&"藍".to_string()),
        "Space path (skip_learning=false) should surface learned `藍`, got {:?}",
        texts,
    );
}

#[test]
fn build_candidates_omits_learning_when_skipped() {
    let mut engine = engine_with_learned("あい", "藍");

    let texts: Vec<String> = engine
        .build_conversion_candidates("あい", 9, true)
        .into_iter()
        .map(|c| c.text)
        .collect();

    assert!(
        !texts.contains(&"藍".to_string()),
        "Explicit skip_learning=true path must drop learned `藍`, got {:?}",
        texts,
    );
}

#[test]
fn tab_key_selects_composing_learning_candidate() {
    // End-to-end: type the reading, press Tab → opt into the auto-suggest
    // list already shown during composing. Enter then commits that selection.
    let mut engine = engine_with_learned("あい", "藍");

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    assert_eq!(engine.input_buf.text, "あい");

    let result = engine.process_key(&press_key(Keysym::TAB));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "藍");

    let commit = engine.process_key(&press_key(Keysym::RETURN));
    assert!(
        commit
            .actions
            .iter()
            .any(|a| matches!(a, EngineAction::Commit(text) if text == "藍"))
    );
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn space_key_keeps_learning_in_composing() {
    // Space stays on the correction-learning-included explicit conversion path.
    let mut engine = engine_with_learned("あい", "藍");

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));

    let result = engine.process_key(&press_key(Keysym::SPACE));
    assert!(result.consumed);
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    let texts: Vec<String> = engine
        .state()
        .candidates()
        .unwrap()
        .candidates()
        .iter()
        .map(|c| c.text.clone())
        .collect();
    assert!(
        texts.contains(&"藍".to_string()),
        "Space must surface learned `藍`, got {:?}",
        texts,
    );
}

#[test]
fn learned_long_sentence_is_not_a_prefix_candidate() {
    let learned_surface = "候補変換と実際の別れ方がちぐはぐになっている気がする。";
    let mut engine = engine_with_learned(
        "こうほへんかんとじっさいのわかれかたがちぐはぐになっているきがする。",
        learned_surface,
    );

    for ch in ['k', 'o', 'u', 'h', 'o'] {
        engine.process_key(&press(ch));
    }

    let candidates = engine
        .composing_candidates
        .as_ref()
        .map(|list| {
            list.candidates()
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(!candidates.contains(&learned_surface));
}

#[test]
fn ordinary_composing_commit_does_not_learn() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    engine.segment_learning = Some(SegmentLearningCache::new(100));

    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::RETURN));

    assert!(
        engine
            .segment_learning
            .as_ref()
            .unwrap()
            .lookup("あい", None, None)
            .is_empty()
    );
}

#[test]
fn explicitly_selected_composing_variant_is_learned() {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    engine.segment_learning = Some(SegmentLearningCache::new(100));
    engine.input_buf.text = "あ".to_string();
    engine.input_buf.cursor_pos = 1;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("ア"),
        romaji_buffer: String::new(),
    };
    let mut candidates = CandidateList::from_strings_with_reading(["あ", "ア"], "あ");
    candidates.select(1);
    engine.composing_candidates = Some(candidates);
    engine.composing_candidate_selected = true;

    engine.process_key(&press_key(Keysym::RETURN));

    let learned = engine
        .segment_learning
        .as_ref()
        .unwrap()
        .lookup("あ", None, None);
    assert!(learned.iter().any(|(entry, _)| entry.surface == "ア"));
}
