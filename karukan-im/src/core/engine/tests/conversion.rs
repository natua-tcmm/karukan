use super::*;

#[test]
fn test_conversion_char_commits_and_continues() {
    let mut engine = InputMethodEngine::new();

    // Type "あい" and enter conversion
    engine.process_key(&press('a'));
    engine.process_key(&press('i'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // Type 'k' during conversion → should commit candidate and start new input
    let result = engine.process_key(&press('k'));
    assert!(result.consumed);

    // Should have committed the conversion
    let has_commit = result
        .actions
        .iter()
        .any(|a| matches!(a, EngineAction::Commit(_)));
    assert!(has_commit, "Should have a commit action");

    // Should now be in Composing with 'k' in preedit
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "k");
}

#[test]
fn test_conversion_char_commits_and_continues_romaji() {
    let mut engine = InputMethodEngine::new();

    // Type "あ" and enter conversion
    engine.process_key(&press('a'));
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Conversion { .. }));

    // Type 'k', 'a' → commits conversion, then starts "か"
    engine.process_key(&press('k'));
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "k");

    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "か");
}

#[test]
fn test_alphabet_mode_space_inserts_literal_space() {
    let mut engine = InputMethodEngine::new();

    // Enter alphabet mode via Shift+N
    engine.process_key(&press_shift('N'));
    assert!(engine.input_mode == InputMode::Alphabet);

    // Type "ew"
    engine.process_key(&press('e'));
    engine.process_key(&press('w'));
    assert_eq!(engine.preedit().unwrap().text(), "New");

    // Space → should insert literal space, NOT start conversion
    engine.process_key(&press_key(Keysym::SPACE));
    assert!(matches!(engine.state(), InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "New ");

    // Type "york"
    engine.process_key(&press('y'));
    engine.process_key(&press('o'));
    engine.process_key(&press('r'));
    engine.process_key(&press('k'));
    assert_eq!(engine.preedit().unwrap().text(), "New york");
}

#[test]
fn dictionary_lattice_emits_multiple_candidates_for_multiple_segments() {
    use karukan_engine::dictionary_source::NormalizedDictionaryEntry;
    use karukan_engine::{DictionaryCategory, DictionarySource};

    let entry = |reading: &str, surface: &str, score: f32| {
        NormalizedDictionaryEntry::new(
            reading,
            surface,
            score,
            DictionarySource::Mozc,
            DictionaryCategory::General,
            None,
        )
        .unwrap()
    };
    let dictionary = Dictionary::build_from_normalized([
        entry("とうきょう", "東京", 0.0),
        entry("えき", "駅", 0.0),
        entry("えき", "驛", 1.0),
    ])
    .unwrap();
    let mut engine = InputMethodEngine::new();
    engine.dicts.system = Some(dictionary);

    let candidates = engine.dictionary_lattice_candidates("とうきょうえき", 9);
    let texts: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.text.as_str())
        .collect();
    assert_eq!(texts[0], "東京駅");
    assert!(texts.contains(&"東京驛"));
    assert!(candidates.len() <= 9);
}
