use super::*;
use std::io::Write;

use karukan_engine::dictionary_source::NormalizedDictionaryEntry;
use karukan_engine::{Dictionary, SegmentLearningCache};
use karukan_engine::{DictionaryCategory, DictionarySource};

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
    let mut cache = SegmentLearningCache::new(100);
    cache.record(reading, surface, None, None);
    engine.segment_learning = Some(cache);
}

fn user_dict_with(reading: &str, surface: &str) -> Dictionary {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    let json = format!(
        r#"[{{"reading":"{reading}","candidates":[{{"surface":"{surface}","score":1.0}}]}}]"#
    );
    tmp.write_all(json.as_bytes()).unwrap();
    tmp.flush().unwrap();
    Dictionary::build_from_json(tmp.path()).unwrap()
}

fn user_dict_with_entries(entries: &[(&str, &str, f32)]) -> Dictionary {
    Dictionary::build_from_normalized(entries.iter().map(|(reading, surface, score)| {
        NormalizedDictionaryEntry::new(
            reading,
            surface,
            *score,
            DictionarySource::User,
            DictionaryCategory::General,
            None,
        )
        .unwrap()
    }))
    .unwrap()
}

fn set_composing_reading(engine: &mut InputMethodEngine, reading: &str) {
    engine.input_buf.text = reading.to_string();
    engine.input_buf.cursor_pos = reading.chars().count();
    engine.state = InputState::Composing {
        preedit: Preedit::with_text(reading),
        romaji_buffer: String::new(),
    };
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
        kind: ComposingChunkKind::Model,
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
fn short_live_conversion_has_hiragana_and_katakana_in_dedicated_slots() {
    let mut engine = make_live_conversion_engine();
    learn(&mut engine, "しよう", "私用");
    engine.input_buf.text = "しよう".to_string();
    engine.input_buf.cursor_pos = 3;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("しよう"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        kind: ComposingChunkKind::Model,
        reading: "しよう".to_string(),
        converted: "使用".to_string(),
        candidates: vec!["使用".to_string(), "仕様".to_string(), "しよう".to_string()],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.preedit().unwrap().text(), "使用");
    assert_eq!(candidates.first().map(String::as_str), Some("使用"));
    assert_eq!(candidates.len(), SHORT_LIVE_CANDIDATE_LIMIT);
    assert_eq!(
        candidates.get(1).map(String::as_str),
        Some("しよう"),
        "short live conversion must reserve candidate 2 for hiragana"
    );
    assert_eq!(
        candidates.get(2).map(String::as_str),
        Some("シヨウ"),
        "short live conversion must reserve candidate 3 for katakana"
    );
    assert!(candidates.iter().any(|candidate| candidate == "私用"));
    assert_eq!(
        engine.composing_candidates.as_ref().unwrap().candidates()[1]
            .description
            .as_deref(),
        Some("[全]ひらがな")
    );
    assert_eq!(
        engine.composing_candidates.as_ref().unwrap().candidates()[2]
            .description
            .as_deref(),
        Some("[全]カタカナ")
    );
}

#[test]
fn five_character_live_conversion_has_dedicated_kana_candidates() {
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = "あいうえお".to_string();
    engine.input_buf.cursor_pos = 5;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("あいうえお"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        kind: ComposingChunkKind::Model,
        reading: "あいうえお".to_string(),
        converted: "相上尾".to_string(),
        candidates: vec![
            "相上尾".to_string(),
            "藍植尾".to_string(),
            "愛上緒".to_string(),
        ],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(candidates.len(), SHORT_LIVE_CANDIDATE_LIMIT);
    assert_eq!(candidates.get(1).map(String::as_str), Some("あいうえお"));
    assert_eq!(candidates.get(2).map(String::as_str), Some("アイウエオ"));
}

#[test]
fn six_character_live_conversion_has_dedicated_kana_candidates() {
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = "あいうえおか".to_string();
    engine.input_buf.cursor_pos = 6;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("あいうえおか"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        kind: ComposingChunkKind::Model,
        reading: "あいうえおか".to_string(),
        converted: "相上丘".to_string(),
        candidates: vec![
            "相上丘".to_string(),
            "藍上丘".to_string(),
            "愛植岡".to_string(),
        ],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(candidates.len(), SHORT_LIVE_CANDIDATE_LIMIT);
    assert_eq!(candidates.get(1).map(String::as_str), Some("あいうえおか"));
    assert_eq!(candidates.get(2).map(String::as_str), Some("アイウエオカ"));
}

#[test]
fn seven_character_live_conversion_keeps_three_whole_candidates() {
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = "あいうえおかき".to_string();
    engine.input_buf.cursor_pos = 7;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("あいうえおかき"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        kind: ComposingChunkKind::Model,
        reading: "あいうえおかき".to_string(),
        converted: "相上尾下記".to_string(),
        candidates: vec![
            "相上尾下記".to_string(),
            "藍植尾夏季".to_string(),
            "愛上緒花器".to_string(),
        ],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(candidates.len(), WHOLE_CANDIDATE_LIMIT);
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate == "あいうえおかき")
    );
}

#[test]
fn short_live_conversion_keeps_only_the_dedicated_katakana_candidate() {
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = "しよう".to_string();
    engine.input_buf.cursor_pos = 3;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("しよう"),
        romaji_buffer: String::new(),
    };
    engine.chunks = vec![ComposingChunk {
        kind: ComposingChunkKind::Model,
        reading: "しよう".to_string(),
        converted: "使用".to_string(),
        candidates: vec![
            "使用".to_string(),
            "シヨウ".to_string(),
            "しヨう".to_string(),
            "仕様".to_string(),
            "私用".to_string(),
            "ｼﾖｳ".to_string(),
        ],
    }];

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(candidates, ["使用", "しよう", "シヨウ", "仕様", "私用"]);
}

#[test]
fn filtered_live_surface_matches_candidate_one_and_space_reuses_the_list() {
    let reading = "して";
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = reading.to_string();
    engine.input_buf.cursor_pos = reading.chars().count();
    engine.state = InputState::Composing {
        preedit: Preedit::with_text(reading),
        romaji_buffer: String::new(),
    };

    let result = engine.apply_background_candidates(
        reading.to_string(),
        reading.to_string(),
        vec![
            "シて".to_string(),
            "仕手".to_string(),
            "し手".to_string(),
            "為手".to_string(),
        ],
    );
    let before = shown_candidate_texts(&result);

    assert_eq!(before, ["仕手", "して", "シテ", "し手", "為手"]);
    assert_eq!(engine.live.text, "仕手");
    assert_eq!(engine.preedit().map(Preedit::text), Some("仕手"));

    engine.process_key(&press_key(Keysym::SPACE));
    let after = engine.state().candidates().unwrap();

    assert_eq!(candidate_texts(after), before);
    assert_eq!(after.cursor(), 1);
    assert_eq!(after.selected_text(), Some("して"));
}

#[test]
fn filtered_background_prefix_keeps_newer_suffix_pending() {
    let mut engine = make_live_conversion_engine();
    engine.input_buf.text = "している".to_string();
    engine.input_buf.cursor_pos = 4;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("している"),
        romaji_buffer: String::new(),
    };

    engine.apply_background_candidates(
        "して".to_string(),
        "している".to_string(),
        vec!["シて".to_string(), "仕手".to_string(), "し手".to_string()],
    );

    assert_eq!(engine.live.applied_reading, "して");
    assert_eq!(engine.live.applied_text, "仕手");
    assert_eq!(engine.live.text, "仕手いる");
    assert_eq!(engine.preedit().map(Preedit::text), Some("仕手いる"));
}

#[test]
fn punctuation_does_not_disable_single_hiragana_priority() {
    for reading in ["し、", "し。", "し〜", "し～"] {
        let suffix: String = reading.chars().skip(1).collect();
        let kanji_candidate = format!("詩{suffix}");
        let katakana_candidate = karukan_engine::hiragana_to_katakana(reading);
        let mut engine = make_live_conversion_engine();
        engine.input_buf.text = reading.to_string();
        engine.input_buf.cursor_pos = reading.chars().count();
        engine.state = InputState::Composing {
            preedit: Preedit::with_text(reading),
            romaji_buffer: String::new(),
        };
        engine.chunks = vec![ComposingChunk {
            kind: ComposingChunkKind::Model,
            reading: reading.to_string(),
            converted: kanji_candidate.clone(),
            candidates: vec![kanji_candidate, katakana_candidate.clone()],
        }];

        let result = engine.refresh_input_state();
        let candidates = shown_candidate_texts(&result);

        assert_eq!(engine.live.text, reading);
        assert_eq!(engine.preedit().unwrap().text(), reading);
        assert_eq!(candidates.first().map(String::as_str), Some(reading));
        assert!(!candidates.contains(&katakana_candidate));
    }
}

#[test]
fn exact_user_dictionary_entry_becomes_live_text_without_model() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("かるかん", "karukan"));
    engine.input_buf.text = "かるかん".to_string();
    engine.input_buf.cursor_pos = 4;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("かるかん"),
        romaji_buffer: String::new(),
    };

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.live.text, "karukan");
    assert_eq!(engine.preedit().map(Preedit::text), Some("karukan"));
    assert_eq!(candidates.first().map(String::as_str), Some("karukan"));
}

#[test]
fn embedded_user_dictionary_entry_is_pinned_without_model() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("かるかん", "karukan"));
    set_composing_reading(&mut engine, "かるかんをつかう");

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.live.text, "karukanをつかう");
    assert_eq!(engine.preedit().map(Preedit::text), Some("karukanをつかう"));
    assert_eq!(
        candidates.first().map(String::as_str),
        Some("karukanをつかう")
    );
    assert_eq!(engine.chunks[0].kind, ComposingChunkKind::UserDictionary);
    assert_eq!(engine.chunks[0].converted, "karukan");
}

#[test]
fn multiple_embedded_entries_survive_digits_and_punctuation() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with_entries(&[
        ("かるかん", "karukan", 0.0),
        ("にこにこ", "ニコニコ", 0.0),
    ]));
    set_composing_reading(&mut engine, "かるかん123、にこにこ");

    engine.refresh_input_state();

    assert_eq!(engine.live.text, "karukan123、ニコニコ");
    assert_eq!(
        engine
            .chunks
            .iter()
            .filter(|chunk| chunk.kind == ComposingChunkKind::UserDictionary)
            .count(),
        2
    );
    assert!(engine.chunks.iter().any(|chunk| {
        chunk.kind == ComposingChunkKind::Passthrough && chunk.reading == "123、"
    }));
}

#[test]
fn embedded_user_dictionary_alternatives_become_whole_candidates() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with_entries(&[
        ("かるかん", "karukan", 0.0),
        ("かるかん", "軽羹", 1.0),
    ]));
    set_composing_reading(&mut engine, "かるかんを");

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(candidates.first().map(String::as_str), Some("karukanを"));
    assert!(candidates.iter().any(|candidate| candidate == "軽羹を"));
}

#[test]
fn exact_katakana_user_surface_survives_short_candidate_filtering() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("にこにこ", "ニコニコ"));
    set_composing_reading(&mut engine, "にこにこ");

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.live.text, "ニコニコ");
    assert_eq!(candidates.first().map(String::as_str), Some("ニコニコ"));
}

#[test]
fn one_character_user_entry_is_ignored_inside_a_longer_reading() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("あ", "亜"));
    set_composing_reading(&mut engine, "あい");

    engine.refresh_input_state();

    assert_ne!(engine.live.text, "亜い");
    assert!(
        engine
            .chunks
            .iter()
            .all(|chunk| chunk.kind != ComposingChunkKind::UserDictionary)
    );
}

#[test]
fn embedded_zu_du_correction_uses_user_dictionary_surface() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("つづく", "続く"));
    set_composing_reading(&mut engine, "つずくよ");

    engine.refresh_input_state();

    assert_eq!(engine.live.text, "続くよ");
    assert_eq!(engine.chunks[0].reading, "つずく");
    assert_eq!(engine.chunks[0].kind, ComposingChunkKind::UserDictionary);
}

#[test]
fn user_entry_can_replace_model_chunks_across_the_length_cap() {
    let mut engine = make_live_conversion_engine();
    engine.config.composing_chunk_len = 2;
    engine.dicts.user = Some(user_dict_with("かるかん", "karukan"));
    set_composing_reading(&mut engine, "かるか");
    engine.refresh_input_state();
    assert_eq!(
        engine
            .chunks
            .iter()
            .map(|chunk| chunk.reading.as_str())
            .collect::<Vec<_>>(),
        ["かる", "か"]
    );

    set_composing_reading(&mut engine, "かるかん");
    engine.refresh_input_state();

    assert_eq!(engine.live.text, "karukan");
    assert_eq!(engine.chunks.len(), 1);
    assert_eq!(engine.chunks[0].reading, "かるかん");
    assert_eq!(engine.chunks[0].kind, ComposingChunkKind::UserDictionary);
}

#[test]
fn stale_model_prefix_cannot_override_a_new_dictionary_span() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("かるかん", "karukan"));
    set_composing_reading(&mut engine, "かるかんを");
    engine.chunks = vec![ComposingChunk {
        reading: "かるかん".to_string(),
        converted: "軽羹".to_string(),
        candidates: vec!["軽羹".to_string()],
        kind: ComposingChunkKind::Model,
    }];

    engine.apply_background_candidates(
        "かるかん".to_string(),
        "かるかんを".to_string(),
        vec!["軽羹".to_string()],
    );

    assert_eq!(engine.live.text, "karukanを");
    assert_eq!(engine.preedit().map(Preedit::text), Some("karukanを"));
}

#[test]
fn zu_du_typo_finds_dictionary_candidate_and_marks_the_corrected_reading() {
    let mut engine = InputMethodEngine::new();
    engine.dicts.system = Some(user_dict_with("きづく", "気付く"));

    let candidates = engine.lookup_dict_candidates("きずく");

    assert_eq!(
        candidates.first().map(|candidate| candidate.text.as_str()),
        Some("気付く")
    );
    assert_eq!(
        candidates
            .first()
            .and_then(|candidate| candidate.description.as_deref()),
        Some("ず・づ補正: きづく")
    );
}

#[test]
fn zu_du_typo_can_use_an_exact_user_dictionary_entry_for_live_conversion() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("つづく", "続く"));
    engine.input_buf.text = "つずく".to_string();
    engine.input_buf.cursor_pos = 3;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("つずく"),
        romaji_buffer: String::new(),
    };

    let result = engine.refresh_input_state();
    let candidates = shown_candidate_texts(&result);

    assert_eq!(engine.live.text, "続く");
    assert_eq!(engine.preedit().map(Preedit::text), Some("続く"));
    assert_eq!(candidates.first().map(String::as_str), Some("続く"));
}

#[test]
fn background_model_result_does_not_replace_exact_user_dictionary_live_text() {
    let mut engine = make_live_conversion_engine();
    engine.dicts.user = Some(user_dict_with("かるかん", "karukan"));
    engine.input_buf.text = "かるかん".to_string();
    engine.input_buf.cursor_pos = 4;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("かるかん"),
        romaji_buffer: String::new(),
    };

    engine.apply_background_candidates(
        "かるかん".to_string(),
        "かるかん".to_string(),
        vec!["軽羹".to_string()],
    );

    assert_eq!(engine.live.text, "karukan");
    assert_eq!(engine.preedit().map(Preedit::text), Some("karukan"));
    assert_eq!(
        engine
            .composing_candidates
            .as_ref()
            .and_then(CandidateList::selected_text),
        Some("karukan")
    );
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
    assert!(
        candidates
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "あい")
    );
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
        kind: ComposingChunkKind::Model,
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
fn space_tab_and_down_start_from_the_same_second_candidate() {
    for keysym in [Keysym::SPACE, Keysym::TAB, Keysym::DOWN] {
        let reading = "しよう";
        let candidates = ["使用", "仕様", "私用"];
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

        let result = engine.process_key(&press_key(keysym));

        assert!(result.consumed, "keysym={keysym:?}");
        assert!(
            matches!(engine.state(), InputState::Conversion { .. }),
            "keysym={keysym:?}"
        );
        let selected = engine.state().candidates().unwrap();
        assert_eq!(selected.cursor(), 1, "keysym={keysym:?}");
        assert_eq!(
            selected.selected_text(),
            Some(candidates[1]),
            "keysym={keysym:?}"
        );
        assert_eq!(
            engine.preedit().map(Preedit::text),
            Some(candidates[1]),
            "keysym={keysym:?}"
        );
    }
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
