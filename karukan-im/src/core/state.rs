//! Input state machine
//!
//! Defines the states of the IME and transitions between them.

use std::ops::Range;

use super::candidate::CandidateList;
use super::preedit::{Preedit, PreeditSegment};

/// One independently selectable span within an explicit conversion.
#[derive(Debug, Clone)]
pub struct ConversionSegment {
    /// Character range in [`ConversionSession::reading`].
    pub reading_range: Range<usize>,
    pub reading: String,
    pub candidates: CandidateList,
    /// Surface currently shown for this segment. Kept separately from the
    /// candidate cursor so rebuilding alternatives never changes the preedit.
    selected_surface: String,
    /// The reading boundary changed and alternatives must be rebuilt before
    /// the next candidate-navigation operation.
    pub candidates_dirty: bool,
    /// Whether the user explicitly changed this segment during conversion.
    pub explicitly_modified: bool,
}

impl ConversionSegment {
    pub fn new(reading_range: Range<usize>, reading: String, candidates: CandidateList) -> Self {
        let selected_surface = candidates.selected_text().unwrap_or(&reading).to_string();
        Self {
            reading_range,
            reading,
            candidates,
            selected_surface,
            candidates_dirty: false,
            explicitly_modified: false,
        }
    }

    pub fn selected_text(&self) -> &str {
        &self.selected_surface
    }

    pub fn sync_selected_surface(&mut self) {
        self.selected_surface = self
            .candidates
            .selected_text()
            .unwrap_or(&self.reading)
            .to_string();
    }
}

/// Complete state for an explicit kana-kanji conversion.
///
/// Commit 12 starts with one segment covering the whole reading. Later
/// commits split it into dictionary-lattice bunsetsu without changing the
/// outer state-machine representation.
#[derive(Debug, Clone)]
pub struct ConversionSession {
    pub reading: String,
    pub segments: Vec<ConversionSegment>,
    pub active_segment: usize,
    /// True while the user is walking the three whole-reading candidates.
    /// Once those are exhausted, conversion stays in segmented mode even if
    /// the dictionary cannot produce more than one segment.
    whole_candidate_phase: bool,
    preedit: Preedit,
}

impl ConversionSession {
    pub fn single(reading: String, candidates: CandidateList) -> Self {
        let reading_len = reading.chars().count();
        let selected = candidates.selected_text().unwrap_or(&reading).to_string();
        let selected_len = selected.chars().count();
        Self {
            reading: reading.clone(),
            segments: vec![ConversionSegment::new(0..reading_len, reading, candidates)],
            active_segment: 0,
            whole_candidate_phase: true,
            preedit: Preedit::from_segments(
                vec![PreeditSegment::highlighted(selected)],
                selected_len,
            ),
        }
    }

    pub fn segmented(reading: String, segments: Vec<ConversionSegment>) -> Self {
        let mut session = Self {
            reading,
            segments,
            active_segment: 0,
            whole_candidate_phase: false,
            preedit: Preedit::new(),
        };
        debug_assert!(session.ranges_are_valid());
        session.rebuild_preedit();
        session
    }

    pub fn preedit(&self) -> &Preedit {
        &self.preedit
    }

    pub fn preedit_mut(&mut self) -> &mut Preedit {
        &mut self.preedit
    }

    pub fn active(&self) -> Option<&ConversionSegment> {
        self.segments.get(self.active_segment)
    }

    pub fn active_mut(&mut self) -> Option<&mut ConversionSegment> {
        self.segments.get_mut(self.active_segment)
    }

    pub fn candidates(&self) -> Option<&CandidateList> {
        self.active().map(|segment| &segment.candidates)
    }

    pub fn candidates_mut(&mut self) -> Option<&mut CandidateList> {
        self.active_mut().map(|segment| &mut segment.candidates)
    }

    pub fn is_whole_candidate_phase(&self) -> bool {
        self.whole_candidate_phase
    }

    pub fn finish_whole_candidate_phase(&mut self) {
        self.whole_candidate_phase = false;
    }

    pub fn selected_text(&self) -> String {
        self.segments
            .iter()
            .map(ConversionSegment::selected_text)
            .collect()
    }

    pub fn move_active_left(&mut self) -> bool {
        if self.active_segment == 0 {
            return false;
        }
        self.active_segment -= 1;
        self.rebuild_preedit();
        true
    }

    pub fn move_active_right(&mut self) -> bool {
        if self.active_segment + 1 >= self.segments.len() {
            return false;
        }
        self.active_segment += 1;
        self.rebuild_preedit();
        true
    }

    /// Every non-empty segment covers the original reading exactly once.
    pub fn ranges_are_valid(&self) -> bool {
        if self.segments.is_empty() {
            return false;
        }
        let reading_chars: Vec<char> = self.reading.chars().collect();
        let mut expected_start = 0;
        for segment in &self.segments {
            if segment.reading_range.start != expected_start
                || segment.reading_range.start >= segment.reading_range.end
                || segment.reading_range.end > reading_chars.len()
                || segment.reading
                    != reading_chars[segment.reading_range.clone()]
                        .iter()
                        .collect::<String>()
            {
                return false;
            }
            expected_start = segment.reading_range.end;
        }
        expected_start == reading_chars.len()
    }

    pub fn rebuild_preedit(&mut self) {
        let mut position = 0;
        let mut caret = 0;
        let segments = self
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                let text = segment.selected_text().to_string();
                position += text.chars().count();
                if index == self.active_segment {
                    caret = position;
                }
                PreeditSegment::new(
                    text,
                    if index == self.active_segment {
                        super::preedit::AttributeType::Highlight
                    } else {
                        super::preedit::AttributeType::Underline
                    },
                )
            })
            .collect();
        self.preedit = Preedit::from_segments(segments, caret);
    }
}

/// The current state of the IME
#[derive(Debug, Clone, Default)]
pub enum InputState {
    /// No input, waiting for user to type
    #[default]
    Empty,

    /// Composing mode - building preedit text (hiragana, katakana, or alphabet)
    Composing {
        /// The preedit string being composed
        preedit: Preedit,
        /// Unconverted romaji buffer (e.g., "k" waiting for next char)
        romaji_buffer: String,
    },

    /// Conversion mode - selecting from candidates
    Conversion { session: ConversionSession },
}

impl InputState {
    /// Check if the engine is in the Empty (idle) state
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Get the current preedit if any
    pub fn preedit(&self) -> Option<&Preedit> {
        match self {
            Self::Empty => None,
            Self::Composing { preedit, .. } => Some(preedit),
            Self::Conversion { session } => Some(session.preedit()),
        }
    }

    /// Get mutable reference to preedit
    pub fn preedit_mut(&mut self) -> Option<&mut Preedit> {
        match self {
            Self::Empty => None,
            Self::Composing { preedit, .. } => Some(preedit),
            Self::Conversion { session } => Some(session.preedit_mut()),
        }
    }

    /// Get candidates in conversion state
    pub fn candidates(&self) -> Option<&CandidateList> {
        match self {
            Self::Conversion { session } => session.candidates(),
            _ => None,
        }
    }

    /// Get mutable reference to candidates
    pub fn candidates_mut(&mut self) -> Option<&mut CandidateList> {
        match self {
            Self::Conversion { session } => session.candidates_mut(),
            _ => None,
        }
    }
}
