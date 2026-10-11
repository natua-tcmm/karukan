use super::*;
use crate::core::engine::morphology::SurfaceSegment;
use crate::core::engine::segmentation::{PreparedSegments, SegmentationWorker};

fn live_engine() -> InputMethodEngine {
    let mut engine = InputMethodEngine::new();
    engine.input_buf.text = "とうきょうえき".into();
    engine.input_buf.cursor_pos = 7;
    engine.state = InputState::Composing {
        preedit: Preedit::with_text("東京駅"),
        romaji_buffer: String::new(),
    };
    engine
}

fn parts() -> Vec<SurfaceSegment> {
    vec![
        SurfaceSegment {
            reading_range: 0..5,
            reading: "とうきょう".into(),
            surface: "東京".into(),
        },
        SurfaceSegment {
            reading_range: 5..7,
            reading: "えき".into(),
            surface: "駅".into(),
        },
    ]
}

#[test]
fn completed_live_result_starts_speculative_morphology() {
    let mut engine = live_engine();
    let (worker, requests, _) = SegmentationWorker::controlled();
    engine.segmentation.worker = Some(worker);
    engine.apply_background_candidates(
        "とうきょうえき".into(),
        "とうきょうえき".into(),
        vec!["東京駅".into(), "東京驛".into()],
    );
    assert!(engine.live_conversion_finished("とうきょうえき"));
    let request = requests.try_recv().unwrap();
    assert_eq!(request.surface, "東京駅");
    assert_eq!(request.reading, "とうきょうえき");
    assert!(!engine.segmentation.entering);
}

#[test]
fn navigating_alternatives_prepares_first_candidate_not_selection() {
    let mut engine = live_engine();
    let (worker, requests, _) = SegmentationWorker::controlled();
    engine.segmentation.worker = Some(worker);
    let mut candidates = CandidateList::from_strings(["東京駅", "東京驛"]);
    candidates.select(1);
    engine.update_composing_candidate_selection(candidates);
    assert_eq!(engine.preedit().unwrap().text(), "東京驛");
    assert_eq!(requests.try_recv().unwrap().surface, "東京駅");
}

#[test]
fn entering_correction_never_waits_and_keeps_first_surface_after_result() {
    let mut engine = live_engine();
    let (worker, requests, results) = SegmentationWorker::controlled();
    engine.segmentation.worker = Some(worker);
    let mut candidates = CandidateList::from_strings(["東京駅", "東京驛"]);
    candidates.select(1);
    engine.composing_candidates = Some(candidates);
    engine.composing_candidate_selected = true;
    engine.process_key(&press_key(Keysym::SPACE));
    let snapshot = requests.try_recv().unwrap();
    assert!(engine.segmentation.entering);
    assert_eq!(engine.preedit().unwrap().text(), "東京駅");

    // No response exists yet: all navigation stays consumed and cannot leak
    // an AppKit function-key scalar into the editor.
    for key in [
        Keysym::SPACE,
        Keysym::TAB,
        Keysym::LEFT,
        Keysym::RIGHT,
        Keysym::UP,
        Keysym::DOWN,
    ] {
        let result = engine.process_key(&press_key(key));
        assert!(result.consumed);
        assert!(
            !result
                .actions
                .iter()
                .any(|action| matches!(action, EngineAction::Commit(_)))
        );
        assert_eq!(engine.preedit().unwrap().text(), "東京駅");
    }
    results
        .send(PreparedSegments {
            snapshot,
            segments: parts(),
        })
        .unwrap();
    engine.poll_live_conversion();
    assert!(!engine.segmentation.entering);
    let InputState::Conversion { session } = &engine.state else {
        panic!()
    };
    assert_eq!(session.selected_text(), "東京駅");
    assert_eq!(session.segments.len(), 2);
    for (segment, surface) in session.segments.iter().zip(["東京", "駅"]) {
        assert_eq!(segment.candidates.candidates()[0].text, surface);
        assert_eq!(segment.selected_text(), surface);
    }
}

#[test]
fn prepared_boundaries_are_installed_immediately_on_entry() {
    let mut engine = live_engine();
    let (worker, requests, results) = SegmentationWorker::controlled();
    engine.segmentation.worker = Some(worker);
    engine.prepare_segmentation("とうきょうえき", "東京駅");
    let snapshot = requests.try_recv().unwrap();
    results
        .send(PreparedSegments {
            snapshot,
            segments: parts(),
        })
        .unwrap();
    engine.poll_live_conversion();
    engine.composing_candidates = Some(CandidateList::from_strings(["東京駅"]));
    engine.start_segmented_conversion_from_composing();
    assert!(!engine.segmentation.entering);
    let InputState::Conversion { session } = &engine.state else {
        panic!()
    };
    assert_eq!(session.segments.len(), 2);
    assert_eq!(session.selected_text(), "東京駅");
}

#[test]
fn escape_restores_live_first_and_its_alternatives_after_segment_correction() {
    let mut engine = live_engine();
    let (worker, requests, results) = SegmentationWorker::controlled();
    engine.segmentation.worker = Some(worker);
    engine.prepare_segmentation("とうきょうえき", "東京駅");
    let snapshot = requests.try_recv().unwrap();
    results
        .send(PreparedSegments {
            snapshot,
            segments: parts(),
        })
        .unwrap();
    engine.poll_live_conversion();
    engine.composing_candidates = Some(CandidateList::from_strings(["東京駅", "東京驛"]));
    engine.start_segmented_conversion_from_composing();
    engine.process_key(&press_key(Keysym::SPACE));
    let result = engine.process_key(&press_key(Keysym::ESCAPE));
    assert!(result.consumed);
    assert!(matches!(engine.state, InputState::Composing { .. }));
    assert_eq!(engine.preedit().unwrap().text(), "東京駅");
    assert_eq!(engine.live.text, "東京駅");
    assert_eq!(
        engine.composing_candidates.as_ref().unwrap().candidates()[1].text,
        "東京驛"
    );
    assert!(!engine.composing_candidate_selected);
}

#[test]
fn obsolete_analysis_cannot_change_a_new_composition() {
    let mut engine = live_engine();
    let (worker, requests, results) = SegmentationWorker::controlled();
    engine.segmentation.worker = Some(worker);
    engine.prepare_segmentation("とうきょうえき", "東京駅");
    let old = requests.try_recv().unwrap();
    engine.reset();
    engine.process_key(&press('a'));
    results
        .send(PreparedSegments {
            snapshot: old,
            segments: parts(),
        })
        .unwrap();
    engine.poll_live_conversion();
    assert_eq!(engine.preedit().unwrap().text(), "あ");
    assert!(engine.segmentation.prepared.is_none());
}

#[test]
fn background_segment_candidates_preserve_original_first_and_current_selection() {
    let mut engine = live_engine();
    let mut session = engine.build_initial_conversion_session(
        "とうきょうえき",
        CandidateList::from_strings(["東京駅"]),
    );
    session.segments[0].candidates = CandidateList::from_strings(["東京", "とうきょう"]);
    session.segments[0].candidates.select(1);
    session.segments[0].sync_selected_surface();
    engine.state = InputState::Conversion { session };
    let (sender, response) = std::sync::mpsc::channel();
    engine
        .pending_segment_models
        .push(crate::core::engine::conversion::PendingSegmentModel {
            reading: "とうきょう".into(),
            range: 0..5,
            session_reading: "とうきょうえき".into(),
            response,
        });
    sender
        .send(vec![karukan_engine::ModelCandidate {
            text: "東亰".into(),
            score: None,
        }])
        .unwrap();
    engine.poll_live_conversion();
    let InputState::Conversion { session } = &engine.state else {
        panic!()
    };
    assert_eq!(session.segments[0].candidates.candidates()[0].text, "東京");
    assert_eq!(session.segments[0].selected_text(), "とうきょう");
    assert!(
        session.segments[0]
            .candidates
            .candidates()
            .iter()
            .any(|candidate| candidate.text == "東亰")
    );
}

#[test]
fn removed_shortcuts_do_not_modify_any_input_state() {
    let mut engine = live_engine();
    for conversion in [false, true] {
        if conversion {
            engine.state = InputState::Conversion {
                session: crate::core::state::ConversionSession::single(
                    "とうきょうえき".into(),
                    CandidateList::from_strings(["東京駅", "東京驛"]),
                ),
            };
        }
        let before = engine.preedit().unwrap().text().to_string();
        for key in [Keysym::PAGE_UP, Keysym::PAGE_DOWN, Keysym::F9, Keysym::F10] {
            assert!(!engine.process_key(&press_key(key)).consumed);
            assert_eq!(engine.preedit().unwrap().text(), before);
        }
        for key in [
            Keysym::SPACE,
            Keysym::KEY_A,
            Keysym::KEY_B,
            Keysym::KEY_E,
            Keysym::KEY_F,
            Keysym::KEY_N,
            Keysym::KEY_P,
            Keysym::KEY_1,
        ] {
            assert!(!engine.process_key(&press_ctrl(key)).consumed);
            assert_eq!(engine.preedit().unwrap().text(), before);
        }
    }
}

#[test]
fn shift_space_commits_fullwidth_space_during_conversion() {
    let mut engine = live_engine();
    engine.state = InputState::Conversion {
        session: crate::core::state::ConversionSession::single(
            "とうきょうえき".into(),
            CandidateList::from_strings(["東京駅"]),
        ),
    };
    let result = engine.process_key(&press_shift(' '));
    let committed: String = result
        .actions
        .iter()
        .filter_map(|action| {
            if let EngineAction::Commit(text) = action {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(committed, "東京駅　");
}

#[test]
fn leading_prolonged_marks_remain_literal_in_explicit_candidates() {
    let mut engine = InputMethodEngine::new();
    let candidates = engine.build_conversion_candidates("ーーすーぱー", 9, false);
    assert!(!candidates.is_empty());
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.text.starts_with("ーー"))
    );
    let candidates = engine.build_conversion_candidates("ーー", 9, false);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].text, "ーー");
}

#[test]
fn raw_private_use_keysyms_never_enter_preedit() {
    let mut engine = live_engine();
    let before = engine.preedit().unwrap().text().to_string();
    for value in [0x01, 0x1b, 0xee00, 0xee02, 0xf700, 0xf701, 0xf702, 0xf703] {
        let result = engine.process_key(&press_key(Keysym(value)));
        assert!(!result.consumed);
        assert_eq!(engine.preedit().unwrap().text(), before);
        assert!(
            !result
                .actions
                .iter()
                .any(|action| matches!(action, EngineAction::Commit(_)))
        );
    }
}
