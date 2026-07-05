use super::*;

// --- One-shot kana form conversion tests ---
//
// Persistent Ctrl+K katakana mode is intentionally not bound anymore.  F6/F7/F8
// provide immediate commit variants for the current composing text instead.

fn committed_text(result: &EngineResult) -> Option<&str> {
    result.actions.iter().find_map(|a| match a {
        EngineAction::Commit(text) => Some(text.as_str()),
        _ => None,
    })
}

#[test]
fn f6_commits_hiragana_immediately() {
    let mut engine = InputMethodEngine::new();

    for ch in "aiueo".chars() {
        engine.process_key(&press(ch));
    }
    assert_eq!(engine.preedit().unwrap().text(), "あいうえお");

    let result = engine.process_key(&press_key(Keysym::F6));
    assert!(result.consumed);
    assert_eq!(committed_text(&result), Some("あいうえお"));
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn f7_commits_full_width_katakana_immediately() {
    let mut engine = InputMethodEngine::new();

    for ch in "aiueo".chars() {
        engine.process_key(&press(ch));
    }

    let result = engine.process_key(&press_key(Keysym::F7));
    assert!(result.consumed);
    assert_eq!(committed_text(&result), Some("アイウエオ"));
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn f8_commits_half_width_katakana_immediately() {
    let mut engine = InputMethodEngine::new();

    for ch in "gaxtukou".chars() {
        engine.process_key(&press(ch));
    }
    assert_eq!(engine.preedit().unwrap().text(), "がっこう");

    let result = engine.process_key(&press_key(Keysym::F8));
    assert!(result.consumed);
    assert_eq!(committed_text(&result), Some("ｶﾞｯｺｳ"));
    assert!(matches!(engine.state(), InputState::Empty));
}

#[test]
fn function_keys_commit_single_buffered_consonant() {
    for (key, expected) in [(Keysym::F6, "k"), (Keysym::F7, "k"), (Keysym::F8, "k")] {
        let mut engine = InputMethodEngine::new();
        engine.process_key(&press('k'));
        assert_eq!(engine.preedit().unwrap().text(), "k");

        let result = engine.process_key(&press_key(key));
        assert!(result.consumed);
        assert_eq!(committed_text(&result), Some(expected));
        assert!(matches!(engine.state(), InputState::Empty));
    }
}

#[test]
fn ctrl_k_is_not_a_katakana_shortcut() {
    let mut engine = InputMethodEngine::new();

    engine.process_key(&press('a'));
    assert_eq!(engine.preedit().unwrap().text(), "あ");

    let result = engine.process_key(&press_ctrl(Keysym::KEY_K));
    assert!(!result.consumed);
    assert_eq!(engine.input_mode, InputMode::Hiragana);
    assert_eq!(engine.preedit().unwrap().text(), "あ");
}
