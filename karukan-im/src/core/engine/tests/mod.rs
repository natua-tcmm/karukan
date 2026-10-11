//! Tests for the IME engine

use super::*;
use crate::core::keycode::KeyModifiers;

mod alphabet;
mod basic;
mod candidates;
mod chunks;
mod conversion;
mod cursor;
mod emoji;
mod katakana;
mod learning;
mod live_conversion;
mod mode_toggle;
mod passthrough;
mod rewriter;
mod segmentation;
mod surrounding;

fn press(ch: char) -> KeyEvent {
    KeyEvent::press(Keysym(ch as u32))
}

fn press_key(keysym: Keysym) -> KeyEvent {
    KeyEvent::press(keysym)
}

fn release_key(keysym: Keysym) -> KeyEvent {
    KeyEvent::new(keysym, KeyModifiers::default(), false)
}

fn press_shift(ch: char) -> KeyEvent {
    KeyEvent::new(
        Keysym(ch as u32),
        KeyModifiers::new().with_shift(true),
        true,
    )
}

fn press_ctrl(keysym: Keysym) -> KeyEvent {
    KeyEvent::new(keysym, KeyModifiers::new().with_control(true), true)
}

fn press_ctrl_shift(keysym: Keysym) -> KeyEvent {
    KeyEvent::new(
        keysym,
        KeyModifiers::new().with_control(true).with_shift(true),
        true,
    )
}

fn make_live_conversion_engine() -> InputMethodEngine {
    InputMethodEngine::new()
}

// Wait for asynchronous boundaries only in tests that inspect the final session.
fn finish_segment_preparation(engine: &mut InputMethodEngine) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while engine.segmentation.entering {
        engine.poll_live_conversion();
        assert!(
            std::time::Instant::now() < deadline,
            "morphology preparation timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
