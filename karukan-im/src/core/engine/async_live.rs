//! Background model-inference worker for live conversion.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use super::ComposingChunk;
use super::chunk::{ChunkPlan, assemble_chunk_candidates, group_chunks, is_japanese};
use super::morphology::model_candidate_preserves_reading;
use super::reading_correction::{interleave_model_candidates, zu_du_reading_variants};
use karukan_engine::{KanaKanjiConverter, ModelCandidate};

#[derive(Debug, Clone)]
pub(super) struct LiveInferenceRequest {
    pub revision: u64,
    pub reading: String,
    pub cursor_pos: usize,
    pub base_context: String,
    pub max_context_len: usize,
    pub chunk_len: usize,
    pub num_candidates: usize,
    pub old_chunks: Vec<ComposingChunk>,
}

#[derive(Debug)]
pub(super) struct LiveInferenceResult {
    pub revision: u64,
    pub reading: String,
    pub chunks: Vec<ComposingChunk>,
    pub candidates: Option<Vec<String>>,
    pub conversion_ms: u64,
}

enum BlockingJob {
    Convert {
        reading: String,
        context: String,
        num_candidates: usize,
        response: Sender<Vec<ModelCandidate>>,
    },
}

#[derive(Default)]
struct WorkerState {
    pending_live: Option<LiveInferenceRequest>,
    blocking: VecDeque<BlockingJob>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<WorkerState>,
    wake: Condvar,
}

/// Owns the single model instance and serializes live and explicit inference.
pub(in crate::core) struct InferenceWorker {
    shared: Arc<Shared>,
    results: Receiver<LiveInferenceResult>,
    thread: Option<JoinHandle<()>>,
    model_name: String,
}

impl InferenceWorker {
    pub(super) fn new(converter: KanaKanjiConverter) -> Self {
        let model_name = converter.model_display_name().to_string();
        let shared = Arc::new(Shared {
            state: Mutex::new(WorkerState::default()),
            wake: Condvar::new(),
        });
        let (result_tx, results) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("karukan-inference".to_string())
            .spawn(move || worker_loop(converter, worker_shared, result_tx))
            .expect("failed to spawn inference worker");
        Self {
            shared,
            results,
            thread: Some(thread),
            model_name,
        }
    }

    pub(super) fn model_display_name(&self) -> &str {
        &self.model_name
    }

    /// Replace the pending live request. Requests are intentionally not queued.
    pub(super) fn submit_live(&self, request: LiveInferenceRequest) {
        let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending_live = Some(request);
        self.shared.wake.notify_one();
    }

    /// Run a user-requested conversion with priority over pending live work.
    pub(super) fn convert_blocking(
        &self,
        reading: &str,
        context: &str,
        num_candidates: usize,
    ) -> Vec<ModelCandidate> {
        let (response, receiver) = mpsc::channel();
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.pending_live = None;
            state.blocking.push_back(BlockingJob::Convert {
                reading: reading.to_string(),
                context: context.to_string(),
                num_candidates,
                response,
            });
            self.shared.wake.notify_one();
        }
        receiver.recv().unwrap_or_default()
    }

    /// Drain completed results and return only the newest one.
    pub(super) fn poll_latest(&self) -> Option<LiveInferenceResult> {
        let mut latest = None;
        while let Ok(result) = self.results.try_recv() {
            latest = Some(result);
        }
        latest
    }
}

impl Drop for InferenceWorker {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            state.shutdown = true;
            self.shared.wake.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn worker_loop(
    converter: KanaKanjiConverter,
    shared: Arc<Shared>,
    result_tx: Sender<LiveInferenceResult>,
) {
    loop {
        let job = {
            let mut state = shared.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(job) = state.blocking.pop_front() {
                    break WorkerJob::Blocking(job);
                }
                if let Some(request) = state.pending_live.take() {
                    break WorkerJob::Live(request);
                }
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };

        match job {
            WorkerJob::Blocking(BlockingJob::Convert {
                reading,
                context,
                num_candidates,
                response,
            }) => {
                let result = converter
                    .convert_scored(&reading, &context, num_candidates.max(1))
                    .unwrap_or_default();
                let _ = response.send(result);
            }
            WorkerJob::Live(request) => {
                let result = convert_live(&converter, request);
                // A newer input may already be pending. The UI can still apply
                // this result to the matching reading prefix while the user
                // continues typing, so do not discard it here.
                let _ = result_tx.send(result);
            }
        }
    }
}

enum WorkerJob {
    Blocking(BlockingJob),
    Live(LiveInferenceRequest),
}

fn live_candidate_texts(
    converter: &KanaKanjiConverter,
    reading: &str,
    context: &str,
    num_candidates: usize,
) -> Vec<String> {
    let groups = zu_du_reading_variants(reading)
        .into_iter()
        .map(|variant| {
            converter
                .convert_scored(&variant, context, num_candidates)
                .unwrap_or_default()
                .into_iter()
                .filter(|candidate| model_candidate_preserves_reading(&candidate.text, &variant))
                .collect()
        })
        .collect();
    let mut values: Vec<String> = interleave_model_candidates(groups, num_candidates)
        .into_iter()
        .map(|candidate| candidate.text)
        .collect();
    if values.is_empty() {
        values.push(reading.to_string());
    }
    values
}

fn convert_live(
    converter: &KanaKanjiConverter,
    request: LiveInferenceRequest,
) -> LiveInferenceResult {
    let started = Instant::now();
    let text: Vec<char> = request.reading.chars().collect();
    let mut old = request.old_chunks;
    let old_lens: Vec<usize> = old.iter().map(|c| c.reading.chars().count()).collect();
    let old_text: Vec<char> = old.iter().flat_map(|c| c.reading.chars()).collect();
    let chunk_len = request.chunk_len.max(1);
    let plan = ChunkPlan::compute(&old_lens, &old_text, &text, chunk_len);
    let mut chunks = Vec::with_capacity(old.len() + 1);
    let mut combined = String::new();

    for chunk in old.drain(..plan.lead_count) {
        combined.push_str(&chunk.converted);
        chunks.push(chunk);
    }
    let trail_start = old.len() - plan.trail_count;
    for slice in group_chunks(&text[plan.mid_start..plan.mid_end], chunk_len) {
        let reading: String = slice.iter().collect();
        let candidates = if reading.chars().next().is_some_and(is_japanese) {
            let context = truncate_tail(
                &format!("{}{}", request.base_context, combined),
                request.max_context_len,
            );
            live_candidate_texts(converter, &reading, &context, request.num_candidates.max(1))
        } else {
            vec![reading.clone()]
        };
        let converted = candidates
            .first()
            .cloned()
            .unwrap_or_else(|| reading.clone());
        combined.push_str(&converted);
        chunks.push(ComposingChunk {
            reading,
            converted,
            candidates,
        });
    }
    for chunk in old.drain(trail_start..) {
        combined.push_str(&chunk.converted);
        chunks.push(chunk);
    }

    let current_chunk = chunk_index(&chunks, request.cursor_pos);
    let mut candidates =
        assemble_chunk_candidates(&chunks, current_chunk, request.num_candidates.max(1));
    candidates.retain(|candidate| candidate != &request.reading);

    LiveInferenceResult {
        revision: request.revision,
        reading: request.reading,
        chunks,
        candidates: (!candidates.is_empty()).then_some(candidates),
        conversion_ms: started.elapsed().as_millis() as u64,
    }
}

fn truncate_tail(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max_chars)).collect()
}

fn chunk_index(chunks: &[ComposingChunk], cursor_pos: usize) -> usize {
    let pos = cursor_pos.saturating_sub(1);
    let mut end = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        end += chunk.reading.chars().count();
        if pos < end {
            return index;
        }
    }
    chunks.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_candidate(text: &str) -> ModelCandidate {
        ModelCandidate {
            text: text.to_string(),
            score: None,
        }
    }

    #[test]
    fn invalid_top_kana_rewrite_is_removed_from_live_candidates() {
        let candidates = validated_live_candidate_texts_for_test(
            "だけど",
            vec![
                model_candidate("だし"),
                model_candidate("ダケド"),
                model_candidate("だけど"),
            ],
        );

        assert_eq!(candidates, ["ダケド", "だけど"]);
    }

    #[test]
    fn raw_reading_is_used_when_every_live_model_candidate_is_invalid() {
        let candidates = validated_live_candidate_texts_for_test(
            "だけど",
            vec![model_candidate("だし"), model_candidate("なので")],
        );

        assert_eq!(candidates, ["だけど"]);
    }

    #[test]
    fn wave_dash_candidate_is_removed_before_the_next_valid_live_candidate() {
        let candidates = validated_live_candidate_texts_for_test(
            "だから",
            vec![model_candidate("だから〜"), model_candidate("ダカラ")],
        );

        assert_eq!(candidates, ["ダカラ"]);
    }

    #[test]
    fn raw_reading_is_used_when_all_live_candidates_add_a_wave_dash() {
        let candidates = validated_live_candidate_texts_for_test(
            "だから",
            vec![model_candidate("だから〜"), model_candidate("だから～")],
        );

        assert_eq!(candidates, ["だから"]);
    }

    fn validated_live_candidate_texts_for_test(
        reading: &str,
        candidates: Vec<ModelCandidate>,
    ) -> Vec<String> {
        let groups = vec![
            candidates
                .into_iter()
                .filter(|candidate| model_candidate_preserves_reading(&candidate.text, reading))
                .collect(),
        ];
        let mut values: Vec<String> = interleave_model_candidates(groups, usize::MAX)
            .into_iter()
            .map(|candidate| candidate.text)
            .collect();
        if values.is_empty() {
            values.push(reading.to_string());
        }
        values
    }
}
