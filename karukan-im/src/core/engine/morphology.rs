//! Morphological segmentation of the live conversion surface.
//!
//! Partial conversion boundaries must come from the most likely converted
//! sentence, not from an independently segmented reading. Lindera's embedded
//! IPADIC provides MeCab-style surface tokens and their readings. Exact token
//! readings anchor the original composing reading; only mismatched tokens get
//! an approximate reading span.

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

#[derive(Debug)]
struct AnalyzedToken {
    surface: String,
    reading: Option<Vec<char>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct AlignmentScore {
    mismatched_tokens: usize,
    edit_distance: usize,
    length_delta: usize,
}

impl AlignmentScore {
    fn add(self, other: Self) -> Self {
        Self {
            mismatched_tokens: self.mismatched_tokens + other.mismatched_tokens,
            edit_distance: self.edit_distance + other.edit_distance,
            length_delta: self.length_delta + other.length_delta,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AlignmentState {
    score: AlignmentScore,
    previous_reading_end: usize,
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

/// Whether the converted surface preserves the number of wave-dash characters
/// in the reading. U+301C (`〜`) and U+FF5E (`～`) are treated as equivalent:
/// the model may normalize one to the other, but must not add or remove one.
pub(super) fn wave_dash_count_matches(surface: &str, reading: &str) -> bool {
    fn count(text: &str) -> usize {
        text.chars().filter(|ch| matches!(ch, '〜' | '～')).count()
    }

    count(surface) == count(reading)
}

/// Whether a model candidate should survive the conservative reading check.
///
/// Wave-dash characters must be preserved by count, treating `〜` and `～` as
/// equivalent. This rejects model additions such as `だから` → `だから〜`
/// before they can reach live display or morphological segmentation.
///
/// Kana-only output can be checked exactly after width/script normalization,
/// so a free-form rewrite such as `だけど` → `だし` is rejected while
/// `ダケド` and `ﾀﾞｹﾄﾞ` remain valid. Kanji output is kept conservatively:
/// tokenizing a surface in isolation cannot disambiguate readings such as
/// `後` (`あと` / `のち`), and rejecting those would damage names and places.
pub(super) fn model_candidate_preserves_reading(surface: &str, reading: &str) -> bool {
    if surface.is_empty() || reading.is_empty() || !wave_dash_count_matches(surface, reading) {
        return false;
    }
    let normalized_surface = karukan_engine::normalize_nfkc(surface);
    let is_kana_only = normalized_surface.chars().all(|ch| {
        matches!(
            ch,
            '\u{3041}'..='\u{3096}'
                | '\u{309D}'..='\u{309F}'
                | '\u{30A1}'..='\u{30FA}'
                | '\u{30FC}'
                | '\u{30FD}'..='\u{30FF}'
        )
    });
    if !is_kana_only {
        return true;
    }

    let normalized_reading =
        karukan_engine::katakana_to_hiragana(&karukan_engine::normalize_nfkc(reading));
    karukan_engine::katakana_to_hiragana(&normalized_surface) == normalized_reading
}

fn fallback_reading(surface: &str) -> Option<String> {
    if contains_kanji(surface) {
        None
    } else {
        Some(karukan_engine::katakana_to_hiragana(surface))
    }
}

fn edit_distance(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (left_index, left_char) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            current[right_index + 1] = if left_char == right_char {
                previous[right_index]
            } else {
                1 + previous[right_index]
                    .min(previous[right_index + 1])
                    .min(current[right_index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}

fn token_score(token: &AnalyzedToken, assigned_reading: &[char]) -> AlignmentScore {
    let Some(expected) = token.reading.as_deref() else {
        let surface_len = token.surface.chars().count();
        return AlignmentScore {
            mismatched_tokens: 1,
            edit_distance: surface_len.abs_diff(assigned_reading.len()),
            length_delta: surface_len.abs_diff(assigned_reading.len()),
        };
    };
    let distance = edit_distance(expected, assigned_reading);
    AlignmentScore {
        mismatched_tokens: usize::from(distance != 0),
        edit_distance: distance,
        length_delta: expected.len().abs_diff(assigned_reading.len()),
    }
}

/// Assign the original reading across the surface tokens.
///
/// The dynamic program first minimizes the number of mismatched tokens. This
/// keeps exact words on both sides fixed and makes the smallest possible local
/// region absorb a model/dictionary reading discrepancy. Edit distance and
/// length difference resolve ties inside that region. A surface token that has
/// no counterpart in the input may receive an empty range; it is merged into
/// its neighboring correction segment afterwards.
fn align_to_original_reading(
    tokens: &[AnalyzedToken],
    reading_chars: &[char],
) -> Option<Vec<Range<usize>>> {
    if tokens.is_empty() || reading_chars.is_empty() {
        return None;
    }

    let mut states = vec![vec![None::<AlignmentState>; reading_chars.len() + 1]; tokens.len() + 1];
    states[0][0] = Some(AlignmentState {
        score: AlignmentScore::default(),
        previous_reading_end: 0,
    });

    for token_index in 0..tokens.len() {
        for reading_start in 0..=reading_chars.len() {
            let Some(previous) = states[token_index][reading_start] else {
                continue;
            };
            for reading_end in reading_start..=reading_chars.len() {
                let score = previous.score.add(token_score(
                    &tokens[token_index],
                    &reading_chars[reading_start..reading_end],
                ));
                let next = &mut states[token_index + 1][reading_end];
                if next.is_none_or(|current| score < current.score) {
                    *next = Some(AlignmentState {
                        score,
                        previous_reading_end: reading_start,
                    });
                }
            }
        }
    }

    let mut reading_end = reading_chars.len();
    let mut ranges = vec![0..0; tokens.len()];
    for token_index in (0..tokens.len()).rev() {
        let state = states[token_index + 1][reading_end]?;
        ranges[token_index] = state.previous_reading_end..reading_end;
        reading_end = state.previous_reading_end;
    }
    (reading_end == 0).then_some(ranges)
}

/// Split the first (live) conversion surface and align every token reading to
/// the original hiragana buffer.
///
/// IPADIC's eighth detail field is the dictionary reading. Kana, punctuation,
/// spaces, and Latin text can use their surface as a lossless fallback.
/// Unknown kanji and reading mismatches are assigned only the local span left
/// between exact neighboring tokens.
pub(super) fn segment_live_surface(surface: &str, reading: &str) -> Option<Vec<SurfaceSegment>> {
    if surface.is_empty() || reading.is_empty() {
        return None;
    }

    let tokenizer = tokenizer()?;
    let mut tokens = tokenizer.tokenize(surface).ok()?;
    if tokens.is_empty() {
        return None;
    }

    let mut analyzed = Vec::with_capacity(tokens.len());
    for token in &mut tokens {
        let surface = token.text.to_string();
        let dictionary_reading = token
            .get_detail(7)
            .filter(|value| !value.is_empty() && *value != "*" && *value != "UNK")
            .map(karukan_engine::katakana_to_hiragana);
        analyzed.push(AnalyzedToken {
            reading: dictionary_reading
                .or_else(|| fallback_reading(&surface))
                .map(|reading| reading.chars().collect()),
            surface,
        });
    }

    let reading_chars: Vec<char> = reading.chars().collect();
    let ranges = align_to_original_reading(&analyzed, &reading_chars)?;
    let mut segments = Vec::<SurfaceSegment>::with_capacity(analyzed.len());
    let mut leading_surface = String::new();
    for (token, reading_range) in analyzed.into_iter().zip(ranges) {
        if reading_range.is_empty() {
            if let Some(previous) = segments.last_mut() {
                previous.surface.push_str(&token.surface);
            } else {
                leading_surface.push_str(&token.surface);
            }
            continue;
        }

        let token_reading: String = reading_chars[reading_range.clone()].iter().collect();
        let surface = if leading_surface.is_empty() {
            token.surface
        } else {
            leading_surface.push_str(&token.surface);
            std::mem::take(&mut leading_surface)
        };
        segments.push(SurfaceSegment {
            reading_range,
            reading: token_reading,
            surface,
        });
    }
    (!segments.is_empty()).then_some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_kana_rewrite_that_changes_the_reading() {
        assert!(!model_candidate_preserves_reading("だし", "だけど"));
    }

    #[test]
    fn accepts_matching_kana_across_width_and_script_variants() {
        assert!(model_candidate_preserves_reading("だけど", "だけど"));
        assert!(model_candidate_preserves_reading("ダケド", "だけど"));
        assert!(model_candidate_preserves_reading("ﾀﾞｹﾄﾞ", "だけど"));
    }

    #[test]
    fn rejects_model_candidates_that_add_or_remove_a_wave_dash() {
        assert!(!model_candidate_preserves_reading("だから〜", "だから"));
        assert!(!model_candidate_preserves_reading("だから～", "だから"));
        assert!(!model_candidate_preserves_reading("だから", "だから〜"));
        assert!(!model_candidate_preserves_reading("だから〜〜", "だから～"));
    }

    #[test]
    fn treats_both_wave_dash_forms_as_equivalent_by_count() {
        assert!(model_candidate_preserves_reading("だから～", "だから〜"));
        assert!(model_candidate_preserves_reading("だから〜", "だから～"));
    }

    #[test]
    fn keeps_kanji_candidates_when_surface_reading_is_ambiguous() {
        assert!(model_candidate_preserves_reading("後", "あと"));
        assert!(model_candidate_preserves_reading("後", "のち"));
    }

    #[test]
    fn keeps_non_kana_candidates_that_need_a_richer_reading_check() {
        assert!(model_candidate_preserves_reading("1つ", "ひとつ"));
        assert!(model_candidate_preserves_reading("後だし", "あと"));
    }

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
    fn assigns_only_the_mismatched_token_an_approximate_reading() {
        let segments = segment_live_surface("東京ミステリー駅", "とうきょうなぞえき").unwrap();

        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.surface.as_str(), segment.reading.as_str()))
                .collect::<Vec<_>>(),
            [
                ("東京", "とうきょう"),
                ("ミステリー", "なぞ"),
                ("駅", "えき")
            ]
        );
    }

    #[test]
    fn distributes_a_mismatched_region_without_moving_an_exact_suffix() {
        let segments = segment_live_surface("東京都駅", "とうきょうえき").unwrap();

        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.surface.as_str())
                .collect::<String>(),
            "東京都駅"
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| segment.reading.as_str())
                .collect::<String>(),
            "とうきょうえき"
        );
        assert_eq!(segments.last().unwrap().surface, "駅");
        assert_eq!(segments.last().unwrap().reading, "えき");
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
