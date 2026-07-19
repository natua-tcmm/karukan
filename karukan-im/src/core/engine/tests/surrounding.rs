use super::*;

// --- Surrounding Text Tests ---

#[test]
fn test_surrounding_text_sets_context() {
    let mut engine = InputMethodEngine::new();
    engine.config.max_api_context_len = 50;

    engine.set_surrounding_context("エディタの文章", "");
    assert_eq!(
        engine.surrounding_context.as_ref().unwrap().left.as_deref(),
        Some("エディタの文章")
    );
}

#[test]
fn test_surrounding_text_overwrites_context() {
    let mut engine = InputMethodEngine::new();
    engine.config.max_api_context_len = 50;

    // First, set some internal context (without surrounding text)
    engine.surrounding_context = Some(SurroundingContext {
        left: Some("古い内部文脈".to_string()),
        right: None,
    });

    // Now set surrounding text - should completely overwrite
    engine.set_surrounding_context("エディタからの新しい文脈", "");

    let left = engine
        .surrounding_context
        .as_ref()
        .unwrap()
        .left
        .as_deref()
        .unwrap();
    assert_eq!(left, "エディタからの新しい文脈");
    assert!(!left.contains("古い"));
}

#[test]
fn test_surrounding_text_multiple_updates() {
    let mut engine = InputMethodEngine::new();

    // Simulate multiple key events with surrounding text updates
    engine.set_surrounding_context("最初の文脈", "");
    assert_eq!(
        engine.surrounding_context.as_ref().unwrap().left.as_deref(),
        Some("最初の文脈")
    );

    // User types, editor updates surrounding text
    engine.set_surrounding_context("最初の文脈あ", "");
    assert_eq!(
        engine.surrounding_context.as_ref().unwrap().left.as_deref(),
        Some("最初の文脈あ")
    );

    // User commits, editor updates again
    engine.set_surrounding_context("最初の文脈あい", "");
    let left = engine
        .surrounding_context
        .as_ref()
        .unwrap()
        .left
        .as_deref()
        .unwrap();
    assert_eq!(left, "最初の文脈あい");

    // No garbage from internal tracking
    assert!(!left.contains("古い"));
}

#[test]
fn test_surrounding_text_respects_max_length() {
    let mut engine = InputMethodEngine::new();
    engine.config.max_api_context_len = 10;

    // Use a string longer than max_api_context_len
    let long_text = "あ".repeat(20);
    engine.set_surrounding_context(&long_text, "");

    // Should be truncated to last 10 chars
    assert_eq!(
        engine
            .surrounding_context
            .as_ref()
            .unwrap()
            .left
            .as_ref()
            .unwrap()
            .chars()
            .count(),
        10
    );
}

#[test]
fn test_reset_clears_all_state() {
    let mut engine = InputMethodEngine::new();

    // Set up various state
    engine.surrounding_context = Some(SurroundingContext {
        left: Some("文脈テキスト".to_string()),
        right: Some("右側テキスト".to_string()),
    });

    // Type something to change state
    engine.process_key(&press('a'));

    // Reset
    engine.reset();

    // State should be cleared, but surrounding_context is intentionally preserved
    // (it is set once at activate time and persists through the session)
    assert!(engine.surrounding_context.is_some());
    assert!(matches!(engine.state(), InputState::Empty));
}

// --- Surrounding Context (Left/Right) Tests ---

#[test]
fn test_set_surrounding_context_both() {
    let mut engine = InputMethodEngine::new();

    engine.set_surrounding_context("左側テキスト", "右側テキスト");

    let ctx = engine.surrounding_context.as_ref().unwrap();
    assert_eq!(ctx.left.as_deref(), Some("左側テキスト"));
    assert_eq!(ctx.right.as_deref(), Some("右側テキスト"));
}

#[test]
fn test_set_surrounding_context_left_only() {
    let mut engine = InputMethodEngine::new();

    engine.set_surrounding_context("左側のみ", "");

    let ctx = engine.surrounding_context.as_ref().unwrap();
    assert_eq!(ctx.left.as_deref(), Some("左側のみ"));
    assert!(ctx.right.is_none());
}

#[test]
fn test_set_surrounding_context_right_only() {
    let mut engine = InputMethodEngine::new();

    engine.set_surrounding_context("", "右側のみ");

    let ctx = engine.surrounding_context.as_ref().unwrap();
    assert!(ctx.left.is_none());
    assert_eq!(ctx.right.as_deref(), Some("右側のみ"));
}

#[test]
fn test_set_surrounding_context_truncation() {
    let mut engine = InputMethodEngine::new();
    engine.config.max_api_context_len = 5;

    // Use strings longer than max_api_context_len
    engine.set_surrounding_context("左側が長すぎるテキスト", "右側が長すぎるテキスト");

    let ctx = engine.surrounding_context.as_ref().unwrap();

    // Left: keep last 5 chars
    let left = ctx.left.as_ref().unwrap();
    assert_eq!(left.chars().count(), 5);
    assert!(left.contains("テキスト")); // last part

    // Right: keep first 5 chars
    let right = ctx.right.as_ref().unwrap();
    assert_eq!(right.chars().count(), 5);
    assert!(right.contains("右側が")); // first part
}

#[test]
fn test_aux_text_hides_surrounding_context() {
    let mut engine = InputMethodEngine::new();
    engine.set_surrounding_context("左側", "右側");

    engine.process_key(&press('a'));
    let aux = engine.format_aux_composing();

    assert_eq!(aux, "あ");
    assert!(!aux.contains("lctx:"), "aux was: {aux}");
    assert!(!aux.contains("rctx:"), "aux was: {aux}");
    assert!(!aux.contains("左側"), "aux was: {aux}");
    assert!(!aux.contains("右側"), "aux was: {aux}");
}
