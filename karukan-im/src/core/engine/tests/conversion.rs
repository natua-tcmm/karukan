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
fn live_surface_morphology_initializes_conversion_segments() {
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
fn exhausting_whole_candidates_segments_the_live_first_surface() {
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
    engine.input_buf.text = "とうきょうえき".to_string();
    engine.input_buf.cursor_pos = 7;
    engine.live.text = "東京駅".to_string();
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("東京駅"),
        romaji_buffer: String::new(),
    };

    engine.process_key(&press_key(Keysym::SPACE));
    let InputState::Conversion { session } = engine.state() else {
        panic!("whole conversion expected");
    };
    assert!(session.is_whole_candidate_phase());
    assert_eq!(session.segments.len(), 1);
    assert_eq!(session.candidates().unwrap().len(), WHOLE_CANDIDATE_LIMIT);
    assert_eq!(session.candidates().unwrap().cursor(), 1);
    assert_ne!(
        session.candidates().unwrap().selected_text(),
        Some("東京駅")
    );

    engine.process_key(&press_key(Keysym::SPACE));
    let surface_before_segmentation = engine.preedit().unwrap().text().to_string();
    engine.process_key(&press_key(Keysym::SPACE));

    let InputState::Conversion { session } = engine.state() else {
        panic!("segmented conversion expected");
    };
    assert!(!session.is_whole_candidate_phase());
    assert_eq!(session.segments.len(), 2);
    assert_eq!(session.segments[0].reading, "とうきょう");
    assert_eq!(session.segments[1].reading, "えき");
    assert_ne!(surface_before_segmentation, "東京駅");
    assert_eq!(session.selected_text(), "東京駅");
    assert_eq!(session.preedit().text(), "東京駅");
}

#[test]
fn exhausting_composing_candidates_segments_the_live_first_surface() {
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
    engine.input_buf.text = "とうきょうえき".to_string();
    engine.input_buf.cursor_pos = 7;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("東京駅"),
        romaji_buffer: String::new(),
    };
    let mut candidates = CandidateList::from_strings_with_reading(
        ["東京駅", "とうきょうえき", "トウキョウエキ"],
        "とうきょうえき",
    );
    candidates.select(2);
    engine.composing_candidates = Some(candidates);
    engine.composing_candidate_selected = true;

    engine.process_key(&press_key(Keysym::SPACE));

    let InputState::Conversion { session } = engine.state() else {
        panic!("segmented conversion expected");
    };
    assert!(!session.is_whole_candidate_phase());
    assert_eq!(session.segments.len(), 2);
    assert_eq!(session.selected_text(), "東京駅");
    assert_eq!(session.preedit().text(), "東京駅");
}

#[test]
fn unalignable_whole_surface_remains_a_single_segment() {
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
        CandidateList::from_strings_with_reading(["東京都駅"], "とうきょうえき"),
    );

    assert_eq!(session.segments.len(), 1);
    assert_eq!(session.selected_text(), "東京都駅");
    assert_eq!(session.preedit().text(), "東京都駅");
    assert_eq!(session.segments[0].reading, "とうきょうえき");
}

#[test]
fn surface_with_a_mismatched_morphological_reading_stays_whole() {
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
        "とうきょうなぞえき",
        CandidateList::from_strings_with_reading(["東京ミステリー駅"], "とうきょうなぞえき"),
    );

    assert_eq!(session.segments.len(), 1);
    assert_eq!(session.selected_text(), "東京ミステリー駅");
    assert_eq!(session.segments[0].reading, "とうきょうなぞえき");
}

#[test]
fn long_live_surface_enters_segmented_mode_without_changing_text() {
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
    let reading = "ほんらいのようといがいでは".repeat(4);
    let surface = "本来の用途以外では".repeat(4);
    assert!(reading.chars().count() > EngineConfig::default().composing_chunk_len);

    let mut engine = make_live_conversion_engine();
    engine.dicts.system = Some(dictionary);
    engine.input_buf.text = reading.clone();
    engine.input_buf.cursor_pos = reading.chars().count();
    engine.live.text = surface.clone();
    engine.composing_candidates = Some(CandidateList::from_strings_with_reading(
        [&surface],
        &reading,
    ));
    engine.state = InputState::Composing {
        preedit: Preedit::with_text(&surface),
        romaji_buffer: String::new(),
    };

    engine.process_key(&press_key(Keysym::SPACE));
    engine.process_key(&press_key(Keysym::SPACE));

    let InputState::Conversion { session } = engine.state() else {
        panic!("segmented conversion expected");
    };
    assert!(!session.is_whole_candidate_phase());
    assert!(session.segments.len() > 1);
    assert_eq!(session.selected_text(), surface);
    assert_eq!(session.preedit().text(), surface);
    assert!(
        session
            .segments
            .iter()
            .all(|segment| { segment.candidates.candidates()[0].text == segment.selected_text() })
    );
}

#[test]
fn initial_segmentation_aligns_each_surface_with_its_reading() {
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
        entry("となり", "隣り", 0.0),
        entry("となり", "隣", 1.0),
        entry("の", "野", 0.0),
        entry("の", "の", 1.0),
        entry("きゃくは", "客は", 0.0),
    ])
    .unwrap();
    let mut engine = InputMethodEngine::new();
    engine.dicts.system = Some(dictionary);
    let session = engine.build_initial_conversion_session(
        "となりのきゃくは",
        CandidateList::from_strings_with_reading(["隣の客は"], "となりのきゃくは"),
    );

    assert_eq!(session.segments.len(), 4);
    assert_eq!(session.selected_text(), "隣の客は");
    assert_eq!(session.segments[0].reading, "となり");
    assert_eq!(session.segments[0].selected_text(), "隣");
    assert_eq!(session.segments[1].reading, "の");
    assert_eq!(session.segments[1].selected_text(), "の");
    assert_eq!(session.segments[1].candidates.selected_text(), Some("の"));
    assert_eq!(session.segments[2].reading, "きゃく");
    assert_eq!(session.segments[2].selected_text(), "客");
    assert_eq!(session.segments[3].reading, "は");
    assert_eq!(session.segments[3].selected_text(), "は");
}

#[test]
fn left_and_right_move_the_active_conversion_segment() {
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![
        ConversionSegment::new(
            0..3,
            "きょう".into(),
            CandidateList::from_strings_with_reading(["今日", "京"], "きょう"),
        ),
        ConversionSegment::new(
            3..5,
            "いく".into(),
            CandidateList::from_strings_with_reading(["行く", "往く"], "いく"),
        ),
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
        ConversionSegment::new(
            0..8,
            "じっちゅうはっく".into(),
            CandidateList::from_strings(["十中八九"]),
        ),
        ConversionSegment::new(
            8..11,
            "あたる".into(),
            CandidateList::from_strings(["当たる"]),
        ),
        ConversionSegment::new(11..13, "かも".into(), CandidateList::from_strings(["かも"])),
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
fn shift_left_splits_a_single_conversion_segment() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::ConversionSession;

    let mut engine = InputMethodEngine::new();
    engine.state = InputState::Conversion {
        session: ConversionSession::single(
            "じっちゅうはっく".into(),
            CandidateList::from_strings(["十中八九"]),
        ),
    };

    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    let result = engine.process_key(&shift_left);
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };

    assert!(result.consumed);
    assert_eq!(session.segments.len(), 2);
    assert_eq!(session.segments[0].reading, "じっちゅうはっ");
    assert_eq!(session.segments[0].reading_range, 0..7);
    assert_eq!(session.segments[1].reading, "く");
    assert_eq!(session.segments[1].reading_range, 7..8);
    assert!(!session.segments[0].candidates_dirty);
    assert!(session.segments[1].candidates_dirty);
    assert!(
        session
            .segments
            .iter()
            .all(|segment| !segment.explicitly_modified)
    );
    assert!(
        session
            .segments
            .iter()
            .all(|segment| { segment.candidates.selected_text() == Some(segment.selected_text()) })
    );
    assert!(session.ranges_are_valid());
}

#[test]
fn shift_right_merges_a_one_character_trailing_segment() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::ConversionSession;

    let mut engine = InputMethodEngine::new();
    engine.state = InputState::Conversion {
        session: ConversionSession::single(
            "じっちゅうはっく".into(),
            CandidateList::from_strings(["十中八九"]),
        ),
    };
    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    let shift_right = KeyEvent::new(Keysym::RIGHT, KeyModifiers::new().with_shift(true), true);

    engine.process_key(&shift_left);
    engine.process_key(&shift_right);
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };

    assert_eq!(session.segments.len(), 1);
    assert_eq!(session.segments[0].reading_range, 0..8);
    assert_eq!(session.selected_text(), "じっちゅうはっく");
    assert_eq!(session.preedit().text(), "じっちゅうはっく");
    assert_eq!(
        session.segments[0].candidates.selected_text(),
        Some(session.segments[0].selected_text())
    );
    assert!(session.ranges_are_valid());
}

#[test]
fn boundary_resize_preserves_surface_only_with_an_exact_alignment() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::{ConversionSegment, ConversionSession};
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
        Dictionary::build_from_normalized([entry("となり", "隣"), entry("きゃく", "客")]).unwrap();
    let segments = vec![
        ConversionSegment::new(
            0..4,
            "となりの".into(),
            CandidateList::from_strings(["隣の"]),
        ),
        ConversionSegment::new(4..7, "きゃく".into(), CandidateList::from_strings(["客"])),
    ];
    let mut engine = InputMethodEngine::new();
    engine.dicts.system = Some(dictionary);
    engine.state = InputState::Conversion {
        session: ConversionSession::segmented("となりのきゃく".into(), segments),
    };

    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    engine.process_key(&shift_left);
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };

    assert_eq!(session.selected_text(), "隣の客");
    assert_eq!(session.segments[0].reading, "となり");
    assert_eq!(session.segments[0].selected_text(), "隣");
    assert_eq!(session.segments[1].reading, "のきゃく");
    assert_eq!(session.segments[1].selected_text(), "の客");
    assert!(
        session
            .segments
            .iter()
            .all(|segment| { segment.candidates.selected_text() == Some(segment.selected_text()) })
    );
}

#[test]
fn boundary_resize_shows_the_same_candidate_list_used_by_first_navigation() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::ConversionSession;

    let mut engine = InputMethodEngine::new();
    engine.state = InputState::Conversion {
        session: ConversionSession::single(
            "じっちゅうはっく".into(),
            CandidateList::from_strings(["十中八九"]),
        ),
    };
    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    let resize = engine.process_key(&shift_left);
    let shown_after_resize = resize
        .actions
        .iter()
        .find_map(|action| match action {
            EngineAction::ShowCandidates(candidates) => Some(
                candidates
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.text.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap();

    let navigate = engine.process_key(&press_key(Keysym::SPACE));
    let shown_after_navigation = navigate
        .actions
        .iter()
        .find_map(|action| match action {
            EngineAction::ShowCandidates(candidates) => Some(
                candidates
                    .candidates()
                    .iter()
                    .map(|candidate| candidate.text.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap();
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };

    assert_eq!(shown_after_resize, shown_after_navigation);
    assert!(!session.segments[0].candidates_dirty);
    assert!(session.segments[0].candidates.len() > 1);
    assert!(session.segments[1].candidates_dirty);
    assert_eq!(
        session.segments[0].candidates.selected_text(),
        Some(session.segments[0].selected_text())
    );
}

#[test]
fn resizing_boundaries_does_not_enter_segment_learning() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::ConversionSession;

    let mut engine = InputMethodEngine::new();
    engine.segment_learning = Some(karukan_engine::SegmentLearningCache::new(100));
    engine.state = InputState::Conversion {
        session: ConversionSession::single(
            "じっちゅうはっく".into(),
            CandidateList::from_strings(["十中八九"]),
        ),
    };
    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    engine.process_key(&shift_left);
    engine.process_key(&press_key(Keysym::RETURN));

    let cache = engine.segment_learning.as_ref().unwrap();
    assert!(cache.lookup("じっちゅうはっ", None, Some("九")).is_empty());
    assert!(cache.lookup("く", Some("八"), None).is_empty());
}

#[test]
fn shift_left_splits_the_final_conversion_segment() {
    use crate::core::keycode::KeyModifiers;
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![
        ConversionSegment::new(0..3, "きょう".into(), CandidateList::from_strings(["今日"])),
        ConversionSegment::new(3..5, "いく".into(), CandidateList::from_strings(["行く"])),
    ];
    let mut session = ConversionSession::segmented("きょういく".into(), segments);
    session.active_segment = 1;
    session.rebuild_preedit();
    let mut engine = InputMethodEngine::new();
    engine.state = InputState::Conversion { session };

    let shift_left = KeyEvent::new(Keysym::LEFT, KeyModifiers::new().with_shift(true), true);
    engine.process_key(&shift_left);
    let InputState::Conversion { session } = engine.state() else {
        panic!("conversion state expected");
    };

    assert_eq!(session.segments.len(), 3);
    assert_eq!(session.segments[1].reading, "い");
    assert_eq!(session.segments[1].reading_range, 3..4);
    assert_eq!(session.segments[2].reading, "く");
    assert_eq!(session.segments[2].reading_range, 4..5);
    assert!(session.ranges_are_valid());
}

#[test]
fn explicitly_selected_segment_is_learned_with_right_context() {
    use crate::core::state::{ConversionSegment, ConversionSession};

    let segments = vec![
        ConversionSegment::new(
            0..2,
            "あと".into(),
            CandidateList::from_strings(["後", "あと"]),
        ),
        ConversionSegment::new(2..3, "、".into(), CandidateList::from_strings(["、"])),
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

    let segments = vec![ConversionSegment::new(
        0..2,
        "あと".into(),
        CandidateList::from_strings(["後", "あと"]),
    )];
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
