//! Reading-wide dictionary lattice and bounded K-best search.

use std::collections::HashMap;

use crate::dict::{Dictionary, DictionaryCategory, DictionarySource};

/// Which dictionary layer produced a lattice edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatticeDictionaryKind {
    User,
    System,
}

/// A dictionary participating in lattice construction.
pub struct LatticeDictionary<'a> {
    pub dictionary: &'a Dictionary,
    pub kind: LatticeDictionaryKind,
    /// Per-edge ranking adjustment. Lower scores rank better.
    pub score_bias: f32,
}

/// Bounds applied to lattice construction and K-best search.
#[derive(Debug, Clone, Copy)]
pub struct LatticeLimits {
    pub segment_candidates: usize,
    pub beam_width: usize,
    pub max_paths: usize,
    pub unknown_penalty: f32,
    pub segment_penalty: f32,
}

impl Default for LatticeLimits {
    fn default() -> Self {
        Self {
            segment_candidates: 5,
            beam_width: 20,
            max_paths: 9,
            unknown_penalty: 10_000.0,
            segment_penalty: 1.0,
        }
    }
}

/// One selected edge in a lattice path.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticeSegment {
    /// Character offsets into the original reading.
    pub char_start: usize,
    pub char_end: usize,
    pub reading: String,
    pub surface: String,
    pub score: f32,
    pub source: Option<DictionarySource>,
    pub category: DictionaryCategory,
    pub description: Option<String>,
    pub dictionary_kind: Option<LatticeDictionaryKind>,
}

/// A complete reading-to-surface path.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticePath {
    pub surface: String,
    pub score: f32,
    pub segments: Vec<LatticeSegment>,
}

impl LatticePath {
    fn empty() -> Self {
        Self {
            surface: String::new(),
            score: 0.0,
            segments: Vec::new(),
        }
    }

    pub fn has_unknown(&self) -> bool {
        self.segments.iter().any(|segment| segment.source.is_none())
    }
}

fn byte_offsets(reading: &str) -> Vec<usize> {
    reading
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(reading.len()))
        .collect()
}

fn push_bounded(paths: &mut Vec<LatticePath>, path: LatticePath, beam_width: usize) {
    if let Some(existing) = paths
        .iter_mut()
        .find(|existing| existing.surface == path.surface)
    {
        if path.score < existing.score {
            *existing = path;
        }
    } else {
        paths.push(path);
    }
    paths.sort_by(|left, right| left.score.total_cmp(&right.score));
    paths.truncate(beam_width);
}

/// Construct a lattice from every dictionary prefix at every character and
/// return bounded K-best complete paths.
pub fn search_dictionary_lattice(
    reading: &str,
    dictionaries: &[LatticeDictionary<'_>],
    limits: LatticeLimits,
) -> Vec<LatticePath> {
    if reading.is_empty() || limits.max_paths == 0 || limits.beam_width == 0 {
        return Vec::new();
    }
    let offsets = byte_offsets(reading);
    let char_len = offsets.len() - 1;
    let byte_to_char: HashMap<usize, usize> = offsets
        .iter()
        .copied()
        .enumerate()
        .map(|(char_index, byte_offset)| (byte_offset, char_index))
        .collect();
    let mut states = vec![Vec::<LatticePath>::new(); char_len + 1];
    states[0].push(LatticePath::empty());

    for char_start in 0..char_len {
        if states[char_start].is_empty() {
            continue;
        }
        let previous_paths = states[char_start].clone();
        let byte_start = offsets[char_start];
        let suffix = &reading[byte_start..];

        for layer in dictionaries {
            for matched in layer.dictionary.common_prefix_search(suffix) {
                let byte_end = byte_start + matched.reading.len();
                let Some(&char_end) = byte_to_char.get(&byte_end) else {
                    continue;
                };
                for candidate in matched.candidates.iter().take(limits.segment_candidates) {
                    let edge_score = candidate.score + layer.score_bias + limits.segment_penalty;
                    for previous in &previous_paths {
                        let mut path = previous.clone();
                        path.surface.push_str(&candidate.surface);
                        path.score += edge_score;
                        path.segments.push(LatticeSegment {
                            char_start,
                            char_end,
                            reading: matched.reading.to_string(),
                            surface: candidate.surface.clone(),
                            score: edge_score,
                            source: Some(candidate.source),
                            category: candidate.category,
                            description: candidate.description.clone(),
                            dictionary_kind: Some(layer.kind),
                        });
                        push_bounded(&mut states[char_end], path, limits.beam_width);
                    }
                }
            }
        }

        // A one-character hiragana fallback guarantees a complete path even
        // where no dictionary contains the reading.
        let char_end = char_start + 1;
        let unknown = &reading[offsets[char_start]..offsets[char_end]];
        for previous in &previous_paths {
            let mut path = previous.clone();
            path.surface.push_str(unknown);
            path.score += limits.unknown_penalty;
            path.segments.push(LatticeSegment {
                char_start,
                char_end,
                reading: unknown.to_string(),
                surface: unknown.to_string(),
                score: limits.unknown_penalty,
                source: None,
                category: DictionaryCategory::General,
                description: None,
                dictionary_kind: None,
            });
            push_bounded(&mut states[char_end], path, limits.beam_width);
        }
    }

    let mut complete = std::mem::take(&mut states[char_len]);
    complete.sort_by(|left, right| left.score.total_cmp(&right.score));
    complete.truncate(limits.max_paths);
    complete
}

#[cfg(test)]
mod tests {
    use crate::dictionary_source::NormalizedDictionaryEntry;

    use super::*;

    fn entry(reading: &str, surface: &str, score: f32) -> NormalizedDictionaryEntry {
        NormalizedDictionaryEntry::new(
            reading,
            surface,
            score,
            DictionarySource::Mozc,
            DictionaryCategory::General,
            None,
        )
        .unwrap()
    }

    #[test]
    fn builds_viterbi_and_k_best_paths_from_overlapping_entries() {
        let dictionary = Dictionary::build_from_normalized([
            entry("とうきょう", "東京", 2.0),
            entry("と", "都", 50.0),
            entry("きょう", "京", 50.0),
            entry("えき", "駅", 1.0),
            entry("えき", "驛", 20.0),
        ])
        .unwrap();
        let paths = search_dictionary_lattice(
            "とうきょうえき",
            &[LatticeDictionary {
                dictionary: &dictionary,
                kind: LatticeDictionaryKind::System,
                score_bias: 0.0,
            }],
            LatticeLimits {
                max_paths: 3,
                ..LatticeLimits::default()
            },
        );
        assert_eq!(paths[0].surface, "東京駅");
        assert_eq!(paths[0].segments.len(), 2);
        assert!(paths.iter().any(|path| path.surface == "東京驛"));
        assert!(paths.windows(2).all(|pair| pair[0].score <= pair[1].score));
    }

    #[test]
    fn combines_user_and_system_dictionary_edges() {
        let user = Dictionary::build_from_normalized([entry("あと", "あと", 0.0)]).unwrap();
        let system =
            Dictionary::build_from_normalized([entry("で", "で", 0.0), entry("す", "す", 0.0)])
                .unwrap();
        let paths = search_dictionary_lattice(
            "あとです",
            &[
                LatticeDictionary {
                    dictionary: &user,
                    kind: LatticeDictionaryKind::User,
                    score_bias: -100.0,
                },
                LatticeDictionary {
                    dictionary: &system,
                    kind: LatticeDictionaryKind::System,
                    score_bias: 0.0,
                },
            ],
            LatticeLimits::default(),
        );
        assert_eq!(paths[0].surface, "あとです");
        assert_eq!(
            paths[0].segments[0].dictionary_kind,
            Some(LatticeDictionaryKind::User)
        );
        assert!(!paths[0].has_unknown());
    }

    #[test]
    fn emits_unknown_fallback_with_character_ranges() {
        let paths = search_dictionary_lattice("かな", &[], LatticeLimits::default());
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].surface, "かな");
        assert_eq!(paths[0].segments[1].char_start, 1);
        assert_eq!(paths[0].segments[1].char_end, 2);
        assert!(paths[0].has_unknown());
    }
}
