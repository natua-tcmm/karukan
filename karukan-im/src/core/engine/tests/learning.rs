//! Tests for the learning cache and composing-time suggestion selection.
//!
//! Space opens the explicit conversion list with learning candidates included.
//! Tab/Down select the auto-suggest list already shown during composing.

use karukan_engine::LearningCache;

use super::*;

/// Engine seeded with a learning entry `reading → surface`, no kanji model.
/// We bypass `init.rs` (which gates learning on settings + file I/O) and just
/// inject a populated `LearningCache` directly.
fn engine_with_learned(reading: &str, surface: &str) -> InputMethodEngine {
    let mut engine = InputMethodEngine::new();
    engine.converters.kanji = None;
    let mut cache = LearningCache::new(100);
    cache.record(reading, surface);
    engine.learning = Some(cache);
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
    // Space stays on the learning-included explicit conversion path.
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
