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
}

impl ConversionSegment {
    pub fn selected_text(&self) -> &str {
        self.candidates.selected_text().unwrap_or(&self.reading)
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
    preedit: Preedit,
}

impl ConversionSession {
    pub fn single(reading: String, candidates: CandidateList) -> Self {
        let reading_len = reading.chars().count();
        let selected = candidates.selected_text().unwrap_or(&reading).to_string();
        let selected_len = selected.chars().count();
        Self {
            reading: reading.clone(),
            segments: vec![ConversionSegment {
                reading_range: 0..reading_len,
                reading,
                candidates,
            }],
            active_segment: 0,
            preedit: Preedit::from_segments(
                vec![PreeditSegment::highlighted(selected)],
                selected_len,
            ),
        }
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

    pub fn selected_text(&self) -> String {
        self.segments
            .iter()
            .map(ConversionSegment::selected_text)
            .collect()
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
