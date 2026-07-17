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

#[test]
fn best_lattice_path_initializes_conversion_segments() {
    use karukan_engine::dictionary_source::NormalizedDictionaryEntry;
    use karukan_engine::{DictionaryCategory, DictionarySource};

    let entry = |reading: &str, surface: &str| {
        NormalizedDictionaryEntry::new(
            reading,
            surface,
            0.0,
            DictionarySource::Mozc,
            DictionaryCategory::General,
            None,
        )
        .unwrap()
    };
    let dictionary =
        Dictionary::build_from_normalized([entry("とうきょう", "東京"), entry("えき", "駅")])
            .unwrap();
    let mut engine = InputMethodEngine::new();
    engine.dicts.system = Some(dictionary);
    let session = engine.build_initial_conversion_session(
        "とうきょうえき",
        CandidateList::from_strings_with_reading(["東京駅"], "とうきょうえき"),
    );

    assert_eq!(session.segments.len(), 2);
    assert_eq!(session.segments[0].reading_range, 0..5);
    assert_eq!(session.segments[0].selected_text(), "東京");
    assert_eq!(session.segments[1].reading_range, 5..7);
    assert_eq!(session.segments[1].selected_text(), "駅");
    assert_eq!(session.preedit().text(), "東京駅");
    assert_eq!(session.preedit().attributes().len(), 2);
    assert_eq!(
        session.preedit().attributes()[0].attr_type,
        AttributeType::Highlight
    );
    assert_eq!(
        session.preedit().attributes()[1].attr_type,
        AttributeType::Underline
    );
}

#[test]
fn left_and_right_move_the_active_conversion_segment() {
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![
        ConversionSegment {
            reading_range: 0..3,
            reading: "きょう".into(),
            candidates: CandidateList::from_strings_with_reading(["今日", "京"], "きょう"),
            explicitly_modified: false,
        },
        ConversionSegment {
            reading_range: 3..5,
            reading: "いく".into(),
            candidates: CandidateList::from_strings_with_reading(["行く", "往く"], "いく"),
            explicitly_modified: false,
        },
    ];
    let mut engine = InputMethodEngine::new();
    engine.state = InputState::Conversion {
        session: ConversionSession::segmented("きょういく".into(), segments),
    };

    let result = engine.process_key(&press_key(Keysym::RIGHT));
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };
    assert_eq!(session.active_segment, 1);
    assert_eq!(engine.candidates().unwrap().selected_text(), Some("行く"));
    assert!(result.actions.iter().any(|action| matches!(
        action,
        EngineAction::ShowCandidates(candidates)
            if candidates.selected_text() == Some("行く")
    )));
    assert_eq!(
        session.preedit().attributes()[0].attr_type,
        AttributeType::Underline
    );
    assert_eq!(
        session.preedit().attributes()[1].attr_type,
        AttributeType::Highlight
    );

    engine.process_key(&press_key(Keysym::LEFT));
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };
    assert_eq!(session.active_segment, 0);
}

#[test]
fn shift_arrows_resize_only_the_active_and_next_segments() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![
        ConversionSegment {
            reading_range: 0..8,
            reading: "じっちゅうはっく".into(),
            candidates: CandidateList::from_strings(["十中八九"]),
            explicitly_modified: false,
        },
        ConversionSegment {
            reading_range: 8..11,
            reading: "あたる".into(),
            candidates: CandidateList::from_strings(["当たる"]),
            explicitly_modified: false,
        },
        ConversionSegment {
            reading_range: 11..13,
            reading: "かも".into(),
            candidates: CandidateList::from_strings(["かも"]),
            explicitly_modified: false,
        },
    ];
    let mut engine = InputMethodEngine::new();
    engine.state = InputState::Conversion {
        session: ConversionSession::segmented("じっちゅうはっくあたるかも".into(), segments),
    };

    let shift_right = KeyEvent::new(Keysym::RIGHT, KeyModifiers::new().with_shift(true), true);
    engine.process_key(&shift_right);
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };
    assert_eq!(session.segments[0].reading, "じっちゅうはっくあ");
    assert_eq!(session.segments[0].reading_range, 0..9);
    assert_eq!(session.segments[1].reading, "たる");
    assert_eq!(session.segments[1].reading_range, 9..11);
    assert_eq!(session.segments[2].candidates.selected_text(), Some("かも"));
    assert!(session.ranges_are_valid());

    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    engine.process_key(&shift_left);
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };
    assert_eq!(session.segments[0].reading, "じっちゅうはっく");
    assert_eq!(session.segments[1].reading, "あたる");
    assert!(session.ranges_are_valid());
}

#[test]
fn explicitly_selected_segment_is_learned_with_right_context() {
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![
        ConversionSegment {
            reading_range: 0..2,
            reading: "あと".into(),
            candidates: CandidateList::from_strings(["後", "あと"]),
            explicitly_modified: false,
        },
        ConversionSegment {
            reading_range: 2..3,
            reading: "、".into(),
            candidates: CandidateList::from_strings(["、"]),
            explicitly_modified: false,
        },
    ];
    let mut engine = InputMethodEngine::new();
    engine.segment_learning = Some(karukan_engine::SegmentLearningCache::new(100));
    engine.state = InputState::Conversion {
        session: ConversionSession::segmented("あと、".into(), segments),
    };

    engine.select_candidate_on_page(1);
    engine.process_key(&press_key(Keysym::RETURN));

    let learned = engine
        .segment_learning
        .as_ref()
        .unwrap()
        .lookup("あと", None, Some("、"));
    assert_eq!(learned.len(), 1);
    assert_eq!(learned[0].0.surface, "あと");
}

#[test]
fn accepting_initial_segment_does_not_enter_segment_learning() {
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![ConversionSegment {
        reading_range: 0..2,
        reading: "あと".into(),
        candidates: CandidateList::from_strings(["後", "あと"]),
        explicitly_modified: false,
    }];
    let mut engine = InputMethodEngine::new();
    engine.segment_learning = Some(karukan_engine::SegmentLearningCache::new(100));
    engine.state = InputState::Conversion {
        session: ConversionSession::segmented("あと".into(), segments),
    };

    engine.process_key(&press_key(Keysym::RETURN));

    assert!(
        engine
            .segment_learning
            .as_ref()
            .unwrap()
            .lookup("あと", None, None)
            .is_empty()
    );
}

#[test]
fn segment_learning_precedes_initial_whole_reading_candidate() {
    let mut engine = InputMethodEngine::new();
    let mut cache = karukan_engine::SegmentLearningCache::new(100);
    cache.record("あと", "あと", None, Some("、"));
    engine.segment_learning = Some(cache);
    engine.set_surrounding_context("", "、");

    let session = engine
        .build_initial_conversion_session("あと", CandidateList::from_strings(["後", "あと"]));

    assert_eq!(session.selected_text(), "あと");
    assert_eq!(
        session.segments[0].candidates.candidates()[0]
            .description
            .as_deref(),
        Some("文節修正")
    );
}
