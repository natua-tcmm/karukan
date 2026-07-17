//! Bounded splitting and beam composition for long explicit conversions.

use std::collections::HashSet;

pub(super) const MAX_SEGMENT_CANDIDATES: usize = 5;
pub(super) const MAX_SEARCH_STATES: usize = 20;
pub(super) const MAX_FINAL_CANDIDATES: usize = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConversionSpan {
    pub text: String,
    pub passthrough: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RankedText {
    pub text: String,
    /// Lower is better.
    pub cost: f32,
}

fn is_punctuation(ch: char) -> bool {
    ch.is_ascii_punctuation()
        || matches!(
            ch,
            '、' | '。'
                | '，'
                | '．'
                | '！'
                | '？'
                | '：'
                | '；'
                | '…'
                | '‥'
                | '・'
                | '「'
                | '」'
                | '『'
                | '』'
                | '【'
                | '】'
                | '（'
                | '）'
                | '［'
                | '］'
                | '｛'
                | '｝'
        )
}

fn slice_chars(chars: &[char], start: usize, end: usize) -> String {
    chars[start..end].iter().collect()
}

/// Split punctuation first, then prefer dictionary-lattice boundaries, and
/// finally apply a hard maximum length.
pub(super) fn split_conversion_reading(
    reading: &str,
    max_chars: usize,
    lattice_boundaries: &[usize],
) -> Vec<ConversionSpan> {
    let chars: Vec<char> = reading.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let max_chars = max_chars.max(1);
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < chars.len() {
        if is_punctuation(chars[cursor]) {
            let start = cursor;
            cursor += 1;
            while cursor < chars.len() && is_punctuation(chars[cursor]) {
                cursor += 1;
            }
            spans.push(ConversionSpan {
                text: slice_chars(&chars, start, cursor),
                passthrough: true,
            });
            continue;
        }

        let run_start = cursor;
        while cursor < chars.len() && !is_punctuation(chars[cursor]) {
            cursor += 1;
        }
        let run_end = cursor;
        let mut part_start = run_start;
        while run_end - part_start > max_chars {
            let hard_end = part_start + max_chars;
            let preferred = lattice_boundaries
                .iter()
                .copied()
                .filter(|boundary| *boundary > part_start && *boundary <= hard_end)
                .max()
                .unwrap_or(hard_end);
            spans.push(ConversionSpan {
                text: slice_chars(&chars, part_start, preferred),
                passthrough: false,
            });
            part_start = preferred;
        }
        if part_start < run_end {
            spans.push(ConversionSpan {
                text: slice_chars(&chars, part_start, run_end),
                passthrough: false,
            });
        }
    }
    spans
}

/// Concatenate per-segment alternatives while retaining only the best bounded
/// search states. Identical complete text is deduplicated by best cost.
pub(super) fn combine_segment_options(
    segments: &[Vec<RankedText>],
    max_states: usize,
    max_final: usize,
) -> Vec<RankedText> {
    if segments.is_empty() || max_states == 0 || max_final == 0 {
        return Vec::new();
    }
    let mut states = vec![RankedText {
        text: String::new(),
        cost: 0.0,
    }];
    for segment in segments {
        let mut next = Vec::new();
        for state in &states {
            for option in segment {
                next.push(RankedText {
                    text: format!("{}{}", state.text, option.text),
                    cost: state.cost + option.cost,
                });
            }
        }
        next.sort_by(|left, right| left.cost.total_cmp(&right.cost));
        let mut seen = HashSet::new();
        next.retain(|candidate| seen.insert(candidate.text.clone()));
        next.truncate(max_states);
        states = next;
        if states.is_empty() {
            break;
        }
    }
    states.truncate(max_final);
    states
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_order_is_punctuation_lattice_then_hard_limit() {
        let spans = split_conversion_reading("あいうえお。かきくけこさし", 4, &[3, 5, 8, 10, 12]);
        assert_eq!(
            spans,
            vec![
                ConversionSpan {
                    text: "あいう".into(),
                    passthrough: false,
                },
                ConversionSpan {
                    text: "えお".into(),
                    passthrough: false,
                },
                ConversionSpan {
                    text: "。".into(),
                    passthrough: true,
                },
                ConversionSpan {
                    text: "かきくけ".into(),
                    passthrough: false,
                },
                ConversionSpan {
                    text: "こさし".into(),
                    passthrough: false,
                },
            ]
        );
    }

    #[test]
    fn segment_beam_is_bounded_and_sorted() {
        let segments = vec![
            (0..5)
                .map(|index| RankedText {
                    text: format!("A{index}"),
                    cost: index as f32,
                })
                .collect(),
            (0..5)
                .map(|index| RankedText {
                    text: format!("B{index}"),
                    cost: index as f32,
                })
                .collect(),
            (0..5)
                .map(|index| RankedText {
                    text: format!("C{index}"),
                    cost: index as f32,
                })
                .collect(),
        ];
        let combined = combine_segment_options(&segments, 20, 9);
        assert_eq!(combined.len(), 9);
        assert_eq!(combined[0].text, "A0B0C0");
        assert!(combined.windows(2).all(|pair| pair[0].cost <= pair[1].cost));
    }
}
