//! Partial alignment between a whole converted surface and reading segments.
//!
//! Exact candidate matches are retained as independent correction segments.
//! Consecutive regions that cannot be proven are kept as one coarse span, so
//! preserving the visible text never requires character-count guessing.

use std::collections::HashMap;
use std::ops::Range;

use crate::core::candidate::CandidateList;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PartialSurfaceAlignment {
    Exact {
        segment_index: usize,
        candidate_index: usize,
        surface_range: Range<usize>,
    },
    Unmatched {
        segment_range: Range<usize>,
        surface_range: Range<usize>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PartialAlignmentScore {
    exact_segments: usize,
    exact_surface_chars: usize,
    unmatched_groups: usize,
}

impl PartialAlignmentScore {
    fn with_exact(self, surface_chars: usize) -> Self {
        Self {
            exact_segments: self.exact_segments + 1,
            exact_surface_chars: self.exact_surface_chars + surface_chars,
            ..self
        }
    }

    fn with_unmatched(self) -> Self {
        Self {
            unmatched_groups: self.unmatched_groups + 1,
            ..self
        }
    }

    fn is_better_than(self, other: Self) -> bool {
        (
            self.exact_segments,
            self.exact_surface_chars,
            usize::MAX - self.unmatched_groups,
        ) > (
            other.exact_segments,
            other.exact_surface_chars,
            usize::MAX - other.unmatched_groups,
        )
    }
}

#[derive(Debug, Clone)]
struct PartialAlignmentResult {
    score: PartialAlignmentScore,
    pieces: Vec<PartialSurfaceAlignment>,
}

/// Align as much of a whole surface as possible to independently generated
/// segment candidates. Consecutive regions that cannot be proven by an exact
/// candidate match stay merged into one unmatched span.
///
/// This deliberately avoids character-count splitting: a span such as
/// `のきゃく` may remain coarse, but `客` is never assigned to `の` merely
/// because their character counts happen to fit.
pub(super) fn partially_align_surface_to_candidates(
    surface: &str,
    candidate_lists: &[CandidateList],
) -> Option<Vec<PartialSurfaceAlignment>> {
    fn visit(
        segment_index: usize,
        surface_index: usize,
        surface_chars: &[char],
        candidate_chars: &[Vec<Vec<char>>],
        memo: &mut HashMap<(usize, usize), Option<PartialAlignmentResult>>,
    ) -> Option<PartialAlignmentResult> {
        if segment_index == candidate_chars.len() && surface_index == surface_chars.len() {
            return Some(PartialAlignmentResult {
                score: PartialAlignmentScore::default(),
                pieces: Vec::new(),
            });
        }
        if segment_index == candidate_chars.len() || surface_index == surface_chars.len() {
            return None;
        }
        if let Some(cached) = memo.get(&(segment_index, surface_index)) {
            return cached.clone();
        }

        let mut best: Option<PartialAlignmentResult> = None;
        for (candidate_index, candidate) in candidate_chars[segment_index].iter().enumerate() {
            if candidate.is_empty()
                || !surface_chars[surface_index..].starts_with(candidate.as_slice())
            {
                continue;
            }
            let surface_end = surface_index + candidate.len();
            if let Some(mut suffix) = visit(
                segment_index + 1,
                surface_end,
                surface_chars,
                candidate_chars,
                memo,
            ) {
                suffix.score = suffix.score.with_exact(candidate.len());
                suffix.pieces.insert(
                    0,
                    PartialSurfaceAlignment::Exact {
                        segment_index,
                        candidate_index,
                        surface_range: surface_index..surface_end,
                    },
                );
                if best
                    .as_ref()
                    .is_none_or(|current| suffix.score.is_better_than(current.score))
                {
                    best = Some(suffix);
                }
            }
        }

        // One unmatched edge may consume any non-empty consecutive range on
        // both sides. Because the score minimizes unmatched groups after
        // maximizing exact matches, adjacent ambiguity remains one coarse
        // segment instead of being split by character count.
        for segment_end in segment_index + 1..=candidate_chars.len() {
            for surface_end in surface_index + 1..=surface_chars.len() {
                if let Some(mut suffix) = visit(
                    segment_end,
                    surface_end,
                    surface_chars,
                    candidate_chars,
                    memo,
                ) {
                    suffix.score = suffix.score.with_unmatched();
                    suffix.pieces.insert(
                        0,
                        PartialSurfaceAlignment::Unmatched {
                            segment_range: segment_index..segment_end,
                            surface_range: surface_index..surface_end,
                        },
                    );
                    if best
                        .as_ref()
                        .is_none_or(|current| suffix.score.is_better_than(current.score))
                    {
                        best = Some(suffix);
                    }
                }
            }
        }

        memo.insert((segment_index, surface_index), best.clone());
        best
    }

    if surface.is_empty() || candidate_lists.is_empty() {
        return None;
    }
    let surface_chars: Vec<char> = surface.chars().collect();
    let candidate_chars: Vec<Vec<Vec<char>>> = candidate_lists
        .iter()
        .map(|candidates| {
            candidates
                .candidates()
                .iter()
                .map(|candidate| candidate.text.chars().collect())
                .collect()
        })
        .collect();
    visit(0, 0, &surface_chars, &candidate_chars, &mut HashMap::new()).map(|result| result.pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_exact_neighbors_and_merges_only_the_unknown_middle() {
        let candidates = [
            CandidateList::from_strings(["東京"]),
            CandidateList::from_strings(["なぞ", "ナゾ"]),
            CandidateList::from_strings(["駅"]),
        ];

        let result =
            partially_align_surface_to_candidates("東京ミステリー駅", &candidates).unwrap();

        assert_eq!(
            result,
            vec![
                PartialSurfaceAlignment::Exact {
                    segment_index: 0,
                    candidate_index: 0,
                    surface_range: 0..2,
                },
                PartialSurfaceAlignment::Unmatched {
                    segment_range: 1..2,
                    surface_range: 2..7,
                },
                PartialSurfaceAlignment::Exact {
                    segment_index: 2,
                    candidate_index: 0,
                    surface_range: 7..8,
                },
            ]
        );
    }

    #[test]
    fn does_not_split_an_entirely_unmatched_surface_by_character_count() {
        let candidates = [
            CandidateList::from_strings(["の", "野"]),
            CandidateList::from_strings(["客"]),
        ];

        let result = partially_align_surface_to_candidates("別表記", &candidates).unwrap();

        assert_eq!(
            result,
            vec![PartialSurfaceAlignment::Unmatched {
                segment_range: 0..2,
                surface_range: 0..3,
            }]
        );
    }
}
