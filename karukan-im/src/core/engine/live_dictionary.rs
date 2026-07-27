//! User-dictionary spans pinned inside live conversion.

use std::cmp::Ordering;
use std::collections::HashMap;

use karukan_engine::Dictionary;

use super::reading_correction::zu_du_reading_variants;
use super::{InputMethodEngine, should_prioritize_single_hiragana};

/// One non-overlapping user-dictionary span selected for live conversion.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LiveUserDictionaryMatch {
    /// Character offsets in the original (un-corrected) reading.
    pub char_start: usize,
    pub char_end: usize,
    /// Original reading slice. This always concatenates back to the input.
    pub reading: String,
    /// Dictionary surfaces in priority order. The first is pinned in preedit.
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone)]
struct MatchOption {
    char_start: usize,
    char_end: usize,
    candidates: Vec<String>,
    score: f32,
}

#[derive(Debug, Clone, Default)]
struct Selection {
    covered_chars: usize,
    match_count: usize,
    score: f32,
    matches: Vec<MatchOption>,
}

fn selection_order(left: &Selection, right: &Selection) -> Ordering {
    left.covered_chars
        .cmp(&right.covered_chars)
        // Fewer spans win when coverage is equal, so one long registered term
        // beats several adjacent short terms.
        .then_with(|| right.match_count.cmp(&left.match_count))
        // Lower dictionary scores rank better.
        .then_with(|| right.score.total_cmp(&left.score))
}

fn update_best(slot: &mut Option<Selection>, candidate: Selection) {
    if slot
        .as_ref()
        .is_none_or(|current| selection_order(&candidate, current).is_gt())
    {
        *slot = Some(candidate);
    }
}

/// Select user-dictionary spans for a complete live-conversion reading.
///
/// Embedded one-character entries are deliberately ignored. A one-character
/// entry is still eligible when it covers the entire reading, preserving the
/// existing whole-reading exact-match behavior (the caller separately keeps
/// the single-hiragana safety rule).
pub(super) fn select_live_user_dictionary_matches(
    dictionary: Option<&Dictionary>,
    reading: &str,
) -> Vec<LiveUserDictionaryMatch> {
    let Some(dictionary) = dictionary else {
        return Vec::new();
    };
    let original_chars: Vec<char> = reading.chars().collect();
    let char_len = original_chars.len();
    if char_len == 0 {
        return Vec::new();
    }

    // The same character range may match both the original spelling and a
    // ず/づ-corrected spelling. Merge their surfaces while retaining variant
    // order, which keeps the exact spelling ahead of corrected alternatives.
    let mut by_range: HashMap<(usize, usize), MatchOption> = HashMap::new();
    for variant in zu_du_reading_variants(reading) {
        let variant_chars: Vec<char> = variant.chars().collect();
        if variant_chars.len() != char_len {
            continue;
        }
        for char_start in 0..char_len {
            let suffix: String = variant_chars[char_start..].iter().collect();
            for result in dictionary.common_prefix_search(&suffix) {
                let matched_len = result.reading.chars().count();
                let char_end = char_start + matched_len;
                if char_end > char_len
                    || (matched_len < 2 && !(char_start == 0 && char_end == char_len))
                {
                    continue;
                }
                let Some(first) = result.candidates.first() else {
                    continue;
                };
                let entry = by_range
                    .entry((char_start, char_end))
                    .or_insert_with(|| MatchOption {
                        char_start,
                        char_end,
                        candidates: Vec::new(),
                        score: first.score,
                    });
                for candidate in result.candidates {
                    if !entry.candidates.contains(&candidate.surface) {
                        entry.candidates.push(candidate.surface.clone());
                    }
                }
            }
        }
    }

    let mut starting_at = vec![Vec::<MatchOption>::new(); char_len];
    for option in by_range.into_values() {
        starting_at[option.char_start].push(option);
    }
    for options in &mut starting_at {
        options.sort_by(|left, right| {
            (right.char_end - right.char_start)
                .cmp(&(left.char_end - left.char_start))
                .then_with(|| left.score.total_cmp(&right.score))
        });
    }

    // Weighted interval scheduling over character positions. Skipping a
    // character leaves it for AI/passthrough conversion; taking an interval
    // pins that registered surface.
    let mut best = vec![None::<Selection>; char_len + 1];
    best[0] = Some(Selection::default());
    for char_index in 0..char_len {
        let Some(current) = best[char_index].clone() else {
            continue;
        };
        update_best(&mut best[char_index + 1], current.clone());
        for option in &starting_at[char_index] {
            let mut next = current.clone();
            next.covered_chars += option.char_end - option.char_start;
            next.match_count += 1;
            next.score += option.score;
            next.matches.push(option.clone());
            update_best(&mut best[option.char_end], next);
        }
    }

    best[char_len]
        .take()
        .unwrap_or_default()
        .matches
        .into_iter()
        .map(|selected| LiveUserDictionaryMatch {
            char_start: selected.char_start,
            char_end: selected.char_end,
            reading: original_chars[selected.char_start..selected.char_end]
                .iter()
                .collect(),
            candidates: selected.candidates,
        })
        .collect()
}

impl InputMethodEngine {
    /// Resolve the user-dictionary spans eligible for the current live text.
    pub(super) fn live_user_dictionary_matches(
        &self,
        reading: &str,
    ) -> Vec<LiveUserDictionaryMatch> {
        if should_prioritize_single_hiragana(self.input_mode, reading) {
            return Vec::new();
        }
        select_live_user_dictionary_matches(self.dicts.user.as_ref(), reading)
    }
}

#[cfg(test)]
mod tests {
    use karukan_engine::dictionary_source::NormalizedDictionaryEntry;
    use karukan_engine::{DictionaryCategory, DictionarySource};

    use super::*;

    fn dictionary(entries: &[(&str, &str, f32)]) -> Dictionary {
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

    #[test]
    fn maximizes_registered_character_coverage() {
        let dict = dictionary(&[("あい", "愛", 0.0), ("いうえ", "言う絵", 0.0)]);
        let matches = select_live_user_dictionary_matches(Some(&dict), "あいうえ");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].reading, "いうえ");
        assert_eq!(matches[0].candidates, ["言う絵"]);
    }

    #[test]
    fn fewer_long_spans_win_when_coverage_is_equal() {
        let dict = dictionary(&[
            ("あいうえ", "一語", 5.0),
            ("あい", "前", 0.0),
            ("うえ", "後", 0.0),
        ]);
        let matches = select_live_user_dictionary_matches(Some(&dict), "あいうえ");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].reading, "あいうえ");
        assert_eq!(matches[0].candidates, ["一語"]);
    }

    #[test]
    fn dictionary_score_breaks_equal_coverage_ties() {
        let dict = dictionary(&[("あい", "高コスト", 10.0), ("いう", "低コスト", 0.0)]);
        let matches = select_live_user_dictionary_matches(Some(&dict), "あいう");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].reading, "いう");
        assert_eq!(matches[0].candidates, ["低コスト"]);
    }

    #[test]
    fn ignores_one_character_entries_only_when_embedded() {
        let dict = dictionary(&[("あ", "亜", 0.0)]);

        assert!(select_live_user_dictionary_matches(Some(&dict), "あい").is_empty());
        assert_eq!(
            select_live_user_dictionary_matches(Some(&dict), "あ")[0].candidates,
            ["亜"]
        );
    }

    #[test]
    fn corrected_zu_du_match_keeps_original_character_range() {
        let dict = dictionary(&[("つづく", "続く", 0.0)]);
        let matches = select_live_user_dictionary_matches(Some(&dict), "つずくよ");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].char_start, 0);
        assert_eq!(matches[0].char_end, 3);
        assert_eq!(matches[0].reading, "つずく");
        assert_eq!(matches[0].candidates, ["続く"]);
    }

    #[test]
    fn keeps_alternative_surfaces_in_dictionary_order() {
        let dict = dictionary(&[("かるかん", "karukan", 0.0), ("かるかん", "軽羹", 1.0)]);
        let matches = select_live_user_dictionary_matches(Some(&dict), "かるかんを");

        assert_eq!(matches[0].candidates, ["karukan", "軽羹"]);
    }
}
