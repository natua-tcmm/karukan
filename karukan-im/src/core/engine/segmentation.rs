//! Speculative morphology, independent of the model-inference thread.

use std::sync::mpsc::{self, Receiver, Sender};

use super::morphology::{SurfaceSegment, segment_live_surface};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub generation: u64,
    pub reading: String,
    pub surface: String,
}

pub(super) struct PreparedSegments {
    pub snapshot: Snapshot,
    pub segments: Vec<SurfaceSegment>,
}

pub(super) struct SegmentationWorker {
    requests: Option<Sender<Snapshot>>,
    results: Receiver<PreparedSegments>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SegmentationWorker {
    pub fn new() -> Self {
        let (requests, incoming) = mpsc::channel::<Snapshot>();
        let (outgoing, results) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("karukan-morphology".into())
            .spawn(move || {
                while let Ok(mut snapshot) = incoming.recv() {
                    // Only analyze the most recent snapshot still waiting.
                    while let Ok(newer) = incoming.try_recv() {
                        snapshot = newer;
                    }
                    let segments = segment_live_surface(&snapshot.surface, &snapshot.reading)
                        .unwrap_or_else(|| {
                            vec![SurfaceSegment {
                                reading_range: 0..snapshot.reading.chars().count(),
                                reading: snapshot.reading.clone(),
                                surface: snapshot.surface.clone(),
                            }]
                        });
                    if outgoing
                        .send(PreparedSegments { snapshot, segments })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("failed to spawn morphology worker");
        Self {
            requests: Some(requests),
            results,
            thread: Some(thread),
        }
    }

    pub fn submit(&self, snapshot: Snapshot) {
        if let Some(requests) = &self.requests {
            let _ = requests.send(snapshot);
        }
    }

    #[cfg(test)]
    pub fn controlled() -> (Self, Receiver<Snapshot>, Sender<PreparedSegments>) {
        let (requests, incoming) = mpsc::channel();
        let (outgoing, results) = mpsc::channel();
        (
            Self {
                requests: Some(requests),
                results,
                thread: None,
            },
            incoming,
            outgoing,
        )
    }

    pub fn poll(&self) -> Option<PreparedSegments> {
        let mut latest = None;
        while let Ok(result) = self.results.try_recv() {
            latest = Some(result);
        }
        latest
    }
}

impl Drop for SegmentationWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub(super) struct SegmentationPreparation {
    pub worker: Option<SegmentationWorker>,
    pub generation: u64,
    pub requested: Option<Snapshot>,
    pub prepared: Option<Vec<SurfaceSegment>>,
    pub entering: bool,
    pub live_candidates: Option<(String, crate::core::candidate::CandidateList)>,
}

impl SegmentationPreparation {
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.requested = None;
        self.prepared = None;
        self.entering = false;
        self.live_candidates = None;
    }
}
