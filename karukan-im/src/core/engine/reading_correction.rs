//! Conservative reading alternatives for common Japanese spelling mistakes.

use std::collections::HashSet;

use karukan_engine::ModelCandidate;

/// The original reading plus at most three `ず`/`づ` alternatives.
///
/// The original spelling always remains first. Alternatives with one changed
/// character are generated before alternatives with multiple changes so a
/// long or pathological input cannot cause unbounded model/dictionary work.
pub(super) fn zu_du_reading_variants(reading: &str) -> Vec<String> {
    const MAX_VARIANTS: usize = 4;

    let original: Vec<char> = reading.chars().collect();
    let replacements: Vec<(usize, char)> = original
        .iter()
        .enumerate()
        .filter_map(|(index, ch)| {
            let replacement = match ch {
                'ず' => 'づ',
                'づ' => 'ず',
                'ズ' => 'ヅ',
                'ヅ' => 'ズ',
                _ => return None,
            };
            Some((index, replacement))
        })
        .collect();
    let mut variants = vec![original.clone()];

    fn add_combinations(
        original: &[char],
        replacements: &[(usize, char)],
        start: usize,
        remaining: usize,
        selected: &mut Vec<usize>,
        variants: &mut Vec<Vec<char>>,
    ) {
        if variants.len() == MAX_VARIANTS {
            return;
        }
        if remaining == 0 {
            let mut variant = original.to_vec();
            for selected_index in selected.iter().copied() {
                let (char_index, replacement) = replacements[selected_index];
                variant[char_index] = replacement;
            }
            variants.push(variant);
            return;
        }
        for replacement_index in start..replacements.len() {
            if replacements.len() - replacement_index < remaining {
                break;
            }
            selected.push(replacement_index);
            add_combinations(
                original,
                replacements,
                replacement_index + 1,
                remaining - 1,
                selected,
                variants,
            );
            selected.pop();
            if variants.len() == MAX_VARIANTS {
                return;
            }
        }
    }

    for change_count in 1..=replacements.len() {
        add_combinations(
            &original,
            &replacements,
            0,
            change_count,
            &mut Vec::new(),
            &mut variants,
        );
        if variants.len() == MAX_VARIANTS {
            break;
        }
    }

    variants
        .into_iter()
        .map(|chars| chars.into_iter().collect())
        .collect()
}

/// Interleave model beams by rank while keeping the original-reading beam
/// first. This reserves visible candidate space for a corrected-reading result
/// without allowing typo correction to replace the ordinary top candidate.
pub(super) fn interleave_model_candidates(
    groups: Vec<Vec<ModelCandidate>>,
    limit: usize,
) -> Vec<ModelCandidate> {
    if limit == 0 {
        return Vec::new();
    }
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    let max_group_len = groups.iter().map(Vec::len).max().unwrap_or(0);

    for rank in 0..max_group_len {
        for group in &groups {
            let Some(candidate) = group.get(rank) else {
                continue;
            };
            if seen.insert(candidate.text.clone()) {
                merged.push(candidate.clone());
                if merged.len() == limit {
                    return merged;
                }
            }
        }
    }
    merged
}

/// Candidate annotation used only when an alternative spelling produced the
/// result. Existing dictionary descriptions are retained alongside it.
pub(super) fn correction_description(
    existing: Option<String>,
    corrected_reading: &str,
) -> Option<String> {
    let correction = format!("ず・づ補正: {corrected_reading}");
    Some(match existing {
        Some(description) => format!("{description} / {correction}"),
        None => correction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(text: &str) -> ModelCandidate {
        ModelCandidate {
            text: text.to_string(),
            score: None,
        }
    }

    #[test]
    fn original_reading_stays_first_and_single_changes_come_before_combined_change() {
        assert_eq!(
            zu_du_reading_variants("ずづズヅ"),
            ["ずづズヅ", "づづズヅ", "ずずズヅ", "ずづヅヅ"]
        );
        assert_eq!(
            zu_du_reading_variants("ずづ"),
            ["ずづ", "づづ", "ずず", "づず"]
        );
    }

    #[test]
    fn reading_without_target_kana_has_only_the_original_variant() {
        assert_eq!(zu_du_reading_variants("こんにちは"), ["こんにちは"]);
    }

    #[test]
    fn model_beams_are_interleaved_and_deduplicated() {
        let merged = interleave_model_candidates(
            vec![
                vec![model("築く"), model("きずく")],
                vec![model("気付く"), model("築く")],
            ],
            3,
        );
        let texts: Vec<_> = merged
            .iter()
            .map(|candidate| candidate.text.as_str())
            .collect();
        assert_eq!(texts, ["築く", "気付く", "きずく"]);
    }

    #[test]
    fn zero_model_candidate_limit_produces_no_candidates() {
        assert!(interleave_model_candidates(vec![vec![model("築く")]], 0).is_empty());
    }
}
