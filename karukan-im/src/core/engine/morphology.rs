//! Morphological segmentation of the live conversion surface.
//!
//! Partial conversion boundaries must come from the most likely converted
//! sentence, not from an independently segmented reading. Lindera's embedded
//! IPADIC provides MeCab-style surface tokens and their readings. We only use
//! the result when every token reading maps exactly and consecutively onto the
//! original composing reading; an uncertain mapping stays as one whole span.

use std::ops::Range;
use std::sync::OnceLock;

use lindera::dictionary::{DictionaryKind, load_dictionary_from_kind};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::tokenizer::Tokenizer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SurfaceSegment {
    pub reading_range: Range<usize>,
    pub reading: String,
    pub surface: String,
}

static TOKENIZER: OnceLock<Result<Tokenizer, String>> = OnceLock::new();

fn tokenizer() -> Option<&'static Tokenizer> {
    TOKENIZER
        .get_or_init(|| {
            let dictionary = load_dictionary_from_kind(DictionaryKind::IPADIC)
                .map_err(|error| error.to_string())?;
            let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
            Ok(Tokenizer::new(segmenter))
        })
        .as_ref()
        .ok()
}

/// Load the embedded dictionary during normal engine initialization so the
/// first transition into partial conversion does not pay the setup cost.
pub(super) fn warm_up() {
    let _ = tokenizer();
}

fn contains_kanji(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch,
            '\u{3400}'..='\u{4DBF}'
                | '\u{4E00}'..='\u{9FFF}'
                | '\u{F900}'..='\u{FAFF}'
                | '\u{20000}'..='\u{2FA1F}'
        )
    })
}

fn fallback_reading(surface: &str) -> Option<String> {
    if contains_kanji(surface) {
        None
    } else {
        Some(karukan_engine::katakana_to_hiragana(surface))
    }
}

/// Split the first (live) conversion surface and align every token reading to
/// the original hiragana buffer.
///
/// IPADIC's eighth detail field is the dictionary reading. Unknown kanji cannot
/// be mapped safely, while kana, punctuation, spaces, and Latin text can use
/// their surface as a lossless fallback. Any mismatch rejects the entire
/// analysis instead of manufacturing a boundary from character counts.
pub(super) fn segment_live_surface(surface: &str, reading: &str) -> Option<Vec<SurfaceSegment>> {
    if surface.is_empty() || reading.is_empty() {
        return None;
    }

    let tokenizer = tokenizer()?;
    let mut tokens = tokenizer.tokenize(surface).ok()?;
    if tokens.is_empty() {
        return None;
    }

    let reading_chars: Vec<char> = reading.chars().collect();
    let mut reading_start = 0;
    let mut segments = Vec::with_capacity(tokens.len());

    for token in &mut tokens {
        let surface = token.text.to_string();
        let dictionary_reading = token
            .get_detail(7)
            .filter(|value| !value.is_empty() && *value != "*" && *value != "UNK")
            .map(karukan_engine::katakana_to_hiragana);
        let token_reading = dictionary_reading.or_else(|| fallback_reading(&surface))?;
        let token_chars: Vec<char> = token_reading.chars().collect();
        let reading_end = reading_start + token_chars.len();

        if reading_end > reading_chars.len()
            || reading_chars[reading_start..reading_end] != token_chars
        {
            return None;
        }

        segments.push(SurfaceSegment {
            reading_range: reading_start..reading_end,
            reading: token_reading,
            surface,
        });
        reading_start = reading_end;
    }

    (reading_start == reading_chars.len()).then_some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_the_live_surface_with_ipadic_boundaries() {
        let segments =
            segment_live_surface("本来の用途以外では", "ほんらいのようといがいでは").unwrap();
        let surfaces: Vec<_> = segments
            .iter()
            .map(|segment| segment.surface.as_str())
            .collect();
        let readings: Vec<_> = segments
            .iter()
            .map(|segment| segment.reading.as_str())
            .collect();

        assert_eq!(surfaces, ["本来", "の", "用途", "以外", "で", "は"]);
        assert_eq!(readings, ["ほんらい", "の", "ようと", "いがい", "で", "は"]);
    }

    #[test]
    fn rejects_a_surface_whose_analysis_does_not_match_the_input_reading() {
        assert!(segment_live_surface("東京ミステリー駅", "とうきょうなぞえき").is_none());
    }

    #[test]
    fn preserves_non_japanese_runs_when_their_surface_matches() {
        let segments = segment_live_surface("今日は ChatGPT。", "きょうは ChatGPT。").unwrap();
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.surface.as_str())
                .collect::<Vec<_>>()
                .concat(),
            "今日は ChatGPT。"
        );
    }
}
